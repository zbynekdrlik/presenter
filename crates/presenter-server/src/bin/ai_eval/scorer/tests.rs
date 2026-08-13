//! Layer-1 scorer unit tests — hand-written trace fixtures, ZERO live
//! infrastructure (no `AppState`, no network, no LLM). One "good trace
//! passes everything" test, plus at least one failing fixture per
//! violation class the scorer checks (#680 acceptance).

use super::score_trace;
use crate::corpus::{Case, DeleteGateExpectation, Expected, OverriddenVerse, VerbatimVerse};
use crate::trace::Trace;
use presenter_server::ai::{ChatMessage, ToolCallFunction, ToolCallMessage};
use serde_json::json;
use std::path::PathBuf;

// --- fixture builders ---

fn case(id: &str, slice: &str, expected: Expected) -> Case {
    Case {
        id: id.to_string(),
        slice: slice.to_string(),
        user_message: "test message".to_string(),
        setup: None,
        expected,
        source_path: PathBuf::new(),
    }
}

fn trace(case_id: &str, slice: &str, char_limit: u32, conversation: Vec<ChatMessage>) -> Trace {
    Trace {
        case_id: case_id.to_string(),
        slice: slice.to_string(),
        candidate_url: "http://test.invalid".to_string(),
        candidate_model: "test-model".to_string(),
        char_limit,
        prior_turn_count: 0,
        conversation,
        final_response: Some("Hotovo.".to_string()),
        error: None,
        seed_failed: false,
        duration_ms: 0,
        usage: None,
        turns: Vec::new(),
        stalled_retry_loop: None,
        captured_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

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

fn assistant_text(text: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".to_string(),
        content: Some(text.to_string()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        preview: None,
    }
}

fn long_emphasis_args(limit: usize) -> serde_json::Value {
    let text = "A".repeat(limit + 20);
    json!({"name": "x", "items": [{"kind": "emphasis", "text": text}]})
}

fn short_emphasis_args() -> serde_json::Value {
    json!({"name": "x", "items": [{"kind": "emphasis", "text": "OK"}]})
}

// --- good trace ---

#[test]
fn good_verbatim_verse_trace_passes_every_check() {
    let items = json!({
        "name": "Sermon",
        "items": [
            {"kind": "verse", "number": 16, "text": "Veď Boh tak miloval svet.",
             "book": "Ján", "chapter": 3, "translation": "SEB"}
        ]
    });
    let conv = vec![
        assistant_tool_call("t1", "load_bible_verses", json!({})),
        tool_result(
            "t1",
            "load_bible_verses",
            json!([{"number": 16, "text": "Veď Boh tak miloval svet."}]),
        ),
        assistant_tool_call("t2", "create_bible_presentation", items),
        tool_result(
            "t2",
            "create_bible_presentation",
            json!({"id": "x", "name": "Sermon", "slide_count": 1}),
        ),
        assistant_text("Hotovo."),
    ];
    let c = case(
        "ba-01",
        "bible-authoring",
        Expected {
            tool_sequence: vec![
                "load_bible_verses".into(),
                "create_bible_presentation".into(),
            ],
            verbatim_verses: vec![VerbatimVerse {
                reference: "Ján 3:16".into(),
                translation: "SEB".into(),
                text: "Veď Boh tak miloval svet.".into(),
            }],
            ..Default::default()
        },
    );
    let t = trace("ba-01", "bible-authoring", 320, conv);
    let score = score_trace(&c, &t);
    assert!(
        score.passed,
        "expected pass, got failures: {:?}",
        score.failures
    );
}

// --- sequencing sanity ---

#[test]
fn tool_sequence_violation_fails() {
    let conv = vec![assistant_tool_call(
        "t1",
        "create_bible_presentation",
        json!({"name": "x", "items": []}),
    )];
    let c = case(
        "seq-x",
        "bible-authoring",
        Expected {
            tool_sequence: vec![
                "load_bible_verses".into(),
                "create_bible_presentation".into(),
            ],
            ..Default::default()
        },
    );
    let t = trace("seq-x", "bible-authoring", 320, conv);
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(score.failures.iter().any(|f| f.contains("toolSequence")));
}

#[test]
fn max_iterations_violation_fails() {
    let mut conv = Vec::new();
    for i in 0..12 {
        let id = format!("t{i}");
        conv.push(assistant_tool_call(&id, "get_style_guide", json!({})));
        conv.push(tool_result(&id, "get_style_guide", json!({"ok": true})));
    }
    let c = case("loop-x", "worship-crud", Expected::default());
    let t = trace("loop-x", "worship-crud", 320, conv);
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(score.failures.iter().any(|f| f.contains("iteration count")));
}

// --- real packer/validator replay (validationErrors) ---

#[test]
fn validation_error_never_fired_fails() {
    let items = json!({"name": "x", "items": [
        {"kind": "verse", "number": 1, "text": "short", "book": "Ján", "chapter": 1, "translation": "SEB"}
    ]});
    let conv = vec![assistant_tool_call(
        "t1",
        "create_bible_presentation",
        items,
    )];
    let c = case(
        "adv-x",
        "adversarial",
        Expected {
            validation_errors: vec!["main_exceeds_character_limit".into()],
            ..Default::default()
        },
    );
    let t = trace("adv-x", "adversarial", 320, conv);
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(score.failures.iter().any(|f| f.contains("never fired")));
}

#[test]
fn validation_error_fires_but_never_self_corrects_fails() {
    let conv = vec![assistant_tool_call(
        "t1",
        "create_bible_presentation",
        long_emphasis_args(320),
    )];
    let c = case(
        "adv-x",
        "adversarial",
        Expected {
            validation_errors: vec!["main_exceeds_character_limit".into()],
            self_correct_within_retries: Some(3),
            ..Default::default()
        },
    );
    let t = trace("adv-x", "adversarial", 320, conv);
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(score
        .failures
        .iter()
        .any(|f| f.contains("no self-correction succeeded")));
}

#[test]
fn validation_error_fires_then_self_corrects_passes() {
    let conv = vec![
        assistant_tool_call("t1", "create_bible_presentation", long_emphasis_args(320)),
        tool_result(
            "t1",
            "create_bible_presentation",
            json!({"error": "slide_validation", "rule": "main_exceeds_character_limit"}),
        ),
        assistant_tool_call("t2", "create_bible_presentation", short_emphasis_args()),
        tool_result("t2", "create_bible_presentation", json!({"id": "x"})),
    ];
    let c = case(
        "adv-x",
        "adversarial",
        Expected {
            validation_errors: vec!["main_exceeds_character_limit".into()],
            self_correct_within_retries: Some(3),
            ..Default::default()
        },
    );
    let t = trace("adv-x", "adversarial", 320, conv);
    let score = score_trace(&c, &t);
    assert!(
        score.passed,
        "expected pass, got failures: {:?}",
        score.failures
    );
}

#[test]
fn unexpected_validation_error_on_well_formed_case_fails() {
    // No expected.validationErrors — any replayed failure is a Layer-1
    // regression, even on a case that isn't declared "adversarial".
    let conv = vec![assistant_tool_call(
        "t1",
        "create_bible_presentation",
        long_emphasis_args(320),
    )];
    let c = case("ba-x", "bible-authoring", Expected::default());
    let t = trace("ba-x", "bible-authoring", 320, conv);
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(score
        .failures
        .iter()
        .any(|f| f.contains("unexpected validation error")));
}

// --- verse-text fidelity ---

#[test]
fn verbatim_verse_text_mismatch_fails() {
    let items = json!({"name": "x", "items": [
        {"kind": "verse", "number": 16, "text": "WRONG TEXT",
         "book": "Ján", "chapter": 3, "translation": "SEB"}
    ]});
    let conv = vec![assistant_tool_call(
        "t1",
        "create_bible_presentation",
        items,
    )];
    let c = case(
        "ba-x",
        "bible-authoring",
        Expected {
            verbatim_verses: vec![VerbatimVerse {
                reference: "Ján 3:16".into(),
                translation: "SEB".into(),
                text: "Veď Boh tak miloval svet.".into(),
            }],
            ..Default::default()
        },
    );
    let t = trace("ba-x", "bible-authoring", 320, conv);
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(score.failures.iter().any(|f| f.contains("text mismatch")));
}

#[test]
fn verbatim_verse_missing_from_submission_fails() {
    let c = case(
        "ba-x",
        "bible-authoring",
        Expected {
            verbatim_verses: vec![VerbatimVerse {
                reference: "Ján 3:16".into(),
                translation: "SEB".into(),
                text: "x".into(),
            }],
            ..Default::default()
        },
    );
    let t = trace("ba-x", "bible-authoring", 320, Vec::new());
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(score.failures.iter().any(|f| f.contains("not found")));
}

#[test]
fn overridden_verse_silently_reverted_to_db_text_fails() {
    let items = json!({"name": "x", "items": [
        {"kind": "verse", "number": 1, "text": "DB WORDING",
         "book": "Ján", "chapter": 1, "translation": "SEB"}
    ]});
    let conv = vec![assistant_tool_call(
        "t1",
        "create_bible_presentation",
        items,
    )];
    let c = case(
        "ba-x",
        "bible-authoring",
        Expected {
            overridden_verses: vec![OverriddenVerse {
                reference: "Ján 1:1".into(),
                translation: "SEB".into(),
                expected_text: "SERMON WORDING".into(),
                db_text: "DB WORDING".into(),
            }],
            ..Default::default()
        },
    );
    let t = trace("ba-x", "bible-authoring", 320, conv);
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(score
        .failures
        .iter()
        .any(|f| f.contains("silently reverted")));
}

#[test]
fn overridden_verse_matching_sermon_wording_passes() {
    let items = json!({"name": "x", "items": [
        {"kind": "verse", "number": 1, "text": "SERMON WORDING",
         "book": "Ján", "chapter": 1, "translation": "SEB"}
    ]});
    let conv = vec![assistant_tool_call(
        "t1",
        "create_bible_presentation",
        items,
    )];
    let c = case(
        "ba-x",
        "bible-authoring",
        Expected {
            overridden_verses: vec![OverriddenVerse {
                reference: "Ján 1:1".into(),
                translation: "SEB".into(),
                expected_text: "SERMON WORDING".into(),
                db_text: "DB WORDING".into(),
            }],
            ..Default::default()
        },
    );
    let t = trace("ba-x", "bible-authoring", 320, conv);
    let score = score_trace(&c, &t);
    assert!(
        score.passed,
        "expected pass, got failures: {:?}",
        score.failures
    );
}

// --- delete-intent gate ---

#[test]
fn delete_gate_blocked_expected_but_allowed_fails() {
    let conv = vec![
        assistant_tool_call("t1", "delete_presentation", json!({"presentation_id": "x"})),
        tool_result("t1", "delete_presentation", json!({"ok": true})),
    ];
    let c = case(
        "wc-x",
        "worship-crud",
        Expected {
            delete_gate: Some(DeleteGateExpectation::Blocked),
            ..Default::default()
        },
    );
    let t = trace("wc-x", "worship-crud", 320, conv);
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(score.failures.iter().any(|f| f.contains("NOT blocked")));
}

#[test]
fn delete_gate_blocked_expected_and_blocked_passes() {
    let conv = vec![
        assistant_tool_call("t1", "delete_presentation", json!({"presentation_id": "x"})),
        tool_result(
            "t1",
            "delete_presentation",
            json!({"error": "delete_blocked", "reason": "no delete intent"}),
        ),
    ];
    let c = case(
        "wc-x",
        "worship-crud",
        Expected {
            delete_gate: Some(DeleteGateExpectation::Blocked),
            ..Default::default()
        },
    );
    let t = trace("wc-x", "worship-crud", 320, conv);
    let score = score_trace(&c, &t);
    assert!(
        score.passed,
        "expected pass, got failures: {:?}",
        score.failures
    );
}

#[test]
fn delete_gate_allowed_expected_but_no_call_made_fails() {
    let c = case(
        "wc-x",
        "worship-crud",
        Expected {
            delete_gate: Some(DeleteGateExpectation::Allowed),
            ..Default::default()
        },
    );
    let t = trace("wc-x", "worship-crud", 320, Vec::new());
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(score.failures.iter().any(|f| f.contains("no delete_")));
}

#[test]
fn delete_gate_allowed_expected_and_allowed_passes() {
    let conv = vec![
        assistant_tool_call("t1", "delete_presentation", json!({"presentation_id": "x"})),
        tool_result("t1", "delete_presentation", json!({"ok": true})),
    ];
    let c = case(
        "wc-x",
        "worship-crud",
        Expected {
            delete_gate: Some(DeleteGateExpectation::Allowed),
            ..Default::default()
        },
    );
    let t = trace("wc-x", "worship-crud", 320, conv);
    let score = score_trace(&c, &t);
    assert!(
        score.passed,
        "expected pass, got failures: {:?}",
        score.failures
    );
}

// --- trace-level (run_agent) error ---

#[test]
fn trace_level_error_fails_regardless_of_conversation_content() {
    let c = case("x", "worship-crud", Expected::default());
    let mut t = trace("x", "worship-crud", 320, Vec::new());
    t.error = Some("connection refused".to_string());
    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(
        !score.seed_failed,
        "a genuine run_agent error is NOT a seed failure"
    );
    assert!(score
        .failures
        .iter()
        .any(|f| f.contains("run_agent returned an error")));
}

// --- seed failure (#662 defect 1) ---

#[test]
fn seed_failed_trace_is_classified_separately_from_a_candidate_error() {
    let c = case("ba-x", "bible-authoring", Expected::default());
    let mut t = trace("ba-x", "bible-authoring", 320, Vec::new());
    t.error = Some("seeding failed: environment variable PRESENTER_BIBLE_KJV must be set".into());
    t.seed_failed = true;

    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(
        score.seed_failed,
        "score_trace must propagate Trace::seed_failed onto CaseScore"
    );
    assert!(
        score
            .failures
            .iter()
            .any(|f| f.contains("NOT a model result")),
        "the failure message must explicitly disclaim being a model result: {:?}",
        score.failures
    );
}

// --- stalled retry loop (#662 defect 7) ---

#[test]
fn stalled_retry_loop_trace_is_classified_separately_from_a_generic_candidate_error() {
    let c = case("adv-10", "adversarial", Expected::default());
    let mut t = trace("adv-10", "adversarial", 320, Vec::new());
    // A stalled loop like adv-10's real one ends in a generic run_agent
    // error too (the eventual context-ceiling crash) — the stalled-loop
    // classification must win over the generic one.
    t.error = Some("AI API error: Failed to parse tool call arguments as JSON".into());
    t.stalled_retry_loop = Some(
        "stalled retry loop: tool 'create_bible_presentation' failed identically \
         (error 'invalid_verse_item') 8 times in a row"
            .into(),
    );

    let score = score_trace(&c, &t);
    assert!(!score.passed);
    assert!(
        score.stalled_retry_loop,
        "score_trace must propagate Trace::stalled_retry_loop onto CaseScore"
    );
    assert!(
        !score.seed_failed,
        "a stalled retry loop is a candidate result, not a seed failure"
    );
    assert!(
        score
            .failures
            .iter()
            .any(|f| f.contains("stalled") && f.contains("invalid_verse_item")),
        "the failure message must name the stall, not just relay the generic error: {:?}",
        score.failures
    );
    assert!(
        !score
            .failures
            .iter()
            .any(|f| f.contains("run_agent returned an error")),
        "the stalled-loop classification must WIN over the generic error branch: {:?}",
        score.failures
    );
}
