# Read-only VectorDBBench results viewer, served unauthenticated on Cloud Run.
#
# Build context is this directory. `src/` must hold a checkout of
# infino-ai/VectorDBBench; the deploy workflow puts it there.
#
#   git clone --depth 1 https://github.com/infino-ai/VectorDBBench src
#   docker build -t vdbbench-viewer .

FROM python:3.11-slim-bookworm AS build

ENV PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1

# Present for dependencies without a cp311 manylinux wheel; discarded with this stage.
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential \
 && rm -rf /var/lib/apt/lists/*

COPY src/ /src/
COPY harden.py /harden.py

# Aborts the build if the run-mode UI cannot be located and removed.
RUN python /harden.py /src/vectordb_bench

# Installing resolves the dependency set from pyproject, so this image and a
# bench run agree on versions without a second requirements file to drift.
# The package itself is then dropped: the app runs from the source tree at
# /app, which keeps results/ and the Streamlit pages/ layout intact.
RUN python -m venv /venv \
 && /venv/bin/pip install /src \
 && /venv/bin/pip uninstall --yes vectordb-bench


FROM python:3.11-slim-bookworm

COPY --from=build /venv /venv
COPY --from=build /src/vectordb_bench /app/vectordb_bench

ENV PATH=/venv/bin:$PATH \
    PYTHONPATH=/app \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    HOME=/home/viewer \
    # vectordb_bench opens this at import and creates its parent, so it has to
    # be somewhere the unprivileged user can write.
    LOG_FILE=/tmp/vdbbench-viewer/vectordb_bench.log

RUN useradd --create-home --uid 10001 viewer
USER viewer
WORKDIR /app

# Matches `gcloud run deploy --port`; Cloud Run sets $PORT to the same value.
EXPOSE 8501

HEALTHCHECK --interval=30s --timeout=5s --start-period=60s \
  CMD ["python", "-c", "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8501/_stcore/health').read()"]

ENTRYPOINT ["streamlit", "run", "/app/vectordb_bench/frontend/vdbbench.py", \
            "--server.port=8501", \
            "--server.address=0.0.0.0", \
            "--server.headless=true", \
            "--server.fileWatcherType=none", \
            "--browser.gatherUsageStats=false", \
            "--client.toolbarMode=viewer"]
