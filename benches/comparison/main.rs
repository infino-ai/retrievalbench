// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! The single comparison benchmark binary — same positional grammar as
//! infino's own bench binary.
//!
//! Current engine scope: LanceDB for FTS/vector/SQL, Tantivy for FTS,
//! and FAISS/turbovec for compressed-vector cells.
//!
//! ```text
//! cargo bench                            # everything, all phases
//! cargo bench -- all                     # same as above
//! cargo bench -- superfile               # all 3 superfile modalities
//! cargo bench -- superfile vector        # one cell
//! cargo bench -- supertable fts warm     # one cell, one phase
//! cargo bench -- supertable sql build cold
//! ```
//!
//! Token vocabulary:
//!   tier        : `superfile` | `supertable`        (omitted => both)
//!   modality    : `fts` | `vector` | `sql`          (omitted => all three)
//!   phase       : `build` | `warm` | `cold` | `search` (= warm+cold)
//!                 (omitted => all three phases)
//!   `all`       : explicit "every tier × modality × phase" (the default).
//!
//! The cells run = (selected tiers) × (selected modalities). Phases apply
//! to the supertable tier; superfile cells always run build + search.
//!
//! Scale (`INFINO_BENCH_SUPERFILE_DOCS`, `INFINO_BENCH_SUPERTABLE_DOCS`)
//! and object-store backend (`INFINO_BENCH_STORE`) are env knobs.

use infino_bench_utils::corpus;
use tracing_subscriber::EnvFilter;

/// Corpus spec / staging dir carried to the isolated ingest children.
///
/// `build_shape_isolated` re-execs this binary with only `SHAPE_ENV`
/// set — it forwards no argv — so a corpus named on the command line
/// alone would reach the parent and not the child, and the child (which
/// performs the measured ingest) would silently build the synthetic
/// generator while the parent grades against the real dataset. The
/// child does inherit our environment, so the spec travels there.
const CORPUS_ENV: &str = "INFINO_BENCH_COMPARISON_CORPUS";
const CORPUS_DIR_ENV: &str = "INFINO_BENCH_COMPARISON_CORPUS_DIR";

/// Install the corpus source recorded in the environment, if any.
/// Must run before ANY corpus read (the source resolves once per
/// process) and before the shape-child intercept below.
/// Surface engine tracing (drain declines, serving-mode fallbacks) on
/// stderr, honoring `RUST_LOG`. Defaults to `infino=warn`: a warning here
/// usually means a cell is not measuring what its label claims.
fn init_engine_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("infino=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn install_corpus_from_env() {
    if let Ok(spec) = std::env::var(CORPUS_ENV) {
        let dir = std::env::var(CORPUS_DIR_ENV).ok();
        if let Err(err) = corpus::set_source(&spec, dir.as_deref()) {
            eprintln!("[comparison] {err}");
            std::process::exit(2);
        }
    }
}

#[path = "superfile.rs"]
mod superfile;
#[path = "supertable.rs"]
mod supertable;
#[path = "table_writes.rs"]
mod table_writes;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tier {
    Superfile,
    Supertable,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Modality {
    Fts,
    Vector,
    Sql,
}

#[derive(Clone, Copy)]
struct Phases {
    build: bool,
    warm: bool,
    cold: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SpecialCell {
    TrackACodec,
    TrackAWrites,
}

const ALL_PHASES: Phases = Phases {
    build: true,
    warm: true,
    cold: true,
};

fn run_cell(tier: Tier, modality: Modality, phases: Phases) {
    let label = match (tier, modality) {
        (Tier::Superfile, Modality::Fts) => "superfile fts",
        (Tier::Superfile, Modality::Vector) => "superfile vector",
        (Tier::Superfile, Modality::Sql) => "superfile sql",
        (Tier::Supertable, Modality::Fts) => "supertable fts",
        (Tier::Supertable, Modality::Vector) => "supertable vector",
        (Tier::Supertable, Modality::Sql) => "supertable sql",
    };
    eprintln!(
        "[comparison] === {label} (build={}, warm={}, cold={}) ===",
        phases.build, phases.warm, phases.cold
    );
    match (tier, modality) {
        (Tier::Superfile, Modality::Fts) => superfile::fts::run(),
        (Tier::Superfile, Modality::Vector) => superfile::vector::run(),
        (Tier::Superfile, Modality::Sql) => superfile::sql::run(),
        (Tier::Supertable, Modality::Fts) => {
            supertable::fts::run(phases.build, phases.warm, phases.cold)
        }
        (Tier::Supertable, Modality::Vector) => {
            supertable::vector::run(phases.build, phases.warm, phases.cold)
        }
        (Tier::Supertable, Modality::Sql) => {
            supertable::sql::run(phases.build, phases.warm, phases.cold)
        }
    }
}

fn print_usage_and_exit(code: i32) -> ! {
    eprintln!(
        "Usage:\n  cargo bench -- [tier] [modality] [phase ...]\n\
         \n\
         Tier      : superfile | supertable        (omitted => both)\n\
         Modality  : fts | vector | sql            (omitted => all three)\n\
         Phase     : build | warm | cold | search  (search = warm+cold; omitted => all)\n\
         all       : every tier x modality x phase (the default for a bare `cargo bench`)\n\
         Special   : track-a-codec | track-a-writes\n\
         corpus=<spec>     : synthetic (default) | annb:<slug> | hf:<owner/repo> | parquet:<dir>\n\
         corpus-dir=<path> : where downloadable corpora are staged\n\
         \n\
         Examples:\n\
         \x20 cargo bench\n\
         \x20 cargo bench -- supertable\n\
         \x20 cargo bench -- superfile fts\n\
         \x20 cargo bench -- supertable sql warm\n"
    );
    std::process::exit(code);
}

fn parse_args() -> (Vec<Tier>, Vec<Modality>, Phases, Option<SpecialCell>) {
    // Drop harness flags (e.g. a stray `--bench`); only positional tokens
    // are ours.
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();

    if std::env::args().any(|a| matches!(a.as_str(), "help" | "-h" | "--help")) {
        print_usage_and_exit(0);
    }

    let mut tiers: Vec<Tier> = Vec::new();
    let mut modalities: Vec<Modality> = Vec::new();
    let mut corpus_spec: Option<String> = None;
    let mut corpus_dir: Option<String> = None;
    let mut build = false;
    let mut warm = false;
    let mut cold = false;
    let mut special = None;
    let mut unknown: Vec<String> = Vec::new();

    for arg in &args {
        match arg.as_str() {
            "all" => {}
            "superfile" => {
                if !tiers.contains(&Tier::Superfile) {
                    tiers.push(Tier::Superfile);
                }
            }
            "supertable" => {
                if !tiers.contains(&Tier::Supertable) {
                    tiers.push(Tier::Supertable);
                }
            }
            "fts" => {
                if !modalities.contains(&Modality::Fts) {
                    modalities.push(Modality::Fts);
                }
            }
            "vector" => {
                if !modalities.contains(&Modality::Vector) {
                    modalities.push(Modality::Vector);
                }
            }
            "sql" => {
                if !modalities.contains(&Modality::Sql) {
                    modalities.push(Modality::Sql);
                }
            }
            "track-a-codec" => special = Some(SpecialCell::TrackACodec),
            "track-a-writes" => special = Some(SpecialCell::TrackAWrites),
            "build" => build = true,
            "warm" => warm = true,
            "cold" => cold = true,
            "search" => {
                warm = true;
                cold = true;
            }
            other => match other.split_once('=') {
                Some(("corpus", spec)) => corpus_spec = Some(spec.to_string()),
                Some(("corpus-dir", dir)) => corpus_dir = Some(dir.to_string()),
                _ => unknown.push(other.to_string()),
            },
        }
    }

    if !unknown.is_empty() {
        eprintln!("[comparison] unknown selector(s): {}", unknown.join(", "));
        print_usage_and_exit(2);
    }

    // Publish before installing: the ingest children inherit this env,
    // which is the only channel that reaches them (see CORPUS_ENV).
    if let Some(spec) = corpus_spec.as_deref() {
        // SAFETY: single-threaded startup, before any child is spawned.
        unsafe {
            std::env::set_var(CORPUS_ENV, spec);
            if let Some(dir) = corpus_dir.as_deref() {
                std::env::set_var(CORPUS_DIR_ENV, dir);
            }
        }
        if let Err(err) = corpus::set_source(spec, corpus_dir.as_deref()) {
            eprintln!("[comparison] {err}");
            print_usage_and_exit(2);
        }
        eprintln!("[comparison] corpus = {}", corpus::corpus_label());
    } else if corpus_dir.is_some() {
        eprintln!("[comparison] corpus-dir= given without corpus=; nothing would read it");
        print_usage_and_exit(2);
    }

    let tiers = if tiers.is_empty() {
        vec![Tier::Superfile, Tier::Supertable]
    } else {
        tiers
    };
    let modalities = if modalities.is_empty() {
        vec![Modality::Fts, Modality::Vector, Modality::Sql]
    } else {
        modalities
    };
    let phases = if build || warm || cold {
        Phases { build, warm, cold }
    } else {
        ALL_PHASES
    };
    (tiers, modalities, phases, special)
}

fn main() {
    init_engine_logging();
    // The corpus must be installed before the shape-child intercept: a
    // child returns from that branch without ever reaching parse_args,
    // so the env is its only source for the dataset it ingests.
    install_corpus_from_env();
    // Isolated per-shape supertable ingest child (the supertable runner
    // re-execs this binary with `INFINO_BENCH_SUPERTABLE_SHAPE` set).
    // Without this intercept the child ignores the shape protocol and
    // re-runs the whole comparison suite — recursively, since its own
    // supertable cells spawn further children.
    if infino_bench_utils::supertable::handle_shape_child_from_env() {
        return;
    }

    let (tiers, modalities, phases, special) = parse_args();
    match special {
        Some(SpecialCell::TrackACodec) => {
            supertable::vector::codec_curve();
            return;
        }
        Some(SpecialCell::TrackAWrites) => {
            table_writes::run();
            return;
        }
        None => {}
    }
    for tier in tiers {
        for &modality in &modalities {
            run_cell(tier, modality, phases);
        }
    }
}
