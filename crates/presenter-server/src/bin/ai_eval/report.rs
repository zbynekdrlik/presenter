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
    /// Mirrors `CaseScore::stalled_retry_loop` (#662 defect 7) — see its
    /// own doc comment.
    pub stalled_retry_loop: bool,
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
    /// Count of `Trace::turns[].finishReason == "length"` across every
    /// case (#662 defect 6) — a length-truncated response used to be
    /// silently indistinguishable anywhere in the harness's own output;
    /// this is the report-level answer to "how many of my LLM calls got
    /// cut off by the context/token ceiling".
    pub finish_reason_length: usize,
    /// Count of `cases[].stalledRetryLoop == true` (#662 defect 7) — a
    /// candidate failure MODE (unproductive retry, usually ending in a
    /// context-ceiling crash), surfaced distinctly from a generic error.
    pub stalled_retry_loop_total: usize,
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
    let finish_reason_length = results
        .iter()
        .flat_map(|(_, t, _)| t.turns.iter())
        .filter(|turn| turn.finish_reason.as_deref() == Some("length"))
        .count();
    let stalled_retry_loop_total = results
        .iter()
        .filter(|(_, _, s)| s.stalled_retry_loop)
        .count();

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
            stalled_retry_loop: score.stalled_retry_loop,
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
        finish_reason_length,
        stalled_retry_loop_total,
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
    if report.finish_reason_length > 0 {
        println!(
            "  {} LLM call(s) hit finishReason=\"length\" (context/token ceiling truncation)",
            report.finish_reason_length
        );
    }
    if report.stalled_retry_loop_total > 0 {
        println!(
            "  {} case(s) stalled in an unproductive retry loop (candidate failure mode, \
             not a harness crash)",
            report.stalled_retry_loop_total
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::Expected;
    use presenter_server::ai::agent::TurnMetadata;
    use std::path::PathBuf;

    fn case(id: &str, slice: &str) -> Case {
        Case {
            id: id.to_string(),
            slice: slice.to_string(),
            user_message: "test".to_string(),
            setup: None,
            expected: Expected::default(),
            source_path: PathBuf::new(),
        }
    }

    fn trace_with_turns(case_id: &str, duration_ms: u64, turns: Vec<TurnMetadata>) -> Trace {
        Trace {
            case_id: case_id.to_string(),
            slice: "worship-crud".to_string(),
            candidate_url: "http://test.invalid".to_string(),
            candidate_model: "test-model".to_string(),
            char_limit: 0,
            prior_turn_count: 0,
            conversation: Vec::new(),
            final_response: None,
            error: None,
            seed_failed: false,
            duration_ms,
            usage: None,
            turns,
            stalled_retry_loop: None,
            captured_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn score(case_id: &str, passed: bool) -> CaseScore {
        CaseScore {
            case_id: case_id.to_string(),
            passed,
            seed_failed: false,
            stalled_retry_loop: false,
            failures: Vec::new(),
        }
    }

    /// #662 defect 6: `finishReasonLength` in the report summary counts
    /// every `turns[].finishReason == "length"` across ALL cases, not just
    /// the first one it finds.
    #[test]
    fn finish_reason_length_counts_across_every_case() {
        let c1 = case("a", "worship-crud");
        let t1 = trace_with_turns(
            "a",
            100,
            vec![
                TurnMetadata {
                    finish_reason: Some("stop".into()),
                    reasoning_content_len: None,
                },
                TurnMetadata {
                    finish_reason: Some("length".into()),
                    reasoning_content_len: Some(50),
                },
            ],
        );
        let s1 = score("a", false);

        let c2 = case("b", "worship-crud");
        let t2 = trace_with_turns(
            "b",
            50,
            vec![TurnMetadata {
                finish_reason: Some("length".into()),
                reasoning_content_len: None,
            }],
        );
        let s2 = score("b", true);

        let results = vec![(&c1, &t1, s1), (&c2, &t2, s2)];
        let report = build_report(&results);

        assert_eq!(
            report.finish_reason_length, 2,
            "one 'length' turn in case a, one 'length' turn in case b"
        );
        assert_eq!(report.total_duration_ms, 150);
    }

    #[test]
    fn finish_reason_length_is_zero_when_nothing_was_truncated() {
        let c1 = case("a", "worship-crud");
        let t1 = trace_with_turns(
            "a",
            10,
            vec![TurnMetadata {
                finish_reason: Some("stop".into()),
                reasoning_content_len: None,
            }],
        );
        let s1 = score("a", true);

        let results = vec![(&c1, &t1, s1)];
        let report = build_report(&results);
        assert_eq!(report.finish_reason_length, 0);
    }

    /// #662 defect 7: `stalledRetryLoopTotal` counts `CaseScore.stalled_retry_loop`
    /// across every case, and `CaseResult.stalledRetryLoop` carries it
    /// per-case into `results.json`.
    #[test]
    fn stalled_retry_loop_total_counts_across_every_case() {
        let c1 = case("adv-10", "adversarial");
        let t1 = trace_with_turns("adv-10", 5000, Vec::new());
        let mut s1 = score("adv-10", false);
        s1.stalled_retry_loop = true;

        let c2 = case("wc-01", "worship-crud");
        let t2 = trace_with_turns("wc-01", 10, Vec::new());
        let s2 = score("wc-01", true);

        let results = vec![(&c1, &t1, s1), (&c2, &t2, s2)];
        let report = build_report(&results);

        assert_eq!(report.stalled_retry_loop_total, 1);
        assert!(report.cases[0].stalled_retry_loop);
        assert!(!report.cases[1].stalled_retry_loop);
    }
}
