//! #850 A2b (spec §5.10): the legacy `/bible/trigger` path (Companion, the AI
//! `trigger_bible_verse` tool) puts on the Resolume wall EXACTLY what the
//! slide path puts there for the same `BibleSlideOutput` — one shape of
//! output for every path. Real host workers against a mock Arena.

use super::trigger_tests::seb_main_kjv_secondary;
use super::*;
use crate::resolume::tests::mount_full_composition;
use presenter_core::{ResolumeHost, ResolumeHostId};
use std::time::Duration;
use wiremock::MockServer;

/// Text params of the four Bible lanes (A and B) in the shared composition.
const BIBLE_TEXT_PARAMS: [i64; 8] = [30, 31, 35, 36, 40, 41, 45, 46];
/// Their clips, connected once a verse is on air.
const BIBLE_CLIPS: [i64; 8] = [300, 301, 350, 351, 400, 401, 450, 451];

/// `state`'s Resolume registry drives one host: a mock Arena with every
/// Bible lane clip.
async fn wall_for(state: &AppState) -> MockServer {
    let server = MockServer::start().await;
    mount_full_composition(&server).await;
    let addr = server.address();
    let now = Utc::now();
    let host = ResolumeHost::new(
        ResolumeHostId::new(),
        "Mock wall".into(),
        addr.ip().to_string(),
        addr.port(),
        true,
        now,
        now,
    );
    state.resolume_registry.set_hosts(vec![host]).await;
    server
}

/// The verse writes (param → text) and clip connects one show produced, in a
/// stable order; waits until the worker connected a whole lane (4 clips).
async fn wall_pushes(server: &MockServer) -> (Vec<(i64, String)>, Vec<i64>) {
    for _ in 0..200 {
        let requests = server.received_requests().await.unwrap_or_default();
        let mut connects: Vec<i64> = requests
            .iter()
            .filter(|req| req.method.as_str() == "POST")
            .filter_map(|req| {
                BIBLE_CLIPS.into_iter().find(|clip| {
                    req.url.path() == format!("/api/v1/composition/clips/by-id/{clip}/connect")
                })
            })
            .collect();
        if connects.len() >= 4 {
            let mut writes: Vec<(i64, String)> = requests
                .iter()
                .filter(|req| req.method.as_str() == "PUT")
                .filter_map(|req| {
                    let param = BIBLE_TEXT_PARAMS.into_iter().find(|param| {
                        req.url.path() == format!("/api/v1/parameter/by-id/{param}")
                    })?;
                    let body: serde_json::Value = serde_json::from_slice(&req.body).ok()?;
                    Some((param, body["value"].as_str()?.to_string()))
                })
                .collect();
            writes.sort();
            connects.sort_unstable();
            return (writes, connects);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the Bible lane was never connected on the mock wall");
}

/// Shows `reference` through the legacy trigger and the same output through
/// the slide path on a second state; returns the output and both walls'
/// pushes.
async fn legacy_and_slide_walls(
    reference: &BibleReference,
    overrides: BibleTriggerOverrides,
) -> (
    BibleSlideOutput,
    (Vec<(i64, String)>, Vec<i64>),
    (Vec<(i64, String)>, Vec<i64>),
) {
    let legacy_state = seb_main_kjv_secondary().await;
    let legacy_wall = wall_for(&legacy_state).await;
    legacy_state
        .trigger_bible_passage(
            "slk-seb",
            reference,
            overrides,
            crate::state::bible_show_log::BibleShowOrigin::internal(
                crate::state::bible_show_log::BibleShowSource::Trigger,
            ),
        )
        .await
        .expect("legacy trigger");
    let output = legacy_state
        .active_bible_slide_output()
        .await
        .expect("the legacy trigger stores the wall output");
    let legacy = wall_pushes(&legacy_wall).await;

    let slide_state = seb_main_kjv_secondary().await;
    let slide_wall = wall_for(&slide_state).await;
    slide_state
        .trigger_bible_slide_output(
            output.clone(),
            BibleSlideReferenceMetadata::default(),
            BibleShowOrigin {
                source: crate::state::bible_show_log::BibleShowSource::Api,
                device: None,
                client_ip: "-".to_string(),
            },
        )
        .await;
    let slide = wall_pushes(&slide_wall).await;
    (output, legacy, slide)
}

/// The text each Bible lane clip received (lane A or B, whichever was used).
fn lane_texts(writes: &[(i64, String)], params: [i64; 2]) -> Vec<&str> {
    writes
        .iter()
        .filter(|(param, _)| params.contains(param))
        .map(|(_, text)| text.as_str())
        .collect()
}

#[tokio::test]
async fn a_legacy_trigger_writes_the_wall_like_the_slide_path() {
    let reference = BibleReference::new_with_code("1 Ján", "1JN", 62, 1, 1, 3).expect("reference");

    let (output, legacy, slide) =
        legacy_and_slide_walls(&reference, BibleTriggerOverrides::default()).await;

    assert_eq!(legacy, slide, "legacy and slide path pushes differ");
    let (writes, _) = &legacy;
    assert_eq!(lane_texts(writes, [35, 36]), vec!["1 Ján 1:1-3 (SEB)"]);
    // #824: the translate reference names the SECONDARY translation's book.
    assert_eq!(lane_texts(writes, [45, 46]), vec!["1 John 1:1-3 (KJV)"]);
    assert_eq!(
        lane_texts(writes, [40, 41]),
        vec![output.secondary_text.as_str()]
    );
}

#[tokio::test]
async fn an_empty_translation_text_sends_no_secondary_reference() {
    // The known limit of the removed legacy Resolume branch: with a stored
    // secondary translation and an empty `translationText`, it still sent
    // "1 Ján 1:1 (KJV)" to `#bible-translate-reference` under an empty
    // translation lane. The slide output carries no secondary reference.
    let reference = BibleReference::new_with_code("1 Ján", "1JN", 62, 1, 1, 1).expect("reference");
    let overrides = BibleTriggerOverrides {
        main_text: None,
        translation_text: Some(String::new()),
    };

    let (output, legacy, slide) = legacy_and_slide_walls(&reference, overrides).await;

    assert_eq!(output.secondary_text, "");
    assert_eq!(output.secondary_reference, "");
    assert_eq!(legacy, slide, "legacy and slide path pushes differ");
    let (writes, _) = &legacy;
    assert_eq!(lane_texts(writes, [45, 46]), vec![""]);
    assert_eq!(lane_texts(writes, [40, 41]), vec![""]);
}
