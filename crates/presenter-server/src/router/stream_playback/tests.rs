//! #843 playback routes, end to end: a playback element in an output names a
//! Drive folder (a wiremock Drive), a sync pass mirrors it into an isolated
//! temp dir, and the playlist / file routes serve it — with Range support.

use super::{parse_range, ByteRange};
use crate::drive::DriveClient;
use crate::playback::SyncTrigger;
use crate::router::build_router;
use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use presenter_core::stream::{ContentTransition, Frame, ImageFit, SceneKind, StreamElementProps};
use presenter_core::PlaybackPlaylist;
use tower::ServiceExt;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FOLDER_ID: &str = "1SynthRootFolderZZZZZZZZZZZZZZZZ9";
const PHOTO: &str = "1SynthPhotoAAAAAAAAAAAAAAAAAAAA1";
const CLIP: &str = "1SynthClipCCCCCCCCCCCCCCCCCCCCCC3";
const CLIP_BYTES: &[u8] = b"0123456789abcdefghij";
const LAST_MODIFIED: &str = "Mon, 14 Sep 2026 10:09:42 GMT";

fn folder_link() -> String {
    format!("https://drive.google.com/drive/folders/{FOLDER_ID}?usp=sharing")
}

#[test]
fn ranges_are_parsed_like_rfc_9110() {
    let len = 100;
    assert_eq!(parse_range(None, len), ByteRange::Full);
    assert_eq!(
        parse_range(Some("bytes=0-9"), len),
        ByteRange::Partial { start: 0, end: 9 }
    );
    assert_eq!(
        parse_range(Some("bytes=90-"), len),
        ByteRange::Partial { start: 90, end: 99 }
    );
    assert_eq!(
        parse_range(Some("bytes=95-500"), len),
        ByteRange::Partial { start: 95, end: 99 }
    );
    assert_eq!(
        parse_range(Some("bytes=-10"), len),
        ByteRange::Partial { start: 90, end: 99 }
    );
    assert_eq!(
        parse_range(Some("bytes=-500"), len),
        ByteRange::Partial { start: 0, end: 99 }
    );
    assert_eq!(
        parse_range(Some("bytes=100-"), len),
        ByteRange::Unsatisfiable
    );
    assert_eq!(parse_range(Some("bytes=-0"), len), ByteRange::Unsatisfiable);
    assert_eq!(parse_range(Some("bytes=0-"), 0), ByteRange::Unsatisfiable);
    // Ignored (served whole): multi-range, another unit, junk, reversed.
    for ignored in [
        "bytes=0-1,5-6",
        "items=0-1",
        "bytes=abc",
        "bytes=9-3",
        "bytes",
    ] {
        assert_eq!(
            parse_range(Some(ignored), len),
            ByteRange::Full,
            "{ignored}"
        );
    }
}

/// An isolated state: own temp asset dir, Drive = the mock.
async fn test_state(drive: &MockServer) -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut state = AppState::in_memory().await.expect("in_memory state");
    state.set_stream_assets_dir(dir.path().join("stream-assets"));
    state.set_playback_drive_client(DriveClient::for_test(&drive.uri()));
    (state, dir)
}

/// The folder has a photo, a subfolder (skipped) and a clip.
async fn mount_drive(server: &MockServer) {
    let entry = |id: &str, mime: &str, name: &str| {
        format!(
            r#"<div class="flip-entry" id="entry-{id}" tabindex="0" role="link"><div class="flip-entry-info"><a href="https://drive.google.com/file/d/{id}/view?usp=drive_web" target="_blank"><div class="flip-entry-list-icon"><img src="https://drive-thirdparty.googleusercontent.com/16/type/{mime}" alt=""/></div><div class="flip-entry-title">{name}</div></a></div></div>"#
        )
    };
    let html = format!(
        r#"<html><body><div class="flip-entries">{}{}{}</div></body></html>"#,
        entry(PHOTO, "image/png", "photo.png"),
        r#"<div class="flip-entry" id="entry-1SynthFolderBBBBBBBBBBBBBBBBBBBB2"><a href="https://drive.google.com/drive/folders/1SynthFolderBBBBBBBBBBBBBBBBBBBB2"><div aria-label="Folder"></div><div class="flip-entry-title">Sub</div></a></div>"#,
        entry(CLIP, "video/mp4", "clip.mp4"),
    );
    Mock::given(method("GET"))
        .and(path("/embeddedfolderview"))
        .and(query_param("id", FOLDER_ID))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(html.into_bytes(), "text/html; charset=utf-8"),
        )
        .mount(server)
        .await;
    for (id, mime, body) in [
        (PHOTO, "image/png", b"png-photo-bytes".as_slice()),
        (CLIP, "video/mp4", CLIP_BYTES),
    ] {
        Mock::given(method("HEAD"))
            .and(path("/download"))
            .and(query_param("id", id))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", mime)
                    .insert_header("content-length", body.len().to_string().as_str())
                    .insert_header("last-modified", LAST_MODIFIED),
            )
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/download"))
            .and(query_param("id", id))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body.to_vec(), mime))
            .mount(server)
            .await;
    }
}

/// An own output with one base scene holding a playback element on `link`.
async fn seed_element(state: &AppState, slug: &str, link: &str) {
    let repo = state.repository();
    repo.create_stream_output(slug, "Playback test")
        .await
        .expect("output");
    let scene = repo
        .create_stream_scene(slug, "Base", SceneKind::Base)
        .await
        .expect("scene");
    repo.create_stream_element(
        scene.id,
        StreamElementProps::Playback {
            source_url: link.to_string(),
            fit: ImageFit::Contain,
            frame: Frame {
                x_pct: 0.0,
                y_pct: 0.0,
                w_pct: 50.0,
                h_pct: 50.0,
            },
            opacity: 1.0,
            image_seconds: 5,
            muted: true,
            content_transition: ContentTransition::default(),
        },
    )
    .await
    .expect("playback element");
}

async fn get(state: &AppState, uri: &str, range: Option<&str>) -> axum::response::Response {
    let mut request = Request::builder().method("GET").uri(uri);
    if let Some(range) = range {
        request = request.header(header::RANGE, range);
    }
    build_router(state.clone())
        .oneshot(request.body(Body::empty()).expect("request"))
        .await
        .expect("response")
}

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec()
}

async fn playlist(state: &AppState, link: &str) -> PlaybackPlaylist {
    let response = get(state, &format!("/stream/api/playback?url={link}"), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&body_bytes(response).await).expect("playlist json")
}

fn header_of(response: &axum::response::Response, name: header::HeaderName) -> String {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn a_pass_mirrors_the_folder_and_the_routes_serve_it() {
    let drive = MockServer::start().await;
    mount_drive(&drive).await;
    let (state, _dir) = test_state(&drive).await;
    seed_element(&state, "t843-serve", &folder_link()).await;

    // Before any sync: an empty playlist, no error.
    let before = playlist(&state, &folder_link()).await;
    assert!(before.items.is_empty() && before.synced_at.is_none());

    state.run_playback_pass(SyncTrigger::Periodic).await;

    let list = playlist(&state, &folder_link()).await;
    assert_eq!(list.folder_id, FOLDER_ID);
    let names: Vec<&str> = list.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        ["photo.png", "clip.mp4"],
        "subfolder skipped, order kept"
    );
    assert!(list.synced_at.is_some());
    assert_eq!(list.error, None);
    assert!(!list.syncing);

    // The clip, whole.
    let clip_url = list.items[1].url.clone();
    assert!(clip_url.starts_with(&format!("/stream/playback/{FOLDER_ID}/{CLIP}?v=")));
    let whole = get(&state, &clip_url, None).await;
    assert_eq!(whole.status(), StatusCode::OK);
    assert_eq!(header_of(&whole, header::CONTENT_TYPE), "video/mp4");
    assert_eq!(header_of(&whole, header::ACCEPT_RANGES), "bytes");
    assert_eq!(header_of(&whole, header::CONTENT_LENGTH), "20");
    assert_eq!(body_bytes(whole).await, CLIP_BYTES);

    // One range.
    let part = get(&state, &clip_url, Some("bytes=2-5")).await;
    assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header_of(&part, header::CONTENT_RANGE), "bytes 2-5/20");
    assert_eq!(header_of(&part, header::CONTENT_LENGTH), "4");
    assert_eq!(body_bytes(part).await, b"2345");

    // A range past the end.
    let past = get(&state, &clip_url, Some("bytes=40-")).await;
    assert_eq!(past.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(header_of(&past, header::CONTENT_RANGE), "bytes */20");

    // The photo is served with its listing MIME type.
    let photo = get(&state, &list.items[0].url, None).await;
    assert_eq!(header_of(&photo, header::CONTENT_TYPE), "image/png");
    assert_eq!(body_bytes(photo).await, b"png-photo-bytes");
}

#[tokio::test]
async fn files_are_id_addressed_only() {
    let drive = MockServer::start().await;
    mount_drive(&drive).await;
    let (state, _dir) = test_state(&drive).await;
    seed_element(&state, "t843-ids", &folder_link()).await;
    state.run_playback_pass(SyncTrigger::Periodic).await;

    for uri in [
        // Not in the manifest (the skipped subfolder) / unknown.
        format!("/stream/playback/{FOLDER_ID}/1SynthFolderBBBBBBBBBBBBBBBBBBBB2"),
        format!("/stream/playback/{FOLDER_ID}/nope"),
        // Not Drive ids: never joined into a path.
        format!("/stream/playback/{FOLDER_ID}/manifest.json"),
        format!("/stream/playback/{FOLDER_ID}/..%2F..%2Fpresenter.db"),
        format!("/stream/playback/short/{CLIP}"),
    ] {
        let response = get(&state, &uri, None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn a_file_that_left_the_folder_is_still_served_for_a_while() {
    let drive = MockServer::start().await;
    mount_drive(&drive).await;
    let (state, _dir) = test_state(&drive).await;
    seed_element(&state, "t843-retire", &folder_link()).await;
    state.run_playback_pass(SyncTrigger::Periodic).await;
    let photo_url = playlist(&state, &folder_link()).await.items[0].url.clone();

    // The photo leaves the folder: only the clip is listed now.
    drive.reset().await;
    Mock::given(method("GET"))
        .and(path("/embeddedfolderview"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(
                r#"<html><body><div class="flip-entries"><div class="flip-entry" id="entry-{CLIP}"><a href="https://drive.google.com/file/d/{CLIP}/view"><div class="flip-entry-list-icon"><img src="https://drive-thirdparty.googleusercontent.com/16/type/video/mp4" alt=""/></div><div class="flip-entry-title">clip.mp4</div></a></div></div></body></html>"#
            )
            .into_bytes(),
            "text/html; charset=utf-8",
        ))
        .mount(&drive)
        .await;
    Mock::given(method("HEAD"))
        .and(query_param("id", CLIP))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-length", CLIP_BYTES.len().to_string().as_str())
                .insert_header("last-modified", LAST_MODIFIED),
        )
        .mount(&drive)
        .await;
    state.run_playback_pass(SyncTrigger::Periodic).await;

    let names: Vec<String> = playlist(&state, &folder_link())
        .await
        .items
        .into_iter()
        .map(|i| i.name)
        .collect();
    assert_eq!(names, ["clip.mp4"]);
    // An output whose playlist is up to 60 s old still gets the photo.
    let response = get(&state, &photo_url, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_bytes(response).await, b"png-photo-bytes");
}

#[tokio::test]
async fn a_link_that_is_not_a_drive_folder_is_refused() {
    let drive = MockServer::start().await;
    let (state, _dir) = test_state(&drive).await;
    let response = get(
        &state,
        "/stream/api/playback?url=https://example.com/x",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = String::from_utf8(body_bytes(response).await).expect("utf8");
    assert!(body.contains("Google Drive"), "{body}");
}

#[tokio::test]
async fn a_refresh_shows_syncing_until_the_pass_ran() {
    let drive = MockServer::start().await;
    mount_drive(&drive).await;
    let (state, _dir) = test_state(&drive).await;

    // No element names the folder yet: the refresh still syncs the editor's link.
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/stream/api/playback/refresh")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "url": folder_link() }).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(playlist(&state, &folder_link()).await.syncing);

    // (The worker is not running in tests — run its pass by hand.)
    state.run_playback_pass(SyncTrigger::Forced).await;

    let list = playlist(&state, &folder_link()).await;
    assert!(!list.syncing);
    assert_eq!(list.items.len(), 2);
}

/// A JSON request through the real router.
async fn send_json(
    state: &AppState,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) -> axum::response::Response {
    build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response")
}

#[tokio::test]
async fn a_playback_element_is_created_and_patched_through_the_stream_api() {
    let drive = MockServer::start().await;
    let (state, _dir) = test_state(&drive).await;
    let repo = state.repository();
    repo.create_stream_output("t843-api", "Playback API")
        .await
        .expect("output");
    let scene = repo
        .create_stream_scene("t843-api", "Base", SceneKind::Base)
        .await
        .expect("scene");

    // The element body is the flattened props enum (tag `kind`, snake_case).
    let created = send_json(
        &state,
        "POST",
        &format!("/stream/api/scenes/{}/elements", scene.id),
        serde_json::json!({
            "kind": "playback",
            "source_url": folder_link(),
            "fit": "cover",
            "frame": { "xPct": 5, "yPct": 5, "wPct": 40, "hPct": 40 },
            "opacity": 0.9
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let element: serde_json::Value =
        serde_json::from_slice(&body_bytes(created).await).expect("element json");
    assert_eq!(element["props"]["kind"], "playback");
    // Omitted fields take their defaults.
    assert_eq!(element["props"]["image_seconds"], 5);
    assert_eq!(element["props"]["muted"], true);
    let id = element["id"].as_i64().expect("element id");

    let patched = send_json(
        &state,
        "PATCH",
        &format!("/stream/api/elements/{id}"),
        serde_json::json!({
            "kind": "playback",
            "source_url": folder_link(),
            "fit": "contain",
            "frame": { "xPct": 5, "yPct": 5, "wPct": 40, "hPct": 40 },
            "opacity": 1.0,
            "image_seconds": 12,
            "muted": false,
            "content_transition": { "mode": "fade_through", "duration_ms": 600 }
        }),
    )
    .await;
    assert_eq!(patched.status(), StatusCode::OK);
    let def = state
        .repository()
        .load_output_def("t843-api")
        .await
        .expect("def");
    match &def.scenes[0].elements[0].props {
        StreamElementProps::Playback {
            image_seconds,
            muted,
            fit,
            ..
        } => {
            assert_eq!(
                (*image_seconds, *muted, *fit),
                (12, false, ImageFit::Contain)
            );
        }
        other => panic!("expected playback, got {other:?}"),
    }

    // A link that is not a Drive folder: 422 from core validation.
    let refused = send_json(
        &state,
        "PATCH",
        &format!("/stream/api/elements/{id}"),
        serde_json::json!({
            "kind": "playback",
            "source_url": "https://example.com/photos",
            "fit": "contain",
            "frame": { "xPct": 5, "yPct": 5, "wPct": 40, "hPct": 40 },
            "opacity": 1.0
        }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_bare_refresh_post_marks_every_referenced_folder() {
    let drive = MockServer::start().await;
    mount_drive(&drive).await;
    let (state, _dir) = test_state(&drive).await;
    seed_element(&state, "t843-bare", &folder_link()).await;

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/stream/api/playback/refresh")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(playlist(&state, &folder_link()).await.syncing);
}

#[tokio::test]
async fn a_refresh_mark_whose_element_vanished_does_not_stay_syncing() {
    let drive = MockServer::start().await;
    mount_drive(&drive).await;
    let (state, _dir) = test_state(&drive).await;
    seed_element(&state, "t843-stale", &folder_link()).await;

    // "Obnoviť" without a link marks every referenced folder…
    state.request_playback_refresh(None).await;
    assert!(playlist(&state, &folder_link()).await.syncing);
    // …but the element is deleted before the worker runs.
    state
        .repository()
        .delete_stream_output("t843-stale")
        .await
        .expect("delete output");
    state.run_playback_pass(SyncTrigger::Forced).await;

    assert!(!playlist(&state, &folder_link()).await.syncing);
}

#[tokio::test]
async fn a_failed_sync_is_reported_and_the_items_keep_playing() {
    let drive = MockServer::start().await;
    mount_drive(&drive).await;
    let (state, _dir) = test_state(&drive).await;
    seed_element(&state, "t843-fail", &folder_link()).await;
    state.run_playback_pass(SyncTrigger::Periodic).await;

    // The folder stops being shared publicly.
    drive.reset().await;
    Mock::given(method("GET"))
        .and(path("/embeddedfolderview"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&drive)
        .await;
    state.run_playback_pass(SyncTrigger::Periodic).await;

    let list = playlist(&state, &folder_link()).await;
    assert_eq!(list.items.len(), 2, "the last good playlist stays");
    let error = list.error.unwrap_or_default();
    assert!(error.contains("nie je verejný"), "{error}");
}

#[tokio::test]
async fn an_unused_mirror_is_removed_after_its_grace_period() {
    let drive = MockServer::start().await;
    let (state, _dir) = test_state(&drive).await;
    let root = state.playback_dir();
    let write = |id: &str, synced_hours_ago: i64| {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let synced = chrono::Utc::now() - chrono::Duration::hours(synced_hours_ago);
        let manifest = serde_json::json!({ "folderId": id, "items": [], "syncedAt": synced });
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).expect("manifest");
        dir
    };
    let stale = write("1StaleFolderXXXXXXXXXXXXXXXXXXXX1", 48);
    let recent = write("1RecentFolderXXXXXXXXXXXXXXXXXXX2", 1);

    state.run_playback_pass(SyncTrigger::Periodic).await;

    assert!(!stale.exists(), "unused for 2 days: removed");
    assert!(recent.exists(), "synced an hour ago: kept");
}
