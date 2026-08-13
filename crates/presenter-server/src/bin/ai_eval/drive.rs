//! The `drive` stage: run the REAL `run_agent` loop against a configurable
//! candidate model endpoint, once per corpus case, writing one trace JSON
//! per case. Never re-implements the agent loop — just wires it up.

use crate::corpus::Case;
use crate::seed::{build_state_for_case, prior_turns_to_messages};
use crate::trace::{now_rfc3339, Trace};
use presenter_server::ai::agent::run_agent;
use presenter_server::ai::{AiSettings, ChatMessage};

/// Drive one case through the real agent loop and capture its trace. Never
/// panics and never propagates an error up to the caller — a seeding
/// failure or a `run_agent` error is captured INSIDE the trace's `error`
/// field so one bad case can never abort the whole corpus run (the caller
/// loops over many cases and must keep going).
pub async fn drive_case(case: &Case, candidate_url: &str, candidate_model: &str) -> Trace {
    let mut conversation = prior_turns_to_messages(case.setup.as_ref());
    let prior_turn_count = conversation.len();

    let state = match build_state_for_case(case).await {
        Ok(state) => state,
        Err(e) => {
            return failed_trace(
                case,
                candidate_url,
                candidate_model,
                prior_turn_count,
                conversation,
                format!("seeding failed: {e:#}"),
            )
        }
    };

    let char_limit = match state.get_bible_preferences().await {
        Ok(prefs) => prefs.character_limit,
        Err(e) => {
            return failed_trace(
                case,
                candidate_url,
                candidate_model,
                prior_turn_count,
                conversation,
                format!("reading bible preferences failed: {e:#}"),
            )
        }
    };

    let settings = AiSettings {
        api_url: candidate_url.to_string(),
        api_key: None,
        model: candidate_model.to_string(),
        system_prompt_extra: None,
    };

    let (final_response, error) = match run_agent(
        &case.user_message,
        &mut conversation,
        &state,
        &settings,
        None,
    )
    .await
    {
        Ok((response, _actions)) => (Some(response), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };

    Trace {
        case_id: case.id.clone(),
        slice: case.slice.clone(),
        candidate_url: candidate_url.to_string(),
        candidate_model: candidate_model.to_string(),
        char_limit,
        prior_turn_count,
        conversation,
        final_response,
        error,
        seed_failed: false,
        captured_at: now_rfc3339(),
    }
}

/// Build a trace recording a failure that happened BEFORE (or instead of)
/// ever calling the candidate model — `char_limit: 0` is a harmless
/// placeholder since the scorer never reaches any check that reads it once
/// `trace.error.is_some()` (see `scorer::score_trace`'s early return).
fn failed_trace(
    case: &Case,
    candidate_url: &str,
    candidate_model: &str,
    prior_turn_count: usize,
    conversation: Vec<ChatMessage>,
    error: String,
) -> Trace {
    Trace {
        case_id: case.id.clone(),
        slice: case.slice.clone(),
        candidate_url: candidate_url.to_string(),
        candidate_model: candidate_model.to_string(),
        char_limit: 0,
        prior_turn_count,
        conversation,
        final_response: None,
        error: Some(error),
        // TODO(#662 defect 1, RED): every caller of this helper today is a
        // pre-model (seeding) failure, so this should be `true` — left
        // `false` here on purpose as the RED commit; the GREEN commit turns
        // this into `true` and the test below goes green.
        seed_failed: false,
        captured_at: now_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::Expected;
    use std::path::PathBuf;

    /// A bible-authoring case with no `setup` at all — `build_state_for_case`
    /// still triggers `refresh_default_bible_translations` purely from
    /// `case.slice`, so nothing in `setup` needs to exist for seeding to be
    /// exercised.
    fn bible_authoring_case_with_no_setup() -> Case {
        Case {
            id: "red-defect1-seed-failure".to_string(),
            slice: "bible-authoring".to_string(),
            user_message: "test message".to_string(),
            setup: None,
            expected: Expected::default(),
            source_path: PathBuf::new(),
        }
    }

    /// #662 defect 1 (RED): a bible-authoring/adversarial case whose
    /// seeding fails (here: the 5 `PRESENTER_BIBLE_*` env vars unset, so
    /// `AppState::refresh_default_bible_translations` fails on the very
    /// first spec — deterministic, no filesystem/network involved) must be
    /// marked `seedFailed: true` in its trace, distinct from a genuine
    /// candidate/model error. Never dials `candidate_url` — seeding fails
    /// before `run_agent` is ever reached, so no network dependency exists
    /// either way. Against today's code this is RED: `failed_trace` always
    /// writes `seed_failed: false`.
    #[tokio::test]
    async fn seeding_failure_is_marked_seed_failed_not_a_candidate_error() {
        for var in [
            "PRESENTER_BIBLE_KJV",
            "PRESENTER_BIBLE_SEB",
            "PRESENTER_BIBLE_ROHACEK",
            "PRESENTER_BIBLE_SEVP",
            "PRESENTER_BIBLE_MILOST",
        ] {
            std::env::remove_var(var);
        }

        let case = bible_authoring_case_with_no_setup();
        let trace = drive_case(&case, "http://candidate.invalid", "unused-model").await;

        assert!(
            trace.error.is_some(),
            "seeding must fail with the bible env vars unset"
        );
        assert!(
            trace.seed_failed,
            "a seeding failure must be marked seedFailed:true, distinct from a \
             candidate/model error — got error: {:?}",
            trace.error
        );
        assert!(
            trace.final_response.is_none(),
            "run_agent must never have been reached — seeding failed first"
        );
    }
}
