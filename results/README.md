# Benchmark Results

This directory contains timestamped JSON files of benchmark results from each benchmark run.

## File Format

Each file is named with the ISO 8601 timestamp of when the benchmark was run:
```
YYYY-MM-DD_HH-MM-SS.json
```

## JSON Structure

Each results file contains:

```json
{
  "timestamp": 1234567890,
  "iso_timestamp": "2024-05-27_14-30-45",
  "results": {
    "superfile_fts_build__tantivy_1thread_1000000docs": {
      "commit_hash": "abc1234",
      "os": "macos",
      "mean_ns": 1234567890.0,
      "peak_rss_bytes": 1073741824
    },
    "superfile_fts_build__tantivy_default_threads_1000000docs": {
      "commit_hash": "abc1234",
      "os": "macos",
      "mean_ns": 12345678900.0,
      "peak_rss_bytes": 1610612736
    },
    "superfile_fts_search__single_rare_tantivy_top10": {
      "commit_hash": "abc1234",
      "os": "macos",
      "mean_ns": 1234567.0,
      "peak_rss_bytes": 1073741824
    }
  }
}
```

### Fields

- `timestamp`: Unix timestamp of when the benchmark was run
- `iso_timestamp`: Human-readable ISO 8601 timestamp
- `results`: Object mapping fully qualified benchmark names to result objects
  - **key**: `{group}___{benchmark_id}` (e.g., `superfile_fts_build__tantivy_1thread_1000000docs`)
  - **value**: result object containing:
    - `commit_hash`: 7-character git commit hash
    - `os`: Operating system where the benchmark ran (e.g., "linux", "macos", "windows")
    - `mean_ns`: Mean execution time in nanoseconds (or null if unavailable)
    - `peak_rss_bytes`: Peak memory usage in bytes (or null if unavailable)

## Usage

Results are automatically collected when benchmarks run. To see results from a benchmark run:

```bash
cargo bench --bench fts-superfile
```

The JSON results will be written to `results/YYYY-MM-DD_HH-MM-SS.json`.

## Future: Web Interface

A web interface will be built to:
- Compare benchmark results across runs
- Filter by timestamp, metric, and engine
- Visualize performance trends over time
