//! Layer-1 deterministic structural scorer (report §6.4, `corpus/SCHEMA.md`'s
//! "Layer-1 check → schema field mapping" table).
//!
//! **Pure and trace-only: no `AppState`, no DB, no network, no LLM.** Every
//! check here either (a) re-derives its verdict from the trace's OWN
//! recorded content (delete-gate outcome, tool-call ordering), or (b)
//! REPLAYS a captured tool call's raw arguments through the REAL production
//! functions (`parse_bible_items`, `compose_bible_items_into_slides`,
//! `validate_bible_slide` — all pure, imported from `presenter_server`, see
//! `bible_replay.rs`). This is what makes the scorer unit-testable against
//! hand-written trace fixtures with zero live infrastructure, per #680's
//! explicit requirement.

mod bible_replay;
mod turn_analysis;

#[cfg(test)]
mod tests;

use crate::corpus::Case;
use crate::trace::Trace;

/// One case's Layer-1 verdict. `failures` is empty iff `passed`.
#[derive(Debug, Clone)]
pub struct CaseScore {
    pub case_id: String,
    pub passed: bool,
    /// Mirrors `Trace::seed_failed` (#662 defect 1) — `true` means this
    /// case never even reached the candidate model (a harness/environment
    /// problem), so `failed` here is NOT a model-quality result. Callers
    /// (`report.rs`) surface this as its own bucket, distinct from a
    /// genuine candidate/model failure.
    pub seed_failed: bool,
    /// Mirrors `Trace::stalled_retry_loop` (#662 defect 7) — `true` means
    /// the candidate got stuck retrying an identical failing tool call
    /// `drive::STALLED_RETRY_THRESHOLD`+ times in a row, usually ending in
    /// a harness-visible crash (context-ceiling truncation) that is
    /// otherwise indistinguishable from a genuine infra/network error.
    /// This IS a candidate-quality result (unlike `seed_failed`) — just a
    /// distinctly NAMED failure mode, not a generic "run_agent returned an
    /// error".
    pub stalled_retry_loop: bool,
    pub failures: Vec<String>,
}

/// Score one trace against its case's `expected` block. See the module doc
/// for what "pure" means here — this function touches nothing but its two
/// arguments.
pub fn score_trace(case: &Case, trace: &Trace) -> CaseScore {
    let mut failures = Vec::new();

    if trace.seed_failed {
        // A case that never reached the candidate model is a harness/
        // environment problem, not a model-quality result — classified
        // distinctly so it can never masquerade as "the model scored 0%
        // here" (#662 smoke-run finding).
        let reason = trace.error.as_deref().unwrap_or("(no reason recorded)");
        failures.push(format!(
            "seed failed — harness/environment issue, NOT a model result: {reason}"
        ));
        return CaseScore {
            case_id: case.id.clone(),
            passed: false,
            seed_failed: true,
            stalled_retry_loop: false,
            failures,
        };
    }

    if let Some(reason) = &trace.stalled_retry_loop {
        // Checked BEFORE the generic `trace.error` branch: a stalled
        // retry loop usually ALSO ends in a `trace.error` (the eventual
        // context-ceiling crash), and this is the more specific,
        // actionable classification — a candidate failure MODE, not a
        // generic "run_agent returned an error" (#662 defect 7).
        failures.push(format!(
            "candidate stalled in an unproductive retry loop: {reason}"
        ));
        return CaseScore {
            case_id: case.id.clone(),
            passed: false,
            seed_failed: false,
            stalled_retry_loop: true,
            failures,
        };
    }

    if let Some(err) = &trace.error {
        // A run that errored produced no conversation worth checking
        // further — every other check would just report "not found" noise
        // on top of the real cause.
        failures.push(format!("run_agent returned an error: {err}"));
        return CaseScore {
            case_id: case.id.clone(),
            passed: false,
            seed_failed: false,
            stalled_retry_loop: false,
            failures,
        };
    }

    let start = trace.prior_turn_count.min(trace.conversation.len());
    let turn = &trace.conversation[start..];

    let attempts = bible_replay::collect_bible_presentation_attempts(turn, trace.char_limit);

    turn_analysis::check_tool_sequence(case, turn, &mut failures);
    turn_analysis::check_max_iterations(case, turn, &mut failures);
    turn_analysis::check_delete_gate(case, turn, &mut failures);
    bible_replay::check_validation_errors(case, &attempts, &mut failures);
    bible_replay::check_verbatim_verses(case, &attempts, &mut failures);
    bible_replay::check_overridden_verses(case, &attempts, &mut failures);

    CaseScore {
        case_id: case.id.clone(),
        passed: failures.is_empty(),
        seed_failed: false,
        stalled_retry_loop: false,
        failures,
    }
}
