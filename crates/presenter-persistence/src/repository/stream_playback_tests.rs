//! #843 stream repository tests for the `playback` element kind (a Google
//! Drive folder played in a loop). Shares `stream_tests.rs`'s fixtures; kept in
//! its own file because that one sits near the 1000-line file cap.

use super::stream_tests::{as_repo_error, frame, repo};
use super::RepositoryError;
use presenter_core::stream::{ContentTransition, ImageFit, SceneKind, StreamElementProps};

fn playback(source_url: &str) -> StreamElementProps {
    StreamElementProps::Playback {
        source_url: source_url.to_string(),
        fit: ImageFit::Cover,
        frame: frame(),
        opacity: 0.8,
        image_seconds: 9,
        muted: false,
        content_transition: ContentTransition::FadeThrough { duration_ms: 500 },
    }
}

#[tokio::test]
async fn playback_round_trips_through_the_def_and_validates_the_link() {
    let repo = repo().await;
    let scene = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();

    // A playback element (no migration: `stream_elements.kind` is free text)
    // round-trips through the def assembly with every field intact.
    let props = playback("https://drive.google.com/drive/folders/1AbCdEfGhIjKlMnOpQrStUvWxYz012");
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
        .expect("playback element present in def");
    assert_eq!(stored.props, props);
    assert_eq!(stored.props.kind_str(), "playback");

    // A new element may have no link yet.
    repo.create_stream_element(scene.id, playback(""))
        .await
        .unwrap();

    // A link that is not a Drive folder is refused 422 by core validate_props.
    let err = repo
        .create_stream_element(scene.id, playback("https://example.com/photos"))
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
}
