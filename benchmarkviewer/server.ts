import { serve } from "bun";
import { join } from "path";
import { readdirSync, readFileSync } from "fs";

const PORT = 3000;
const RESULTS_DIR = join(import.meta.dir, "..", "results");

function getResultsByTimestamp() {
  try {
    const files = readdirSync(RESULTS_DIR).filter((f) => f.endsWith(".json"));
    const resultsByTimestamp: Record<string, any> = {};

    for (const file of files) {
      try {
        const content = readFileSync(join(RESULTS_DIR, file), "utf-8");
        const data = JSON.parse(content);
        const timestamp = data.iso_timestamp;

        if (timestamp) {
          resultsByTimestamp[timestamp] = data.results;
        }
      } catch (e) {
        console.error(`Error reading ${file}:`, e);
      }
    }

    return resultsByTimestamp;
  } catch (e) {
    console.error("Error reading results directory:", e);
    return {};
  }
}

function getAllBenchmarks(
  resultsByTimestamp: Record<string, any>,
  timestamp: string,
): string[] {
  const groups = resultsByTimestamp[timestamp];
  if (!groups) return [];

  const benchmarks: Set<string> = new Set();
  for (const group of Object.values(groups)) {
    if (typeof group === "object" && group !== null) {
      Object.keys(group).forEach((benchName) => {
        benchmarks.add(benchName);
      });
    }
  }
  return Array.from(benchmarks).sort();
}

const server = serve({
  port: PORT,
  fetch(req) {
    const url = new URL(req.url);

    if (url.pathname === "/") {
      return new Response(getHTML(), {
        headers: { "Content-Type": "text/html" },
      });
    }

    if (url.pathname === "/api/timestamps") {
      const resultsByTimestamp = getResultsByTimestamp();
      const timestamps = Object.keys(resultsByTimestamp).sort().reverse();
      console.log("Available timestamps:", timestamps);
      console.log(
        "Results structure:",
        Object.keys(resultsByTimestamp).map((ts) => {
          const groups = resultsByTimestamp[ts] || {};
          let totalBenchmarks = 0;
          for (const group of Object.values(groups)) {
            if (typeof group === "object" && group !== null) {
              totalBenchmarks += Object.keys(group).length;
            }
          }
          return {
            ts,
            groups: Object.keys(groups).length,
            benchmarkCount: totalBenchmarks,
          };
        }),
      );
      return Response.json({ timestamps });
    }

    if (url.pathname === "/api/benchmarks") {
      const timestamp = url.searchParams.get("timestamp");
      console.log(`/api/benchmarks called with timestamp: "${timestamp}"`);
      const resultsByTimestamp = getResultsByTimestamp();

      const benchmarks: Set<string> = new Set();
      if (timestamp && resultsByTimestamp[timestamp]) {
        const groups = resultsByTimestamp[timestamp];
        console.log({ groups });
        for (const groupName of Object.keys(groups)) {
          const group = groups[groupName];
          if (typeof group === "object" && group !== null) {
            Object.keys(group).forEach((benchName) => {
              const benchNameWithGroup = `${groupName}|${benchName}`;
              benchmarks.add(benchNameWithGroup);
            });
          }
        }
      }

      const benchmarkList = Array.from(benchmarks).sort();
      console.log(
        `Found ${benchmarkList.length} benchmarks for timestamp "${timestamp}"`,
      );
      return Response.json({ benchmarks: benchmarkList });
    }

    if (url.pathname === "/api/result") {
      const timestamp = url.searchParams.get("timestamp");
      const group = url.searchParams.get("group");
      const benchmark = url.searchParams.get("benchmark");
      console.log(
        `/api/result called with timestamp: "${timestamp}", group: "${group}", benchmark: "${benchmark}"`,
      );
      const resultsByTimestamp = getResultsByTimestamp();

      let result = null;
      if (timestamp && group && benchmark) {
        const groupData = resultsByTimestamp[timestamp]?.[group];
        if (groupData) {
          result = groupData[benchmark];
        }
      } else if (timestamp && benchmark) {
        // Fallback: try to find benchmark in any group
        const groups = resultsByTimestamp[timestamp];
        if (groups) {
          for (const groupData of Object.values(groups)) {
            if (
              typeof groupData === "object" &&
              groupData !== null &&
              benchmark in groupData
            ) {
              result = groupData[benchmark];
              break;
            }
          }
        }
      }
      console.log(`Result found: ${result ? "yes" : "no"}`);
      if (result) {
        return Response.json(result);
      } else {
        return Response.json(null);
      }
    }

    return new Response("Not found", { status: 404 });
  },
});

console.log(`Benchmark Viewer running at http://localhost:${PORT}`);

function getHTML() {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Benchmark Viewer</title>
  <style>
    * {
      margin: 0;
      padding: 0;
      box-sizing: border-box;
    }

    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      background: #f5f5f5;
      padding: 20px;
    }

    .container {
      max-width: 1200px;
      margin: 0 auto;
      background: white;
      border-radius: 8px;
      padding: 30px;
      box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
    }

    h1 {
      margin-bottom: 30px;
      color: #333;
    }

    .selectors {
      display: flex;
      flex-direction: column;
      gap: 20px;
      margin-bottom: 30px;
    }

    .selector-row {
      display: grid;
      grid-template-columns: 1fr 1fr 1fr;
      gap: 15px;
      padding: 15px;
      background: #fafafa;
      border-radius: 6px;
    }

    .selector-group {
      display: flex;
      flex-direction: column;
    }

    label {
      margin-bottom: 8px;
      font-weight: 600;
      color: #555;
      font-size: 14px;
    }

    select {
      padding: 10px;
      border: 1px solid #ddd;
      border-radius: 4px;
      font-size: 14px;
      cursor: pointer;
    }

    select[multiple] {
      padding: 5px;
      min-height: 150px;
    }

    select[multiple] option {
      padding: 5px;
    }

    select:focus {
      outline: none;
      border-color: #0066cc;
      box-shadow: 0 0 0 3px rgba(0, 102, 204, 0.1);
    }

    select:disabled {
      background: #f0f0f0;
      cursor: not-allowed;
      opacity: 0.6;
    }

    .benchmark-checkbox {
      display: flex;
      align-items: center;
      padding: 6px 4px;
      cursor: pointer;
      border-radius: 3px;
      transition: background-color 0.15s;
    }

    .benchmark-checkbox:hover {
      background-color: #f0f0f0;
    }

    .benchmark-checkbox input[type="checkbox"] {
      cursor: pointer;
      margin-right: 8px;
    }

    .benchmark-checkbox label {
      flex: 1;
      cursor: pointer;
      margin: 0;
      font-weight: normal;
    }

    .comparison {
      margin-top: 30px;
    }

    .comparison-header {
      display: grid;
      gap: 15px;
      margin-bottom: 15px;
      font-weight: 600;
      color: #555;
      border-bottom: 2px solid #ddd;
      padding-bottom: 10px;
      font-size: 12px;
      text-transform: uppercase;
      align-items: center;
    }

    .comparison-row {
      display: grid;
      gap: 15px;
      padding: 12px 0;
      border-bottom: 1px solid #eee;
      align-items: flex-start;
    }

    .benchmark-name-cell {
      font-weight: 500;
      color: #333;
      word-break: break-word;
    }

    .metric-name {
      color: #333;
      font-weight: 500;
      font-size: 14px;
      margin-bottom: 4px;
    }

    .result-value {
      display: flex;
      flex-direction: column;
      gap: 4px;
    }

    .value-with-metadata {
      color: #666;
      font-family: "Monaco", "Courier New", monospace;
      font-size: 13px;
    }

    .result-metadata {
      display: flex;
      align-items: center;
      gap: 8px;
      font-size: 11px;
      color: #999;
    }

    .result-commit {
      font-family: "Monaco", "Courier New", monospace;
      font-size: 11px;
    }

    .result-os {
      font-size: 14px;
      min-width: 20px;
    }

    .value {
      color: #666;
      font-family: "Monaco", "Courier New", monospace;
      font-size: 13px;
    }

    .comparison-value {
      background: #f0f0f0;
      padding: 8px;
      border-radius: 4px;
      font-size: 13px;
    }

    .faster {
      background: #d4edda;
      color: #155724;
    }

    .slower {
      background: #f8d7da;
      color: #721c24;
    }

    .equal {
      background: #fff3cd;
      color: #856404;
    }

    .placeholder {
      color: #999;
      font-style: italic;
      padding: 20px;
      text-align: center;
    }

    .error {
      color: #721c24;
      background: #f8d7da;
      padding: 12px;
      border-radius: 4px;
      margin-top: 20px;
    }

    .tab-button {
      border-bottom: 3px solid transparent;
      color: #999;
    }

    .tab-button:hover {
      color: #0066cc;
    }

    .tab-button.active {
      color: #0066cc;
      border-bottom-color: #0066cc;
    }

    .db-group {
      display: grid;
      grid-template-columns: 1fr 1.5fr auto;
      gap: 15px;
      padding: 15px;
      background: #fafafa;
      border-radius: 6px;
      margin-bottom: 15px;
      align-items: flex-start;
    }

    .db-group-field {
      display: flex;
      flex-direction: column;
    }

    .db-group-field label {
      margin-bottom: 8px;
      font-weight: 600;
      color: #555;
      font-size: 14px;
    }

    .db-group-field select {
      padding: 10px;
      border: 1px solid #ddd;
      border-radius: 4px;
      font-size: 14px;
      cursor: pointer;
    }

    .db-group-field select:focus {
      outline: none;
      border-color: #0066cc;
      box-shadow: 0 0 0 3px rgba(0, 102, 204, 0.1);
    }

    .db-remove-btn {
      padding: 8px 12px;
      background: #f44336;
      color: white;
      border: none;
      border-radius: 4px;
      cursor: pointer;
      font-size: 12px;
      margin-top: 26px;
    }

    .db-remove-btn:hover {
      background: #d32f2f;
    }
  </style>
</head>
<body>
  <div class="container">
    <h1>Benchmark Viewer</h1>

    <div style="display: flex; gap: 10px; margin-bottom: 30px; border-bottom: 2px solid #ddd;">
      <button id="tab-progression" class="tab-button active" style="padding: 12px 20px; background: none; border: none; cursor: pointer; font-weight: 600;">Progression</button>
      <button id="tab-db-comparison" class="tab-button" style="padding: 12px 20px; background: none; border: none; cursor: pointer; font-weight: 600;">DB Comparison</button>
    </div>

    <div id="progression-view">
    <div class="selectors">
      <div class="selector-row">
        <div class="selector-group">
          <label for="timestamp1">Date/Time 1</label>
          <select id="timestamp1">
            <option value="">Select a run...</option>
          </select>
        </div>
        <div class="selector-group">
          <label for="timestamp2">Date/Time 2</label>
          <select id="timestamp2">
            <option value="">Select a run...</option>
          </select>
        </div>
        <div class="selector-group">
          <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
            <label>Benchmarks</label>
            <div style="display: flex; gap: 6px;">
              <button id="selectAllBenchmarks" style="padding: 4px 8px; font-size: 12px; cursor: pointer;">Select All</button>
              <button id="clearBenchmarks" style="padding: 4px 8px; font-size: 12px; cursor: pointer;">Clear</button>
            </div>
          </div>
          <input id="benchmarks-search" type="text" placeholder="Search benchmarks..." style="width: 100%; padding: 8px; border: 1px solid #ddd; border-radius: 4px; margin-bottom: 8px; font-size: 13px; display: none;" />
          <div id="benchmarks-container" style="border: 1px solid #ddd; border-radius: 4px; padding: 8px; max-height: 150px; overflow-y: auto; background: white; min-height: 50px;">
            <div style="color: #999; padding: 20px; text-align: center;">Select a date first</div>
          </div>
        </div>
      </div>
    </div>

    <div id="comparison" class="comparison"></div>
    </div>

    <div id="db-comparison-view" style="display: none;">
      <div class="selectors">
        <div id="db-groups-container"></div>
        <button id="addMoreBtn" style="padding: 10px 20px; background: #0066cc; color: white; border: none; border-radius: 4px; cursor: pointer; font-weight: 600;">Add more</button>
      </div>
      <div id="db-comparison-results"></div>
    </div>
  </div>

  <script>
    const timestamp1Select = document.getElementById("timestamp1");
    const timestamp2Select = document.getElementById("timestamp2");
    const benchmarksContainer = document.getElementById("benchmarks-container");
    const clearBenchmarksBtn = document.getElementById("clearBenchmarks");
    const selectAllBenchmarksBtn = document.getElementById("selectAllBenchmarks");
    const benchmarksSearchInput = document.getElementById("benchmarks-search");
    const comparisonDiv = document.getElementById("comparison");

    async function loadTimestamps() {
      try {
        const res = await fetch("/api/timestamps");
        const data = await res.json();

        data.timestamps.forEach((ts) => {
          const option1 = document.createElement("option");
          option1.value = ts;
          option1.textContent = ts;
          timestamp1Select.appendChild(option1);

          const option2 = document.createElement("option");
          option2.value = ts;
          option2.textContent = ts;
          timestamp2Select.appendChild(option2);
        });

        restoreFromURL();
      } catch (e) {
        comparisonDiv.innerHTML = \`<div class="error">Error loading timestamps: \${e.message}</div>\`;
      }
    }

    async function loadBenchmarks() {
      // Use the first selected timestamp to load benchmarks
      const timestampId = timestamp1Select.value || timestamp2Select.value;

      benchmarksContainer.innerHTML = "";

      if (!timestampId) {
        benchmarksContainer.innerHTML = '<div style="color: #999; padding: 20px; text-align: center;">Select a date first</div>';
        return;
      }

      try {
        const res = await fetch(\`/api/benchmarks?timestamp=\${encodeURIComponent(timestampId)}\`);
        const data = await res.json();

        console.log("Loaded benchmarks for", timestampId, ":", data);

        if (!data.benchmarks || data.benchmarks.length === 0) {
          console.warn("No benchmarks found for timestamp:", timestampId);
          benchmarksContainer.innerHTML = '<div style="color: #999; padding: 20px; text-align: center;">No benchmarks found</div>';
          return;
        }

        benchmarksSearchInput.style.display = "block";
        benchmarksSearchInput.value = "";

        data.benchmarks.forEach((name) => {
          const checkboxDiv = document.createElement("div");
          checkboxDiv.className = "benchmark-checkbox";
          checkboxDiv.setAttribute("data-benchmark-name", name.toLowerCase());

          const checkbox = document.createElement("input");
          checkbox.type = "checkbox";
          checkbox.value = name;
          checkbox.id = "bench-" + name;

          const label = document.createElement("label");
          label.htmlFor = "bench-" + name;
          label.textContent = name;

          checkboxDiv.appendChild(checkbox);
          checkboxDiv.appendChild(label);
          benchmarksContainer.appendChild(checkboxDiv);

          checkbox.addEventListener("change", () => { updateComparison(); updateURL(); saveTabState("progression"); });
        });
      } catch (e) {
        console.error("Error loading benchmarks:", e);
        benchmarksContainer.innerHTML = '<div style="color: #d32f2f; padding: 20px; text-align: center;">Error loading benchmarks</div>';
      }
    }

    function formatTime(ns) {
      if (ns === null) return "N/A";
      if (ns < 1000) return ns.toFixed(0) + " ns";
      if (ns < 1000000) return (ns / 1000).toFixed(2) + " µs";
      if (ns < 1000000000) return (ns / 1000000).toFixed(2) + " ms";
      return (ns / 1000000000).toFixed(2) + " s";
    }

    function formatBytes(bytes) {
      if (bytes === null) return "N/A";
      const kb = bytes / 1024;
      const mb = kb / 1024;
      const gb = mb / 1024;
      if (gb >= 1) return gb.toFixed(2) + " GiB";
      if (mb >= 1) return mb.toFixed(2) + " MiB";
      if (kb >= 1) return kb.toFixed(2) + " KiB";
      return bytes + " B";
    }

    function getOSIcon(os) {
      const icons = {
        "linux": "🐧",
        "macos": "🍎",
        "windows": "🪟",
      };
      return icons[os] || "💻";
    }

    function getSelectedBenchmarks() {
      return Array.from(benchmarksContainer.querySelectorAll("input[type='checkbox']:checked")).map(cb => cb.value);
    }

    function updateURL() {
      const ts1 = timestamp1Select.value;
      const ts2 = timestamp2Select.value;
      const selectedBenchmarks = getSelectedBenchmarks();

      const params = new URLSearchParams();
      if (ts1) params.set("ts1", ts1);
      if (ts2) params.set("ts2", ts2);
      if (selectedBenchmarks.length > 0) params.set("benchmarks", selectedBenchmarks.join(","));

      const newURL = params.toString() ? \`?\${params.toString()}\` : window.location.pathname;
      window.history.pushState({}, "", newURL);
    }

    function restoreFromURL() {
      const params = new URLSearchParams(window.location.search);
      const ts1 = params.get("ts1");
      const ts2 = params.get("ts2");
      const benchmarksStr = params.get("benchmarks");

      if (ts1) {
        timestamp1Select.value = ts1;
      }
      if (ts2) {
        timestamp2Select.value = ts2;
      }

      // Load benchmarks and restore selection
      if (ts1 || ts2) {
        loadBenchmarks().then(() => {
          if (benchmarksStr) {
            const benchmarkList = benchmarksStr.split(",");
            benchmarksContainer.querySelectorAll("input[type='checkbox']").forEach(checkbox => {
              checkbox.checked = benchmarkList.includes(checkbox.value);
            });
          }
          updateComparison();
        });
      }
    }

    async function updateComparison() {
      const ts1 = timestamp1Select.value;
      const ts2 = timestamp2Select.value;
      const selectedBenchmarks = getSelectedBenchmarks();

      comparisonDiv.innerHTML = "";

      if (!ts1 || !ts2 || selectedBenchmarks.length === 0) {
        comparisonDiv.innerHTML = '<div class="placeholder">Select both date/times and at least one benchmark to compare</div>';
        return;
      }

      try {
        // Fetch all benchmark results
        const promises = selectedBenchmarks.map(async (benchFull) => {
          // Extract group and benchmark from "group|benchmark" format
          const [benchGroup, benchName] = benchFull.split("|");
          const res1 = await fetch(\`/api/result?timestamp=\${encodeURIComponent(ts1)}&group=\${encodeURIComponent(benchGroup)}&benchmark=\${encodeURIComponent(benchName)}\`);
          const res2 = await fetch(\`/api/result?timestamp=\${encodeURIComponent(ts2)}&group=\${encodeURIComponent(benchGroup)}&benchmark=\${encodeURIComponent(benchName)}\`);
          const r1 = await res1.json();
          const r2 = await res2.json();
          return { bench: benchFull, r1, r2 };
        });

        const results = await Promise.all(promises);

        // Build header
        let html = \`<div class="comparison-header" style="grid-template-columns: 2fr 1fr 1fr 1fr;">\`;
        html += \`<div>Benchmark</div>\`;
        html += \`<div>\${ts1}</div>\`;
        html += \`<div>\${ts2}</div>\`;
        html += \`<div>Comparison</div>\`;
        html += \`</div>\`;

        // Build rows for each benchmark
        for (const { bench, r1, r2 } of results) {
          if (!r1 || !r2) continue;

          const t1 = r1.mean_ns;
          const t2 = r2.mean_ns;

          let comparisonText = "—";
          let comparisonClass = "";
          if (t1 !== null && t2 !== null) {
            const ratio = t1 / t2;
            // Consider values equal if ratio is within 1% (0.99 to 1.01)
            if (Math.abs(ratio - 1) < 0.01) {
              comparisonClass = "equal";
              comparisonText = "Equal";
            } else {
              const faster = ratio < 1 ? "1" : "2";
              const speedup = ratio < 1 ? (1 / ratio).toFixed(2) : ratio.toFixed(2);
              comparisonClass = faster === "1" ? "faster" : "slower";
              comparisonText = faster === "1" ?
                \`\${speedup}x faster\` :
                \`\${speedup}x slower\`;
            }
          }

          const benchmarkShort = bench.length > 40 ? bench.substring(0, 37) + "..." : bench;

          html += \`
            <div class="comparison-row" style="grid-template-columns: 2fr 1fr 1fr 1fr;">
              <div class="benchmark-name-cell" title="\${bench}">\${benchmarkShort}</div>
              <div class="result-value">
                <div class="value-with-metadata">\${t1 !== null ? formatTime(t1) : "N/A"}</div>
                <div class="result-metadata">
                  <span class="result-commit">commit: \${r1.commit_hash}</span>
                  <span class="result-os" title="OS: \${r1.os}">\${getOSIcon(r1.os)}</span>
                </div>
              </div>
              <div class="result-value">
                <div class="value-with-metadata">\${t2 !== null ? formatTime(t2) : "N/A"}</div>
                <div class="result-metadata">
                  <span class="result-commit">commit: \${r2.commit_hash}</span>
                  <span class="result-os" title="OS: \${r2.os}">\${getOSIcon(r2.os)}</span>
                </div>
              </div>
              <div class="comparison-value \${comparisonClass}">\${comparisonText}</div>
            </div>
          \`;
        }

        if (results.length === 0) {
          comparisonDiv.innerHTML = '<div class="placeholder">No benchmark results found</div>';
        } else {
          comparisonDiv.innerHTML = html;
        }
      } catch (e) {
        comparisonDiv.innerHTML = \`<div class="error">Error loading results: \${e.message}</div>\`;
      }
    }

    timestamp1Select.addEventListener("change", () => { loadBenchmarks(); updateComparison(); updateURL(); saveTabState("progression"); });
    timestamp2Select.addEventListener("change", () => { loadBenchmarks(); updateComparison(); updateURL(); saveTabState("progression"); });

    selectAllBenchmarksBtn.addEventListener("click", () => {
      benchmarksContainer.querySelectorAll(".benchmark-checkbox").forEach(checkboxDiv => {
        if (checkboxDiv.style.display !== "none") {
          const checkbox = checkboxDiv.querySelector("input[type='checkbox']");
          if (checkbox) {
            checkbox.checked = true;
          }
        }
      });
      updateComparison();
      updateURL();
      saveTabState("progression");
    });

    clearBenchmarksBtn.addEventListener("click", () => {
      benchmarksContainer.querySelectorAll("input[type='checkbox']").forEach(checkbox => {
        checkbox.checked = false;
      });
      updateComparison();
      updateURL();
      saveTabState("progression");
    });

    benchmarksSearchInput.addEventListener("input", () => {
      const searchTerm = benchmarksSearchInput.value.toLowerCase();
      benchmarksContainer.querySelectorAll(".benchmark-checkbox").forEach(checkboxDiv => {
        const benchmarkName = checkboxDiv.getAttribute("data-benchmark-name");
        const matches = benchmarkName.includes(searchTerm);
        checkboxDiv.style.display = matches ? "flex" : "none";
      });
    });

    // Tab switching with persistence
    const progressionBtn = document.getElementById("tab-progression");
    const dbComparisonBtn = document.getElementById("tab-db-comparison");
    const progressionView = document.getElementById("progression-view");
    const dbComparisonView = document.getElementById("db-comparison-view");

    function saveDBComparisonURL() {
      const groups = dbGroupsContainer.querySelectorAll(".db-group");
      const selections = [];

      groups.forEach((group) => {
        const timestampSelect = group.querySelector("select[id^='db-timestamp-']");
        const benchmarkContainer = group.querySelector("div[id^='db-benchmark-container-']");

        if (timestampSelect && benchmarkContainer) {
          const timestamp = timestampSelect.value;
          if (timestamp) {
            // Collect all checked benchmarks from this group
            const checkedBenchmarks = Array.from(benchmarkContainer.querySelectorAll("input[type='checkbox']:checked"))
              .map(cb => cb.value);

            if (checkedBenchmarks.length > 0) {
              selections.push({ timestamp, benchmarks: checkedBenchmarks });
            }
          }
        }
      });

      const params = new URLSearchParams();
      params.set("tab", "db-comparison");
      if (selections.length > 0) {
        params.set("db-selections", JSON.stringify(selections));
      }

      const newURL = params.toString() ? \`?\${params.toString()}\` : window.location.pathname;
      window.history.pushState({}, "", newURL);
    }

    function saveTabState(tabName) {
      localStorage.setItem("activeTab", tabName);
      if (tabName === "progression") {
        localStorage.setItem("progression_ts1", timestamp1Select.value);
        localStorage.setItem("progression_ts2", timestamp2Select.value);
        const selectedBenchmarks = getSelectedBenchmarks();
        localStorage.setItem("progression_benchmarks", JSON.stringify(selectedBenchmarks));
      } else if (tabName === "db-comparison") {
        saveDBComparisonURL();
      }
    }

    function showTab(tabName) {
      if (tabName === "progression") {
        progressionView.style.display = "block";
        dbComparisonView.style.display = "none";
        progressionBtn.classList.add("active");
        dbComparisonBtn.classList.remove("active");
      } else {
        progressionView.style.display = "none";
        dbComparisonView.style.display = "block";
        dbComparisonBtn.classList.add("active");
        progressionBtn.classList.remove("active");
      }
      saveTabState(tabName);
    }

    progressionBtn.addEventListener("click", () => showTab("progression"));
    dbComparisonBtn.addEventListener("click", () => showTab("db-comparison"));

    // Restore tab on load
    function restoreTab() {
      const params = new URLSearchParams(window.location.search);
      const urlTab = params.get("tab");
      const savedTab = urlTab || localStorage.getItem("activeTab") || "progression";
      showTab(savedTab);

      if (savedTab === "progression") {
        const savedTs1 = localStorage.getItem("progression_ts1");
        const savedTs2 = localStorage.getItem("progression_ts2");
        const savedBenchmarks = localStorage.getItem("progression_benchmarks");

        if (savedTs1) {
          timestamp1Select.value = savedTs1;
        }
        if (savedTs2) {
          timestamp2Select.value = savedTs2;
        }

        // Trigger loading benchmarks if timestamps are set
        if (savedTs1 || savedTs2) {
          loadBenchmarks().then(() => {
            if (savedBenchmarks) {
              const benchmarkList = JSON.parse(savedBenchmarks);
              benchmarksContainer.querySelectorAll("input[type='checkbox']").forEach(checkbox => {
                checkbox.checked = benchmarkList.includes(checkbox.value);
              });
            }
            updateComparison();
          });
        }
      } else if (savedTab === "db-comparison") {
        const dbSelectionsStr = params.get("db-selections");
        if (dbSelectionsStr) {
          const selections = JSON.parse(dbSelectionsStr);

          // Clear existing groups and recreate them
          dbGroupsContainer.innerHTML = "";

          // Create groups from URL params
          for (let i = 0; i < selections.length; i++) {
            const selection = selections[i];
            createDBGroup().then(() => {
              const groups = dbGroupsContainer.querySelectorAll(".db-group");
              const lastGroup = groups[groups.length - 1];

              if (lastGroup) {
                const timestampSelect = lastGroup.querySelector("select[id^='db-timestamp-']");
                const benchmarkContainer = lastGroup.querySelector("div[id^='db-benchmark-container-']");

                if (timestampSelect && selection.timestamp) {
                  timestampSelect.value = selection.timestamp;
                  // Trigger benchmark loading
                  timestampSelect.dispatchEvent(new Event("change"));

                  // Set benchmarks after benchmarks are loaded
                  setTimeout(() => {
                    if (benchmarkContainer) {
                      // Handle both new format (benchmarks array) and old format (single benchmark)
                      let benchmarksToCheck = [];
                      if (selection.benchmarks && Array.isArray(selection.benchmarks)) {
                        benchmarksToCheck = selection.benchmarks;
                      } else if (selection.benchmark) {
                        benchmarksToCheck = [selection.benchmark];
                      }

                      benchmarksToCheck.forEach(benchName => {
                        const checkbox = benchmarkContainer.querySelector(\`input[value="\${benchName}"]\`);
                        if (checkbox) {
                          checkbox.checked = true;
                        }
                      });
                      if (benchmarksToCheck.length > 0) {
                        updateDBComparison();
                      }
                    }
                  }, 500);
                }
              }
            });
          }
        }
      }
    }

    // DB Comparison functionality
    const dbGroupsContainer = document.getElementById("db-groups-container");
    const addMoreBtn = document.getElementById("addMoreBtn");
    const dbComparisonResults = document.getElementById("db-comparison-results");
    let dbGroupCounter = 0;

    async function loadDBTimestamps() {
      try {
        const res = await fetch("/api/timestamps");
        const data = await res.json();
        return data.timestamps;
      } catch (e) {
        console.error("Error loading timestamps:", e);
        return [];
      }
    }

    async function loadDBBenchmarks(timestamp) {
      try {
        const res = await fetch(\`/api/benchmarks?timestamp=\${encodeURIComponent(timestamp)}\`);
        const data = await res.json();
        return data.benchmarks || [];
      } catch (e) {
        console.error("Error loading benchmarks:", e);
        return [];
      }
    }

    async function createDBGroup() {
      const groupId = dbGroupCounter++;
      const timestamps = await loadDBTimestamps();

      const groupDiv = document.createElement("div");
      groupDiv.className = "db-group";
      groupDiv.id = \`db-group-\${groupId}\`;

      // Date/Time dropdown
      const timestampField = document.createElement("div");
      timestampField.className = "db-group-field";
      timestampField.innerHTML = \`
        <label for="db-timestamp-\${groupId}">Date/Time</label>
        <select id="db-timestamp-\${groupId}">
          <option value="">Select a run...</option>
          \${timestamps.map(ts => \`<option value="\${ts}">\${ts}</option>\`).join("")}
        </select>
      \`;

      // Benchmark field with search and multi-select
      const benchmarkField = document.createElement("div");
      benchmarkField.className = "db-group-field";

      const benchmarkLabelContainer = document.createElement("div");
      benchmarkLabelContainer.style.display = "flex";
      benchmarkLabelContainer.style.justifyContent = "space-between";
      benchmarkLabelContainer.style.alignItems = "center";
      benchmarkLabelContainer.style.marginBottom = "8px";

      const benchmarkLabel = document.createElement("label");
      benchmarkLabel.textContent = "Benchmarks";

      const selectAllDBBtn = document.createElement("button");
      selectAllDBBtn.textContent = "Select All";
      selectAllDBBtn.style.padding = "4px 8px";
      selectAllDBBtn.style.fontSize = "12px";
      selectAllDBBtn.style.cursor = "pointer";
      selectAllDBBtn.disabled = true;

      benchmarkLabelContainer.appendChild(benchmarkLabel);
      benchmarkLabelContainer.appendChild(selectAllDBBtn);

      const benchmarkSearch = document.createElement("input");
      benchmarkSearch.type = "text";
      benchmarkSearch.placeholder = "Search benchmarks...";
      benchmarkSearch.id = \`db-benchmark-search-\${groupId}\`;
      benchmarkSearch.style.width = "100%";
      benchmarkSearch.style.padding = "8px";
      benchmarkSearch.style.border = "1px solid #ddd";
      benchmarkSearch.style.borderRadius = "4px";
      benchmarkSearch.style.marginBottom = "8px";
      benchmarkSearch.style.fontSize = "13px";
      benchmarkSearch.disabled = true;

      const benchmarkContainer = document.createElement("div");
      benchmarkContainer.id = \`db-benchmark-container-\${groupId}\`;
      benchmarkContainer.style.border = "1px solid #ddd";
      benchmarkContainer.style.borderRadius = "4px";
      benchmarkContainer.style.padding = "8px";
      benchmarkContainer.style.maxHeight = "150px";
      benchmarkContainer.style.overflowY = "auto";
      benchmarkContainer.style.background = "white";
      benchmarkContainer.style.minHeight = "50px";

      benchmarkField.appendChild(benchmarkLabelContainer);
      benchmarkField.appendChild(benchmarkSearch);
      benchmarkField.appendChild(benchmarkContainer);

      // Remove button
      const removeBtn = document.createElement("button");
      removeBtn.className = "db-remove-btn";
      removeBtn.textContent = "Remove";
      removeBtn.addEventListener("click", () => {
        groupDiv.remove();
        updateDBComparison();
        saveDBComparisonURL();
      });

      groupDiv.appendChild(timestampField);
      groupDiv.appendChild(benchmarkField);
      groupDiv.appendChild(removeBtn);

      // Handle timestamp change
      const timestampSelect = groupDiv.querySelector(\`#db-timestamp-\${groupId}\`);

      timestampSelect.addEventListener("change", async () => {
        benchmarkContainer.innerHTML = "";
        benchmarkSearch.disabled = true;
        benchmarkSearch.value = "";
        selectAllDBBtn.disabled = true;

        if (timestampSelect.value) {
          const benchmarks = await loadDBBenchmarks(timestampSelect.value);
          benchmarks.forEach((benchName) => {
            const checkboxDiv = document.createElement("div");
            checkboxDiv.className = "benchmark-checkbox";
            checkboxDiv.setAttribute("data-benchmark-name", benchName.toLowerCase());

            const checkbox = document.createElement("input");
            checkbox.type = "checkbox";
            checkbox.value = benchName;
            checkbox.id = "db-bench-" + groupId + "-" + benchName;

            const label = document.createElement("label");
            label.htmlFor = checkbox.id;
            label.textContent = benchName;

            checkboxDiv.appendChild(checkbox);
            checkboxDiv.appendChild(label);
            benchmarkContainer.appendChild(checkboxDiv);

            checkbox.addEventListener("change", () => {
              updateDBComparison();
              saveDBComparisonURL();
            });
          });
          benchmarkSearch.disabled = false;
          selectAllDBBtn.disabled = false;
        }
        updateDBComparison();
        saveDBComparisonURL();
      });

      selectAllDBBtn.addEventListener("click", () => {
        benchmarkContainer.querySelectorAll(".benchmark-checkbox").forEach(checkboxDiv => {
          if (checkboxDiv.style.display !== "none") {
            const checkbox = checkboxDiv.querySelector("input[type='checkbox']");
            if (checkbox) {
              checkbox.checked = true;
            }
          }
        });
        updateDBComparison();
        saveDBComparisonURL();
      });

      benchmarkSearch.addEventListener("input", () => {
        const searchTerm = benchmarkSearch.value.toLowerCase();
        benchmarkContainer.querySelectorAll(".benchmark-checkbox").forEach(checkboxDiv => {
          const benchmarkName = checkboxDiv.getAttribute("data-benchmark-name");
          const matches = benchmarkName.includes(searchTerm);
          checkboxDiv.style.display = matches ? "flex" : "none";
        });
      });

      dbGroupsContainer.appendChild(groupDiv);
    }

    async function updateDBComparison() {
      const groups = dbGroupsContainer.querySelectorAll(".db-group");
      const selections = [];

      groups.forEach((group, idx) => {
        const timestampSelect = group.querySelector("select[id^='db-timestamp-']");
        const benchmarkContainer = group.querySelector("div[id^='db-benchmark-container-']");

        if (!timestampSelect || !benchmarkContainer) return;

        const timestamp = timestampSelect.value;
        if (!timestamp) return;

        // Collect all checked benchmarks from this group
        const checkedBenchmarks = Array.from(benchmarkContainer.querySelectorAll("input[type='checkbox']:checked"))
          .map(cb => cb.value);

        checkedBenchmarks.forEach(benchmarkFull => {
          // Extract group and benchmark from "group|benchmark" format
          const [benchGroup, benchName] = benchmarkFull.split("|");
          selections.push({ timestamp, group: benchGroup, benchmark: benchName, benchmarkFull });
        });
      });

      if (selections.length === 0) {
        dbComparisonResults.innerHTML = '<div class="placeholder">Select benchmarks to compare</div>';
        return;
      }

      try {
        // Fetch all results
        const allResults = [];
        for (const selection of selections) {
          const res = await fetch(
            \`/api/result?timestamp=\${encodeURIComponent(selection.timestamp)}&group=\${encodeURIComponent(selection.group)}&benchmark=\${encodeURIComponent(selection.benchmark)}\`
          );
          const result = await res.json();
          if (result) {
            allResults.push({
              ...selection,
              result
            });
          }
        }

        // Group results by group name
        const groupedByName = {};
        allResults.forEach(item => {
          if (!groupedByName[item.group]) {
            groupedByName[item.group] = [];
          }
          groupedByName[item.group].push(item);
        });

        // Group by database set signature
        const groupsByDBSet = {};
        for (const [groupName, items] of Object.entries(groupedByName)) {
          const databases = new Set();
          items.forEach(item => {
            if (item.result.database) {
              databases.add(item.result.database);
            }
          });
          const dbSignature = Array.from(databases).sort().join('|');
          if (!groupsByDBSet[dbSignature]) {
            groupsByDBSet[dbSignature] = { databases: Array.from(databases).sort(), groups: {} };
          }
          groupsByDBSet[dbSignature].groups[groupName] = items;
        }

        // Generate HTML for each unique database set
        let html = '';
        for (const [dbSignature, { databases: dbArray, groups: tableGroups }] of Object.entries(groupsByDBSet)) {
          // Create table header
          html += '<table style="width: 100%; border-collapse: collapse; margin-bottom: 30px;">';
          html += '<thead><tr>';
          html += '<th style="border: 1px solid #ddd; padding: 8px; text-align: left;">Group</th>';
          dbArray.forEach(db => {
            html += \`<th style="border: 1px solid #ddd; padding: 8px; text-align: left;">\${db}</th>\`;
          });
          html += '<th style="border: 1px solid #ddd; padding: 8px; text-align: left;">Fastest</th>';
          html += '</tr></thead>';

          html += '<tbody>';
          // Create a row for each group
          for (const [groupName, items] of Object.entries(tableGroups)) {
            html += '<tr>';
            html += \`<td style="border: 1px solid #ddd; padding: 8px; font-weight: 600;">\${groupName}</td>\`;

            // Collect times for each database
            const dbTimes = {};
            let minTime = Infinity;
            let fastestDb = '';

            items.forEach(item => {
              if (item.result.mean_ns !== null) {
                dbTimes[item.result.database] = item.result.mean_ns;
                if (item.result.mean_ns < minTime) {
                  minTime = item.result.mean_ns;
                  fastestDb = item.result.database;
                }
              }
            });

            // Add cells for each database
            dbArray.forEach(db => {
              const time = dbTimes[db];
              const timeStr = time !== undefined ? formatTime(time) : "—";
              const isFastest = db === fastestDb && time !== undefined;
              const style = isFastest ? 'background: #d4edda;' : '';
              html += \`<td style="border: 1px solid #ddd; padding: 8px; \${style}">\${timeStr}</td>\`;
            });

            // Fastest column
            html += \`<td style="border: 1px solid #ddd; padding: 8px; font-weight: 600;">\${fastestDb || "—"}</td>\`;
            html += '</tr>';
          }
          html += '</tbody>';
          html += '</table>';
        }

        dbComparisonResults.innerHTML = html;
      } catch (e) {
        console.error("Error in DB comparison:", e);
        dbComparisonResults.innerHTML = \`<div class="error">Error: \${e.message}</div>\`;
      }
    }

    addMoreBtn.addEventListener("click", async () => {
      await createDBGroup();
    });

    // Initialize with one group when switching to DB view
    dbComparisonBtn.addEventListener("click", async () => {
      if (dbGroupsContainer.children.length === 0) {
        await createDBGroup();
      }
    });

    loadTimestamps();
    restoreTab();
  </script>
</body>
</html>`;
}
