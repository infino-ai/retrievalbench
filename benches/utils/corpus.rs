//! Bench corpus + query + ground-truth utilities. Re-exported from
//! `infino::test_helpers::bench_corpus` so both this crate and infino's
//! own bench tree measure against an identical corpus, queries, and
//! brute-force ground truth — without that shared source the
//! head-to-head tables here couldn't be combined with infino's
//! published latency numbers in any apples-to-apples way.
//!
//! All implementation lives in infino under the `test-helpers` feature
//! (auto-enabled via dev-deps). Re-exporting here keeps the existing
//! `crate::corpus::...` call sites in bench files working without a
//! mechanical rename.

#![allow(unused_imports)]

pub use infino::test_helpers::bench_corpus::*;
