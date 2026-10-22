//! #870 Companion nameplate GRID: a slot is a 1-based position in the default
//! output's person-plate list. The slot LABEL variables (`nameplate_slot_<n>_*`)
//! and the `stream_nameplate_slot` PRESS both resolve the slot against the same
//! list (`list_stream_nameplates`: position, then id), so a button fires the
//! plate its label shows — also after a reorder. A press carrying the label's
//! plate id (`expect_id`) is refused when the slot changed in between, and
//! only the `stream` output (the one with slot variables) accepts presses.

use super::catalog_tests::{isolated_state, next_frame, ClientWs};
use super::nameplate_slot::{SlotMode, SlotPress};
use super::nameplates::{resolve_nameplates, NameplatePlate, NAMEPLATE_MIN_SLOTS};
use super::stream::StreamCommand;
use super::tests::{expect_ack, expect_error};
use super::variables::CompanionVariableState;
use super::*;
use crate::live::LiveEvent;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use futures_util::SinkExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tower::ServiceExt;

/// The output with slot variables (the migration-seeded default output).
const STREAM: &str = "stream";

fn plate(id: i64, name: &str, role: &str) -> NameplatePlate {
    NameplatePlate {
        id,
        name: name.to_string(),
        role: role.to_string(),
    }
}

/// The variables a state renders, as a name → value map.
fn rendered(state: &CompanionVariableState) -> HashMap<String, String> {
    state
        .to_variables()
        .into_iter()
        .map(|v| (v.name, v.value))
        .collect()
}

/// `(name, role, id)` of slot `n`, or `None` when the slot is not defined.
fn slot(vars: &HashMap<String, String>, n: usize) -> Option<(String, String, String)> {
    Some((
        vars.get(&format!("nameplate_slot_{n}_name"))?.clone(),
        vars.get(&format!("nameplate_slot_{n}_role"))?.clone(),
        vars.get(&format!("nameplate_slot_{n}_id"))?.clone(),
    ))
}

fn owned(name: &str, role: &str, id: &str) -> Option<(String, String, String)> {
    Some((name.to_string(), role.to_string(), id.to_string()))
}

/// Add plates `names` to output `slug` (in that order, role `r-<name>`).
/// Returns the plate ids in creation order.
async fn add_plates(state: &AppState, slug: &str, names: &[&str]) -> Vec<i64> {
    let mut ids = Vec::new();
    for name in names {
        let created = state
            .repository()
            .create_stream_nameplate(slug, name, &format!("r-{name}"))
            .await
            .unwrap();
        ids.push(created.id);
    }
    ids
}

/// A fresh state (its own in-memory DB) with plates `names` on `stream`.
async fn stream_with_plates(names: &[&str]) -> (AppState, Vec<i64>) {
    let state = AppState::in_memory().await.unwrap();
    let ids = add_plates(&state, STREAM, names).await;
    (state, ids)
}

/// The id of the PERSON plate on air in `slug`, if any.
async fn on_air_id(state: &AppState, slug: &str) -> Option<i64> {
    state
        .stream_nameplate_active(slug)
        .await
        .and_then(|active| active.nameplate_id)
}

/// `DELETE /stream/api/outputs/{slug}` through the real router.
async fn delete_output_via_api(state: &AppState, slug: &str) -> StatusCode {
    let request = Request::builder()
        .method(Method::DELETE)
        .uri(format!("/stream/api/outputs/{slug}"))
        .body(Body::empty())
        .unwrap();
    crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap()
        .status()
}

// ---- Slot variables -------------------------------------------------------

#[test]
fn slot_variables_follow_list_order_and_always_cover_eleven_slots() {
    let mut state = CompanionVariableState::default();
    state.apply_nameplates(vec![
        plate(9, "Ján Novák", "pastor"),
        plate(4, "Eva Malá", ""),
    ]);
    let vars = rendered(&state);

    // List order, not id order: plate 9 is first in the list.
    assert_eq!(slot(&vars, 1), owned("Ján Novák", "pastor", "9"));
    assert_eq!(slot(&vars, 2), owned("Eva Malá", "", "4"));
    // Slots past the list exist with empty texts, up to the 11-slot minimum.
    for n in 3..=NAMEPLATE_MIN_SLOTS {
        assert_eq!(slot(&vars, n), owned("", "", ""), "slot {n} must be empty");
    }
    assert_eq!(slot(&vars, 12), None, "no slot past the minimum");
}

#[test]
fn slot_variables_exist_with_no_plates_at_all() {
    let vars = rendered(&CompanionVariableState::default());
    for n in 1..=NAMEPLATE_MIN_SLOTS {
        assert_eq!(slot(&vars, n), owned("", "", ""), "slot {n}");
    }
    assert_eq!(slot(&vars, 12), None);
}

#[test]
fn slot_variables_grow_past_eleven_with_more_plates() {
    let plates: Vec<NameplatePlate> = (1..=13)
        .map(|i| plate(100 + i, &format!("Osoba {i}"), "host"))
        .collect();
    let mut state = CompanionVariableState::default();
    state.apply_nameplates(plates);
    let vars = rendered(&state);

    assert_eq!(slot(&vars, 12), owned("Osoba 12", "host", "112"));
    assert_eq!(slot(&vars, 13), owned("Osoba 13", "host", "113"));
    assert_eq!(slot(&vars, 14), None);
}

#[tokio::test]
async fn reorder_moves_the_slot_variables() {
    let (state, ids) = stream_with_plates(&["Anna", "Boris", "Cyril"]).await;
    let (a, b, c) = (ids[0], ids[1], ids[2]);

    let mut variables = CompanionVariableState::default();
    variables.apply_nameplates(resolve_nameplates(&state, STREAM).await);
    let before = rendered(&variables);
    assert_eq!(slot(&before, 1), owned("Anna", "r-Anna", &a.to_string()));
    assert_eq!(slot(&before, 2), owned("Boris", "r-Boris", &b.to_string()));

    state
        .repository()
        .set_nameplate_order(STREAM, vec![c, a, b])
        .await
        .unwrap();
    variables.apply_nameplates(resolve_nameplates(&state, STREAM).await);
    let after = rendered(&variables);
    assert_eq!(slot(&after, 1), owned("Cyril", "r-Cyril", &c.to_string()));
    assert_eq!(slot(&after, 2), owned("Anna", "r-Anna", &a.to_string()));
    assert_eq!(slot(&after, 3), owned("Boris", "r-Boris", &b.to_string()));
}

// ---- `stream_nameplate_slot` parsing --------------------------------------

/// The parsed press of a `stream_nameplate_slot` payload (panics otherwise).
fn parsed_press(payload: Value) -> SlotPress {
    match parse_command("stream_nameplate_slot", payload) {
        Ok(CompanionCommand::Stream(StreamCommand::NameplateSlot(press))) => press,
        Ok(_) => panic!("expected Stream(NameplateSlot)"),
        Err(message) => panic!("expected a parsed press, got: {message}"),
    }
}

#[test]
fn slot_command_parses_with_toggle_stream_and_no_check_defaults() {
    assert_eq!(
        parsed_press(json!({ "slot": 2 })),
        SlotPress {
            output: STREAM.to_string(),
            slot: 2,
            mode: SlotMode::Toggle,
            expect_id: None,
            expect_empty: false,
        }
    );
    // Parsing keeps whatever output was named (trimmed); the PRESS refuses
    // anything but `stream` (`slot_press_for_another_output_is_refused`).
    assert_eq!(
        parsed_press(
            json!({ "slot": 11, "mode": "show", "expect_id": 7, "output": " moderator " })
        ),
        SlotPress {
            output: "moderator".to_string(),
            slot: 11,
            mode: SlotMode::Show,
            expect_id: Some(7),
            expect_empty: false,
        }
    );
    // A label showing an EMPTY slot sends `expect_empty` instead of an id.
    assert_eq!(
        parsed_press(json!({ "slot": 3, "expect_empty": true })),
        SlotPress {
            output: STREAM.to_string(),
            slot: 3,
            mode: SlotMode::Toggle,
            expect_id: None,
            expect_empty: true,
        }
    );
}

#[test]
fn slot_command_refuses_a_bad_slot_or_payload() {
    let cases: Vec<(&str, Value)> = vec![
        ("slot 0", json!({ "slot": 0 })),
        ("missing slot", json!({ "mode": "toggle" })),
        ("negative slot", json!({ "slot": -1 })),
        ("fractional slot", json!({ "slot": 2.5 })),
        ("string slot", json!({ "slot": "2" })),
        ("unknown mode", json!({ "slot": 1, "mode": "hide" })),
        ("string expect_id", json!({ "slot": 1, "expect_id": "7" })),
        (
            "string expect_empty",
            json!({ "slot": 1, "expect_empty": "yes" }),
        ),
        (
            "expect_id with expect_empty",
            json!({ "slot": 1, "expect_id": 7, "expect_empty": true }),
        ),
        ("unknown field", json!({ "slot": 1, "id": 4 })),
    ];
    for (label, payload) in cases {
        assert!(
            parse_command("stream_nameplate_slot", payload).is_err(),
            "{label} must be a parse error"
        );
    }
    let message = match parse_command("stream_nameplate_slot", json!({ "slot": 0 })) {
        Err(message) => message,
        Ok(_) => panic!("slot 0 must be refused"),
    };
    assert!(message.contains("1 or higher"), "unexpected: {message}");
}

// ---- `stream_nameplate_slot` execution ------------------------------------

#[tokio::test]
async fn slot_press_toggles_the_plate_at_that_position() {
    let (state, ids) = stream_with_plates(&["Anna", "Boris", "Cyril"]).await;
    let press = json!({ "slot": 2 });

    expect_ack(&state, "stream_nameplate_slot", press.clone()).await;
    assert_eq!(on_air_id(&state, STREAM).await, Some(ids[1]));
    let active = state.stream_nameplate_active(STREAM).await.unwrap();
    assert_eq!(active.primary, "Boris");
    assert_eq!(active.secondary, "r-Boris");

    // Toggle: the same slot again takes it off air.
    expect_ack(&state, "stream_nameplate_slot", press).await;
    assert!(state.stream_nameplate_active(STREAM).await.is_none());
}

#[tokio::test]
async fn slot_press_in_show_mode_keeps_the_plate_on_air() {
    let (state, ids) = stream_with_plates(&["Anna", "Boris"]).await;
    let press = json!({ "slot": 1, "mode": "show" });

    expect_ack(&state, "stream_nameplate_slot", press.clone()).await;
    expect_ack(&state, "stream_nameplate_slot", press).await;
    assert_eq!(on_air_id(&state, STREAM).await, Some(ids[0]));
}

#[tokio::test]
async fn slot_press_without_expect_id_after_a_reorder_fires_the_plate_now_there() {
    let (state, ids) = stream_with_plates(&["Anna", "Boris", "Cyril"]).await;
    state
        .repository()
        .set_nameplate_order(STREAM, vec![ids[0], ids[2], ids[1]])
        .await
        .unwrap();

    // No `expect_id` (an older button / an empty module cache): today's
    // behaviour, the slot fires whatever plate it holds now.
    expect_ack(&state, "stream_nameplate_slot", json!({ "slot": 2 })).await;
    assert_eq!(on_air_id(&state, STREAM).await, Some(ids[2]));
}

#[tokio::test]
async fn slot_press_whose_expect_id_matches_fires() {
    let (state, ids) = stream_with_plates(&["Anna", "Boris"]).await;
    expect_ack(
        &state,
        "stream_nameplate_slot",
        json!({ "slot": 2, "expect_id": ids[1] }),
    )
    .await;
    assert_eq!(on_air_id(&state, STREAM).await, Some(ids[1]));
}

#[tokio::test]
async fn slot_press_whose_plate_moved_since_the_label_is_refused() {
    let (state, ids) = stream_with_plates(&["Anna", "Boris", "Cyril"]).await;
    // The button still shows Boris in slot 2, but Presenter moved Cyril there.
    state
        .repository()
        .set_nameplate_order(STREAM, vec![ids[0], ids[2], ids[1]])
        .await
        .unwrap();

    let message = expect_error(
        &state,
        "stream_nameplate_slot",
        json!({ "slot": 2, "expect_id": ids[1] }),
    )
    .await;
    assert!(
        message.contains("nameplate slot 2 changed — press again"),
        "unexpected: {message}"
    );
    assert!(state.stream_nameplate_active(STREAM).await.is_none());
}

#[tokio::test]
async fn slot_press_whose_labelled_plate_left_the_slot_is_refused() {
    let (state, ids) = stream_with_plates(&["Anna", "Boris"]).await;
    // The button still shows Boris in slot 2, but Boris was deleted.
    state
        .repository()
        .delete_stream_nameplate(ids[1])
        .await
        .unwrap();

    let message = expect_error(
        &state,
        "stream_nameplate_slot",
        json!({ "slot": 2, "expect_id": ids[1] }),
    )
    .await;
    assert!(message.contains("slot 2 changed"), "unexpected: {message}");
    assert!(state.stream_nameplate_active(STREAM).await.is_none());
}

#[tokio::test]
async fn slot_press_whose_empty_label_was_filled_is_refused() {
    let (state, _ids) = stream_with_plates(&["Anna", "Boris"]).await;
    // The button still shows slot 3 as empty, but Cyril was just added there.
    let cyril = add_plates(&state, STREAM, &["Cyril"]).await;

    let message = expect_error(
        &state,
        "stream_nameplate_slot",
        json!({ "slot": 3, "expect_empty": true }),
    )
    .await;
    assert!(
        message.contains("nameplate slot 3 changed — press again"),
        "unexpected: {message}"
    );
    assert!(state.stream_nameplate_active(STREAM).await.is_none());
    // A slot that is still empty keeps the plain "empty slot" refusal.
    let message = expect_error(
        &state,
        "stream_nameplate_slot",
        json!({ "slot": 4, "expect_empty": true }),
    )
    .await;
    assert!(message.contains("slot 4 is empty"), "unexpected: {message}");
    // The ids stay usable: slot 3 with Cyril's id fires him.
    expect_ack(
        &state,
        "stream_nameplate_slot",
        json!({ "slot": 3, "expect_id": cyril[0] }),
    )
    .await;
    assert_eq!(on_air_id(&state, STREAM).await, Some(cyril[0]));
}

#[tokio::test]
async fn empty_slot_press_is_refused_and_puts_nothing_on_air() {
    let (state, _ids) = stream_with_plates(&["Anna", "Boris"]).await;

    let message = expect_error(&state, "stream_nameplate_slot", json!({ "slot": 3 })).await;
    assert!(
        message.contains("slot 3 is empty in output stream"),
        "unexpected: {message}"
    );
    assert!(state.stream_nameplate_active(STREAM).await.is_none());
}

#[tokio::test]
async fn slot_press_for_another_output_is_refused() {
    let (state, _stream_ids) = stream_with_plates(&["Anna"]).await;
    state
        .repository()
        .create_stream_output("t870-other", "Other")
        .await
        .unwrap();
    add_plates(&state, "t870-other", &["Zuzana"]).await;

    // Only `stream` has slot labels: another output's slot 1 must never fire
    // from a button that shows `stream`'s slot 1.
    let message = expect_error(
        &state,
        "stream_nameplate_slot",
        json!({ "slot": 1, "output": "t870-other" }),
    )
    .await;
    assert!(
        message.contains(r#"only for output "stream", not "t870-other""#),
        "unexpected: {message}"
    );
    assert!(state.stream_nameplate_active("t870-other").await.is_none());
    assert!(state.stream_nameplate_active(STREAM).await.is_none());
}

#[tokio::test]
async fn slot_press_after_the_stream_output_is_deleted_is_refused() {
    let state = AppState::in_memory().await.unwrap();
    state
        .repository()
        .delete_stream_output(STREAM)
        .await
        .unwrap();
    let message = expect_error(&state, "stream_nameplate_slot", json!({ "slot": 1 })).await;
    assert!(
        message.contains(r#"unknown stream output "stream""#),
        "unexpected: {message}"
    );
}

// ---- Output delete ---------------------------------------------------------

#[tokio::test]
async fn deleting_the_stream_output_announces_its_plate_list_change() {
    let state = AppState::in_memory().await.unwrap();
    state
        .repository()
        .create_stream_output("t870-gone", "Gone")
        .await
        .unwrap();
    let mut rx = state.live_hub().subscribe();

    assert_eq!(
        delete_output_via_api(&state, "t870-gone").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        delete_output_via_api(&state, STREAM).await,
        StatusCode::NO_CONTENT
    );

    let mut changed = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let LiveEvent::StreamNameplatesChanged { output } = event {
            changed.push(output);
        }
    }
    // Companion tracks only `stream`; another output's delete stays silent, so
    // an editor still showing it never refetches a 404 plate list (#855 E2E).
    assert_eq!(changed, vec![STREAM.to_string()]);
}

#[tokio::test]
async fn deleting_the_stream_output_with_a_plate_on_air_announces_it_hidden() {
    let (state, ids) = stream_with_plates(&["Anna"]).await;
    expect_ack(&state, "stream_nameplate_slot", json!({ "slot": 1 })).await;
    assert_eq!(on_air_id(&state, STREAM).await, Some(ids[0]));
    let mut rx = state.live_hub().subscribe();

    assert_eq!(
        delete_output_via_api(&state, STREAM).await,
        StatusCode::NO_CONTENT
    );

    let mut hidden = false;
    while let Ok(event) = rx.try_recv() {
        if let LiveEvent::StreamNameplate { output, active } = event {
            hidden |= output == STREAM && active.is_none();
        }
    }
    assert!(hidden, "the on-air plate must be announced hidden");
    assert!(state.stream_nameplate_active(STREAM).await.is_none());
}

// ---- The real `/companion/ws` session --------------------------------------

/// The value of variable `name` in a `variables` frame, if present.
fn frame_value<'a>(frame: &'a Value, name: &str) -> Option<&'a str> {
    frame["values"]
        .as_array()?
        .iter()
        .find(|var| var["name"] == name)?["value"]
        .as_str()
}

/// The plate names of a `nameplates` frame, in list order.
fn frame_plate_names(frame: &Value) -> Vec<String> {
    frame["plates"]
        .as_array()
        .map(|plates| {
            plates
                .iter()
                .filter_map(|p| p["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Read frames until a `variables` frame whose slot 1 label is `name`; return
/// the frames seen before it plus that frame.
async fn until_slot_one(ws: &mut ClientWs, name: &str) -> (Vec<Value>, Value) {
    let mut seen = Vec::new();
    loop {
        let frame = next_frame(ws).await;
        if frame["type"] == "variables"
            && frame_value(&frame, "nameplate_slot_1_name") == Some(name)
        {
            return (seen, frame);
        }
        seen.push(frame);
    }
}

/// The plate names of the LAST `nameplates` frame among `frames`.
fn last_plate_list(frames: &[Value]) -> Vec<String> {
    let list = frames
        .iter()
        .rev()
        .find(|frame| frame["type"] == "nameplates")
        .expect("a nameplates frame before the variables");
    frame_plate_names(list)
}

// The plate list reaches the module BEFORE the variables (on connect, and on
// every list change), so the module has defined the slot variables when their
// values arrive; a reorder pushes the new slot labels, and deleting the output
// clears them.
// Multi-thread runtime: a real axum server + a real WS client run concurrently.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn companion_socket_sends_the_plate_list_first_and_live_slot_labels() {
    let (state, _dir) = isolated_state(true).await;
    let ids = add_plates(&state, STREAM, &["Anna", "Boris"]).await;
    let (anna, boris) = (ids[0], ids[1]);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (mut ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}/companion/ws"))
        .await
        .unwrap();
    let hello = json!({ "type": "hello", "client": "slot-test" });
    ws.send(WsMessage::Text(hello.to_string().into()))
        .await
        .unwrap();

    // Connect: welcome, then the plate list, then the variables.
    assert_eq!(next_frame(&mut ws).await["type"], "welcome");
    let list = next_frame(&mut ws).await;
    assert_eq!(list["type"], "nameplates", "plate list first: {list}");
    assert_eq!(frame_plate_names(&list), vec!["Anna", "Boris"]);
    let vars = next_frame(&mut ws).await;
    assert_eq!(
        vars["type"], "variables",
        "variables after the list: {vars}"
    );
    assert_eq!(frame_value(&vars, "nameplate_slot_1_name"), Some("Anna"));
    assert_eq!(frame_value(&vars, "nameplate_slot_1_role"), Some("r-Anna"));
    let anna_id = anna.to_string();
    assert_eq!(
        frame_value(&vars, "nameplate_slot_1_id"),
        Some(anna_id.as_str())
    );
    assert_eq!(frame_value(&vars, "nameplate_slot_2_name"), Some("Boris"));
    assert_eq!(frame_value(&vars, "nameplate_slot_3_name"), Some(""));
    assert_eq!(frame_value(&vars, "nameplate_slot_11_id"), Some(""));

    // Reorder in Presenter → the new list, then slot 1 = Boris.
    state
        .repository()
        .set_nameplate_order(STREAM, vec![boris, anna])
        .await
        .unwrap();
    state.stream_nameplates_changed(STREAM);
    let (before, vars) = until_slot_one(&mut ws, "Boris").await;
    assert_eq!(last_plate_list(&before), vec!["Boris", "Anna"]);
    let boris_id = boris.to_string();
    assert_eq!(
        frame_value(&vars, "nameplate_slot_1_id"),
        Some(boris_id.as_str())
    );
    assert_eq!(frame_value(&vars, "nameplate_slot_2_name"), Some("Anna"));

    // Deleting the output (its plates cascade) → an empty list, blank labels.
    assert_eq!(
        delete_output_via_api(&state, STREAM).await,
        StatusCode::NO_CONTENT
    );
    let (before, vars) = until_slot_one(&mut ws, "").await;
    assert!(last_plate_list(&before).is_empty());
    assert_eq!(frame_value(&vars, "nameplate_slot_1_id"), Some(""));
    assert_eq!(frame_value(&vars, "nameplate_slot_2_name"), Some(""));

    ws.close(None).await.unwrap();
    server.abort();
}
