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
        captured_at: now_rfc3339(),
    }
}
