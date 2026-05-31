//! 003 M15d — 1M-segment manifest stress bench.
//!
//! Validates the petabyte-scale single-node claim at the
//! manifest layer. Builds a synthetic corpus of N parts × M
//! superfiles per part (1M superfiles at the full scale: 100 ×
//! 10K) via direct manifest manipulation — the plan
//! explicitly notes the 1 TB on-disk index wouldn't fit on
//! a laptop; the manifest layer is what's measured. All
//! synthetic superfiles share ONE actual superfile on disk
//! (the dummy segment) so query phases that resolve a
//! `SuperfileUri` actually find bytes.
//!
//! ## Scale knobs
//!
//!   - Default (smoke): 10 parts × 10 superfiles = 100
//!     superfiles. Runs in seconds; useful for harness
//!     validation.
//!   - `INFINO_BENCH_M15D_MEDIUM=1`: 100 × 100 = 10K
//!     superfiles.
//!   - `INFINO_BENCH_M15D_FULL=1`: 100 × 10K = 1M
//!     superfiles. The plan's petabyte gate.
//!
//! ## Phases (per the M15 plan spec)
//!
//!   - (a) **cold open**: time `Supertable::open` against
//!     the persisted N-part manifest. Asserts < 5 s at
//!     full scale; smaller scales should be << 1 s.
//!   - (b) **single-partition commit**: writer appends 1
//!     segment; M15a rewrites exactly one part. Reports
//!     wall time.
//!   - (c) **high-selectivity query**: BM25 search for a
//!     term that hits exactly one part's `term_bloom_union`.
//!     M15c list-prune routes the query to that part;
//!     n_parts_loaded grows by 1. Reports wall time.
//!   - (d) **low-selectivity query**: BM25 for a term in
//!     every part. Loads all parts. Reports wall time.
//!   - (e) **refresh**: a sibling handle commits 1
//!     segment; refresh on the consumer loads exactly 1
//!     new part (inherit-via-content-hash for the other
//!     99). Reports wall time.

#![deny(clippy::unwrap_used)]

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use arrow_array::{Decimal128Array, LargeStringArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use bytes::Bytes;

use infino::superfile::builder::{BuilderOptions, FtsConfig, SuperfileBuilder};
use infino::superfile::fts::reader::BoolMode;
use infino::superfile::fts::tokenize::Tokenizer;
use infino::supertable::manifest::bloom::BloomBuilder;
use infino::supertable::manifest::commit::{self as commit_mod};
use infino::supertable::manifest::list::{
    FORMAT_VERSION as LIST_FORMAT_VERSION, FtsColumnInfo, ManifestList, ManifestListEntry,
    PartitionStrategy,
};
use infino::supertable::manifest::part::{self as part_mod, ContentHash, ManifestPart, PartId};
use infino::supertable::reader_cache::{ColdFetchMode, DiskCacheConfig, DiskCacheStore, LruPolicy};
use infino::supertable::storage::{LocalFsStorageProvider, StorageProvider};
use infino::supertable::{
    FtsSummary, ScalarStatsTable, SuperfileEntry, SuperfileUri, Supertable, SupertableOptions,
};
use infino::test_helpers::default_tokenizer;
use tempfile::TempDir;
use uuid::Uuid;

const COMMON_TERM: &str = "common";
const SHARED_SEGMENT_TITLE: &str = "common term in the dummy segment";

fn resolve_scale() -> (usize, usize) {
    if std::env::var("INFINO_BENCH_M15D_FULL").is_ok() {
        (100, 10_000)
    } else if std::env::var("INFINO_BENCH_M15D_MEDIUM").is_ok() {
        (100, 100)
    } else {
        (10, 10)
    }
}

fn unique_term_for(part_idx: usize) -> String {
    format!("uniqp{part_idx:04}")
}

fn schema_id_title() -> Arc<Schema> {
    Arc::new(Schema::new(vec![Field::new(
        "title",
        DataType::LargeUtf8,
        false,
    )]))
}

fn make_options(storage: Arc<dyn StorageProvider>) -> SupertableOptions {
    let tk: Arc<dyn Tokenizer> = default_tokenizer();
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("pool"),
    );
    SupertableOptions::new(
        schema_id_title(),
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![],
        Some(tk),
    )
    .expect("opts")
    .with_writer_pool(pool)
    .with_storage(storage)
}

fn make_cache(storage: Arc<dyn StorageProvider>, cache_root: &Path) -> Arc<DiskCacheStore> {
    let cfg = DiskCacheConfig {
        cache_root: cache_root.to_path_buf(),
        disk_budget_bytes: 1 << 30,
        cold_fetch_mode: ColdFetchMode::HybridWithPrefetch,
        cold_fetch_streams: 4,
        cold_fetch_chunk_bytes: 1 << 20,
        mmap_cold_threshold_secs: 0,
        mmap_sweep_interval_secs: 0,
        eviction: Box::new(LruPolicy::new()),
        verify_crc_on_open: true,
        prefetch_concurrency: 8,
    };
    let pinned: Arc<dyn Fn() -> HashSet<_> + Send + Sync> = Arc::new(HashSet::new);
    DiskCacheStore::new(storage, cfg, pinned).expect("cache")
}

/// Build a minimal valid superfile bytes blob containing
/// the COMMON_TERM. Same blob is reused for every synthetic
/// SuperfileEntry's URI (which is shared); the segment-level
/// per-row scans during phase (c)/(d) all hit this one
/// file.
fn build_shared_segment_bytes() -> Bytes {
    // SuperfileBuilder takes the effective schema (with the
    // id column included). The supertable layer's
    // `effective_schema()` would prepend `_id: Decimal128(38, 0)`
    // automatically at commit time — we mirror that by hand
    // here because this is the direct-builder path.
    let effective_schema = Arc::new(Schema::new(vec![
        Field::new("_id", DataType::Decimal128(38, 0), false),
        Field::new("title", DataType::LargeUtf8, false),
    ]));
    let tk: Arc<dyn Tokenizer> = default_tokenizer();
    let opts = BuilderOptions::new(
        effective_schema.clone(),
        "_id",
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![],
        Some(tk),
    );
    let mut b = SuperfileBuilder::new(opts).expect("builder");
    let ids = Decimal128Array::from(vec![0i128, 1])
        .with_precision_and_scale(38, 0)
        .expect("decimal128");
    let batch = RecordBatch::try_new(
        effective_schema,
        vec![
            Arc::new(ids),
            Arc::new(LargeStringArray::from(vec![
                SHARED_SEGMENT_TITLE,
                SHARED_SEGMENT_TITLE,
            ])),
        ],
    )
    .expect("batch");
    b.add_batch(&batch, &[]).expect("add_batch");
    let bytes = b.finish().expect("finish");
    bytes.into()
}

/// Build N parts × M superfiles per part. Returns a list of
/// `(ManifestPart, ManifestListEntry)` ready to PUT.
///
/// Per-part design:
///   - One shared Arc-backed `Bloom` containing COMMON_TERM
///     + a part-unique term (`uniqpNNNN`).
///   - All M superfiles in the part share that Bloom via
///     `Arc::clone` — total memory is one Bloom (~64 KiB)
///     per part, not per segment.
///   - Each segment carries unique `id_min`/`id_max`
///     (sequential across the full corpus) + the shared
///     URI of the dummy segment file on disk.
fn build_synthetic_parts(
    n_parts: usize,
    n_segments_per_part: usize,
    shared_uri: SuperfileUri,
) -> Vec<(ManifestPart, ManifestListEntry)> {
    let mut out = Vec::with_capacity(n_parts);
    let mut doc_cursor: u64 = 0;
    for part_idx in 0..n_parts {
        // Per-part Bloom: COMMON_TERM + unique term.
        let part_unique = unique_term_for(part_idx);
        let mut bb = BloomBuilder::new();
        bb.insert(COMMON_TERM.as_bytes());
        bb.insert(part_unique.as_bytes());
        let shared_bloom = bb.finish();

        let summary = FtsSummary {
            term_bloom: shared_bloom.clone(),
            n_terms_distinct: 2,
            term_range: (
                COMMON_TERM.as_bytes().to_vec(),
                part_unique.as_bytes().to_vec(),
            ),
        };

        let mut superfiles: Vec<Arc<SuperfileEntry>> = Vec::with_capacity(n_segments_per_part);
        for _ in 0..n_segments_per_part {
            let id_min = doc_cursor as i128;
            let id_max = doc_cursor as i128; // 1-doc superfiles
            doc_cursor += 1;
            let mut fts_summary: HashMap<String, FtsSummary> = HashMap::new();
            // Clone the FtsSummary; the Bloom inside is
            // Arc-backed so this is a refcount bump.
            fts_summary.insert("title".to_string(), summary.clone());
            superfiles.push(Arc::new(SuperfileEntry {
                superfile_id: Uuid::new_v4(),
                uri: shared_uri,
                n_docs: 1,
                id_min,
                id_max,
                scalar_stats: ScalarStatsTable::new(),
                fts_summary,
                vector_summary: HashMap::new(),
                partition_key: 0u32.to_le_bytes().to_vec(),
                partition_hint: Some(0),
                subsection_offsets: None,
            }));
        }

        let part = ManifestPart {
            format_version: part_mod::FORMAT_VERSION.into(),
            part_id: PartId::new_v4(),
            superfiles,
        };
        let compressed = part_mod::encode(&part, 3);
        let size_compressed = compressed.len() as u64;
        let content_hash = ContentHash::of(&compressed);
        let size_uncompressed = zstd::stream::decode_all(compressed.as_slice())
            .map(|v| v.len() as u64)
            .unwrap_or(size_compressed);
        let aggregates = infino::supertable::manifest::aggregates::compute(&part.superfiles);

        let entry = ManifestListEntry {
            part_id: part.part_id,
            uri: commit_mod::part_uri(&content_hash),
            n_superfiles: part.superfiles.len() as u64,
            size_bytes_compressed: size_compressed,
            size_bytes_uncompressed: size_uncompressed,
            content_hash,
            partition_key: 0u32.to_le_bytes().to_vec(),
            id_range: aggregates.id_range,
            scalar_stats_agg: aggregates.scalar_stats_agg,
            fts_summary_agg: aggregates.fts_summary_agg,
            vector_summary_agg: aggregates.vector_summary_agg,
        };
        out.push((part, entry));
    }
    out
}

async fn persist_synthetic_corpus(
    storage: &Arc<dyn StorageProvider>,
    shared_uri: SuperfileUri,
    shared_bytes: Bytes,
    parts_and_entries: Vec<(ManifestPart, ManifestListEntry)>,
) -> Result<(), Box<dyn std::error::Error>> {
    // PUT the dummy segment once.
    let seg_path = format!("data/seg-{}.sf", shared_uri.0);
    storage.put_atomic(&seg_path, shared_bytes).await?;

    let (parts, entries): (Vec<_>, Vec<_>) = parts_and_entries.into_iter().unzip();

    // Build + write the list. commit_manifest writes parts
    // + list + pointer in one shot.
    let list = ManifestList {
        format_version: LIST_FORMAT_VERSION.into(),
        manifest_id: 1,
        options_hash: ContentHash([0u8; 32]),
        schema: Vec::new(),
        id_column: "doc_id".into(),
        fts_columns: vec![FtsColumnInfo {
            column: "title".into(),
        }],
        vector_columns: Vec::new(),
        partition_strategy: PartitionStrategy::Hash {
            column: "doc_id".into(),
            n_buckets: 1,
        },
        parts: entries,
    };
    let parts_refs: Vec<&ManifestPart> = parts.iter().collect();
    commit_mod::commit_manifest(storage.as_ref(), None, &list, &parts_refs, 3).await?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct OpenSnapshot {
    n_parts: usize,
    n_parts_loaded: usize,
}

fn open_snapshot(st: &Supertable) -> OpenSnapshot {
    let r = st.reader();
    let m = r.manifest();
    let list = m.list.as_ref().expect("list");
    let n_parts = list.parts.len();
    let n_parts_loaded = list
        .parts
        .iter()
        .filter(|e| {
            m.parts
                .get(&e.part_id)
                .and_then(|c| c.value().get().cloned())
                .is_some()
        })
        .count();
    OpenSnapshot {
        n_parts,
        n_parts_loaded,
    }
}

async fn phase_a_cold_open(storage: &Arc<dyn StorageProvider>, cache_dir: &Path) -> Supertable {
    let cache = make_cache(Arc::clone(storage), cache_dir);
    let t0 = Instant::now();
    let st = Supertable::open(
        make_options(Arc::clone(storage))
            .with_eager_load_threshold(0) // force lazy for the bench
            .with_disk_cache(Arc::clone(&cache)),
    )
    .await
    .expect("open");
    let wall = t0.elapsed();
    let snap = open_snapshot(&st);
    eprintln!(
        "[m15d] (a) cold open: wall={:.3}s n_parts={} n_parts_loaded={} (must be 0 in lazy mode)",
        wall.as_secs_f64(),
        snap.n_parts,
        snap.n_parts_loaded
    );
    assert_eq!(snap.n_parts_loaded, 0, "lazy open should load 0 parts");
    if std::env::var("INFINO_BENCH_M15D_FULL").is_ok() {
        assert!(
            wall.as_secs_f64() < 5.0,
            "phase (a) cold open at FULL scale must be < 5s; got {:.2}s",
            wall.as_secs_f64()
        );
    }
    st
}

fn phase_b_single_partition_commit(st: &Supertable) {
    // Append + commit one row. M15a rewrites the latest
    // part for partition 0 (single-bucket Hash default
    // for the WRITER's strategy, distinct from the
    // synthetic list's strategy which is also Hash{1}).
    // The user-facing batch has only the user columns;
    // the supertable injects `_id` at append time.
    let batch = RecordBatch::try_new(
        schema_id_title(),
        vec![Arc::new(LargeStringArray::from(vec![
            "written via writer.commit",
        ]))],
    )
    .expect("batch");

    let t0 = Instant::now();
    {
        let mut w = st.writer().expect("writer");
        w.append(&batch).expect("append");
        w.commit().expect("commit");
    }
    let wall = t0.elapsed();
    let snap = open_snapshot(st);
    eprintln!(
        "[m15d] (b) single-partition commit: wall={:.3}s n_parts={} manifest_id={}",
        wall.as_secs_f64(),
        snap.n_parts,
        st.manifest_id(),
    );
}

fn phase_c_high_selectivity_query(st: &Supertable, target_part_idx: usize) {
    let pre = open_snapshot(st);
    let term = unique_term_for(target_part_idx);
    let t0 = Instant::now();
    let hits = retrievalbench::corpus::block_on_inmem(st.reader().bm25_search(
        "title",
        &term,
        10,
        BoolMode::Or,
    ))
    .expect("bm25");
    let wall = t0.elapsed();
    let post = open_snapshot(st);
    let delta = post.n_parts_loaded.saturating_sub(pre.n_parts_loaded);
    eprintln!(
        "[m15d] (c) high-selectivity query ({term}): wall={:.3}s n_parts_loaded_delta={} hits={}",
        wall.as_secs_f64(),
        delta,
        hits.len()
    );
    // The unique-term bloom routes to exactly one part.
    // FP rate of the default 1024-block bloom at 2 distinct
    // terms is ~0, so we expect exactly 1 part loaded.
    assert!(
        delta <= 2,
        "high-selectivity query should load 1 part (allow ≤2 for bloom FP); got {delta}"
    );
}

fn phase_d_low_selectivity_query(st: &Supertable) {
    let pre = open_snapshot(st);
    let t0 = Instant::now();
    let _ = retrievalbench::corpus::block_on_inmem(st.reader().bm25_search(
        "title",
        COMMON_TERM,
        10,
        BoolMode::Or,
    ))
    .expect("bm25");
    let wall = t0.elapsed();
    let post = open_snapshot(st);
    eprintln!(
        "[m15d] (d) low-selectivity query ({COMMON_TERM}): wall={:.3}s \
         n_parts_loaded_delta={} (every part has the common term)",
        wall.as_secs_f64(),
        post.n_parts_loaded.saturating_sub(pre.n_parts_loaded)
    );
}

async fn phase_e_refresh(consumer: &Supertable, storage: &Arc<dyn StorageProvider>) {
    // Sibling: open another handle + commit 1 segment.
    let sibling_cache_dir = TempDir::new().expect("sibling cache");
    let sibling_cache = make_cache(Arc::clone(storage), sibling_cache_dir.path());
    let sibling = Supertable::open(
        make_options(Arc::clone(storage))
            .with_eager_load_threshold(0)
            .with_disk_cache(Arc::clone(&sibling_cache)),
    )
    .await
    .expect("sibling open");
    let batch = RecordBatch::try_new(
        schema_id_title(),
        vec![Arc::new(LargeStringArray::from(vec!["sibling commit"]))],
    )
    .expect("batch");
    {
        let mut w = sibling.writer().expect("sibling writer");
        w.append(&batch).expect("append");
        w.commit().expect("sibling commit");
    }
    drop(sibling);

    let pre = open_snapshot(consumer);
    let t0 = Instant::now();
    let advanced = consumer.refresh().await.expect("refresh");
    let wall = t0.elapsed();
    let post = open_snapshot(consumer);
    eprintln!(
        "[m15d] (e) refresh: wall={:.3}s advanced={} n_parts_loaded_delta={} \
         (inherit-via-content-hash for unchanged parts)",
        wall.as_secs_f64(),
        advanced,
        post.n_parts_loaded.saturating_sub(pre.n_parts_loaded)
    );
    assert!(
        advanced,
        "refresh must report advancement after sibling commit"
    );
}

pub fn run() {
    let (n_parts, n_segs_per_part) = resolve_scale();
    let total = n_parts * n_segs_per_part;
    eprintln!(
        "[m15d] config: n_parts={} n_segments_per_part={} total_segments={}",
        n_parts, n_segs_per_part, total
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("rt");

    let storage_dir = TempDir::new().expect("storage tempdir");
    let cache_dir = TempDir::new().expect("cache tempdir");

    rt.block_on(async {
        let storage: Arc<dyn StorageProvider> =
            Arc::new(LocalFsStorageProvider::new(storage_dir.path()).expect("provider"));

        // --- Synthetic corpus build (off the clock) ---
        let shared_uri = SuperfileUri::new_v4();
        let shared_bytes = build_shared_segment_bytes();
        let t_build = Instant::now();
        let parts_and_entries = build_synthetic_parts(n_parts, n_segs_per_part, shared_uri);
        let build_elapsed = t_build.elapsed();
        eprintln!(
            "[m15d] synthetic corpus built: {} parts × {} superfiles = {} total ({:.2}s)",
            n_parts,
            n_segs_per_part,
            total,
            build_elapsed.as_secs_f64()
        );

        let t_persist = Instant::now();
        persist_synthetic_corpus(&storage, shared_uri, shared_bytes, parts_and_entries)
            .await
            .expect("persist");
        eprintln!(
            "[m15d] synthetic corpus persisted: {:.2}s",
            t_persist.elapsed().as_secs_f64()
        );

        // --- Phase (a) cold open ---
        let consumer = phase_a_cold_open(&storage, cache_dir.path()).await;

        // --- Phase (b) single-partition commit ---
        phase_b_single_partition_commit(&consumer);

        // --- Phase (c) high-selectivity query ---
        // Pick a middle part for the unique-term query.
        phase_c_high_selectivity_query(&consumer, n_parts / 2);

        // --- Phase (d) low-selectivity query ---
        phase_d_low_selectivity_query(&consumer);

        // --- Phase (e) refresh after sibling commit ---
        phase_e_refresh(&consumer, &storage).await;
    });

    eprintln!("[m15d] done");
}
