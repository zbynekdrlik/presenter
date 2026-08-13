//! The `drive` stage: run the REAL `run_agent` loop against a configurable
//! candidate model endpoint, once per corpus case, writing one trace JSON
//! per case. Never re-implements the agent loop — just wires it up.

use crate::corpus::Case;
use crate::seed::{build_state_for_case, prior_turns_to_messages};
use crate::trace::{now_rfc3339, Trace};
use presenter_server::ai::agent::run_agent;
use presenter_server::ai::{AiSettings, ChatMessage};
use std::time::Instant;

/// Drive one case through the real agent loop and capture its trace. Never
/// panics and never propagates an error up to the caller — a seeding
/// failure or a `run_agent` error is captured INSIDE the trace's `error`
/// field so one bad case can never abort the whole corpus run (the caller
/// loops over many cases and must keep going).
pub async fn drive_case(case: &Case, candidate_url: &str, candidate_model: &str) -> Trace {
    let started = Instant::now();
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
                started,
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
                started,
            )
        }
    };

    let settings = AiSettings {
        api_url: candidate_url.to_string(),
        api_key: None,
        model: candidate_model.to_string(),
        system_prompt_extra: None,
    };

    let (final_response, error, turns) = match run_agent(
        &case.user_message,
        &mut conversation,
        &state,
        &settings,
        None,
    )
    .await
    {
        Ok((response, _actions, turn_metadata)) => (Some(response), None, turn_metadata),
        Err(e) => (None, Some(format!("{e:#}")), Vec::new()),
    };

    // #662 defect 7: scan only THIS turn's own activity (never any seeded
    // prior-turn history) — same slicing convention `scorer::score_trace`
    // already uses for `prior_turn_count`.
    let turn_start = prior_turn_count.min(conversation.len());
    let stalled_retry_loop = detect_stalled_retry_loop(&conversation[turn_start..]);

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
        duration_ms: elapsed_ms(started),
        // Always None today — see TraceUsage's doc comment + #687.
        usage: None,
        turns,
        stalled_retry_loop,
        captured_at: now_rfc3339(),
    }
}

/// Milliseconds elapsed since `started`, saturating into `u64` — a per-case
/// eval run is seconds, never anywhere near `u64::MAX` ms, but a bare
/// `as u64` on a `u128` is a silent-truncation footgun clippy flags, so
/// this makes the (harmless, never-hit) saturation explicit instead.
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Build a trace recording a failure that happened BEFORE (or instead of)
/// ever calling the candidate model — `char_limit: 0` is a harmless
/// placeholder since the scorer never reaches any check that reads it once
/// `trace.error.is_some()` (see `scorer::score_trace`'s early return). Every
/// caller today is a pre-model (seeding) failure — `build_state_for_case`
/// or the immediately following bible-preferences read — so `seed_failed`
/// is always `true` here; a genuine `run_agent` error is captured directly
/// in `drive_case`'s main body instead, with `seed_failed: false`.
fn failed_trace(
    case: &Case,
    candidate_url: &str,
    candidate_model: &str,
    prior_turn_count: usize,
    conversation: Vec<ChatMessage>,
    error: String,
    started: Instant,
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
        seed_failed: true,
        duration_ms: elapsed_ms(started),
        usage: None,
        turns: Vec::new(),
        stalled_retry_loop: None,
        captured_at: now_rfc3339(),
    }
}

/// Loud report for every trace whose case could not even be SEEDED — a
/// harness/environment failure before the candidate model was ever called,
/// distinct from a candidate/model failure. `None` when no trace is
/// seed-failed. `main.rs::run_drive` both PRINTS this (so the operator sees
/// which case + why, without opening any trace file) and treats its
/// presence as fatal: a seed failure is not model-evaluation data and must
/// never silently degrade into a "0% pass" result at `score-l1` (#662
/// smoke-run finding — previously `drive` exited 0 with a healthy-looking
/// "Wrote N trace(s)" even when most cases never reached the model at all).
pub fn seed_failure_report(traces: &[Trace]) -> Option<String> {
    let failed: Vec<&Trace> = traces.iter().filter(|t| t.seed_failed).collect();
    if failed.is_empty() {
        return None;
    }
    let mut msg = format!(
        "{} of {} case(s) could not be seeded — excluded from model evaluation:\n",
        failed.len(),
        traces.len()
    );
    for t in &failed {
        let reason = t.error.as_deref().unwrap_or("(no reason recorded)");
        msg.push_str(&format!("  {}: {reason}\n", t.case_id));
    }
    msg.push_str(
        "Fix the harness/environment (see reasons above) before evaluating model quality — \
         each failed trace is written with seedFailed:true so score-l1 never scores it as a \
         model failure.",
    );
    Some(msg)
}

/// Consecutive-identical-failure threshold past which a retry loop is a
/// STALLED one rather than legitimate self-correction (#662 defect 7 —
/// the reasoning-on rerun's `adv-10`: 8 near-identical
/// `create_bible_presentation` retries, all failing the same way, until
/// the accumulated context crashed the request with a malformed-JSON HTTP
/// 500 — a harness-visible CRASH for what was really a candidate failure
/// mode).
const STALLED_RETRY_THRESHOLD: usize = 3;

/// One failed tool call's "shape" for stall detection — coarse ON
/// PURPOSE: the tool name, the SET of argument top-level keys (not their
/// values — a model retrying with different wording in the same fields is
/// still the same unproductive pattern, per the ticket's "same tool, same
/// args shape, same error class"), and the error/rule key the tool result
/// reported. Two failures with the same shape are "the same mistake,
/// retried".
#[derive(Debug, Clone, PartialEq, Eq)]
struct FailedCallShape {
    tool: String,
    arg_keys: Vec<String>,
    error_class: String,
}

/// Sorted top-level JSON object keys of a tool call's raw `arguments`
/// string — empty when the arguments aren't a JSON object (malformed or
/// unparseable arguments are their own distinct, degenerate "shape" of
/// empty keys, which is fine: a genuinely different malformed-arguments
/// attempt naturally won't repeat with the SAME error_class either).
fn arg_keys(arguments_json: &str) -> Vec<String> {
    let mut keys: Vec<String> = serde_json::from_str::<serde_json::Value>(arguments_json)
        .ok()
        .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
        .unwrap_or_default();
    keys.sort();
    keys
}

/// The `error` (or, for the bible validator's rule-keyed failures,
/// `rule`) field of a tool RESULT's JSON content — `None` for a
/// successful call (no such key) or unparseable content.
fn tool_result_error_class(content: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    v.get("error")
        .and_then(serde_json::Value::as_str)
        .or_else(|| v.get("rule").and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

/// Every tool call in `conversation`, in order, as `Some(shape)` when it
/// FAILED (its paired tool result carried an `error`/`rule` key) or `None`
/// when it succeeded (or its result couldn't be matched/parsed) — a `None`
/// entry breaks any in-progress streak of identical failures.
fn failed_call_shapes(conversation: &[ChatMessage]) -> Vec<Option<FailedCallShape>> {
    let mut shapes = Vec::new();
    for msg in conversation {
        let Some(tool_calls) = &msg.tool_calls else {
            continue;
        };
        for tc in tool_calls {
            let result_content = conversation
                .iter()
                .find(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some(tc.id.as_str()))
                .and_then(|m| m.content.as_deref());
            let shape = result_content
                .and_then(tool_result_error_class)
                .map(|error_class| FailedCallShape {
                    tool: tc.function.name.clone(),
                    arg_keys: arg_keys(&tc.function.arguments),
                    error_class,
                });
            shapes.push(shape);
        }
    }
    shapes
}

/// Scan `conversation` for `STALLED_RETRY_THRESHOLD` (or more) CONSECUTIVE
/// tool calls with the identical failure "shape" (same tool, same
/// argument key set, same error class) — a model stuck retrying a mistake
/// it never diagnoses, not legitimate self-correction (#662 defect 7).
/// Pure, trace-only (no `AppState`/network) — same discipline as every
/// other Layer-1-style check in this harness (`scorer::turn_analysis`).
/// Returns a human-readable description naming the offending tool + error
/// + repeat count when found, `None` otherwise.
pub fn detect_stalled_retry_loop(conversation: &[ChatMessage]) -> Option<String> {
    let shapes = failed_call_shapes(conversation);
    let mut run_shape: Option<FailedCallShape> = None;
    let mut run_len = 0usize;

    for shape in shapes {
        if shape == run_shape {
            if shape.is_some() {
                run_len += 1;
            }
        } else {
            run_len = usize::from(shape.is_some());
            run_shape = shape;
        }

        if run_len >= STALLED_RETRY_THRESHOLD {
            let s = run_shape
                .as_ref()
                .expect("run_len >= 1 implies run_shape is Some");
            return Some(format!(
                "stalled retry loop: tool '{}' failed identically (error '{}') {} times in a row",
                s.tool, s.error_class, run_len
            ));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::Expected;
    use presenter_server::ai::{ToolCallFunction, ToolCallMessage};
    use std::path::PathBuf;

    fn assistant_tool_call(id: &str, name: &str, args: serde_json::Value) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![ToolCallMessage {
                id: id.to_string(),
                call_type: "function".to_string(),
                function: ToolCallFunction {
                    name: name.to_string(),
                    arguments: args.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
            preview: None,
        }
    }

    fn tool_result(id: &str, name: &str, content: serde_json::Value) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: Some(content.to_string()),
            tool_calls: None,
            tool_call_id: Some(id.to_string()),
            name: Some(name.to_string()),
            preview: None,
        }
    }

    /// #662 defect 7: a stalled retry loop — 3 CONSECUTIVE
    /// `create_bible_presentation` calls, identical argument key set, all
    /// failing with the identical `invalid_verse_item` error — must be
    /// detected (mirrors the reasoning-on rerun's `adv-10`, which did this
    /// 8 times before the accumulated context crashed the request).
    #[test]
    fn stalled_retry_loop_of_identical_failures_is_detected() {
        let args = serde_json::json!({"name": "x", "items": []});
        let conv = vec![
            assistant_tool_call("t1", "create_bible_presentation", args.clone()),
            tool_result(
                "t1",
                "create_bible_presentation",
                serde_json::json!({"error": "invalid_verse_item"}),
            ),
            assistant_tool_call("t2", "create_bible_presentation", args.clone()),
            tool_result(
                "t2",
                "create_bible_presentation",
                serde_json::json!({"error": "invalid_verse_item"}),
            ),
            assistant_tool_call("t3", "create_bible_presentation", args),
            tool_result(
                "t3",
                "create_bible_presentation",
                serde_json::json!({"error": "invalid_verse_item"}),
            ),
        ];

        let result = detect_stalled_retry_loop(&conv);
        assert!(
            result.is_some(),
            "3 consecutive identical create_bible_presentation failures must be \
             detected as a stalled retry loop"
        );
        let msg = result.unwrap();
        assert!(
            msg.contains("create_bible_presentation"),
            "message must name the offending tool: {msg}"
        );
    }

    /// A genuine self-correction — 2 attempts with DIFFERENT arg shapes and
    /// different error classes, then success — must NOT be flagged. Proves
    /// the detector isn't just "N failed tool calls total".
    #[test]
    fn different_shaped_failures_do_not_count_as_a_stalled_loop() {
        let conv = vec![
            assistant_tool_call(
                "t1",
                "create_bible_presentation",
                serde_json::json!({"name": "x"}),
            ),
            tool_result(
                "t1",
                "create_bible_presentation",
                serde_json::json!({"error": "missing_items"}),
            ),
            assistant_tool_call(
                "t2",
                "create_bible_presentation",
                serde_json::json!({"name": "x", "items": []}),
            ),
            tool_result(
                "t2",
                "create_bible_presentation",
                serde_json::json!({"error": "invalid_verse_item"}),
            ),
            assistant_tool_call(
                "t3",
                "create_bible_presentation",
                serde_json::json!({"name": "x", "items": [{"kind": "verse"}]}),
            ),
            tool_result(
                "t3",
                "create_bible_presentation",
                serde_json::json!({"id": "ok"}),
            ),
        ];
        assert!(
            detect_stalled_retry_loop(&conv).is_none(),
            "3 DIFFERENTLY-shaped attempts (different arg keys, different \
             errors, then success) must not be flagged"
        );
    }

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

    /// #662 defect 1: a bible-authoring/adversarial case whose seeding
    /// fails (here: the 5 `PRESENTER_BIBLE_*` env vars unset, so
    /// `AppState::refresh_default_bible_translations` fails on the very
    /// first spec — deterministic, no filesystem/network involved) must be
    /// marked `seedFailed: true` in its trace, distinct from a genuine
    /// candidate/model error. Never dials `candidate_url` — seeding fails
    /// before `run_agent` is ever reached, so no network dependency exists
    /// either way.
    #[tokio::test]
    async fn seeding_failure_is_marked_seed_failed_not_a_candidate_error() {
        // Mutates process-global env state — matches the existing precedent
        // in presenter-importer/src/bible.rs's own bible-env-var tests. Safe
        // here specifically because no OTHER test in this binary (ai_eval,
        // built only under the ai-eval feature) reads or sets these 5 vars,
        // so there is nothing to race against within this test binary's
        // process. If a future test in THIS file (or a sibling module of
        // ai_eval) ever needs one of these vars set, this test would need
        // its own isolation (e.g. a mutex, or moving env mutation out of
        // #[tokio::test]'s multi-threaded runtime).
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

    fn trace_fixture(case_id: &str, seed_failed: bool, error: Option<&str>) -> Trace {
        Trace {
            case_id: case_id.to_string(),
            slice: "bible-authoring".to_string(),
            candidate_url: "http://test.invalid".to_string(),
            candidate_model: "test-model".to_string(),
            char_limit: 0,
            prior_turn_count: 0,
            conversation: Vec::new(),
            final_response: None,
            error: error.map(str::to_string),
            seed_failed,
            duration_ms: 0,
            usage: None,
            turns: Vec::new(),
            stalled_retry_loop: None,
            captured_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn seed_failure_report_is_none_when_nothing_seed_failed() {
        let traces = vec![
            trace_fixture("a", false, None),
            trace_fixture(
                "b",
                false,
                Some("run_agent returned an error: connection refused"),
            ),
        ];
        assert!(
            seed_failure_report(&traces).is_none(),
            "a genuine candidate/model error must never trip the seed-failure report"
        );
    }

    #[test]
    fn seed_failure_report_names_every_seed_failed_case_and_its_reason() {
        let traces = vec![
            trace_fixture("a", false, None),
            trace_fixture("b", true, Some("seeding failed: env var unset")),
            trace_fixture("c", true, Some("seeding failed: bad path")),
        ];
        let report =
            seed_failure_report(&traces).expect("must report when ANY trace is seed-failed");
        assert!(
            report.contains("2 of 3"),
            "must state the seed-failed count out of the total: {report}"
        );
        assert!(
            report.contains("b: seeding failed: env var unset"),
            "must name case b + its reason: {report}"
        );
        assert!(
            report.contains("c: seeding failed: bad path"),
            "must name case c + its reason: {report}"
        );
    }
}
