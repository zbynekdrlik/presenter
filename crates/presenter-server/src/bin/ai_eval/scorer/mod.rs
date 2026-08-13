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
    pub failures: Vec<String>,
}

/// Score one trace against its case's `expected` block. See the module doc
/// for what "pure" means here — this function touches nothing but its two
/// arguments.
pub fn score_trace(case: &Case, trace: &Trace) -> CaseScore {
    let mut failures = Vec::new();

    if let Some(err) = &trace.error {
        // A run that errored produced no conversation worth checking
        // further — every other check would just report "not found" noise
        // on top of the real cause.
        failures.push(format!("run_agent returned an error: {err}"));
        return CaseScore {
            case_id: case.id.clone(),
            passed: false,
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
        failures,
    }
}
