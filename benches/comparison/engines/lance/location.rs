// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Shared LanceDB storage location for the peer adapters, pluggable over
//! object stores.
//!
//! lancedb delegates storage to object_store, which uniformizes every
//! cloud behind a URI scheme + string options — so each store is a row
//! of data ([`StoreSpec`]), not a code path. Adding a store = one entry
//! in [`STORES`] + the matching lancedb cargo feature; URIs, labels, and
//! error messages all derive from the table.
//!
//! Selection mirrors infino's bench env contract: `INFINO_BENCH_STORE`
//! picks the store explicitly — never inferred from which credential
//! happens to be set. The s3s-fs emulator has no Lance peer.

use std::time::{SystemTime, UNIX_EPOCH};

use tempfile::TempDir;
use tokio::runtime::Runtime;

/// Default prefix root under the bucket/container.
const DEFAULT_PREFIX_ROOT: &str = "retrievalbench-lance";

/// One object-store backend, as data. Adding a store = appending a row
/// here + enabling the matching lancedb cargo feature.
pub struct StoreSpec {
    /// `INFINO_BENCH_STORE` token selecting this store.
    pub token: &'static str,
    /// Report/engine label naming what was measured.
    pub label: &'static str,
    /// object_store URI scheme (bare, no `://`).
    pub scheme: &'static str,
    /// Env vars naming the bucket/container, in priority order.
    pub container_envs: &'static [&'static str],
    /// Env var overriding the prefix root.
    pub prefix_env: &'static str,
    /// (env var, object_store option key) mappings, in priority order —
    /// first env present wins per option key. Explicit because lance
    /// does not reliably autoload creds from env.
    pub cred_envs: &'static [(&'static str, &'static str)],
}

pub const STORES: &[StoreSpec] = &[
    StoreSpec {
        token: "s3",
        label: "lancedb-s3",
        scheme: "s3",
        container_envs: &["INFINO_REAL_S3_BUCKET", "INFINO_TEST_REAL_S3_BUCKET"],
        prefix_env: "INFINO_REAL_S3_PREFIX",
        cred_envs: &[
            ("AWS_REGION", "aws_region"),
            ("AWS_DEFAULT_REGION", "aws_region"),
        ],
    },
    StoreSpec {
        token: "azure",
        label: "lancedb-azure",
        scheme: "az",
        container_envs: &[
            "INFINO_REAL_AZURE_CONTAINER",
            "INFINO_TEST_REAL_AZURE_CONTAINER",
        ],
        prefix_env: "INFINO_REAL_AZURE_PREFIX",
        cred_envs: &[
            ("AZURE_STORAGE_ACCOUNT_NAME", "azure_storage_account_name"),
            ("AZURE_STORAGE_ACCOUNT_KEY", "azure_storage_account_key"),
        ],
    },
];

fn tokens() -> String {
    STORES
        .iter()
        .map(|s| s.token)
        .collect::<Vec<_>>()
        .join("|")
}

/// A store resolved against an environment: plain data, no live env
/// dependency. Resolution is the only place the environment is read.
#[derive(Debug)]
pub struct ResolvedStore {
    label: &'static str,
    scheme: &'static str,
    container: String,
    prefix_root: String,
    storage_options: Vec<(String, String)>,
}

impl ResolvedStore {
    /// Resolve `token` against `env`. The environment is injected, so
    /// every path is unit-testable without process-env mutation.
    pub fn resolve(
        token: &str,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        if token == "s3s_fs" || token.is_empty() {
            return Err(format!(
                "LanceDB peer needs a real object store: INFINO_BENCH_STORE={}",
                tokens()
            ));
        }
        let spec = STORES.iter().find(|s| s.token == token).ok_or_else(|| {
            format!("unknown INFINO_BENCH_STORE={token} (want s3s_fs|{})", tokens())
        })?;
        let container = spec
            .container_envs
            .iter()
            .find_map(|name| env(name).filter(|v| !v.is_empty()))
            .ok_or_else(|| {
                format!(
                    "INFINO_BENCH_STORE={token} requires {}",
                    spec.container_envs[0]
                )
            })?;
        let prefix_root = env(spec.prefix_env)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_PREFIX_ROOT.to_string());
        let mut storage_options: Vec<(String, String)> = Vec::new();
        for (var, key) in spec.cred_envs {
            if storage_options.iter().any(|(k, _)| k == key) {
                continue;
            }
            if let Some(v) = env(var).filter(|v| !v.is_empty()) {
                storage_options.push((key.to_string(), v));
            }
        }
        Ok(Self {
            label: spec.label,
            scheme: spec.scheme,
            container,
            prefix_root,
            storage_options,
        })
    }

    pub fn from_env() -> Result<Self, String> {
        let env = |name: &str| std::env::var(name).ok();
        let token = env("INFINO_BENCH_STORE").unwrap_or_else(|| "s3s_fs".to_string());
        Self::resolve(&token, &env)
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    pub fn storage_options(&self) -> &[(String, String)] {
        &self.storage_options
    }

    /// Table-dir URI for `lancedb::connect`.
    pub fn uri(&self, leaf: &str) -> String {
        format!(
            "{}://{}/{}/{leaf}",
            self.scheme,
            self.container,
            self.prefix_root.trim_matches('/')
        )
    }
}

/// The configured store's report label. Panics with the resolver's
/// message when no real store is configured — callers gate on
/// `tiers::supertable_backend_check()` first.
pub fn backend_label() -> &'static str {
    ResolvedStore::from_env().unwrap_or_else(|e| panic!("{e}")).label()
}

pub(crate) enum LanceStorage {
    Local { _dir: TempDir },
    /// Per-run unique remote prefix; `delete` drops the table.
    Remote,
}

pub(crate) struct LanceLocation {
    pub uri: String,
    pub storage_options: Vec<(String, String)>,
    pub storage: LanceStorage,
}

impl LanceLocation {
    pub fn local() -> Self {
        let dir = tempfile::tempdir().expect("lance tempdir");
        let uri = dir.path().to_str().expect("utf8 temp path").to_string();
        Self {
            uri,
            storage_options: Vec::new(),
            storage: LanceStorage::Local { _dir: dir },
        }
    }

    /// Unique per-run location on the configured remote store.
    pub fn remote(prefix: &str) -> Self {
        let store = ResolvedStore::from_env().unwrap_or_else(|e| panic!("{e}"));
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_nanos();
        let leaf = format!("{prefix}-{}-{unique}", std::process::id());
        Self {
            uri: store.uri(&leaf),
            storage_options: store.storage_options().to_vec(),
            storage: LanceStorage::Remote,
        }
    }
}

pub(crate) fn new_runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test environment as data — no process-env mutation.
    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn selection_is_explicit_never_inferred() {
        // Emulator selected ⇒ no Lance peer, even with every cred set.
        let env = env_of(&[
            ("INFINO_REAL_S3_BUCKET", "bkt"),
            ("INFINO_REAL_AZURE_CONTAINER", "cont"),
        ]);
        assert!(ResolvedStore::resolve("s3s_fs", &env).is_err());
        assert!(ResolvedStore::resolve("", &env).is_err());
        // Unknown token names the valid set.
        let err = ResolvedStore::resolve("gcs", &env).unwrap_err();
        assert!(err.contains("s3"), "unknown-store error lists tokens: {err}");
    }

    #[test]
    fn s3_resolves_with_bucket_and_fails_without() {
        let store = ResolvedStore::resolve("s3", &env_of(&[("INFINO_REAL_S3_BUCKET", "bkt")]))
            .expect("s3 resolves");
        assert_eq!(store.label(), "lancedb-s3");
        assert_eq!(store.uri("fts-1-2"), "s3://bkt/retrievalbench-lance/fts-1-2");

        let err = ResolvedStore::resolve("s3", &env_of(&[("INFINO_REAL_AZURE_CONTAINER", "c")]))
            .unwrap_err();
        assert!(err.contains("INFINO_REAL_S3_BUCKET"), "{err}");
    }

    #[test]
    fn azure_resolves_with_container_and_fails_without() {
        let store = ResolvedStore::resolve(
            "azure",
            &env_of(&[("INFINO_REAL_AZURE_CONTAINER", "cont")]),
        )
        .expect("azure resolves");
        assert_eq!(store.label(), "lancedb-azure");
        assert_eq!(store.uri("fts-1-2"), "az://cont/retrievalbench-lance/fts-1-2");

        let err = ResolvedStore::resolve("azure", &env_of(&[("INFINO_REAL_S3_BUCKET", "b")]))
            .unwrap_err();
        assert!(err.contains("INFINO_REAL_AZURE_CONTAINER"), "{err}");
    }

    #[test]
    fn fallback_container_env_and_prefix_override_apply() {
        let store = ResolvedStore::resolve(
            "azure",
            &env_of(&[
                ("INFINO_TEST_REAL_AZURE_CONTAINER", "test-cont"),
                ("INFINO_REAL_AZURE_PREFIX", "custom/root"),
            ]),
        )
        .expect("fallback container env resolves");
        assert_eq!(store.uri("x"), "az://test-cont/custom/root/x");
    }

    #[test]
    fn cred_options_map_env_to_object_store_keys_first_wins() {
        let store = ResolvedStore::resolve(
            "s3",
            &env_of(&[
                ("INFINO_REAL_S3_BUCKET", "bkt"),
                ("AWS_REGION", "eu-west-1"),
                ("AWS_DEFAULT_REGION", "us-east-1"), // loses: AWS_REGION listed first
            ]),
        )
        .expect("s3 resolves");
        assert_eq!(
            store.storage_options(),
            &[("aws_region".to_string(), "eu-west-1".to_string())]
        );

        let store = ResolvedStore::resolve(
            "azure",
            &env_of(&[
                ("INFINO_REAL_AZURE_CONTAINER", "cont"),
                ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
                ("AZURE_STORAGE_ACCOUNT_KEY", "key"),
            ]),
        )
        .expect("azure resolves");
        assert_eq!(
            store.storage_options(),
            &[
                ("azure_storage_account_name".to_string(), "acct".to_string()),
                ("azure_storage_account_key".to_string(), "key".to_string()),
            ]
        );
    }

    #[test]
    fn every_spec_row_is_well_formed() {
        // Registration-time invariants for all stores, present and future.
        for spec in STORES {
            assert!(!spec.token.is_empty());
            assert!(spec.label.starts_with("lancedb-"), "{}", spec.label);
            assert!(!spec.scheme.contains(':'), "bare scheme: {}", spec.scheme);
            assert!(!spec.container_envs.is_empty(), "{}: no container env", spec.token);
            assert_ne!(spec.token, "s3s_fs", "emulator cannot be a spec row");
        }
        let mut tokens: Vec<_> = STORES.iter().map(|s| s.token).collect();
        tokens.sort_unstable();
        tokens.dedup();
        assert_eq!(tokens.len(), STORES.len(), "duplicate store token");
    }
}
