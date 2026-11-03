# Rally: OpenSearch with Infino against Lucene

Races Elasticsearch, stock OpenSearch and OpenSearch with the
[Infino plugin](https://github.com/infino-ai/opensearch-infino) against each other with
[Rally](https://github.com/elastic/rally), one VM per engine and one Rally runner, and reports
ingest, every search task, storage, CPU and memory side by side.

It is the [infinohq](https://github.com/infinohq/infino) AWS Rally harness
(`utils/perf-test-scripts/rally/` and `deploy/terraform/rally/`) on Google Cloud. It keeps that
harness's Rally fork, race flags, track parameters, `rally.ini`, engine settings and machine
size. `gcloud` replaces Terraform, and the plugin is added as an engine.

| File | Runs on | What it does |
| --- | --- | --- |
| `benchmark.sh` | your machine or CI | creates the VMs and firewall rules, sets everything up, races each engine, collects results, writes the comparison, deletes the VMs |
| `engine-setup.sh` | each engine VM | puts the data on the local SSDs, installs the engine from its release tarball (and the plugin, bridge library and `infino.yaml` for the Infino engines), starts it on :9200 |
| `runner-setup.sh` | the runner VM | installs the infinohq Rally fork (branch `fork`) on the local SSD |
| `race.sh` | the runner, or your machine | one `esrally race` against one engine, as the AWS harness ran it |
| `sampler.py` | each engine VM | CPU and memory samples every 5 s during the race |
| `compare.py` | your machine or CI | the comparison report (`comparison.txt`, `comparison.json`) |
| `local.sh` | your machine | races engines that are already running, then compares them |
| `../.github/workflows/rally-cloud.yml` | GitHub Actions | builds the plugin, runs `benchmark.sh` once per engine, compares |

## Engines

| Name | What runs |
| --- | --- |
| `elasticsearch` | Elasticsearch `ES_VERSION`, security off |
| `opensearch` | OpenSearch `OS_VERSION`, security off (Lucene) |
| `opensearch-infino` | OpenSearch `OS_VERSION` with the plugin in default mode: tables on the VM's local SSD |
| `opensearch-infino-proxy` | OpenSearch `OS_VERSION` with the plugin in proxy mode: tables in the Infino Cloud account `INFINO_PLATFORM_URL` / `INFINO_DATABASE`, key `INFINO_API_KEY` |

Every engine gets the same settings as on AWS: the same machine, a 16 GB locked heap,
single-node discovery and no replicas.

All engines run from release tarballs on Ubuntu 24.04. The plugin's bridge library needs a
newer glibc than the OpenSearch container image has, and running every engine the same way
keeps the comparison even.

Both OpenSearch engines run without the bundle's `opensearch-neural-search` plugin. It and the
Infino plugin each register a query-phase searcher, and OpenSearch starts with only one.

## Run from GitHub

Actions → **Rally head-to-head (Cloud)** → Run workflow.

1. It builds the plugin and bridge from `infino-ai/opensearch-infino` at `plugin_ref`.
2. It races each engine in its own job, on its own runner and engine VMs.
3. It writes the comparison into the run's summary and uploads the run directories.

The plugin itself is never uploaded. Each race job's timeout caps its VMs' billing, and the
job deletes them however it ends.

It signs in to GCP as the other cloud workflows do. The `ci` environment needs:

- `GCP_WIF_PROVIDER`, `GCP_SERVICE_ACCOUNT` and `GCP_PROJECT_ID`;
- `OPENSEARCH_INFINO_TOKEN`, with read access to the plugin repository;
- `INFINO_API_KEY`, for the proxy-mode engine.

## Run from your machine

You need:

- `gcloud`, signed in, with rights to create instances and firewall rules in `PROJECT` (the
  default is gcloud's default project);
- `ssh`, `scp`, `curl` and Python 3;
- for the Infino engines, the plugin zip and bridge library from the plugin repository's
  `./scripts/build.sh`, built for `OS_VERSION`.

```bash
PLUGIN_ZIP=../opensearch-infino/plugin/build/distributions/infino-engine.zip \
BRIDGE_LIB=../opensearch-infino/infino-capi/target/release/libinfino_capi.so \
ENGINES=elasticsearch,opensearch,opensearch-infino \
TRACK=http_logs TEST_MODE=1 \
rally/benchmark.sh
```

`TEST_MODE=1` races 1,000 documents per corpus, a check of the setup in about half an hour.
Drop it for a measurement.

Proxy mode runs against a real account. The key goes to the VM over the SSH session's stdin
and into the OpenSearch keystore, never onto a command line:

```bash
INFINO_PLATFORM_URL=https://api.platform.infino.ws INFINO_DATABASE=<database> \
INFINO_API_KEY=<key> ENGINES=opensearch,opensearch-infino-proxy \
PLUGIN_ZIP=... BRIDGE_LIB=... TRACK=http_logs rally/benchmark.sh
```

Proxy mode creates the track's indices as tables in that database, so use one meant for tests.

Each run creates one VM per engine plus the runner. At the defaults that is
`n2-highmem-4` VMs with one local SSD each. They are deleted when the script exits (on success,
failure or Ctrl-C) unless `KEEP=1`. Their names start with `rally-` and they carry a
`delete-after` label, so the VM reaper sweeps any a killed run leaves.

## Settings

| Variable | Default | Meaning |
| --- | --- | --- |
| `ENGINES` | `elasticsearch,opensearch,opensearch-infino` | comma-separated engine names |
| `TRACK` | `nested` | the Rally track (the AWS default); `http_logs` is the logs track |
| `CHALLENGE` | the track's default | Rally challenge |
| `TRACK_REPOSITORY` | Rally's default | a track repository Rally knows by name |
| `CORPUS_SIZE`, `INGEST_PERCENTAGE`, `BULK_SIZE`, `TARGET_THROUGHPUT` | the track's | track parameters, as on AWS |
| `BULK_INDEXING_CLIENTS` | `8` | track parameter, as on AWS |
| `EXCLUDE_TASKS` | `indices-stats` | tasks no engine runs |
| `INFINO_EXCLUDE_TASKS` | none | tasks only the Infino engines skip |
| `INCLUDE_TASKS` | none | run only these tasks (overrides the exclusions) |
| `TEST_MODE` | `0` | `1` for Rally's test mode |
| `PARALLEL` | `0` | `1` races all engines at once from the one runner, as the AWS harness did; `0` races them one after another so none shares the runner |
| `ES_VERSION`, `OS_VERSION` | `9.1.3`, `3.9.0` | engine releases |
| `RALLY_DISTRIBUTION_VERSION` | `ES_VERSION` | Rally's `--distribution-version` for every engine, as on AWS; it picks the track revision |
| `HEAP_GB` | `16` | engine heap |
| `ENGINE_MACHINE`, `RUNNER_MACHINE` | `n2-highmem-4` | 4 vCPU, 32 GB: the i3en.xlarge the AWS harness used |
| `ENGINE_SSDS`, `RUNNER_SSDS` | `1` | 375 GB local NVMe SSDs per VM; several are striped |
| `PROJECT` | gcloud's default project | where the VMs go |
| `ZONE`, `NETWORK`, `SUBNET` | `us-central1-a`, `default`, none | where the VMs go |
| `VM_SA` | none | a service account for the VMs (with `storage-rw`); by default they have none, since they call no Google API |
| `INFINO_YAML` | none | an `infino.yaml` for the Infino engines' engine settings |
| `ENGINE_PRICE_PER_HOUR` | none | the hourly price of one engine VM; adds each engine's cost of the race |
| `KEEP` | `0` | `1` leaves the VMs running and prints the commands that delete them |
| `TTL_HOURS` | `12` | the `delete-after` label on every VM |
| `RUN` | `rally-<UTC time>` | the run's name, which must start with `rally-`: VM names, labels, firewall rules, results directory |

## Results

`rally/results/<run>/` holds, per engine:

- Rally's CSV report and log;
- the CPU and memory samples;
- the engine machine;
- the bytes of its data directory, and its `_cat/indices`.

It also holds `run.txt` (the run's settings), the setup logs, and `comparison.txt` /
`comparison.json`.

The comparison has:

- **Ingest** (`index-append`): min, median and max throughput; p50, p90, p99 latency.
- **Search**: for every task the track ran, mean throughput, p50 / p90 / p99 service time and
  error rate. Test mode runs too few iterations for percentiles below the 100th, so its
  search rows show throughput and errors only.
- **Storage**: the engine's data directory measured on its VM. Rally's own store size comes
  from the index stats, which do not count the Infino tables, so it is shown but not compared.
  A proxy-mode engine keeps its tables in the account, so its directory holds only the node's
  own state.
- **Resources**: the race's duration, plus:
  - the engine VM's CPU over all cores, as on AWS;
  - the engine process's CPU in core-seconds (the plugin's native threads run inside the
    OpenSearch process and are counted with it);
  - resident and virtual memory;
  - with `ENGINE_PRICE_PER_HOUR`, what each engine's race cost.
- **Pairwise**: each engine against each other on ingest median throughput and p50 service
  time per task, leaving out tasks either one answered with errors.
- **Tasks with errors**: a task an engine refuses shows here. The plugin refuses nested
  queries, so the `nested` track's nested tasks error on it; `terms_enum` is an
  Elasticsearch-only API and errors on both OpenSearch engines. Leave such a task out of every
  engine's comparison, or skip it with `INFINO_EXCLUDE_TASKS`.

`python3 rally/compare.py [--price-per-hour=<USD>] <run dir> [engine ...]` rewrites the
comparison from a run directory.

These are run directories, not published results: nothing here writes to `results/`.

## Engines you already run

`local.sh` races engines that are already up, one after another, with Rally on your machine,
and compares them. Install the fork first:

```bash
python3 -m venv ~/rally-venv
~/rally-venv/bin/pip install -e "git+https://github.com/infinohq/rally.git@fork#egg=esrally"
```

```bash
ESRALLY=~/rally-venv/bin/esrally TRACK=http_logs TEST_MODE=1 \
rally/local.sh opensearch=localhost:9201 opensearch-infino=localhost:9200
```

It takes no CPU or memory samples, and writes no storage figure.
