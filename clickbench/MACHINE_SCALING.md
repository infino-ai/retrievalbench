# ClickBench machines: where everyone runs, and how DataFusion scales

Pulled from the upstream ClickBench data mirrored in the fork (2026-07-27). Purpose: understand the machine landscape (the leaderboard is NOT all on one machine), where the managed warehouses sit, and how DataFusion scales with hardware, so infino's single c6a.4xlarge number is read in the right context.

## TL;DR

- **infino only has a c6a.4xlarge number (35.66s hot).** That is the ClickBench *reference* machine: 16 vCPU. On it, infino is mid-pack (#21/94).
- **The headline leaderboard numbers everyone quotes are on `c8g.metal-48xl` (192 vCPU Graviton4), not c6a.4xlarge.** There the whole field is single-digit seconds (Umbra 1.6s, DuckDB 3.0s, DataFusion 8.5s). infino has never run there.
- **DataFusion scales hard with cores: 45.92s @ 16 vCPU -> 8.54s @ 192 vCPU (5.4x).** The hunch is correct: DataFusion is much more performant on larger instances. Since infino's SQL runs on DataFusion, infino would likely scale similarly (untested).
- **Managed warehouses (Snowflake, Databricks, BigQuery, Redshift) run on their own opaque hardware, not c6a.** Their numbers are not directly comparable on a per-machine basis.

## Three machine classes on the board

| Class | Example machines | vCPU | Who runs here |
|---|---|--:|---|
| Reference (small) | **c6a.4xlarge** (AMD Milan), c8g.4xlarge (Graviton4) | 16 | The standard ClickBench reference. **infino runs here.** Most self-hosted engines report here. |
| Metal (large) | **c8g.metal-48xl** (Graviton4), c7a.metal-48xl (AMD Genoa), c6a.metal (AMD Milan) | 192 | Where the leaderboard HEADLINE numbers come from. The fast single-digit-second results are all here. |
| Managed / own hardware | Snowflake (xs..4xl warehouses), Databricks (x-small..2x-large), Redshift (dc2/ra3 nodes), BigQuery (serverless), ClickHouse Cloud (gcp.N.M) | opaque | Proprietary sizing, not a fixed instance. Priced by credits/slots, not comparable per-machine. |

## DataFusion scaling with hardware (the answer to "better on larger instances")

Hot sum (sum of min hot run over 43 queries), latest runs (2026-06-29). Full result files now in `results/datafusion/` and `results/datafusion-partitioned/`.

| Machine | vCPU | Arch | DataFusion (parquet, single) | DataFusion (partitioned) |
|---|--:|---|--:|--:|
| c6a.xlarge | 4 | AMD Milan | 184.91s | 240.98s |
| c6a.2xlarge | 8 | AMD Milan | 95.35s | 84.63s |
| **c6a.4xlarge** | **16** | AMD Milan | **45.92s** | **42.32s** |
| c8g.4xlarge | 16 | Graviton4 | 19.08s | 16.35s |
| c6a.metal | 192 | AMD Milan | 14.61s | 11.39s |
| c7a.metal-48xl | 192 | AMD Genoa (Zen4) | 9.74s | 7.43s |
| **c8g.metal-48xl** | **192** | **Graviton4** | **8.54s** | **7.27s** |

Two things stand out: DataFusion drops **5.4x** from c6a.4xlarge to c8g.metal-48xl (16 -> 192 vCPU, near-linear on the embarrassingly-parallel scan/agg workload), and **Graviton4 is ~2x faster than same-vCPU AMD** at both sizes (c8g.4xlarge 19.08 vs c6a.4xlarge 45.92; c8g.metal 8.54 vs c6a.metal 14.61). So the machine AND the ISA both matter a lot; a raw hot-sum comparison across machines is meaningless.

## The `c8g.metal-48xl` leaderboard (192 vCPU) — where the fast numbers live

Best per system, hot sum:

| Hot sum | System |
|--:|---|
| 1.61s | Umbra |
| 2.80s | CedarDB |
| 3.01s | DuckDB |
| 3.18s | GizmoSQL |
| 3.69s | ClickHouse |
| 4.48s | Firebolt |
| 4.73s | Polars |
| 5.42s | DuckDB (Parquet, partitioned) |
| 6.19s | DuckDB (Parquet, single) |
| **8.54s** | **DataFusion (parquet)** |

infino is absent (never run on metal). If infino tracks DataFusion (its SQL engine), a metal run would plausibly land in the ~8-15s range, but that is a projection, not a measurement.

## Managed warehouses — own hardware, best hot sum

Not comparable per-machine (proprietary sizing, credit-priced), listed for context only:

| System | Best hot sum | Their "machine" |
|---|--:|---|
| Databricks | 4.29s | small (Photon) |
| ClickHouse Cloud | 4.72s | gcp.2.356 |
| Redshift | 13.25s | 4x.ra3.16xlarge |
| Snowflake | 14.35s | 3xl warehouse |
| BigQuery | 27.33s | serverless |

These run on large clusters behind an API; the number reflects both the engine and however much hardware the tier buys. Upstream ClickBench excludes them from the c6a comparison for exactly this reason.

## Where this leaves infino

- On the **reference machine (c6a.4xlarge)**, infino at 35.66s beats DataFusion (45.92), CedarDB (37.14), Polars (37.62), and ClickHouse-on-Parquet (39.41); loses to DuckDB-parquet (32.48), ClickHouse-native (32.26), GizmoSQL (21.03), Firebolt (21.25).
- The **leaderboard-topping numbers are a metal-machine game**. The c6a.4xlarge leaders (GizmoSQL 21, Firebolt 21) are the same systems that top the metal board, because they scale; the ~14s gap to them on c6a.4xlarge is the real distance.

### infino ON metal — measured 2026-07-27 (was a projection; now real)

**infino hot sum = 6.98s on c7a.metal-48xl** (192 vCPU AMD Zen4; build `RUSTFLAGS=-C target-cpu=native`, thin LTO; 192 cores + AVX-512 confirmed engaged; measured on `experiments/infino-metal-devbox`). Result file: `results/infino/c7a.metal-48xl.json`.

Same-machine ranking (c7a.metal-48xl, best hot sum per system):

| # | System | hot sum |
|--:|---|--:|
| 1 | Umbra | 1.67 |
| 2 | CedarDB | 2.89 |
| ~5 | GizmoSQL / DuckDB | 3.69 |
| 9 | ClickHouse | 4.15 |
| 13 | Firebolt | 5.02 |
| 17 | DuckDB (Parquet, partitioned) | 6.33 |
| 21 | DuckDB (Parquet, single) | 6.91 |
| **~22** | **infino (thin LTO)** | **6.98** |
| 24 | DataFusion (partitioned) | 7.43 |
| 31 | DataFusion (single) | 9.74 |

So the metal run **confirms the c6a story, it does not change it**: infino scales to metal (46s->7s class, same as DataFusion), **beats DataFusion on metal too** (6.98 vs 7.43 / 9.74), sits **tied with DuckDB-parquet**, and lands **mid-pack behind the native-format / in-memory leaders** (Umbra, CedarDB, ClickHouse, DuckDB-memory). infino is a strong *Parquet-reading* engine; the top of the board is a different substrate class (own format, in-memory), which is the structural ceiling for a search-on-Parquet-on-object-storage engine. This is still thin LTO; a fat-LTO build should shave a little more (likely edging past the DuckDB-parquet cluster).

Note on machines: infino ran on **c7a.metal-48xl (AMD Zen4, x86, AVX-512)** because its x86 paths engage there with zero porting risk. The absolute-fastest board machine is **c8g.metal-48xl (Graviton4, ARM)**, ~2x the ISA (DataFusion 8.54 there vs 9.74 on c7a); an infino c8g run needs an aarch64 build validation first but would give a lower absolute number on the same relative footing.

Reference: upstream [ClickBench](https://github.com/ClickHouse/ClickBench). Per-machine result files for DataFusion are in `results/datafusion/<machine>.json` and `results/datafusion-partitioned/<machine>.json`.
