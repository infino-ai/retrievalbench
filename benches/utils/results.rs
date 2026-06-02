//! JSON results emitter for benchmark runs.
//!
//! Stores complete benchmark results as timestamped JSON files in a results directory,
//! enabling comparison and analysis across benchmark runs via a web interface.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

/// Complete benchmark result set for a single run.
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub timestamp: u64,
    pub commit_hash: String,
    pub os: String,
    pub group: String,
    pub bench_id: String,
    pub database: Option<String>,
    pub mean_ns: Option<f64>,
    pub peak_rss_bytes: Option<u64>,
}

/// Aggregates results across multiple benchmarks and groups, emitted as a single JSON file.
#[derive(Debug)]
pub struct ResultsCollector {
    results: HashMap<String, Vec<BenchmarkResult>>,
    timestamp: u64,
    commit_hash: String,
    os: String,
}

impl ResultsCollector {
    /// Create a new collector with the current timestamp, git commit hash, and OS.
    pub fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let commit_hash = get_current_commit_hash().unwrap_or_else(|| "unknown".to_string());
        let os = std::env::consts::OS.to_string();

        Self {
            results: HashMap::new(),
            timestamp,
            commit_hash,
            os,
        }
    }

    /// Add a single benchmark result.
    pub fn add_result(&mut self, result: BenchmarkResult) {
        self.results
            .entry(result.group.clone())
            .or_insert_with(Vec::new)
            .push(result);
    }

    /// Add results from criterion's estimates.json, with separate path group and results group.
    pub fn add_from_criterion_with_group(&mut self, path_group: &str, results_group: &str, bench: &str, database: Option<&str>) {
        let path = format!("target/criterion/{path_group}/{bench}/new/estimates.json");
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                let mean_ns = v
                    .get("mean")
                    .and_then(|m| m.get("point_estimate"))
                    .and_then(|x| x.as_f64());

                let path = format!("target/criterion/{path_group}/{bench}/rss.json");
                let peak_rss_bytes = fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .and_then(|v| v.get("peak_rss_bytes").and_then(|x| x.as_u64()));

                let result = BenchmarkResult {
                    timestamp: self.timestamp,
                    commit_hash: self.commit_hash.clone(),
                    os: self.os.clone(),
                    group: results_group.to_string(),
                    bench_id: bench.to_string(),
                    database: database.map(|s| s.to_string()),
                    mean_ns,
                    peak_rss_bytes,
                };
                self.add_result(result);
            }
        }
    }

    /// Add results from criterion's estimates.json for a group/bench pair.
    pub fn add_from_criterion(&mut self, group: &str, bench: &str, database: Option<&str>) {
        let path = format!("target/criterion/{group}/{bench}/new/estimates.json");
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                let mean_ns = v
                    .get("mean")
                    .and_then(|m| m.get("point_estimate"))
                    .and_then(|x| x.as_f64());

                let path = format!("target/criterion/{group}/{bench}/rss.json");
                let peak_rss_bytes = fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .and_then(|v| v.get("peak_rss_bytes").and_then(|x| x.as_u64()));

                let result = BenchmarkResult {
                    timestamp: self.timestamp,
                    commit_hash: self.commit_hash.clone(),
                    os: self.os.clone(),
                    group: group.to_string(),
                    bench_id: bench.to_string(),
                    database: database.map(|s| s.to_string()),
                    mean_ns,
                    peak_rss_bytes,
                };
                self.add_result(result);
            }
        }
    }

    /// Add results from infino's sibling criterion tree, with separate path group and results group.
    pub fn add_from_infino_with_group(&mut self, path_group: &str, results_group: &str, bench: &str, database: Option<&str>) {
        let path = format!("../infino/target/criterion/{path_group}/{bench}/new/estimates.json");
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                let mean_ns = v
                    .get("mean")
                    .and_then(|m| m.get("point_estimate"))
                    .and_then(|x| x.as_f64());

                let path = format!("../infino/target/criterion/{path_group}/{bench}/rss.json");
                let peak_rss_bytes = fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .and_then(|v| v.get("peak_rss_bytes").and_then(|x| x.as_u64()));

                let result = BenchmarkResult {
                    timestamp: self.timestamp,
                    commit_hash: self.commit_hash.clone(),
                    os: self.os.clone(),
                    group: results_group.to_string(),
                    bench_id: format!("{bench}_infino"),
                    database: database.map(|s| s.to_string()),
                    mean_ns,
                    peak_rss_bytes,
                };
                self.add_result(result);
            }
        }
    }

    /// Add results from infino's sibling criterion tree.
    pub fn add_from_infino(&mut self, group: &str, bench: &str, database: Option<&str>) {
        let path = format!("../infino/target/criterion/{group}/{bench}/new/estimates.json");
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                let mean_ns = v
                    .get("mean")
                    .and_then(|m| m.get("point_estimate"))
                    .and_then(|x| x.as_f64());

                let path = format!("../infino/target/criterion/{group}/{bench}/rss.json");
                let peak_rss_bytes = fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .and_then(|v| v.get("peak_rss_bytes").and_then(|x| x.as_u64()));

                let result = BenchmarkResult {
                    timestamp: self.timestamp,
                    commit_hash: self.commit_hash.clone(),
                    os: self.os.clone(),
                    group: group.to_string(),
                    bench_id: format!("{bench}_infino"),
                    database: database.map(|s| s.to_string()),
                    mean_ns,
                    peak_rss_bytes,
                };
                self.add_result(result);
            }
        }
    }

    /// Emit collected results as a JSON file in the results directory.
    ///
    /// File naming includes the commit hash. If a file for this commit already exists,
    /// new results are merged into it; otherwise a new file is created.
    ///
    /// Returns the path where the file was written, or an error if I/O fails.
    pub fn emit(self) -> std::io::Result<PathBuf> {
        let results_dir = PathBuf::from("results");
        fs::create_dir_all(&results_dir)?;

        // Build the new results structure
        let mut new_results: HashMap<String, Value> = HashMap::new();
        for (group, group_results) in self.results {
            let mut group_data: HashMap<String, Value> = HashMap::new();
            for result in group_results {
                let mut json_result = json!({
                    "commit_hash": result.commit_hash,
                    "os": result.os,
                    "mean_ns": result.mean_ns,
                    "peak_rss_bytes": result.peak_rss_bytes,
                });
                if let Some(db) = &result.database {
                    json_result["database"] = Value::String(db.clone());
                }
                group_data.insert(result.bench_id, json_result);
            }
            new_results.insert(group, serde_json::to_value(group_data)?);
        }

        // Scan results directory for an existing file with the same commit hash
        let mut existing_file_path: Option<PathBuf> = None;
        let mut existing_json: Option<Value> = None;

        if let Ok(entries) = fs::read_dir(&results_dir) {
            for entry in entries.flatten() {
                if let Some(filename) = entry.file_name().into_string().ok() {
                    // Extract commit hash from filename (format: YYYY-MM-DD_HH-MM-SS_COMMIT.json)
                    if let Some(commit_from_file) = extract_commit_hash(&filename) {
                        if commit_from_file == self.commit_hash {
                            if let Ok(text) = fs::read_to_string(entry.path()) {
                                if let Ok(json) = serde_json::from_str::<Value>(&text) {
                                    existing_file_path = Some(entry.path());
                                    existing_json = Some(json);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Determine final filepath and merge results if needed
        let (filepath, final_timestamp, final_iso_timestamp) = if let Some(path) = existing_file_path {
            let existing = existing_json.unwrap();
            let ts = existing.get("timestamp").and_then(|t| t.as_u64()).unwrap_or(self.timestamp);
            let iso_ts = existing.get("iso_timestamp").and_then(|t| t.as_str()).unwrap_or("").to_string();

            // Merge results: new results override existing ones for the same group/bench
            if let Some(existing_results) = existing.get("results").and_then(|r| r.as_object()) {
                for (group, group_data) in existing_results {
                    if !new_results.contains_key(group) {
                        new_results.insert(group.clone(), group_data.clone());
                    } else if let (Some(existing_group), Some(new_group)) =
                        (group_data.as_object(), new_results.get(group).and_then(|v| v.as_object())) {
                        let mut merged_group = new_group.clone();
                        for (bench_id, bench_data) in existing_group {
                            if !merged_group.contains_key(bench_id) {
                                merged_group.insert(bench_id.clone(), bench_data.clone());
                            }
                        }
                        new_results.insert(group.clone(), serde_json::to_value(merged_group)?);
                    }
                }
            }

            (path, ts, iso_ts)
        } else {
            let iso_timestamp = format_iso_timestamp(self.timestamp);
            let filename = format!("{}_{}.json", iso_timestamp, self.commit_hash);
            (results_dir.join(&filename), self.timestamp, iso_timestamp)
        };

        let output = json!({
            "timestamp": final_timestamp,
            "iso_timestamp": final_iso_timestamp,
            "results": new_results,
        });

        fs::write(&filepath, serde_json::to_vec_pretty(&output)?)?;
        eprintln!("[results] emitted {}", filepath.display());
        Ok(filepath)
    }
}

/// Extract commit hash from a filename with format YYYY-MM-DD_HH-MM-SS_COMMIT.json
/// The timestamp part is always 19 characters (YYYY-MM-DD_HH-MM-SS), so the commit
/// hash starts at position 20 (after the underscore that separates timestamp and commit).
fn extract_commit_hash(filename: &str) -> Option<String> {
    if !filename.ends_with(".json") {
        return None;
    }
    let without_ext = &filename[..filename.len() - 5]; // Remove .json
    // Timestamp is always YYYY-MM-DD_HH-MM-SS (19 chars), then underscore, then commit hash
    if without_ext.len() > 20 && without_ext.chars().nth(19) == Some('_') {
        Some(without_ext[20..].to_string())
    } else {
        None
    }
}

/// Format a Unix timestamp as an ISO 8601 timestamp (for filename and display).
fn format_iso_timestamp(secs: u64) -> String {
    // Format: YYYY-MM-DD_HH-MM-SS
    // Using std::time without external crate dependencies
    let total_secs = secs;
    let secs_per_day = 86400;
    let secs_per_hour = 3600;
    let secs_per_min = 60;

    // Extract time of day
    let time_of_day = total_secs % secs_per_day;
    let hours = time_of_day / secs_per_hour;
    let minutes = (time_of_day % secs_per_hour) / secs_per_min;
    let seconds = time_of_day % secs_per_min;

    // Days since epoch (1970-01-01)
    let mut days_since_epoch = total_secs / secs_per_day;

    // Estimate year
    let mut year = 1970;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days_since_epoch < days_in_year as u64 {
            break;
        }
        days_since_epoch -= days_in_year as u64;
        year += 1;
    }

    // Month and day
    let month_days = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 0;
    for (i, &days) in month_days.iter().enumerate() {
        if days_since_epoch < days as u64 {
            month = i as u32 + 1;
            break;
        }
        days_since_epoch -= days as u64;
    }
    let day = days_since_epoch + 1;

    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        year, month, day, hours, minutes, seconds
    )
}

fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Get the current git commit hash of the repository.
/// Returns the short 7-character commit hash, or None if unable to determine.
fn get_current_commit_hash() -> Option<String> {
    use std::process::Command;

    let output = Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        let hash = String::from_utf8(output.stdout)
            .ok()?
            .trim()
            .to_string();
        // Return first 7 characters (short hash)
        Some(hash.chars().take(7).collect())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_iso_timestamp_epoch() {
        let ts = 0; // 1970-01-01 00:00:00
        let fmt = format_iso_timestamp(ts);
        assert_eq!(fmt, "1970-01-01_00-00-00");
    }

    #[test]
    fn test_format_iso_timestamp_known() {
        // 2024-05-27 14:30:45 UTC is approximately 1716823845
        let ts = 1716823845;
        let fmt = format_iso_timestamp(ts);
        // Just verify it has the right structure and contains reasonable values
        assert!(fmt.contains("2024"));
        assert!(fmt.contains("05"));
        assert!(fmt.contains("27"));
    }

    #[test]
    fn test_extract_commit_hash() {
        assert_eq!(
            extract_commit_hash("2026-06-02_12-34-56_abc1234.json"),
            Some("abc1234".to_string())
        );
        assert_eq!(
            extract_commit_hash("2026-06-02_00-10-06_8335a93.json"),
            Some("8335a93".to_string())
        );
        assert_eq!(extract_commit_hash("2026-06-02_12-34-56.json"), None);
        assert_eq!(extract_commit_hash("invalid.txt"), None);
        assert_eq!(extract_commit_hash("no-underscore.json"), None);
    }
}
