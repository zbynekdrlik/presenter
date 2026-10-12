//! #843 Drive client tests: the listing parser against a synthetic page in
//! the live markup (`fixtures/folder_listing.html` — invented ids and names),
//! and list / HEAD / download against a wiremock Drive. Drive is an external
//! service, so a mock is allowed; nothing here reaches Google.

use super::listing::decode_entities;
use super::*;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const LISTING: &str = include_str!("fixtures/folder_listing.html");
const FOLDER_ID: &str = "1SynthRootFolderZZZZZZZZZZZZZZZZ9";
const PHOTO: &str = "1SynthPhotoAAAAAAAAAAAAAAAAAAAA1";
const CLIP: &str = "1SynthClipCCCCCCCCCCCCCCCCCCCCCC3";
const LAST_MODIFIED: &str = "Mon, 14 Sep 2026 10:09:42 GMT";

fn folder(resource_key: Option<&str>) -> DriveFolder {
    DriveFolder {
        id: FOLDER_ID.to_string(),
        resource_key: resource_key.map(str::to_string),
    }
}

#[test]
fn the_listing_yields_every_entry_in_page_order() {
    let entries = parse_listing(LISTING);
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            PHOTO,
            "1SynthFolderBBBBBBBBBBBBBBBBBBBB2",
            CLIP,
            "1SynthMovDDDDDDDDDDDDDDDDDDDDDDD4",
            "1SynthPdfEEEEEEEEEEEEEEEEEEEEEEE5",
            "1SynthPngFFFFFFFFFFFFFFFFFFFFFFF6",
        ]
    );
    assert_eq!(
        entries[0],
        DriveEntry {
            id: PHOTO.to_string(),
            name: "Leto & more.jpg".to_string(),
            mime: Some("image/jpeg".to_string()),
            is_folder: false,
            resource_key: None,
        }
    );
    // A subfolder: folder link + Folder sprite, no MIME icon.
    assert!(entries[1].is_folder);
    assert_eq!(entries[1].mime, None);
    // The video carries its own resourcekey; `&#39;` is decoded.
    assert_eq!(entries[2].name, "Don't stop.mp4");
    assert_eq!(entries[2].mime.as_deref(), Some("video/mp4"));
    assert_eq!(entries[2].resource_key.as_deref(), Some("0-SynthKey_abc"));
    assert_eq!(entries[3].mime.as_deref(), Some("video/quicktime"));
    assert_eq!(entries[4].mime.as_deref(), Some("application/pdf"));
    // The title is trimmed.
    assert_eq!(entries[5].name, "logo.png");
    assert_eq!(entries[5].mime.as_deref(), Some("image/png"));
}

#[test]
fn only_a_listing_page_counts_as_a_listing() {
    assert!(is_listing_page(LISTING));
    let empty = r#"<div id="flip-contents"><div class="flip-entries"></div></div>"#;
    assert!(is_listing_page(empty));
    assert!(parse_listing(empty).is_empty());
    assert!(!is_listing_page(
        "<html><title>Sign in - Google Accounts</title></html>"
    ));
}

#[test]
fn an_entry_whose_id_is_not_a_drive_id_is_dropped() {
    let html = concat!(
        r#"<div class="flip-entries">"#,
        r#"<div class="flip-entry" id="entry-../../etc/passwd" tabindex="0">"#,
        r#"<div class="flip-entry-title">x</div></div></div>"#
    );
    assert!(parse_listing(html).is_empty());
}

#[test]
fn title_entities_are_decoded() {
    assert_eq!(
        decode_entities("a &amp; b &#39;c&#39; &quot;d&quot; &lt;e&gt; &#x41;&#66;"),
        "a & b 'c' \"d\" <e> AB"
    );
    // Unknown entities and bare ampersands stay as they are.
    assert_eq!(decode_entities("R&D &unknown; & x"), "R&D &unknown; & x");
}

async fn mount_listing(server: &MockServer, status: u16, body: &str) {
    Mock::given(method("GET"))
        .and(path("/embeddedfolderview"))
        .and(query_param("id", FOLDER_ID))
        .respond_with(
            ResponseTemplate::new(status)
                .set_body_raw(body.as_bytes().to_vec(), "text/html; charset=utf-8"),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn list_folder_sends_the_id_and_resource_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/embeddedfolderview"))
        .and(query_param("id", FOLDER_ID))
        .and(query_param("resourcekey", "0-RootKey"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(LISTING.as_bytes().to_vec(), "text/html; charset=utf-8"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = DriveClient::for_test(&server.uri());
    let entries = client
        .list_folder(&folder(Some("0-RootKey")))
        .await
        .expect("listing");
    assert_eq!(entries.len(), 6);
}

#[tokio::test]
async fn list_folder_maps_the_failure_statuses() {
    let cases: [(u16, &str); 4] = [
        (401, "<html>auth</html>"),
        (404, "<html>missing</html>"),
        (500, "<html>boom</html>"),
        (200, "<html><title>Sign in</title></html>"),
    ];
    for (status, body) in cases {
        let server = MockServer::start().await;
        mount_listing(&server, status, body).await;
        let client = DriveClient::for_test(&server.uri());
        let err = client
            .list_folder(&folder(None))
            .await
            .expect_err("must fail");
        match (status, &err) {
            (401, DriveError::NotPublic { status: 401 })
            | (404, DriveError::FolderNotFound)
            | (500, DriveError::Status { status: 500 })
            | (200, DriveError::UnexpectedListing) => {}
            _ => panic!("status {status} mapped to {err:?}"),
        }
    }
}

#[tokio::test]
async fn a_dead_drive_is_a_network_error() {
    let client = DriveClient::for_test("http://127.0.0.1:1");
    let err = client
        .list_folder(&folder(None))
        .await
        .expect_err("dead loopback");
    assert!(matches!(err, DriveError::Network { .. }), "{err:?}");
}

fn head(mime: &str, length: usize) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", mime)
        .insert_header("content-length", length.to_string().as_str())
        .insert_header("last-modified", LAST_MODIFIED)
}

#[tokio::test]
async fn head_file_reads_size_and_last_modified() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/download"))
        .and(query_param("id", CLIP))
        .and(query_param("export", "download"))
        .and(query_param("confirm", "t"))
        .and(query_param("resourcekey", "0-SynthKey_abc"))
        .respond_with(head("video/mp4", 9351))
        .mount(&server)
        .await;
    let client = DriveClient::for_test(&server.uri());
    let remote = client
        .head_file(CLIP, Some("0-SynthKey_abc"))
        .await
        .expect("head");
    assert_eq!(
        remote,
        RemoteFile {
            size: Some(9351),
            last_modified: Some(LAST_MODIFIED.to_string()),
        }
    );
}

#[tokio::test]
async fn head_file_refuses_an_html_page_and_maps_404() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(query_param("id", PHOTO))
        .respond_with(head("text/html; charset=utf-8", 2468))
        .mount(&server)
        .await;
    let client = DriveClient::for_test(&server.uri());
    let err = client.head_file(PHOTO, None).await.expect_err("html page");
    assert!(matches!(err, DriveError::HtmlPage), "{err:?}");
    // An unmounted id answers 404.
    let err = client
        .head_file("1Unknown", None)
        .await
        .expect_err("unknown file");
    assert!(matches!(err, DriveError::FileNotFound), "{err:?}");
}

#[tokio::test]
async fn download_file_streams_the_bytes_to_disk() {
    let server = MockServer::start().await;
    let bytes: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    Mock::given(method("GET"))
        .and(path("/download"))
        .and(query_param("id", PHOTO))
        .and(query_param("confirm", "t"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(bytes.clone(), "image/jpeg")
                .insert_header("last-modified", LAST_MODIFIED),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("photo.part");
    let client = DriveClient::for_test(&server.uri());
    let written = client
        .download_file(PHOTO, None, &dest)
        .await
        .expect("download");
    assert_eq!(written.size, Some(bytes.len() as u64));
    assert_eq!(written.last_modified.as_deref(), Some(LAST_MODIFIED));
    assert_eq!(std::fs::read(&dest).expect("read back"), bytes);
}

#[tokio::test]
async fn download_file_refuses_an_html_page_and_an_oversized_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("id", PHOTO))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(b"<html>scan</html>".to_vec(), "text/html"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(query_param("id", CLIP))
        .respond_with(ResponseTemplate::new(200).set_body_raw(vec![7u8; 64], "video/mp4"))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let client = DriveClient::for_test_with_max_bytes(&server.uri(), 32);
    let err = client
        .download_file(PHOTO, None, &dir.path().join("a"))
        .await
        .expect_err("html");
    assert!(matches!(err, DriveError::HtmlPage), "{err:?}");
    let err = client
        .download_file(CLIP, None, &dir.path().join("b"))
        .await
        .expect_err("too large");
    assert!(matches!(err, DriveError::TooLarge { .. }), "{err:?}");
}
