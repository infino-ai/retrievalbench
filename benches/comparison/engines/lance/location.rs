// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Shared dataset location for the LanceDB adapters.
//!
//! `local()` is the in-memory (superfile) tier: a tempdir-backed dataset.
//! `object_store()` is the supertable tier. Its backend follows the same
//! `INFINO_BENCH_STORE` switch the Infino side reads, so both engines in
//! a comparison land on the same store class: `azure` selects an `az://`
//! dataset with the standard `AZURE_STORAGE_ACCOUNT_NAME`/`_KEY`
//! credentials; any other value keeps the original real-S3 behaviour
//! (`INFINO_REAL_S3_BUCKET` + ambient AWS credentials).

use std::env;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use infino_bench_utils::storage_options::azure_storage_options_from_env;
use infino_bench_utils::tiers::{real_s3_bucket_env, real_s3_prefix_root};
use tempfile::TempDir;

/// Store selector, shared with the Infino supertable bench.
const BENCH_STORE_ENV: &str = "INFINO_BENCH_STORE";
/// Azure container name, shared with `infino-bench-utils`.
const AZURE_CONTAINER_ENV: &str = "INFINO_REAL_AZURE_CONTAINER";
/// Azure key-prefix root, mirroring `INFINO_REAL_S3_PREFIX`.
const AZURE_PREFIX_ENV: &str = "INFINO_REAL_AZURE_PREFIX";
/// Default key prefix for comparison datasets on either store.
const DEFAULT_PREFIX_ROOT: &str = "retrievalbench-lance";

/// Report/engine label for the object-store LanceDB peer.
pub fn lance_peer_label() -> &'static str {
    if azure_selected() {
        "lancedb-azure"
    } else {
        "lancedb-s3"
    }
}

/// True when the object-store comparison runs on Azure
/// (`INFINO_BENCH_STORE=azure`), matching the Infino side.
fn azure_selected() -> bool {
    env::var(BENCH_STORE_ENV).as_deref() == Ok("azure")
}

pub(crate) enum LanceStorage {
    Local { _dir: TempDir },
    Remote,
}

pub(crate) struct LanceLocation {
    pub(crate) uri: String,
    pub(crate) storage_options: Vec<(String, String)>,
    pub(crate) storage: LanceStorage,
}

impl LanceLocation {
    pub(crate) fn local() -> Self {
        let dir = tempfile::tempdir().expect("lance tempdir");
        let uri = dir.path().to_str().expect("utf8 temp path").to_string();
        Self {
            uri,
            storage_options: Vec::new(),
            storage: LanceStorage::Local { _dir: dir },
        }
    }

    /// Object-store dataset on the `INFINO_BENCH_STORE` backend.
    pub(crate) fn object_store(prefix: &str) -> Self {
        if azure_selected() {
            Self::azure(prefix)
        } else {
            Self::s3(prefix)
        }
    }

    fn unique_key(prefix: &str, root: &str) -> String {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_nanos();
        format!(
            "{}/{prefix}-{}-{unique}",
            root.trim_matches('/'),
            process::id(),
        )
    }

    fn s3(prefix: &str) -> Self {
        let bucket =
            real_s3_bucket_env().expect("INFINO_REAL_S3_BUCKET required for LanceDB S3 tier");
        let root = real_s3_prefix_root(DEFAULT_PREFIX_ROOT);
        let uri = format!("s3://{bucket}/{}", Self::unique_key(prefix, &root));
        let mut storage_options = Vec::new();
        if let Ok(region) = env::var("AWS_REGION").or_else(|_| env::var("AWS_DEFAULT_REGION")) {
            storage_options.push(("aws_region".to_string(), region));
        }
        Self {
            uri,
            storage_options,
            storage: LanceStorage::Remote,
        }
    }

    fn azure(prefix: &str) -> Self {
        let container = env::var(AZURE_CONTAINER_ENV)
            .expect("INFINO_REAL_AZURE_CONTAINER required for the LanceDB Azure tier");
        let root = env::var(AZURE_PREFIX_ENV).unwrap_or_else(|_| DEFAULT_PREFIX_ROOT.to_string());
        let uri = format!("az://{container}/{}", Self::unique_key(prefix, &root));
        let storage_options = azure_storage_options_from_env().into_iter().collect();
        Self {
            uri,
            storage_options,
            storage: LanceStorage::Remote,
        }
    }
}
