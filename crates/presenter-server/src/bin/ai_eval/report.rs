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
use crate::trace::now_rfc3339;
use anyhow::Context;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct SliceSummary {
    pub slice: String,
    pub total: usize,
    pub passed: usize,
}

#[derive(Debug, Serialize)]
pub struct CaseResult {
    pub case_id: String,
    pub slice: String,
    pub passed: bool,
    pub failures: Vec<String>,
    /// `expected.notes` from the corpus fixture — SCHEMA.md: "which
    /// hard-case category this probes ... plus any rationale a future
    /// reader needs". Carried through so a failed report entry names WHAT
    /// was being tested, not just that it failed (report §6.5: "the report
    /// must name which case category failed").
    pub notes: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub generated_at: String,
    pub total: usize,
    pub passed: usize,
    pub slices: Vec<SliceSummary>,
    pub cases: Vec<CaseResult>,
}

/// `results`: one `(case, CaseScore)` per scored case.
pub fn build_report(results: &[(&Case, CaseScore)]) -> Report {
    let total = results.len();
    let passed = results.iter().filter(|(_, s)| s.passed).count();

    let mut slice_names: Vec<String> = results.iter().map(|(c, _)| c.slice.clone()).collect();
    slice_names.sort();
    slice_names.dedup();

    let slices = slice_names
        .into_iter()
        .map(|slice| {
            let in_slice: Vec<_> = results.iter().filter(|(c, _)| c.slice == slice).collect();
            SliceSummary {
                total: in_slice.len(),
                passed: in_slice.iter().filter(|(_, sc)| sc.passed).count(),
                slice,
            }
        })
        .collect();

    let cases = results
        .iter()
        .map(|(case, score)| CaseResult {
            case_id: score.case_id.clone(),
            slice: case.slice.clone(),
            passed: score.passed,
            failures: score.failures.clone(),
            notes: case.expected.notes.clone(),
        })
        .collect();

    Report {
        generated_at: now_rfc3339(),
        total,
        passed,
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
