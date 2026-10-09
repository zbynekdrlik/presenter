//! Router + filesystem integration tests for the stream-font pipeline (#778),
//! mirroring `router/stream_assets/tests.rs`. Each test builds an
//! `AppState::in_memory()` pointed at its own `TempDir` (the font store is a
//! `fonts/` subdir of that), so the on-disk store is isolated even though the
//! shared-cache in-memory DB is not — assertions target the specific ids/bytes a
//! test created, never global counts.

use crate::router::build_router;
use crate::state::stream_fonts::test_fonts::{
    with_names, with_os2_italic, with_os2_version, with_table_length, with_table_renamed,
};
use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use presenter_core::stream::{SceneKind, StreamElementProps, StreamFont, TextAlign, TextStyle};
use presenter_persistence::NewStreamFont;
use tower::ServiceExt;

const BOUNDARY: &str = "TESTBOUNDARY778";

/// The committed OFL fixture font (family "Gruppo"), shared with the Rust
/// metadata test and the Playwright E2E — one licence-clean font for all layers.
const FIXTURE_TTF: &[u8] =
    include_bytes!("../../../../../tests/e2e/fixtures/fonts/Gruppo-Regular.ttf");

async fn test_state() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut state = AppState::in_memory().await.expect("in_memory state");
    state.set_stream_assets_dir(dir.path().join("stream-assets"));
    (state, dir)
}

fn multipart_body(filename: &str, declared_ct: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {declared_ct}\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

fn upload_request(filename: &str, ct: &str, bytes: &[u8]) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/stream/fonts")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart_body(filename, ct, bytes)))
        .expect("request")
}

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec()
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&body_bytes(response).await).expect("json")
}

async fn upload(state: &AppState, filename: &str, bytes: &[u8]) -> StreamFont {
    let response = build_router(state.clone())
        .oneshot(upload_request(filename, "font/ttf", bytes))
        .await
        .expect("upload");
    assert_eq!(response.status(), StatusCode::OK, "upload should succeed");
    serde_json::from_value(body_json(response).await).expect("StreamFont")
}

fn text_style(family: &str) -> TextStyle {
    TextStyle {
        font_family: family.to_string(),
        size_pct: 8.0,
        color: "#ffffff".to_string(),
        weight: 400,
        align: TextAlign::Center,
        line_height: 1.2,
        shadow: None,
        letter_spacing_em: None,
        uppercase: None,
        italic: None,
    }
}

#[tokio::test]
async fn upload_ttf_stores_row_and_file_with_metadata() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "Gruppo-Regular.ttf", FIXTURE_TTF).await;

    assert_eq!(font.family, "Gruppo", "family parsed from the name table");
    assert_eq!(font.format, "ttf");
    assert!(!font.italic);
    assert!((1..=1000).contains(&font.weight), "weight {}", font.weight);
    assert_eq!(font.size_bytes, FIXTURE_TTF.len() as i64);
    assert_eq!(font.sha256.len(), 64);

    let path = state
        .stream_assets_dir()
        .join("fonts")
        .join(format!("{}.ttf", font.sha256));
    assert!(path.exists(), "font file written to disk: {path:?}");

    let fetched = state.repository().get_stream_font(font.id).await.unwrap();
    assert_eq!(fetched.sha256, font.sha256);
}

#[tokio::test]
async fn re_upload_same_bytes_dedups() {
    let (state, _dir) = test_state().await;
    let first = upload(&state, "a.ttf", FIXTURE_TTF).await;
    let second = upload(&state, "b-other-name.ttf", FIXTURE_TTF).await;
    assert_eq!(first.id, second.id, "identical bytes dedup to one font id");
    assert_eq!(first.sha256, second.sha256);
}

#[tokio::test]
async fn upload_garbage_is_422() {
    let (state, _dir) = test_state().await;
    let response = build_router(state.clone())
        .oneshot(upload_request(
            "fake.ttf",
            "font/ttf",
            b"this is not a font",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn upload_ttc_collection_is_422() {
    let (state, _dir) = test_state().await;
    // A TrueType Collection ("ttcf") — a browser cannot @font-face it.
    let mut bytes = b"ttcf".to_vec();
    bytes.extend_from_slice(&[0u8; 64]);
    let response = build_router(state.clone())
        .oneshot(upload_request("coll.ttc", "font/collection", &bytes))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn upload_woff2_is_422() {
    let (state, _dir) = test_state().await;
    let mut bytes = b"wOF2".to_vec();
    bytes.extend_from_slice(&[0u8; 64]);
    let response = build_router(state.clone())
        .oneshot(upload_request("web.woff2", "font/woff2", &bytes))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn upload_over_5mib_is_413() {
    let (state, _dir) = test_state().await;
    // Valid ttf magic then padded past the 5 MiB business cap (under the raw
    // body-limit layer so the handler's precise 413 fires).
    let mut bytes = vec![0x00, 0x01, 0x00, 0x00];
    bytes.resize(5 * 1024 * 1024 + 16, 0);
    let response = build_router(state.clone())
        .oneshot(upload_request("huge.ttf", "font/ttf", &bytes))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn list_contains_the_uploaded_font() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "l.ttf", FIXTURE_TTF).await;
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/stream/api/fonts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let list: Vec<StreamFont> = serde_json::from_value(body_json(response).await).unwrap();
    assert!(list.iter().any(|f| f.id == font.id && f.family == "Gruppo"));
}

#[tokio::test]
async fn serve_returns_bytes_with_immutable_cache_and_font_mime() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "s.ttf", FIXTURE_TTF).await;
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/stream/fonts/{}", font.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "font/ttf"
    );
    assert_eq!(
        response
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .unwrap(),
        "nosniff"
    );
    assert_eq!(
        body_bytes(response).await,
        FIXTURE_TTF,
        "served bytes match"
    );
}

#[tokio::test]
async fn serve_missing_font_is_404() {
    let (state, _dir) = test_state().await;
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/stream/fonts/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_unused_font_removes_row_and_file() {
    let (state, _dir) = test_state().await;
    // Seed a font with a UNIQUE family + sha directly (NOT the shared Gruppo
    // fixture): `AppState::in_memory()` is a process-wide `cache=shared` DB, and
    // the guarded delete scans elements by family NAME globally — so deleting a
    // deduped Gruppo row here would race the in-use Gruppo element the 409 test
    // leaves behind. A unique unused family is order-independent.
    let sha = "aa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899";
    state
        .font_store()
        .store(sha, "ttf", b"\x00\x01\x00\x00 unused fixture bytes")
        .await
        .unwrap();
    let font = state
        .repository()
        .insert_or_get_stream_font(NewStreamFont {
            sha256: sha.to_string(),
            original_filename: "Unused778.ttf".to_string(),
            family: "UnusedFamily778".to_string(),
            weight: 400,
            italic: false,
            format: "ttf".to_string(),
            size_bytes: 21,
        })
        .await
        .unwrap();
    let path = state
        .stream_assets_dir()
        .join("fonts")
        .join(format!("{}.ttf", font.sha256));
    assert!(path.exists());

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/stream/fonts/{}", font.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(!path.exists(), "file removed from disk");
}

#[tokio::test]
async fn delete_last_in_use_font_is_409_naming_scene() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "used.ttf", FIXTURE_TTF).await;

    // Now that "Gruppo" is an uploaded family, an element may use it.
    let scene = state
        .repository()
        .create_stream_scene("stream", "Uses Gruppo 778", SceneKind::Base)
        .await
        .unwrap();
    state
        .repository()
        .create_stream_element(
            scene.id,
            StreamElementProps::Countdown {
                timer_id: 1,
                style: text_style("Gruppo"),
                frame: presenter_core::stream::Frame {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    w_pct: 50.0,
                    h_pct: 50.0,
                },
                content_transition: presenter_core::stream::ContentTransition::default(),
                r#box: None,
            },
        )
        .await
        .unwrap();

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/stream/fonts/{}", font.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = String::from_utf8(body_bytes(response).await).unwrap();
    assert!(
        body.contains("Uses Gruppo 778"),
        "409 body names the referencing scene: {body}"
    );

    // A refused delete removes nothing.
    let path = state
        .stream_assets_dir()
        .join("fonts")
        .join(format!("{}.ttf", font.sha256));
    assert!(path.exists(), "referenced font file is untouched");
}

#[tokio::test]
async fn fonts_css_has_face_rule_and_etag_revalidation() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "css.ttf", FIXTURE_TTF).await;

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/stream/fonts.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );
    let etag = response
        .headers()
        .get(header::ETAG)
        .expect("etag present")
        .to_str()
        .unwrap()
        .to_string();
    let css = String::from_utf8(body_bytes(response).await).unwrap();
    assert!(css.contains("@font-face"), "css has a face rule: {css}");
    assert!(css.contains("\"Gruppo\""), "css names the family: {css}");
    assert!(
        css.contains(&format!("/stream/fonts/{}", font.id)),
        "css src points at the serve route: {css}"
    );
    assert!(
        css.contains("format(\"truetype\")"),
        "ttf format hint: {css}"
    );

    // A conditional GET with the same ETag → 304 (revalidation, no transfer).
    let cond = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/stream/fonts.css")
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cond.status(), StatusCode::NOT_MODIFIED);
}

// ── #778 reopen: fonts a browser's OpenType sanitiser (OTS) would refuse ──
//
// Font id 71 "Brigends Expanded NL" on SNV/PP declares OS/2 version 5 in a
// 96-byte table (v5 needs 100) — read-fonts accepted it, Chrome's OTS does not
// ("OTS parsing error: OS/2: Failed to read version 5-specific fields"), and
// every page loading /stream/fonts.css logged the rejection. The broken bytes
// are derived in memory from the OFL fixture (`state::stream_fonts::test_fonts`).

async fn get(state: &AppState, uri: &str) -> axum::response::Response {
    build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Seed an ALREADY-STORED face directly (row + file), bypassing the upload
/// gate — the state SNV/PP are in for font id 71. `family` must be unique per
/// test (shared `cache=shared` DB, family-global delete guard).
async fn seed_stored_font(state: &AppState, family: &str, bytes: &[u8]) -> StreamFont {
    let sha = crate::state::stream_assets::sha256_hex(bytes);
    state.font_store().store(&sha, "ttf", bytes).await.unwrap();
    state
        .repository()
        .insert_or_get_stream_font(NewStreamFont {
            sha256: sha,
            original_filename: format!("{family}.ttf"),
            family: family.to_string(),
            weight: 400,
            italic: false,
            format: "ttf".to_string(),
            size_bytes: bytes.len() as i64,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn upload_font_browsers_would_refuse_is_422_with_reason() {
    let (state, _dir) = test_state().await;
    let cases: [(&str, Vec<u8>, &str); 3] = [
        // The exact font-id-71 defect: OS/2 v5 in a v4-sized (96-byte) table.
        (
            "os2-v5-in-96-bytes",
            with_os2_version(FIXTURE_TTF, 5),
            "OS/2",
        ),
        // OS/2 cut below the 78 bytes every version needs.
        (
            "os2-truncated",
            with_table_length(FIXTURE_TTF, b"OS/2", 70),
            "OS/2",
        ),
        // A table OTS requires (`post`) is missing.
        (
            "no-post-table",
            with_table_renamed(FIXTURE_TTF, b"post", b"pozt"),
            "post",
        ),
    ];
    for (label, bytes, names) in cases {
        let response = build_router(state.clone())
            .oneshot(upload_request(&format!("{label}.ttf"), "font/ttf", &bytes))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{label}: a font the browser sanitiser refuses must be rejected at upload"
        );
        let message = body_json(response).await["message"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(
            message.contains(names),
            "{label}: the 422 message names the defect ({names}): {message}"
        );
        let sha = crate::state::stream_assets::sha256_hex(&bytes);
        let path = state
            .stream_assets_dir()
            .join("fonts")
            .join(format!("{sha}.ttf"));
        assert!(!path.exists(), "{label}: a refused font is not stored");
    }
}

#[tokio::test]
async fn stored_font_browsers_refuse_is_hidden_from_list_and_css_but_kept() {
    let (state, _dir) = test_state().await;
    let good = upload(&state, "good.ttf", FIXTURE_TTF).await;
    // The font-id-71 shape, plus 4 trailing bytes so its sha (and thus its
    // dedup row) is unique to this test — trailing data is legal sfnt.
    let mut broken_bytes = with_os2_version(FIXTURE_TTF, 5);
    broken_bytes.extend_from_slice(&[0, 0, 0, 0]);
    let broken = seed_stored_font(&state, "BrokenOs2Face778", &broken_bytes).await;

    let list: Vec<StreamFont> =
        serde_json::from_value(body_json(get(&state, "/stream/api/fonts").await).await).unwrap();
    assert!(
        list.iter().any(|f| f.id == good.id),
        "a valid face stays listed"
    );
    assert!(
        !list.iter().any(|f| f.id == broken.id),
        "a face the browser refuses is NOT offered in the font list/picker"
    );

    let css = String::from_utf8(body_bytes(get(&state, "/stream/fonts.css").await).await).unwrap();
    assert!(
        css.contains(&format!("/stream/fonts/{}\")", good.id)),
        "the valid face keeps its @font-face rule: {css}"
    );
    assert!(
        !css.contains(&format!("/stream/fonts/{}\")", broken.id)),
        "no @font-face for the refused face, so no page fetches it: {css}"
    );
    assert!(
        !css.contains("BrokenOs2Face778"),
        "the refused family is absent from fonts.css: {css}"
    );

    // Hidden, never deleted: the row and the file stay (owner-approved
    // cleanup only), so an explicit DELETE still works.
    state
        .repository()
        .get_stream_font(broken.id)
        .await
        .expect("the refused face's row is kept");
    let path = state
        .stream_assets_dir()
        .join("fonts")
        .join(format!("{}.ttf", broken.sha256));
    assert!(path.exists(), "the refused face's file is kept");
}

#[tokio::test]
async fn stored_font_with_missing_file_is_hidden_from_list_and_css() {
    let (state, _dir) = test_state().await;
    // A row whose bytes are gone from disk would 404 in every browser.
    let font = state
        .repository()
        .insert_or_get_stream_font(NewStreamFont {
            sha256: "bb22cc33dd44ee55ff66778899aabbccddeeff00112233445566778899aabbcc".to_string(),
            original_filename: "MissingFile778.ttf".to_string(),
            family: "MissingFileFamily778".to_string(),
            weight: 400,
            italic: false,
            format: "ttf".to_string(),
            size_bytes: 1,
        })
        .await
        .unwrap();

    let list: Vec<StreamFont> =
        serde_json::from_value(body_json(get(&state, "/stream/api/fonts").await).await).unwrap();
    assert!(
        !list.iter().any(|f| f.id == font.id),
        "a face with no file is not listed"
    );
    let css = String::from_utf8(body_bytes(get(&state, "/stream/fonts.css").await).await).unwrap();
    assert!(
        !css.contains(&format!("/stream/fonts/{}\")", font.id)),
        "no @font-face for a face with no file: {css}"
    );
}

// ── #830: a family whose files carry the default OS/2 weight ─────────────────
//
// Nexa on SNV/PP: Light, Heavy, Black and XBold all declare usWeightClass 400;
// the real style is only in name 17. Each face must still get its own weight,
// or `/stream/fonts.css` emits colliding `@font-face` rules. The faces are
// derived in memory from the OFL fixture (`test_fonts::with_names`).

const TYPO_FAMILY: u16 = 16;
const TYPO_SUBFAMILY: u16 = 17;

/// A face of `family` styled `style` in name 17, OS/2 left at the default 400.
fn family_face(family: &str, style: &str) -> Vec<u8> {
    with_names(
        FIXTURE_TTF,
        &[(TYPO_FAMILY, family), (TYPO_SUBFAMILY, style)],
    )
}

/// The `@font-face` block of `/stream/fonts.css` that serves font `id`.
fn css_rule_for(css: &str, id: i64) -> String {
    let src = format!("url(\"/stream/fonts/{id}\")");
    css.split("@font-face")
        .find(|rule| rule.contains(&src))
        .unwrap_or_else(|| panic!("no @font-face rule for font {id}: {css}"))
        .to_string()
}

#[tokio::test]
async fn a_mislabelled_family_lists_one_weight_per_face() {
    let (state, _dir) = test_state().await;
    let family = "Router830";
    // Heavy is uploaded BEFORE Black: Black's upload must move the stored
    // Heavy face below it (not only the next restart's re-derive).
    let faces = [
        ("Router830-Regular.ttf", family_face(family, "Regular")),
        ("Router830-Light.ttf", family_face(family, "Light")),
        ("Router830-Heavy.ttf", family_face(family, "Heavy")),
        ("Router830-Black.ttf", family_face(family, "Black")),
        (
            "Router830-BlackItalic.ttf",
            with_os2_italic(&family_face(family, "Black Italic")),
        ),
    ];
    let mut ids = Vec::new();
    for (filename, bytes) in &faces {
        ids.push(upload(&state, filename, bytes).await.id);
    }

    let response = get(&state, "/stream/api/fonts").await;
    assert_eq!(response.status(), StatusCode::OK);
    let listed = body_json(response).await;
    let mut faces_listed: Vec<(u64, bool, String)> = listed
        .as_array()
        .expect("font list is an array")
        .iter()
        .filter(|f| f["family"] == family)
        .map(|f| {
            (
                f["weight"].as_u64().expect("weight"),
                f["italic"].as_bool().expect("italic"),
                f["styleName"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    faces_listed.sort();
    assert_eq!(
        faces_listed,
        vec![
            (300, false, "Light".to_string()),
            (400, false, "Regular".to_string()),
            (800, false, "Heavy".to_string()),
            (900, false, "Black".to_string()),
            (900, true, "Black Italic".to_string()),
        ],
        "one distinct weight/style per face, named by its style: {listed}"
    );

    let css = String::from_utf8(body_bytes(get(&state, "/stream/fonts.css").await).await).unwrap();
    let expected = [
        (ids[0], 400, "normal"),
        (ids[1], 300, "normal"),
        (ids[2], 800, "normal"),
        (ids[3], 900, "normal"),
        (ids[4], 900, "italic"),
    ];
    for (id, weight, style) in expected {
        let rule = css_rule_for(&css, id);
        assert!(
            rule.contains(&format!("font-weight: {weight};")),
            "face {id} served at weight {weight}: {rule}"
        );
        assert!(
            rule.contains(&format!("font-style: {style};")),
            "face {id} served as {style}: {rule}"
        );
    }
}

#[tokio::test]
async fn a_heavy_face_uploaded_after_black_answers_with_its_moved_weight() {
    let (state, _dir) = test_state().await;
    let family = "Late830";
    let black = upload(&state, "Late830-Black.ttf", &family_face(family, "Black")).await;
    let heavy = upload(&state, "Late830-Heavy.ttf", &family_face(family, "Heavy")).await;
    assert_eq!(black.weight, 900);
    assert_eq!(
        heavy.weight, 800,
        "the upload answers with the re-derived row"
    );
    assert_eq!(heavy.style_name.as_deref(), Some("Heavy"));
}
