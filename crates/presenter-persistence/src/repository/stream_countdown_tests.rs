//! #785 stream repository tests for the countdown `TextBox` + `letter_spacing_em`
//! fields — split out of `stream_tests.rs`, which sits at the 1000-line file cap.
//! Shares that module's fixtures (`repo`, `frame`, `text_style`, `as_repo_error`).

use super::stream_tests::{as_repo_error, frame, repo, text_style};
use super::RepositoryError;
use presenter_core::stream::{ContentTransition, SceneKind, StreamElementProps};

#[tokio::test]
async fn countdown_box_and_letter_spacing_round_trip_and_validate() {
    use presenter_core::{TextBox, TextStyle};
    let repo = repo().await;
    let scene = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    // A countdown with a letter-spacing style + a background box round-trips
    // through the def-assembly path (proves the #785 fields persist + parse).
    let props = StreamElementProps::Countdown {
        timer_id: 2,
        style: TextStyle {
            letter_spacing_em: Some(0.08),
            ..text_style()
        },
        frame: frame(),
        content_transition: ContentTransition::Cut,
        r#box: Some(TextBox {
            color: "#0f172a".to_string(),
            opacity: 0.6,
            padding_pct: 2.0,
            radius_pct: 1.5,
        }),
    };
    let el = repo
        .create_stream_element(scene.id, props.clone())
        .await
        .unwrap();
    let def = repo.load_output_def("stream").await.unwrap();
    let stored = def
        .scenes
        .iter()
        .flat_map(|s| &s.elements)
        .find(|e| e.id == el.id)
        .expect("countdown element present in def");
    assert_eq!(stored.props, props);

    // Letter spacing out of range is rejected 422 by core validate_props.
    let bad = StreamElementProps::Countdown {
        timer_id: 1,
        style: TextStyle {
            letter_spacing_em: Some(5.0),
            ..text_style()
        },
        frame: frame(),
        content_transition: ContentTransition::Cut,
        r#box: None,
    };
    let err = repo.create_stream_element(scene.id, bad).await.unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
}
