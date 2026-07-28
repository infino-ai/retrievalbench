# Read-only VectorDBBench results viewer, served unauthenticated on Cloud Run.
# Build context is this directory; src/ must hold a VectorDBBench checkout.

FROM python:3.11-slim-bookworm AS build

# The context excludes src/.git, leaving setuptools-scm no version to derive.
ENV PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1 \
    SETUPTOOLS_SCM_PRETEND_VERSION_FOR_VECTORDB_BENCH=0.0.0+viewer

# For dependencies without a cp311 wheel; discarded with this stage.
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential \
 && rm -rf /var/lib/apt/lists/*

COPY src/ /src/
COPY strip_run_mode.py /strip_run_mode.py

# Aborts the build if the run-mode UI cannot be located and removed.
RUN python /strip_run_mode.py /src/vectordb_bench

# Installing resolves dependencies from pyproject, so no second requirements
# file can drift from it. The package then goes: the app runs from the source
# tree at /app, which keeps results/ and the Streamlit pages/ layout intact.
RUN python -m venv /venv \
 && /venv/bin/pip install /src \
 && /venv/bin/pip uninstall --yes vectordb-bench

# components/qps_recall/charts.py imports matplotlib, which upstream's pyproject
# never declares, and calls plt.cm.get_cmap, dropped in 3.9. Both the install
# and the bound go once the fork declares the dep and uses matplotlib.colormaps.
RUN /venv/bin/pip install 'matplotlib<3.9'


FROM python:3.11-slim-bookworm

COPY --from=build /venv /venv
COPY --from=build /src/vectordb_bench /app/vectordb_bench

# vectordb_bench creates LOG_FILE's parent at import, so it must sit somewhere
# the unprivileged user can write.
ENV PATH=/venv/bin:$PATH \
    PYTHONPATH=/app \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    HOME=/home/viewer \
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
