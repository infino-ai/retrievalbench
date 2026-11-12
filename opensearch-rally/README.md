# OpenSearch Rally head-to-head on Google Cloud

Races Elasticsearch, stock OpenSearch and OpenSearch with the
[Infino for OpenSearch](https://github.com/infino-ai/opensearch-infino) plugin against each
other with [Rally](https://github.com/elastic/rally), one VM per engine and one Rally runner,
and reports ingest, every search task, storage, CPU and memory side by side.

Its defaults reproduce an earlier Rally setup Infino ran on AWS: the engines in Docker on
i3en.xlarge instances, provisioned with Terraform. The comments call it "the AWS setup", and
keep its machine shape, heap, track parameters and reports so the numbers stay comparable.

These runs are not published to [`results/`](../results/): a run writes its report to the
workflow's summary and artifacts, or to `opensearch-rally/results/<run>/` locally.

| File | Runs on | What it does |
| --- | --- | --- |
| `benchmark.sh` | your machine | creates the VMs and firewall rules, sets everything up, races each engine, collects results, writes the comparison, deletes the VMs |
| `engine-setup.sh` | each engine VM | puts the data on the local SSDs, installs the engine from its release tarball (and the plugin, bridge library and `infino.yaml` for the Infino engines), starts it on :9200 |
| `runner-setup.sh` | the runner VM | installs Infino's [Rally fork](https://github.com/infinohq/rally) (branch `fork`) on the local SSD |
| `race.sh` | the runner, or your machine | one `esrally race` against one engine, as the AWS setup ran it |
| `sampler.py` | each engine VM | CPU and memory samples every 5 s during the race |
| `compare.py` | your machine | the comparison report (`comparison.txt`, `comparison.json`) |
| `local.sh` | your machine | races engines that are already running, then compares them |
| `.github/workflows/opensearch-rally.yml` | GitHub Actions | builds the plugin, runs `benchmark.sh` once per engine, compares |

## Engines

| Name | What runs |
| --- | --- |
| `elasticsearch` | Elasticsearch `ES_VERSION`, security off |
| `opensearch` | OpenSearch `OS_VERSION`, security off (Lucene) |
| `opensearch-infino` | OpenSearch `OS_VERSION` with the plugin in default mode: tables on the VM's local SSD |
| `opensearch-infino-proxy` | OpenSearch `OS_VERSION` with the plugin in proxy mode: tables in the Infino Cloud account `INFINO_PLATFORM_URL` / `INFINO_DATABASE`, key `INFINO_API_KEY` |

Every engine gets the same machine, a 16 GB locked heap, single-node discovery and no
replicas, as on AWS. All engines run from tarballs on Ubuntu 24.04: the bridge library needs a
newer glibc than the OpenSearch container image has, and running all of them the same way
keeps the comparison even.

## Run from GitHub

The **OpenSearch Rally (GCP)** workflow (`workflow_dispatch` only) runs everything below with
no local setup: it builds the plugin zip and bridge at `plugin_ref` of `plugin_repo`
(`infino-ai/opensearch-infino`), races each engine in its own job on its own runner and engine
VMs (`benchmark.sh` with one engine), then writes the comparison into the run's summary and
uploads every run directory as an artifact. Each race job's timeout is the ceiling on its VMs'
billing.

Like the other cloud workflows, it signs in to GCP with the `ci` environment's
`GCP_WIF_PROVIDER` and `GCP_SERVICE_ACCOUNT`, and creates the VMs in `vars.GCP_PROJECT_ID`
(else the service account's project). The proxy-mode engine also needs the secret
`INFINO_API_KEY` in the `ci` environment.

Dispatches serialize, and a run is never cancelled mid-flight: a cancelled job can leave its
VMs billing. Every VM carries a `delete-after` label, and [`vm-reaper.yml`](../.github/workflows/vm-reaper.yml)
deletes expired `rally-*` VMs and their firewall rules.

## Before you run from your machine

- `gcloud` signed in with rights to create instances and firewall rules in `PROJECT`
  (required), whose network `NETWORK` (default `default`) has the subnet `SUBNET` (default
  `default`) in `ZONE`'s region.
- For the Infino engines, the plugin zip and the bridge library built for the plugin's
  OpenSearch release (`./scripts/build.sh` in [opensearch-infino](https://github.com/infino-ai/opensearch-infino)): `PLUGIN_ZIP` and
  `BRIDGE_LIB`. Match `OS_VERSION` to the release the zip was built for.
- `ssh`, `scp`, `curl` and Python 3 on your machine.

Each run creates one VM per engine plus the runner. At the defaults that is four
`n2-highmem-4` VMs with one local SSD each; they are deleted when the run ends, on success,
failure or Ctrl-C, unless `KEEP=1`.

## Run

```bash
PROJECT=<gcp project> \
PLUGIN_ZIP=<opensearch-infino>/plugin/build/distributions/infino-engine.zip \
BRIDGE_LIB=<opensearch-infino>/infino-capi/target/release/libinfino_capi.so \
ENGINES=elasticsearch,opensearch,opensearch-infino \
TRACK=http_logs \
opensearch-rally/benchmark.sh
```

Proxy mode against a real account (the key is passed to the VM over the SSH session's stdin
and stored in the OpenSearch keystore, never on a command line):

```bash
INFINO_PLATFORM_URL=https://api.platform.infino.ws INFINO_DATABASE=<database> \
INFINO_API_KEY=<key> ENGINES=opensearch,opensearch-infino-proxy \
PLUGIN_ZIP=... BRIDGE_LIB=... TRACK=http_logs opensearch-rally/benchmark.sh
```

A short end-to-end check, with Rally's test mode (1,000 documents per corpus):
`TEST_MODE=1 KEEP=0 ... opensearch-rally/benchmark.sh`.

## Knobs

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
| `PARALLEL` | `0` | `1` races all engines at once from the one runner, as the AWS setup did; `0` races them one after another so none shares the runner |
| `ES_VERSION`, `OS_VERSION` | `9.1.3`, `3.9.0` | engine releases |
| `RALLY_DISTRIBUTION_VERSION` | `ES_VERSION` | Rally's `--distribution-version` for every engine, as on AWS; it picks the track revision |
| `HEAP_GB` | `16` | engine heap |
| `ENGINE_MACHINE`, `RUNNER_MACHINE` | `n2-highmem-4` | 4 vCPU, 32 GB: the i3en.xlarge the AWS setup used |
| `ENGINE_SSDS`, `RUNNER_SSDS` | `1` | 375 GB local NVMe SSDs per VM; several are striped |
| `PROJECT`, `ZONE`, `NETWORK`, `SUBNET` | see above | where the VMs go |
| `VM_SA` | none | a service account for the VMs (with `storage-rw`); by default they have none, since they call no Google API |
| `INFINO_YAML` | none | an `infino.yaml` for the Infino engines' engine settings |
| `INFINO_MODES` | `1` | `0` for a plugin build without index modes, where proxy mode is the only mode |
| `ENGINE_PRICE_PER_HOUR` | none | the hourly price of one engine VM; adds each engine's cost of the race |
| `RESULTS_BUCKET` | none | a `gs://` bucket the run directory is copied to |
| `KEEP` | `0` | `1` leaves the VMs running and prints the commands that delete them |
| `TTL_HOURS` | `12` | the `delete-after` label on every VM |
| `RUN` | `rally-<UTC time>` | the run's name: VM names, labels, firewall rules, results directory. Keep the `rally-` prefix, which `vm-reaper.yml` sweeps |

## Results

`opensearch-rally/results/<run>/` holds, per engine: Rally's CSV report and log, the CPU and memory
samples, the engine machine, the bytes of its data directory and its `_cat/indices`; plus
`run.txt` (the run's settings), the setup logs, and `comparison.txt` / `comparison.json`.

The comparison has:

- **Ingest** (`index-append`): min, median and max throughput; p50, p90, p99 latency.
- **Search**: for every task the track ran, mean throughput, p50 / p90 / p99 service time and
  error rate.
- **Storage**: the engine's data directory measured on its VM. Rally's own store size comes
  from the index stats, which do not count the Infino tables, so it is shown but not compared.
  A proxy-mode engine keeps its tables in the account, so its directory holds only the node's
  own state.
- **Resources**: the engine VM's CPU over all cores (as on AWS), the engine process's CPU in
  core-seconds (the plugin's native threads run inside the OpenSearch process and are
  counted with it), resident and virtual memory, and the race's duration; with
  `ENGINE_PRICE_PER_HOUR`, what each engine's race cost.
- **Pairwise**: each engine against each other on ingest median throughput and p50 service
  time per task, leaving out tasks either one answered with errors.
- **Tasks with errors**: a task an engine refuses (the plugin refuses nested queries, so the
  `nested` track's nested tasks error on it) shows here. Leave it out of every engine's
  comparison, or skip it with `INFINO_EXCLUDE_TASKS`.

`python3 opensearch-rally/compare.py [--price-per-hour=<USD>] <run dir> [engine ...]` rewrites the
comparison from a run directory.

## Engines you already run

`local.sh` races engines that are already up, one after another, with Rally on your machine,
and compares them. Install the fork first:

```bash
python3 -m venv ~/rally-venv
~/rally-venv/bin/pip install -e "git+https://github.com/infinohq/rally.git@fork#egg=esrally"
```

```bash
ESRALLY=~/rally-venv/bin/esrally TRACK=http_logs TEST_MODE=1 \
opensearch-rally/local.sh opensearch=localhost:9201 opensearch-infino=localhost:9200
```

It takes no CPU or memory samples, and writes no storage figure.

## Cleaning up

`benchmark.sh` deletes its VMs and firewall rules when it exits. A run that was killed before
its exit handler ran leaves VMs labelled `purpose=rally-bench,run-id=<run>`; list and delete
them with

```bash
gcloud --project "$PROJECT" compute instances list --filter labels.purpose=rally-bench
```

```bash
gcloud --project "$PROJECT" compute instances delete --zone "$ZONE" <names>
```

and the run's firewall rules, `<run>-ssh` and `<run>-int`, with
`gcloud compute firewall-rules delete`. In the project `vm-reaper.yml` sweeps, it deletes them
once their `delete-after` label has passed.
