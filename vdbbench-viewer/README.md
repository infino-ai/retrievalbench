# VDBBench results viewer

**<https://vdbbench-viewer-q6unoyyhua-uc.a.run.app>**

Public web UI comparing Infino against every other vector backend on QPS,
recall, latency, cost-per-query and full-text search. VectorDBBench's own
Streamlit app, served on Cloud Run at a URL that does not change.

```
bench run  ──►  results bucket  ──►  viewer deploy  ──►  live
   (opt-in)                           (automatic)
```

Publishing is opt-in; everything after it is automatic.

## Publish a run's numbers

Add `publish_results=true` to any bench dispatch:

```sh
gh workflow run vectordbbench-cloud.yml --repo infino-ai/retrievalbench \
  -f mode=both -f publish_results=true
```

Also on `vdbbench-vector.yml` and `vdbbench-fts.yml` for a single leg. The
toggle defaults to off, so ordinary runs publish nothing.

Results go to the bucket, then the viewer redeploys — the page is current when
the run finishes. The run summary shows the hardware, parameters and metrics.

Publishing is all-or-nothing: skipped unless every requested leg succeeded, and
aborted before the upload if any case failed. A partial run would refresh half
the page and leave the rest silently stale.

Deploys serialize on one concurrency group. A superseded deploy loses nothing —
each one syncs the whole bucket.

To pull a number back, delete the object and redeploy:

```sh
gcloud storage rm gs://vdbbench-results-887234897611/Infino/<file>.json \
  --project=infino-ai-engine
```

## Deploy the page

Publishing triggers this automatically. Dispatch it by hand to pick up a new
`vdbbench_ref`, or to restore the page after deleting a result:

```sh
gh workflow run vdbbench-viewer.yml --repo infino-ai/retrievalbench
```

About five minutes. The URL is in the run's step summary.

| Input | Default | Purpose |
|---|---|---|
| `vdbbench_ref` | `main` | which `infino-ai/VectorDBBench` ref supplies the app and peer results |
| `region` | `us-central1` | Cloud Run and Artifact Registry region |
| `gcp_project` | `vars.GCP_PROJECT_ID` | override the target project |
| `runtime_service_account` | `vars.VDBBENCH_VIEWER_RUNTIME_SA` | override the container identity |

## What the page shows

| Source | Contents |
|---|---|
| results bucket, synced at deploy | Infino's numbers, overlaid onto the tree |
| `vectordb_bench/results/` in the fork | every peer backend, as upstream ships them |

Keeping our numbers in a private bucket rather than the public fork leaves raw
results unpublished and the fork clean for upstream merges. `results/` here is
a sync target; its JSON is git-ignored.

Nine read-only pages. The two upstream pages that *start* benchmark runs are
removed from the image — see [Image internals](#image-internals).

## Local development

```sh
git clone --depth 1 https://github.com/infino-ai/VectorDBBench src
docker build -t vdbbench-viewer .
docker run --rm -p 8501:8501 vdbbench-viewer
```

Serves on <http://localhost:8501>. `src/` is git-ignored.

To see what the deployed page would show, sync and rebuild. The overlay is the
last image layer, so this takes under a second:

```sh
gcloud storage rsync gs://vdbbench-results-887234897611/Infino results/Infino \
  --project=infino-ai-engine
docker build -t vdbbench-viewer . && docker run --rm -p 8501:8501 vdbbench-viewer
```

Dropping unpublished JSON into `results/Infino/` works the same way.

A local `init_bench` run writes to `src/vectordb_bench/results/` instead; mount
that over the baked-in tree to view it without copying:

```sh
docker run --rm -p 8501:8501 \
  -v "$PWD/src/vectordb_bench/results:/app/vectordb_bench/results:ro" \
  vdbbench-viewer
```

## Running costs

Idle is free — `--min-instances=0`. The only standing charge is image storage,
five images at ~1.8 GB, under a dollar a month.

An open tab holds a Streamlit websocket, which Cloud Run bills as an in-flight
request for its lifetime. `--timeout=900` drops abandoned tabs; `--max-instances=2`
caps a scraper.

Cold start is the 1.8 GB image pull; the app imports in 0.6 s. `--min-instances=1`
removes the pull and makes an idle service always-billed.

## Taking it down

Teardown saves under a dollar a month — do it to take the page offline, not to
stop charges that are not accruing.

Offline, keeping the service and URL:

```sh
gcloud run services remove-iam-policy-binding vdbbench-viewer \
  --region=us-central1 --project=infino-ai-engine \
  --member=allUsers --role=roles/run.invoker
```

Delete the service:

```sh
gcloud run services delete vdbbench-viewer \
  --region=us-central1 --project=infino-ai-engine --quiet
```

A later redeploy normally returns the same URL, since it derives from project,
region and service name — likely, not guaranteed. If the link is published
anywhere, take it offline instead.

<details>
<summary>Remove everything, including images and identities</summary>

```sh
PROJECT=infino-ai-engine
gcloud run services delete vdbbench-viewer --region=us-central1 --project="$PROJECT" --quiet
gcloud artifacts repositories delete vdbbench-viewer --location=us-central1 --project="$PROJECT" --quiet
# The bucket holds the only copy of every published result.
gcloud storage rm -r "gs://vdbbench-results-887234897611" --project="$PROJECT"
gcloud iam service-accounts delete "vdbbench-ci@$PROJECT.iam.gserviceaccount.com" --project="$PROJECT" --quiet
gcloud iam service-accounts delete "vdbbench-viewer-runtime@$PROJECT.iam.gserviceaccount.com" --project="$PROJECT" --quiet

for V in GCP_PROJECT_ID VDBBENCH_RESULTS_BUCKET VDBBENCH_VIEWER_DEPLOY_SA VDBBENCH_VIEWER_RUNTIME_SA; do
  gh variable delete "$V" --env ci --repo infino-ai/retrievalbench
done
```

Deleting the accounts drops their role bindings. Leave `run.googleapis.com` and
`artifactregistry.googleapis.com` enabled — they are shared with the rest of the
project.

</details>

## Image internals

Dependencies resolve from the fork's `pyproject`, so no second requirements file
can drift. The package is then uninstalled and the app runs from the source
tree, preserving `results/` and Streamlit's `pages/` layout.

`strip_run_mode.py` removes two pages, because the URL is unauthenticated:

| Page | Why it cannot ship |
|---|---|
| `pages/run_test.py` | starts a benchmark run — anyone could make the container pull multi-gigabyte datasets and drive load at an endpoint they choose, billed to us |
| `pages/custom.py` | writes caller-supplied dataset config into the container |

Each edit is anchored to exact upstream text and the build fails on a miss, so
merging `upstream/main` into the fork can break this build. Re-anchor
`strip_run_mode.py` against the new source rather than loosening the match.

Two checks run before the image is pushed: one asserts the removed pages are
absent, the other renders all nine pages through Streamlit's `AppTest`.

## Infrastructure

Deployed to `infino-ai-engine` over Workload Identity Federation. No service
account keys exist.

| Resource | Detail |
|---|---|
| Cloud Run service | `vdbbench-viewer`, `us-central1`, unauthenticated |
| Artifact Registry | `vdbbench-viewer`, Docker, tagged by fork commit |
| Results bucket | `vdbbench-results-887234897611`, `us-central1`, public access prevented |
| Deploy account | `vdbbench-ci` — `run.admin` on the project, `artifactregistry.writer` on the one registry, `storage.objectAdmin` on the results bucket, `iam.serviceAccountUser` on the runtime account |
| Runtime account | `vdbbench-viewer-runtime` — no roles |
| Config | `GCP_PROJECT_ID`, `VDBBENCH_RESULTS_BUCKET`, `VDBBENCH_VIEWER_DEPLOY_SA`, `VDBBENCH_VIEWER_RUNTIME_SA` on the `ci` environment |

The runtime account holds no roles because the container only reads files baked
into its own image; without it Cloud Run defaults to the compute account, which
carries project Editor. The deploy account may only push to the one registry, so
the registry and its cleanup policy are created out of band.

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

BUCKET="gs://vdbbench-results-$PNUM"
gcloud storage buckets create "$BUCKET" --project="$PROJECT" --location="$REGION" \
  --uniform-bucket-level-access --public-access-prevention
gcloud storage buckets add-iam-policy-binding "$BUCKET" --project="$PROJECT" \
  --member="serviceAccount:$DEPLOY_SA" --role=roles/storage.objectAdmin

gcloud projects add-iam-policy-binding "$PROJECT" \
  --member="serviceAccount:$DEPLOY_SA" --role=roles/run.admin --condition=None
gcloud iam service-accounts add-iam-policy-binding "$RUNTIME_SA" --project="$PROJECT" \
  --member="serviceAccount:$DEPLOY_SA" --role=roles/iam.serviceAccountUser

gh variable set GCP_PROJECT_ID --env ci --body "$PROJECT" --repo infino-ai/retrievalbench
gh variable set VDBBENCH_RESULTS_BUCKET --env ci --body "$BUCKET" --repo infino-ai/retrievalbench
gh variable set VDBBENCH_VIEWER_DEPLOY_SA --env ci --body "$DEPLOY_SA" --repo infino-ai/retrievalbench
gh variable set VDBBENCH_VIEWER_RUNTIME_SA --env ci --body "$RUNTIME_SA" --repo infino-ai/retrievalbench
```

</details>

## Troubleshooting

| Failure | Cause |
|---|---|
| `... is unset on the ci environment` | a `ci` variable is missing; see [Infrastructure](#infrastructure) |
| `Artifact Registry 'vdbbench-viewer' missing` | registry deleted, or deploying to a new region |
| `the run produced no result JSON to publish` | the bench failed before writing results |
| `N case(s) did not pass; refusing to publish` | the run had a failed or out-of-range case; nothing was uploaded |
| publish skipped despite `publish_results=true` | a requested leg failed, or none succeeded |
| publish fails on `storage.objects.create` | the deploy account lost `storage.objectAdmin` on the results bucket |
| page missing recent numbers | the run published but no deploy followed |
| build fails in `strip_run_mode.py` | upstream moved the code an anchor targets; re-anchor it |
| `pages raised on render` | a page throws — most often a dependency the fork does not declare |
| deploy rejected on `--allow-unauthenticated` | `constraints/iam.allowedPolicyMemberDomains` is blocking `allUsers` |
