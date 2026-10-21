//! #870 Companion nameplate GRID: a slot is a 1-based position in an output's
//! person-plate list. The slot LABEL variables (`nameplate_slot_<n>_*`) and the
//! `stream_nameplate_slot` PRESS both resolve the slot against the same list
//! (`list_stream_nameplates`: position, then id), so a button fires the plate
//! its label shows — also after a reorder.

use super::catalog_tests::{isolated_state, next_frame, ClientWs};
use super::nameplates::{resolve_nameplates, NameplatePlate, NAMEPLATE_MIN_SLOTS};
use super::stream::{SlotMode, StreamCommand};
use super::tests::{expect_ack, expect_error};
use super::variables::CompanionVariableState;
use super::*;
use futures_util::SinkExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio_tungstenite::tungstenite::Message as WsMessage;

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

/// Create an output with plates `names` (in that order, role `r-<name>`).
/// Returns the plate ids in creation order.
async fn seed_plates(state: &AppState, slug: &str, names: &[&str]) -> Vec<i64> {
    let repo = state.repository();
    repo.create_stream_output(slug, "Test 870").await.unwrap();
    let mut ids = Vec::new();
    for name in names {
        let created = repo
            .create_stream_nameplate(slug, name, &format!("r-{name}"))
            .await
            .unwrap();
        ids.push(created.id);
    }
    ids
}

/// The id of the PERSON plate on air in `slug`, if any.
async fn on_air_id(state: &AppState, slug: &str) -> Option<i64> {
    state
        .stream_nameplate_active(slug)
        .await
        .and_then(|active| active.nameplate_id)
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
    let state = AppState::in_memory().await.unwrap();
    let ids = seed_plates(&state, "t870-vars", &["Anna", "Boris", "Cyril"]).await;
    let (a, b, c) = (ids[0], ids[1], ids[2]);

    let mut variables = CompanionVariableState::default();
    variables.apply_nameplates(resolve_nameplates(&state, "t870-vars").await);
    let before = rendered(&variables);
    assert_eq!(slot(&before, 1), owned("Anna", "r-Anna", &a.to_string()));
    assert_eq!(slot(&before, 2), owned("Boris", "r-Boris", &b.to_string()));

    state
        .repository()
        .set_nameplate_order("t870-vars", vec![c, a, b])
        .await
        .unwrap();
    variables.apply_nameplates(resolve_nameplates(&state, "t870-vars").await);
    let after = rendered(&variables);
    assert_eq!(slot(&after, 1), owned("Cyril", "r-Cyril", &c.to_string()));
    assert_eq!(slot(&after, 2), owned("Anna", "r-Anna", &a.to_string()));
    assert_eq!(slot(&after, 3), owned("Boris", "r-Boris", &b.to_string()));
}

// ---- `stream_nameplate_slot` parsing --------------------------------------

#[test]
fn slot_command_parses_with_toggle_and_stream_defaults() {
    match parse_command("stream_nameplate_slot", json!({ "slot": 2 })) {
        Ok(CompanionCommand::Stream(StreamCommand::NameplateSlot { output, slot, mode })) => {
            assert_eq!(output, "stream");
            assert_eq!(slot, 2);
            assert_eq!(mode, SlotMode::Toggle);
        }
        _ => panic!("expected Stream(NameplateSlot) with defaults"),
    }
    match parse_command(
        "stream_nameplate_slot",
        json!({ "slot": 11, "mode": "show", "output": " moderator " }),
    ) {
        Ok(CompanionCommand::Stream(StreamCommand::NameplateSlot { output, slot, mode })) => {
            assert_eq!(output, "moderator");
            assert_eq!(slot, 11);
            assert_eq!(mode, SlotMode::Show);
        }
        _ => panic!("expected Stream(NameplateSlot) in show mode"),
    }
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
    let state = AppState::in_memory().await.unwrap();
    let ids = seed_plates(&state, "t870-toggle", &["Anna", "Boris", "Cyril"]).await;
    let press = json!({ "slot": 2, "output": "t870-toggle" });

    expect_ack(&state, "stream_nameplate_slot", press.clone()).await;
    assert_eq!(on_air_id(&state, "t870-toggle").await, Some(ids[1]));
    let active = state.stream_nameplate_active("t870-toggle").await.unwrap();
    assert_eq!(active.primary, "Boris");
    assert_eq!(active.secondary, "r-Boris");

    // Toggle: the same slot again takes it off air.
    expect_ack(&state, "stream_nameplate_slot", press).await;
    assert!(state.stream_nameplate_active("t870-toggle").await.is_none());
}

#[tokio::test]
async fn slot_press_in_show_mode_keeps_the_plate_on_air() {
    let state = AppState::in_memory().await.unwrap();
    let ids = seed_plates(&state, "t870-show", &["Anna", "Boris"]).await;
    let press = json!({ "slot": 1, "mode": "show", "output": "t870-show" });

    expect_ack(&state, "stream_nameplate_slot", press.clone()).await;
    expect_ack(&state, "stream_nameplate_slot", press).await;
    assert_eq!(on_air_id(&state, "t870-show").await, Some(ids[0]));
}

#[tokio::test]
async fn slot_press_after_a_reorder_fires_the_plate_now_at_that_position() {
    let state = AppState::in_memory().await.unwrap();
    let ids = seed_plates(&state, "t870-reorder", &["Anna", "Boris", "Cyril"]).await;
    state
        .repository()
        .set_nameplate_order("t870-reorder", vec![ids[0], ids[2], ids[1]])
        .await
        .unwrap();

    expect_ack(
        &state,
        "stream_nameplate_slot",
        json!({ "slot": 2, "output": "t870-reorder" }),
    )
    .await;
    assert_eq!(on_air_id(&state, "t870-reorder").await, Some(ids[2]));
}

#[tokio::test]
async fn empty_slot_press_is_refused_and_puts_nothing_on_air() {
    let state = AppState::in_memory().await.unwrap();
    seed_plates(&state, "t870-empty", &["Anna", "Boris"]).await;

    let message = expect_error(
        &state,
        "stream_nameplate_slot",
        json!({ "slot": 3, "output": "t870-empty" }),
    )
    .await;
    assert!(
        message.contains("slot 3 is empty in output t870-empty"),
        "unexpected: {message}"
    );
    assert!(state.stream_nameplate_active("t870-empty").await.is_none());
}

#[tokio::test]
async fn slot_press_on_an_unknown_output_is_refused() {
    let state = AppState::in_memory().await.unwrap();
    let message = expect_error(
        &state,
        "stream_nameplate_slot",
        json!({ "slot": 1, "output": "t870-missing" }),
    )
    .await;
    assert!(message.contains("t870-missing"), "unexpected: {message}");
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

// The plate list reaches the module BEFORE the variables (on connect, and on
// every list change), so the module has defined the slot variables when their
// values arrive; and a reorder pushes the new slot labels.
// Multi-thread runtime: a real axum server + a real WS client run concurrently.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn companion_socket_sends_the_plate_list_first_and_live_slot_labels() {
    let (state, _dir) = isolated_state(true).await;
    let repo = state.repository();
    let anna = repo
        .create_stream_nameplate("stream", "Anna", "pastor")
        .await
        .unwrap()
        .id;
    let boris = repo
        .create_stream_nameplate("stream", "Boris", "")
        .await
        .unwrap()
        .id;

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
    assert_eq!(frame_value(&vars, "nameplate_slot_1_role"), Some("pastor"));
    let anna_id = anna.to_string();
    assert_eq!(
        frame_value(&vars, "nameplate_slot_1_id"),
        Some(anna_id.as_str())
    );
    assert_eq!(frame_value(&vars, "nameplate_slot_2_name"), Some("Boris"));
    assert_eq!(frame_value(&vars, "nameplate_slot_3_name"), Some(""));
    assert_eq!(frame_value(&vars, "nameplate_slot_11_id"), Some(""));

    // Reorder in Presenter → the new list, then slot 1 = Boris.
    repo.set_nameplate_order("stream", vec![boris, anna])
        .await
        .unwrap();
    state.stream_nameplates_changed("stream");
    let (before, vars) = until_slot_one(&mut ws, "Boris").await;
    let list = before
        .iter()
        .rev()
        .find(|frame| frame["type"] == "nameplates")
        .expect("a nameplates frame before the reordered variables");
    assert_eq!(frame_plate_names(list), vec!["Boris", "Anna"]);
    let boris_id = boris.to_string();
    assert_eq!(
        frame_value(&vars, "nameplate_slot_1_id"),
        Some(boris_id.as_str())
    );
    assert_eq!(frame_value(&vars, "nameplate_slot_2_name"), Some("Anna"));

    ws.close(None).await.unwrap();
    server.abort();
}
