# VDBBench results viewer

Public web UI for VectorDBBench results — Infino against every other backend,
on QPS, recall, latency, cost-per-query and full-text search.

Upstream's Streamlit app, packaged as a container and served on Cloud Run at a
stable URL. Nine read-only pages; the two pages that start benchmark runs are
removed from the image.

## Deploy

```sh
gh workflow run vdbbench-viewer.yml --repo infino-ai/retrievalbench \
  -f vdbbench_ref=main
```

Takes about five minutes. The service URL appears in the run's step summary and
does not change between deploys.

| Input | Default | Purpose |
|---|---|---|
| `vdbbench_ref` | `main` | which `infino-ai/VectorDBBench` ref to publish |
| `region` | `us-central1` | Cloud Run and Artifact Registry region |
| `gcp_project` | `vars.GCP_PROJECT_ID` | override the target project |
| `runtime_service_account` | `vars.VDBBENCH_VIEWER_RUNTIME_SA` | override the container identity |

## What gets published

Results are the `result_*.json` files committed under `vectordb_bench/results/`
in the deployed ref of [`infino-ai/VectorDBBench`](https://github.com/infino-ai/VectorDBBench),
one directory per backend. Merging there is the publish gate — the URL is
public, so nothing appears on it until it is in the fork's history.

Bench runs write results as CI artifacts, which expire. Getting numbers onto the
page means opening a PR against the fork.

## Local development

```sh
git clone --depth 1 https://github.com/infino-ai/VectorDBBench src
docker build -t vdbbench-viewer .
docker run --rm -p 8501:8501 vdbbench-viewer
```

Serves on <http://localhost:8501>. `src/` is git-ignored.

To preview results before they are committed, mount them over the baked-in copy:

```sh
docker run --rm -p 8501:8501 \
  -v "$PWD/src/vectordb_bench/results:/app/vectordb_bench/results:ro" \
  vdbbench-viewer
```

`src/vectordb_bench/results/` is where a local `init_bench` run writes. For
results from CI, note that `results/Infino/` is empty upstream, so git does not
carry it and a fresh clone needs it created:

```sh
gh run download <run-id> --repo infino-ai/retrievalbench \
  -n vectordbbench-results-vector -n vectordbbench-results-fts -D /tmp/vdb
mkdir -p src/vectordb_bench/results/Infino
find /tmp/vdb -name 'result_*.json' -exec cp {} src/vectordb_bench/results/Infino/ \;
```

## How the image is built

Dependencies resolve from the fork's `pyproject`, so no second requirements file
can drift from it. The package is then uninstalled and the app runs from the
source tree, which keeps `results/` and Streamlit's `pages/` layout intact.

`strip_run_mode.py` removes two pages during the build:

| Page | Why it cannot ship |
|---|---|
| `pages/run_test.py` | starts a benchmark run — on a public URL, anyone could make the container pull multi-gigabyte datasets and drive load at an endpoint they choose, billed to us |
| `pages/custom.py` | writes caller-supplied dataset config into the container |

Each edit is anchored to exact upstream text and the build fails on a miss, so
merging `upstream/main` into the fork can break this build. Re-anchor
`strip_run_mode.py` against the new source rather than loosening the match.

Two deploy-time checks guard the result: one asserts the removed pages are
absent from the image, the other renders all nine pages through Streamlit's
`AppTest`. Both run before the image is pushed.

## Cost

Idle costs nothing — `--min-instances=0`, so no container runs when no one is
looking. An open browser tab holds a Streamlit websocket, which Cloud Run bills
as an in-flight request for its whole lifetime; `--timeout=900` drops abandoned
tabs after fifteen minutes. `--max-instances=2` caps the damage from a scraper.
Images are pruned to the five most recent.

Cold start is dominated by pulling the 1.8 GB image; the app itself imports in
0.6 s. Raising `--min-instances` to 1 removes the pull but converts an idle
service from free to always-billed.

## Infrastructure

Deployed to `infino-ai-engine` by GitHub Actions over Workload Identity
Federation. No service account keys exist.

| Resource | Detail |
|---|---|
| Cloud Run service | `vdbbench-viewer`, `us-central1`, unauthenticated |
| Artifact Registry | `vdbbench-viewer`, Docker, tagged by fork commit |
| Deploy account | `vdbbench-ci` — `run.admin` on the project, `artifactregistry.writer` on the one registry, `iam.serviceAccountUser` on the runtime account |
| Runtime account | `vdbbench-viewer-runtime` — no roles |
| Config | `GCP_PROJECT_ID`, `VDBBENCH_VIEWER_DEPLOY_SA`, `VDBBENCH_VIEWER_RUNTIME_SA` on the `ci` environment |

The runtime account holds no roles because the container only reads files baked
into its own image; without it Cloud Run would default to the compute account,
which carries project Editor. The deploy account may only push to the one
registry, so the registry and its cleanup policy are created here rather than by
the workflow.

<details>
<summary>Recreating this in another project</summary>

```sh
PROJECT=infino-ai-engine
PNUM=$(gcloud projects describe "$PROJECT" --format='value(projectNumber)')
REGION=us-central1
DEPLOY_SA="vdbbench-ci@$PROJECT.iam.gserviceaccount.com"
RUNTIME_SA="vdbbench-viewer-runtime@$PROJECT.iam.gserviceaccount.com"

gcloud services enable run.googleapis.com artifactregistry.googleapis.com --project="$PROJECT"

gcloud iam service-accounts create vdbbench-ci \
  --display-name="VDBBench viewer (Cloud Run deploy)" --project="$PROJECT"
gcloud iam service-accounts create vdbbench-viewer-runtime \
  --display-name="VDBBench viewer (Cloud Run runtime)" --project="$PROJECT"

# Only this repository's workflows may become the deploy account.
gcloud iam service-accounts add-iam-policy-binding "$DEPLOY_SA" --project="$PROJECT" \
  --role=roles/iam.workloadIdentityUser \
  --member="principalSet://iam.googleapis.com/projects/$PNUM/locations/global/workloadIdentityPools/github-actions/attribute.repository/infino-ai/retrievalbench"

gcloud artifacts repositories create vdbbench-viewer \
  --repository-format=docker --location="$REGION" --project="$PROJECT" \
  --description="VDBBench results viewer images"
gcloud artifacts repositories add-iam-policy-binding vdbbench-viewer \
  --location="$REGION" --project="$PROJECT" \
  --member="serviceAccount:$DEPLOY_SA" --role=roles/artifactregistry.writer

cat > /tmp/cleanup-policy.json <<'JSON'
[
  {"name": "keep-recent", "action": {"type": "Keep"}, "mostRecentVersions": {"keepCount": 5}},
  {"name": "delete-superseded", "action": {"type": "Delete"}, "condition": {"tagState": "ANY", "olderThan": "7d"}}
]
JSON
gcloud artifacts repositories set-cleanup-policies vdbbench-viewer \
  --location="$REGION" --project="$PROJECT" --policy=/tmp/cleanup-policy.json --no-dry-run

gcloud projects add-iam-policy-binding "$PROJECT" \
  --member="serviceAccount:$DEPLOY_SA" --role=roles/run.admin --condition=None
gcloud iam service-accounts add-iam-policy-binding "$RUNTIME_SA" --project="$PROJECT" \
  --member="serviceAccount:$DEPLOY_SA" --role=roles/iam.serviceAccountUser

gh variable set GCP_PROJECT_ID --env ci --body "$PROJECT" --repo infino-ai/retrievalbench
gh variable set VDBBENCH_VIEWER_DEPLOY_SA --env ci --body "$DEPLOY_SA" --repo infino-ai/retrievalbench
gh variable set VDBBENCH_VIEWER_RUNTIME_SA --env ci --body "$RUNTIME_SA" --repo infino-ai/retrievalbench
```

</details>

## Troubleshooting

| Failure | Cause |
|---|---|
| `... is unset on the ci environment` | a `ci` variable is missing; see Infrastructure above |
| `Artifact Registry 'vdbbench-viewer' missing` | registry deleted, or deploying to a new region |
| build fails in `strip_run_mode.py` | upstream moved the code an anchor targets; re-anchor it |
| `pages raised on render` | a page throws — most often a dependency the fork does not declare |
| deploy rejected on `--allow-unauthenticated` | the `constraints/iam.allowedPolicyMemberDomains` org policy is blocking `allUsers` |
