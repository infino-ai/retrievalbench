# Benchmark Viewer

A simple web app to view and compare benchmark results from the `results/` directory.

## Installation

Requires [Bun](https://bun.sh) to be installed.

## Running

```bash
cd benchmarkviewer
bun run dev
```

The app will start at `http://localhost:3000`

## Usage

1. Select a benchmark run (date/time) in the first dropdown
2. Select a specific benchmark in the second dropdown
3. Repeat for the comparison side
4. View side-by-side comparison of:
   - Mean execution time (with speedup ratio)
   - Peak memory usage (with memory savings percentage)
   - Commit hash and OS information

## Troubleshooting

If benchmarks don't appear after selecting a date:
1. Open browser DevTools (F12)
2. Check the Console tab for error messages
3. Check the server logs in the terminal
4. Verify result files exist in `../results/` directory

## Features

- Automatically loads all `.json` files from `../results/`
- Two-tier dropdown selection (run date → benchmark name)
- Real-time comparison display
- Human-readable time and memory formatting
- Visual indicators for faster/slower performance
