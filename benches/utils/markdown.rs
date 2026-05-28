//! Shared markdown summary emitter for bench harnesses.
//!
//! After criterion finishes timing, each topic's bench function can
//! produce a markdown block summarizing its results. The block is
//! always written to stderr between sentinel comments
//! (`<!-- BEGIN: <anchor_id> -->` / `<!-- END: <anchor_id> -->`)
//! so a reader can grep / copy it out. When
//! `INFINO_BENCH_UPDATE_README=1` is set, the same block additionally
//! replaces the matching section in `benches/README.md` in place,
//! using the same sentinel markers as the anchor.
//!
//! The bench function is responsible for building the markdown body
//! string (using the helpers in this module to format times, derive
//! winners, etc.); this module handles only the print + in-place
//! README rewrite, so future topics can plug in without touching the
//! shared rewrite logic.

use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;

/// One markdown section to emit. `anchor_id` is the stable key that
/// matches the `<!-- BEGIN: ... -->` / `<!-- END: ... -->` markers
/// in `benches/README.md`. `body` is the inner markdown (the markers
/// themselves are added by [`emit`]).
pub struct MarkdownSection {
    pub anchor_id: String,
    pub body: String,
}

/// Emit `section` to stderr, framed by sentinel-comment markers.
/// When `INFINO_BENCH_UPDATE_README=1`, additionally replace the
/// matching section in `benches/README.md`.
pub fn emit(section: &MarkdownSection) {
    let stderr = std::io::stderr();
    let mut out = stderr.lock();
    let _ = writeln!(out);
    let _ = writeln!(out, "<!-- BEGIN: {} -->", section.anchor_id);
    let _ = writeln!(out, "{}", section.body);
    let _ = writeln!(out, "<!-- END: {} -->", section.anchor_id);
    let _ = writeln!(out);

    if std::env::var_os("INFINO_BENCH_UPDATE_README").is_some() {
        let path = resolve_readme_path();
        if let Err(e) = update_readme(&path, section) {
            eprintln!("[markdown] failed to update {}: {e}", path.display(),);
        } else {
            eprintln!(
                "[markdown] updated {} ({})",
                path.display(),
                section.anchor_id
            );
        }
    }
}

/// Locate `benches/README.md` relative to the cargo workspace.
/// Cargo runs benches with `cwd = crate root`, so `benches/README.md`
/// resolves from there. Returns a fallback path if cwd isn't the
/// crate root (in which case the rewrite will fail at I/O and the
/// caller logs the error).
fn resolve_readme_path() -> std::path::PathBuf {
    std::path::PathBuf::from("benches/README.md")
}

fn update_readme(path: &Path, section: &MarkdownSection) -> std::io::Result<()> {
    let begin = format!("<!-- BEGIN: {} -->", section.anchor_id);
    let end = format!("<!-- END: {} -->", section.anchor_id);
    let content = fs::read_to_string(path)?;

    let begin_pos = content.find(&begin).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("marker not found: {begin}"),
        )
    })?;
    let after_begin = begin_pos + begin.len();
    let end_pos = content[after_begin..].find(&end).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("end marker not found after begin: {end}"),
        )
    })? + after_begin;

    let mut new = String::with_capacity(content.len() + section.body.len());
    new.push_str(&content[..after_begin]);
    new.push('\n');
    new.push_str(&section.body);
    new.push('\n');
    new.push_str(&content[end_pos..]);
    fs::write(path, new)?;
    Ok(())
}

// ─── Number formatting ────────────────────────────────────────────────

/// Format a nanosecond duration into a human-readable string with
/// units selected by magnitude (ns / µs / ms / s).
pub fn fmt_time(ns: f64) -> String {
    if ns < 1_000.0 {
        format!("{ns:.0} ns")
    } else if ns < 1_000_000.0 {
        format!("{:.2} µs", ns / 1_000.0)
    } else if ns < 1_000_000_000.0 {
        format!("{:.2} ms", ns / 1_000_000.0)
    } else {
        format!("{:.2} s", ns / 1_000_000_000.0)
    }
}

/// Format a throughput (elements per second) with K/M units.
pub fn fmt_throughput(elements_per_sec: f64) -> String {
    if elements_per_sec >= 1_000_000.0 {
        format!("{:.2} M/s", elements_per_sec / 1_000_000.0)
    } else if elements_per_sec >= 1_000.0 {
        format!("{:.1} K/s", elements_per_sec / 1_000.0)
    } else {
        format!("{elements_per_sec:.0}/s")
    }
}

/// Render a winner ratio. `lhs_ns` and `rhs_ns` are the two engines'
/// mean times; returns a self-describing comparison like
/// `"**infino wins, 1.5× faster than Tantivy**"`, `"tie"`, or
/// `"—"` if either side is missing. Reads naturally inline in a
/// table cell — no need for the reader to infer direction from the
/// row label.
pub fn fmt_winner(
    lhs_label: &str,
    lhs_ns: Option<f64>,
    rhs_label: &str,
    rhs_ns: Option<f64>,
) -> String {
    match (lhs_ns, rhs_ns) {
        (Some(a), Some(b)) if a > 0.0 && b > 0.0 => {
            if a < b {
                format!(
                    "**{lhs_label} wins, {:.1}× faster than {rhs_label}**",
                    b / a
                )
            } else if b < a {
                format!(
                    "**{rhs_label} wins, {:.1}× faster than {lhs_label}**",
                    a / b
                )
            } else {
                "tie".to_string()
            }
        }
        _ => "—".to_string(),
    }
}

// ─── estimates.json reader ────────────────────────────────────────────

/// Read criterion's `mean.point_estimate` (in nanoseconds) for a
/// given group + bench id from retrievalbench's own
/// `target/criterion/` tree. Returns `None` if the file doesn't exist
/// (the bench was filtered out or hasn't run yet) or the JSON can't
/// be parsed.
pub fn read_mean_ns(group: &str, bench: &str) -> Option<f64> {
    let path = format!("target/criterion/{group}/{bench}/new/estimates.json");
    let text = fs::read_to_string(&path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("mean")?.get("point_estimate")?.as_f64()
}

/// Read criterion's `mean.point_estimate` (in nanoseconds) from
/// **infino's** sibling `target/criterion/` tree. Infino measures
/// itself in its own bench harness; retrievalbench reads those numbers
/// here to build head-to-head comparison tables against Lance / Tantivy
/// without re-measuring infino in this process.
///
/// Returns `None` if the file is missing — typically because infino's
/// bench hasn't been run yet. The bench's markdown emitter shows "—"
/// in the infino column in that case, which is visible in the rendered
/// README and the most actionable signal for "run `cargo bench` in
/// `../infino` first."
pub fn read_infino_mean_ns(group: &str, bench: &str) -> Option<f64> {
    let path = format!("../infino/target/criterion/{group}/{bench}/new/estimates.json");
    let text = fs::read_to_string(&path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("mean")?.get("point_estimate")?.as_f64()
}

