# Benchmark Viewer

A web app to view and compare benchmark results from the `results/` directory. Supports both server-based development and static deployment.

## Features

- Two views: Progression (compare two runs) and DB Comparison (compare multiple databases)
- Select all/clear benchmarks (respects search filters)
- Search and filter benchmarks
- Real-time comparison display
- Human-readable time formatting
- Visual indicators for faster/slower performance
- State persistence via URL and localStorage
- Works as a static HTML page (no server required)

## Quick Start (Static Deployment)

### Prerequisites

Requires [Bun](https://bun.sh) to be installed (for building data only).

### Build Static Files

```bash
cd benchmarkviewer
bun run build-static.js
```

This generates `data.json` from all results in the `results/` directory.

### Deploy

Copy these files to your web server or hosting:
- `index.html` — The main application
- `data.json` — Pre-built benchmark data

Then open `index.html` in your browser or serve it with any static HTTP server:

```bash
# Using Python
python3 -m http.server 8000

# Using Node/npx
npx http-server
```

## Development Server

For local development with auto-reload (requires Bun):

```bash
cd benchmarkviewer
bun run server.ts
```

The app will start at `http://localhost:3000`

## Usage

### Progression Tab
1. Select two benchmark runs (Date/Time 1 and Date/Time 2)
2. Click "Select All" to select all available benchmarks (or choose manually)
3. Use the search box to filter benchmarks
4. View side-by-side comparisons with performance ratios

### DB Comparison Tab
1. Click "Add more" to create comparison groups
2. For each group, select a Date/Time and benchmarks
3. Click "Select All" to select all benchmarks in that group
4. Compare results across different database implementations

## Building Updated Data

After new benchmark results are added to `results/`:

```bash
bun run build-static.js
```

Commit the updated `data.json` to version control.

## File Structure

```
benchmarkviewer/
├── index.html           # Static HTML application
├── server.ts            # Dev server (for local development)
├── build-static.js      # Build script to generate data.json
├── data.json            # Pre-built benchmark data (generated)
└── README.md
```

## Deployment Options

### GitHub Pages / Static Hosting
1. Build static files with `bun run build-static.js`
2. Commit `index.html` and `data.json`
3. Deploy to any static hosting (GitHub Pages, Netlify, Vercel, etc.)

### Docker / Container
```dockerfile
FROM nginx:alpine
COPY index.html data.json /usr/share/nginx/html/
```

### Local File
Open `index.html` directly in your browser (works with `file://` protocol)
