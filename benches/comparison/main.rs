// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! The single comparison benchmark binary — same positional grammar as
//! infino's own bench binary.
//!
//! Current engine scope is the adapters already present in this harness:
//! LanceDB for FTS/vector/SQL and Tantivy for FTS. DuckDB/CoreDB are not
//! wired here.
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

#[path = "superfile.rs"]
mod superfile;
#[path = "supertable.rs"]
mod supertable;

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
         \n\
         Examples:\n\
         \x20 cargo bench\n\
         \x20 cargo bench -- supertable\n\
         \x20 cargo bench -- superfile fts\n\
         \x20 cargo bench -- supertable sql warm\n"
    );
    std::process::exit(code);
}

fn parse_args() -> (Vec<Tier>, Vec<Modality>, Phases) {
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
    let mut build = false;
    let mut warm = false;
    let mut cold = false;
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
            "build" => build = true,
            "warm" => warm = true,
            "cold" => cold = true,
            "search" => {
                warm = true;
                cold = true;
            }
            other => unknown.push(other.to_string()),
        }
    }

    if !unknown.is_empty() {
        eprintln!("[comparison] unknown selector(s): {}", unknown.join(", "));
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
    (tiers, modalities, phases)
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

    let (tiers, modalities, phases) = parse_args();
    for tier in tiers {
        for &modality in &modalities {
            run_cell(tier, modality, phases);
        }
    }
}
