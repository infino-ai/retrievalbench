#!/usr/bin/env bun
import { readdirSync, readFileSync, writeFileSync } from "fs";
import { join } from "path";

const RESULTS_DIR = join(import.meta.dir, "..", "results");
const OUTPUT_FILE = join(import.meta.dir, "data.json");

function buildData() {
  try {
    const files = readdirSync(RESULTS_DIR).filter((f) => f.endsWith(".json"));
    const resultsByTimestamp = {};

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

    const output = {
      timestamps: Object.keys(resultsByTimestamp).sort().reverse(),
      data: resultsByTimestamp,
    };

    writeFileSync(OUTPUT_FILE, JSON.stringify(output, null, 2));
    console.log(
      `✓ Built static data file: ${OUTPUT_FILE} (${output.timestamps.length} timestamps)`
    );
    return true;
  } catch (e) {
    console.error("Error building static data:", e);
    return false;
  }
}

buildData();
