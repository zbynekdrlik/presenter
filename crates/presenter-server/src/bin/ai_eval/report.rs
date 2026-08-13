//! Aggregates per-case `CaseScore`s into a printed + persisted summary.
//!
//! Deliberately does NOT apply report §6.5's pass/fail BAR (>=98% Layer-1
//! on worship-crud, etc.) or decide overall gate success — that is
//! `run.sh`'s separate `gate` stage (report §8 step 11, a later ticket).
//! This module only reports what happened, honestly, per case and per
//! slice; `results.json` is written so that future `gate` stage has real
//! data to apply the bar to.

use crate::corpus::Case;
use crate::scorer::CaseScore;
use crate::trace::{now_rfc3339, Trace};
use anyhow::Context;
use serde::Serialize;
use std::path::Path;

// `rename_all = "camelCase"` on all three: this project's serde convention
// (scripts/dev/quality-check.sh's "Serde convention" check) requires it on
// every Serialize struct with a multi-word field. `SliceSummary` has none
// today (adding it is a no-op on its current JSON output) but it nests
// inside `Report` alongside `CaseResult` in the SAME results.json document
// — keeping all three consistent avoids a future multi-word field on
// `SliceSummary` silently re-triggering the gate.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SliceSummary {
    pub slice: String,
    pub total: usize,
    pub passed: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseResult {
    pub case_id: String,
    pub slice: String,
    pub passed: bool,
    /// Mirrors `CaseScore::seed_failed` — see its own doc comment. Lets a
    /// `results.json` reader filter seed-failed cases out of a pass-rate
    /// calculation instead of counting them as model failures.
    pub seed_failed: bool,
    /// From `Trace::duration_ms` (#662 defect 4) — how long THIS case took
    /// to drive, independent of whether it passed.
    pub duration_ms: u64,
    pub failures: Vec<String>,
    /// `expected.notes` from the corpus fixture — SCHEMA.md: "which
    /// hard-case category this probes ... plus any rationale a future
    /// reader needs". Carried through so a failed report entry names WHAT
    /// was being tested, not just that it failed (report §6.5: "the report
    /// must name which case category failed").
    pub notes: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub generated_at: String,
    pub total: usize,
    pub passed: usize,
    /// Count of `cases[].seedFailed == true` — a harness/environment
    /// problem, never a model-quality result (#662 defect 1). Surfaced
    /// separately here so a report reader never mistakes it for part of
    /// the model's pass rate.
    pub seed_failed_total: usize,
    /// Sum of every case's `Trace::duration_ms` (#662 defect 4) — the
    /// smoke-run's own complaint was having to derive this by hand from
    /// consecutive `capturedAt` timestamps.
    pub total_duration_ms: u64,
    pub slices: Vec<SliceSummary>,
    pub cases: Vec<CaseResult>,
}

/// `results`: one `(case, trace, CaseScore)` per scored case — the `Trace`
/// is already in scope at the one call site (`main.rs::run_score_l1`), so
/// duration/seed-failed totals are aggregated here without a second pass
/// over the trace files.
pub fn build_report(results: &[(&Case, &Trace, CaseScore)]) -> Report {
    let total = results.len();
    let passed = results.iter().filter(|(_, _, s)| s.passed).count();
    let seed_failed_total = results.iter().filter(|(_, _, s)| s.seed_failed).count();
    let total_duration_ms: u64 = results.iter().map(|(_, t, _)| t.duration_ms).sum();

    let mut slice_names: Vec<String> = results.iter().map(|(c, _, _)| c.slice.clone()).collect();
    slice_names.sort();
    slice_names.dedup();

    let slices = slice_names
        .into_iter()
        .map(|slice| {
            let in_slice: Vec<_> = results
                .iter()
                .filter(|(c, _, _)| c.slice == slice)
                .collect();
            SliceSummary {
                total: in_slice.len(),
                passed: in_slice.iter().filter(|(_, _, sc)| sc.passed).count(),
                slice,
            }
        })
        .collect();

    let cases = results
        .iter()
        .map(|(case, trace, score)| CaseResult {
            case_id: score.case_id.clone(),
            slice: case.slice.clone(),
            passed: score.passed,
            seed_failed: score.seed_failed,
            duration_ms: trace.duration_ms,
            failures: score.failures.clone(),
            notes: case.expected.notes.clone(),
        })
        .collect();

    Report {
        generated_at: now_rfc3339(),
        total,
        passed,
        seed_failed_total,
        total_duration_ms,
        slices,
        cases,
    }
}

pub fn write_report(report: &Report, path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating report dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(report).context("serializing report")?;
    std::fs::write(path, json).with_context(|| format!("writing report {}", path.display()))?;
    Ok(())
}

pub fn print_summary(report: &Report) {
    println!(
        "\nLayer-1 summary: {}/{} passed",
        report.passed, report.total
    );
    if report.seed_failed_total > 0 {
        println!(
            "  {} case(s) seed-failed (harness/environment issue — excluded from \
             model-quality assessment, not counted as a model result)",
            report.seed_failed_total
        );
    }
    let avg_ms = if report.total > 0 {
        report.total_duration_ms / report.total as u64
    } else {
        0
    };
    println!(
        "  total drive time: {} ms across {} case(s) (avg {} ms/case)",
        report.total_duration_ms, report.total, avg_ms
    );
    for s in &report.slices {
        println!("  {:<16} {}/{}", s.slice, s.passed, s.total);
    }
    for c in &report.cases {
        if !c.passed {
            println!("  FAIL {} ({})", c.case_id, c.slice);
            if !c.notes.is_empty() {
                println!("       notes: {}", c.notes);
            }
            for f in &c.failures {
                println!("       - {f}");
            }
        }
    }
}
