// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Unified third-party comparison bench entry point.
//!
//! Current engine scope is the adapters already present in this harness:
//! LanceDB for FTS/vector/SQL and Tantivy for FTS. DuckDB/CoreDB are not
//! wired here.
//!
//! ```text
//! cargo bench --bench comparison
//! cargo bench --bench comparison -- superfile_fts
//! cargo bench --bench comparison -- superfile_vector superfile_sql
//! cargo bench --bench comparison -- supertable_fts warm
//! ```

#[path = "superfile.rs"]
mod superfile;
#[path = "supertable.rs"]
mod supertable;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Test {
    SuperfileFts,
    SuperfileVector,
    SuperfileSql,
    SupertableFts,
    SupertableVector,
    SupertableSql,
}

#[derive(Clone, Copy)]
struct Phases {
    build: bool,
    warm: bool,
    cold: bool,
}

impl Test {
    const ALL: [Test; 6] = [
        Test::SuperfileFts,
        Test::SuperfileVector,
        Test::SuperfileSql,
        Test::SupertableFts,
        Test::SupertableVector,
        Test::SupertableSql,
    ];

    fn key(self) -> &'static str {
        match self {
            Test::SuperfileFts => "superfile_fts",
            Test::SuperfileVector => "superfile_vector",
            Test::SuperfileSql => "superfile_sql",
            Test::SupertableFts => "supertable_fts",
            Test::SupertableVector => "supertable_vector",
            Test::SupertableSql => "supertable_sql",
        }
    }

    fn from_arg(arg: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|test| test.key() == arg)
    }

    fn run(self, phases: Phases) {
        match self {
            Test::SuperfileFts => superfile::fts::run(),
            Test::SuperfileVector => superfile::vector::run(),
            Test::SuperfileSql => superfile::sql::run(),
            Test::SupertableFts => supertable::fts::run(phases.build, phases.warm, phases.cold),
            Test::SupertableVector => {
                supertable::vector::run(phases.build, phases.warm, phases.cold)
            }
            Test::SupertableSql => supertable::sql::run(phases.build, phases.warm, phases.cold),
        }
    }
}

fn parse_args() -> (Vec<Test>, Phases) {
    let mut tests = Vec::new();
    let mut build = false;
    let mut warm = false;
    let mut cold = false;
    for arg in std::env::args().skip(1).filter(|arg| !arg.starts_with('-')) {
        if let Some(test) = Test::from_arg(&arg) {
            if !tests.contains(&test) {
                tests.push(test);
            }
        } else {
            match arg.as_str() {
                "build" => build = true,
                "warm" => warm = true,
                "cold" => cold = true,
                "search" => {
                    warm = true;
                    cold = true;
                }
                _ => eprintln!("[comparison] ignoring unknown selector {arg:?}"),
            }
        }
    }
    if tests.is_empty() {
        tests.extend(Test::ALL);
    }
    let phases = if build || warm || cold {
        Phases { build, warm, cold }
    } else {
        Phases {
            build: true,
            warm: true,
            cold: true,
        }
    };
    (tests, phases)
}

fn main() {
    // Isolated per-shape supertable ingest child (the supertable runner
    // re-execs this binary with `INFINO_BENCH_SUPERTABLE_SHAPE` set).
    // Without this intercept the child ignores the shape protocol and
    // re-runs the whole comparison suite — recursively, since its own
    // supertable cells spawn further children.
    if infino_bench_utils::supertable::handle_shape_child_from_env() {
        return;
    }

    let (tests, phases) = parse_args();
    for test in tests {
        eprintln!(
            "[comparison] === {} (build={}, warm={}, cold={}) ===",
            test.key(),
            phases.build,
            phases.warm,
            phases.cold
        );
        test.run(phases);
    }
}
