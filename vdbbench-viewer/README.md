# VDBBench results viewer

VectorDBBench's own Streamlit UI, packaged as a container and served on Cloud
Run at a stable public URL. Deployed on demand by
[`vdbbench-viewer.yml`](../.github/workflows/vdbbench-viewer.yml).

The app is upstream's, unchanged, minus the two write-mode pages — see
[Run-mode removal](#run-mode-removal). Results come from whatever is committed under
`vectordb_bench/results/` in the deployed ref of
[`infino-ai/VectorDBBench`](https://github.com/infino-ai/VectorDBBench), so a
merged PR there is the publish gate: nothing reaches the public URL until it is
in the fork's history.

## Deploying

```sh
gh workflow run vdbbench-viewer.yml --repo infino-ai/retrievalbench \
  -f vdbbench_ref=main
```

The run prints the service URL in its step summary. The URL is stable across
deploys — bookmark it once.

## Run-mode removal

The URL is unauthenticated, so `strip_run_mode.py` deletes upstream's two
write-mode pages during the image build:

- `pages/run_test.py` submits a benchmark run. On a public URL a stranger could
  make the container pull multi-gigabyte datasets and drive load at an endpoint
  of their choosing, billed to us.
- `pages/custom.py` writes caller-supplied dataset config into the container.

Every edit is anchored to exact upstream text and the build aborts on a miss, so
merging `upstream/main` into the fork can break this build. That is the intended
failure: re-anchor `strip_run_mode.py` against the new source rather than
loosening the match.

## Cost

`--min-instances=0`, so nothing runs and nothing is billed while no one is
looking. An open browser tab holds a Streamlit websocket, which Cloud Run bills
as an in-flight request for its whole lifetime; `--timeout=900` caps an
abandoned tab at fifteen minutes. Images are pruned by an Artifact Registry
cleanup policy that keeps the five most recent.

Cold start is dominated by pulling the 1.8 GB image, not by the app: importing
`vectordb_bench.interface` takes 0.6 s, since the vector-store clients load
lazily. Raise `--min-instances` to 1 only if that pull becomes annoying — it
converts an idle service from free to always-billed.

## One-time GCP setup

The workflow creates the Artifact Registry repository and its cleanup policy on
first run. Three things it cannot create for itself:

1. **Runtime service account** — the identity the public container runs as. It
   needs no roles; the app only reads files baked into the image.

   ```sh
   gcloud iam service-accounts create vdbbench-viewer-runtime \
     --display-name="VDBBench viewer (Cloud Run runtime)" --project="$PROJECT"
   ```

   Leaving this unset would fall back to the default compute service account,
   which holds project Editor — never acceptable for an unauthenticated
   container. The workflow fails rather than defaulting.

2. **Deploy service account roles** — granted to the account behind
   `secrets.GCP_SERVICE_ACCOUNT`:

   | Role | Needed for |
   |---|---|
   | `roles/run.admin` | deploy the service, make it public |
   | `roles/artifactregistry.admin` | create the repo, push images, set the cleanup policy |
   | `roles/iam.serviceAccountUser` on the runtime SA | run the service as that identity |

3. **Variables on the `ci` environment**, where the cloud secrets already live:

   ```sh
   gh variable set GCP_PROJECT_ID --env ci --body "$PROJECT" \
     --repo infino-ai/retrievalbench
   gh variable set VDBBENCH_VIEWER_RUNTIME_SA --env ci \
     --body "vdbbench-viewer-runtime@$PROJECT.iam.gserviceaccount.com" \
     --repo infino-ai/retrievalbench
   ```

   Both are overridable per run via workflow inputs.

If the deploy fails on `--allow-unauthenticated`, the project's
`constraints/iam.allowedPolicyMemberDomains` org policy is rejecting `allUsers`
and needs an exception for this service.

## Building locally

```sh
git clone --depth 1 https://github.com/infino-ai/VectorDBBench src
docker build -t vdbbench-viewer .
docker run --rm -p 8501:8501 vdbbench-viewer
```

Then open <http://localhost:8501>. `src/` is git-ignored.

### Previewing results that are not committed yet

`src/vectordb_bench/results/` is where a local `init_bench` run writes, one
directory per engine, and it is the path the image reads. Mounting it over the
baked-in copy shows uncommitted results without a rebuild:

```sh
docker run --rm -p 8501:8501 \
  -v "$PWD/src/vectordb_bench/results:/app/vectordb_bench/results:ro" \
  vdbbench-viewer
```

Results from a CI run go to the same place. `results/Infino/` is an empty
directory upstream, so git does not carry it and a fresh clone needs it created:

```sh
gh run download <run-id> --repo infino-ai/retrievalbench \
  -n vectordbbench-results-vector -n vectordbbench-results-fts -D /tmp/vdb
mkdir -p src/vectordb_bench/results/Infino
find /tmp/vdb -name 'result_*.json' -exec cp {} src/vectordb_bench/results/Infino/ \;
```

The deployed service ignores all of this — it only ever shows what the fork has
committed.
