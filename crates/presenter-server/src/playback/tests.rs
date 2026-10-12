//! #843 playback mirror tests: one folder synced against a wiremock Drive into
//! a temp dir (list → HEAD → download changed → manifest → prune), plus the
//! pure scheduling / change-detection policies.

use std::path::Path;

use chrono::{Duration as ChronoDuration, Utc};
use presenter_core::{DriveFolder, PlaybackItemKind};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::manifest::{
    failure_summary, needs_download, playable, read_manifest, to_playlist, version_token,
    write_manifest, FileFailure, Manifest, ManifestItem,
};
use super::sync::{sync_folder, SyncReport};
use super::{
    budget_gib, retry_backoff, should_log_failure, should_prune_unreferenced, should_sync, Budget,
    SyncTrigger,
};
use crate::drive::{DriveClient, DriveError, RemoteFile};

const FOLDER_ID: &str = "1SynthRootFolderZZZZZZZZZZZZZZZZ9";
const PHOTO: &str = "1SynthPhotoAAAAAAAAAAAAAAAAAAAA1";
const CLIP: &str = "1SynthClipCCCCCCCCCCCCCCCCCCCCCC3";
const LOGO: &str = "1SynthPngFFFFFFFFFFFFFFFFFFFFFFF6";
const SUBFOLDER: &str = "1SynthFolderBBBBBBBBBBBBBBBBBBBB2";
const PDF: &str = "1SynthPdfEEEEEEEEEEEEEEEEEEEEEEE5";
const DAY_1: &str = "Mon, 14 Sep 2026 10:09:42 GMT";
const DAY_2: &str = "Tue, 15 Sep 2026 08:00:00 GMT";

/// A budget that never refuses.
fn unlimited() -> Budget {
    Budget {
        limit: u64::MAX,
        used: 0,
    }
}

fn folder() -> DriveFolder {
    DriveFolder {
        id: FOLDER_ID.to_string(),
        resource_key: None,
    }
}

/// One listing entry: `(id, mime or "folder", name)`.
type Entry<'a> = (&'a str, &'a str, &'a str);

/// A synthetic listing page in Drive's markup.
fn listing_html(entries: &[Entry<'_>]) -> String {
    let mut html = String::from(
        r#"<html><body class="flip-embedded"><div id="flip-contents"><div class="flip-entries">"#,
    );
    for (id, mime, name) in entries {
        if *mime == "folder" {
            html.push_str(&format!(
                r#"<div class="flip-entry" id="entry-{id}" tabindex="0" role="link"><div class="flip-entry-info"><a href="https://drive.google.com/drive/folders/{id}" target="_blank"><div class="flip-entry-list-icon"><div aria-label="Folder" class="icon-color-1"></div></div><div class="flip-entry-title">{name}</div></a></div></div>"#
            ));
        } else {
            html.push_str(&format!(
                r#"<div class="flip-entry" id="entry-{id}" tabindex="0" role="link"><div class="flip-entry-info"><a href="https://drive.google.com/file/d/{id}/view?usp=drive_web" target="_blank"><div class="flip-entry-list-icon"><img src="https://drive-thirdparty.googleusercontent.com/16/type/{mime}" alt=""/></div><div class="flip-entry-title">{name}</div></a></div></div>"#
            ));
        }
    }
    html.push_str("</div></div></body></html>");
    html
}

const FULL: [Entry<'static>; 6] = [
    (PHOTO, "image/jpeg", "leto.jpg"),
    (SUBFOLDER, "folder", "Subfolder"),
    (CLIP, "video/mp4", "clip.mp4"),
    (
        "1SynthMovDDDDDDDDDDDDDDDDDDDDDDD4",
        "video/quicktime",
        "phone.MOV",
    ),
    (PDF, "application/pdf", "program.pdf"),
    (LOGO, "image/png", "logo.png"),
];

async fn mount_listing(server: &MockServer, entries: &[Entry<'_>]) {
    Mock::given(method("GET"))
        .and(path("/embeddedfolderview"))
        .and(query_param("id", FOLDER_ID))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            listing_html(entries).into_bytes(),
            "text/html; charset=utf-8",
        ))
        .mount(server)
        .await;
}

/// HEAD + GET of one file on the mock download endpoint.
async fn mount_file(server: &MockServer, id: &str, mime: &str, body: &[u8], last_modified: &str) {
    Mock::given(method("HEAD"))
        .and(path("/download"))
        .and(query_param("id", id))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", mime)
                .insert_header("content-length", body.len().to_string().as_str())
                .insert_header("last-modified", last_modified),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/download"))
        .and(query_param("id", id))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(body.to_vec(), mime)
                .insert_header("last-modified", last_modified),
        )
        .mount(server)
        .await;
}

async fn mount_all_files(server: &MockServer, last_modified: &str) {
    mount_file(server, PHOTO, "image/jpeg", b"jpeg-bytes-1", last_modified).await;
    mount_file(
        server,
        CLIP,
        "video/mp4",
        b"mp4-bytes-of-the-clip",
        last_modified,
    )
    .await;
    mount_file(server, LOGO, "image/png", b"png-bytes", last_modified).await;
}

/// How many GET downloads of `id` the mock received.
async fn downloads_of(server: &MockServer, id: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| {
            r.method.as_str() == "GET"
                && r.url.path() == "/download"
                && r.url.query_pairs().any(|(k, v)| k == "id" && v == id)
        })
        .count()
}

fn ids(manifest: &Manifest) -> Vec<&str> {
    manifest.items.iter().map(|i| i.file_id.as_str()).collect()
}

fn read(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap_or_default()
}

fn leftover_tmp(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".tmp"))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_sync_mirrors_the_playable_files_in_listing_order() {
    let server = MockServer::start().await;
    mount_listing(&server, &FULL).await;
    mount_all_files(&server, DAY_1).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());

    let (manifest, report) = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("sync");

    assert_eq!(ids(&manifest), [PHOTO, CLIP, LOGO]);
    let kinds: Vec<PlaybackItemKind> = manifest.items.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        [
            PlaybackItemKind::Image,
            PlaybackItemKind::Video,
            PlaybackItemKind::Image
        ]
    );
    assert_eq!(manifest.items[1].ext, "mp4");
    assert_eq!(manifest.items[1].size, Some(21));
    assert_eq!(manifest.items[1].last_modified.as_deref(), Some(DAY_1));
    assert!(manifest.synced_at.is_some());
    assert_eq!(manifest.last_error, None);
    assert_eq!(
        report,
        SyncReport {
            listed: 6,
            items: 3,
            downloaded: 3,
            kept: 0,
            skipped: 3,
            failed: 0,
            backed_off: 0,
            pruned: 0,
            bytes: 12 + 21 + 9,
        }
    );
    assert_eq!(read(&dir, &format!("{PHOTO}.jpg")), b"jpeg-bytes-1");
    assert_eq!(read(&dir, &format!("{CLIP}.mp4")), b"mp4-bytes-of-the-clip");
    assert_eq!(read(&dir, &format!("{LOGO}.png")), b"png-bytes");
    // The subfolder, the QuickTime clip and the PDF were never downloaded.
    for skipped in [SUBFOLDER, PDF] {
        assert_eq!(downloads_of(&server, skipped).await, 0);
    }
    assert_eq!(read_manifest(&dir).await, Some(manifest));
    assert!(leftover_tmp(&dir).is_empty());
}

#[tokio::test]
async fn an_unchanged_file_is_not_downloaded_again() {
    let server = MockServer::start().await;
    mount_listing(&server, &FULL).await;
    mount_all_files(&server, DAY_1).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());
    sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("first");

    let (manifest, report) = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("second");

    assert_eq!(ids(&manifest), [PHOTO, CLIP, LOGO]);
    assert_eq!((report.downloaded, report.kept), (0, 3));
    for id in [PHOTO, CLIP, LOGO] {
        assert_eq!(downloads_of(&server, id).await, 1, "{id}");
    }
}

#[tokio::test]
async fn a_changed_file_is_downloaded_again() {
    let server = MockServer::start().await;
    mount_listing(&server, &FULL).await;
    mount_all_files(&server, DAY_1).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());
    sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("first");

    // The photo is replaced on Drive: a new date and new bytes.
    server.reset().await;
    mount_listing(&server, &FULL).await;
    mount_file(&server, PHOTO, "image/jpeg", b"jpeg-bytes-2-new", DAY_2).await;
    mount_file(&server, CLIP, "video/mp4", b"mp4-bytes-of-the-clip", DAY_1).await;
    mount_file(&server, LOGO, "image/png", b"png-bytes", DAY_1).await;
    let (manifest, report) = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("second");

    assert_eq!((report.downloaded, report.kept), (1, 2));
    assert_eq!(read(&dir, &format!("{PHOTO}.jpg")), b"jpeg-bytes-2-new");
    assert_eq!(manifest.items[0].last_modified.as_deref(), Some(DAY_2));
    assert_eq!(manifest.items[0].size, Some(16));
}

#[tokio::test]
async fn an_html_error_page_is_never_cached() {
    let server = MockServer::start().await;
    mount_listing(&server, &[(PHOTO, "image/jpeg", "leto.jpg")]).await;
    Mock::given(method("HEAD"))
        .and(query_param("id", PHOTO))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/jpeg")
                .insert_header("content-length", "2468"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(query_param("id", PHOTO))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(b"<html>Virus scan warning</html>".to_vec(), "text/html"),
        )
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());

    let (manifest, report) = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("sync");

    assert!(manifest.items.is_empty());
    assert_eq!(report.failed, 1);
    assert!(manifest.last_error.is_some(), "the failure is reported");
    assert!(!dir.join(format!("{PHOTO}.jpg")).exists());
    assert!(
        leftover_tmp(&dir).is_empty(),
        "the partial download is removed"
    );
}

#[tokio::test]
async fn a_failed_listing_leaves_the_mirror_untouched() {
    let server = MockServer::start().await;
    mount_listing(&server, &FULL).await;
    mount_all_files(&server, DAY_1).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());
    let (good, _) = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("first");

    // The folder stops being public.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/embeddedfolderview"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let err = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect_err("401");

    assert!(
        matches!(err, DriveError::NotPublic { status: 401 }),
        "{err:?}"
    );
    assert_eq!(read_manifest(&dir).await, Some(good));
    assert_eq!(read(&dir, &format!("{CLIP}.mp4")), b"mp4-bytes-of-the-clip");
}

#[tokio::test]
async fn files_that_left_the_folder_are_pruned() {
    let server = MockServer::start().await;
    mount_listing(&server, &FULL).await;
    mount_all_files(&server, DAY_1).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());
    let mut budget = unlimited();
    sync_folder(&client, &dir, &folder(), false, &mut budget)
        .await
        .expect("first");
    assert_eq!(budget.used, 12 + 21 + 9);

    server.reset().await;
    mount_listing(&server, &[(LOGO, "image/png", "logo.png")]).await;
    mount_all_files(&server, DAY_1).await;
    let (manifest, report) = sync_folder(&client, &dir, &folder(), false, &mut budget)
        .await
        .expect("second");

    assert_eq!(ids(&manifest), [LOGO]);
    assert_eq!(budget.used, 9, "the leaving files' bytes count as free");
    // Retired first: still on disk (and served) while outputs may still list them.
    assert_eq!(report.pruned, 0);
    assert_eq!(manifest.retired.len(), 2);
    assert!(dir.join(format!("{PHOTO}.jpg")).exists());

    // After the grace period the next sync deletes them.
    age_retired(&dir, 10).await;
    let (manifest, report) = sync_folder(&client, &dir, &folder(), false, &mut budget)
        .await
        .expect("third");
    assert_eq!(report.pruned, 2);
    assert!(manifest.retired.is_empty());
    assert!(!dir.join(format!("{PHOTO}.jpg")).exists());
    assert!(!dir.join(format!("{CLIP}.mp4")).exists());
    assert_eq!(read(&dir, &format!("{LOGO}.png")), b"png-bytes");
}

/// Move every retiree's `retired_at` `minutes` into the past.
async fn age_retired(dir: &Path, minutes: i64) {
    let mut manifest = read_manifest(dir).await.expect("manifest");
    for retired in &mut manifest.retired {
        retired.retired_at -= ChronoDuration::minutes(minutes);
    }
    write_manifest(dir, &manifest)
        .await
        .expect("write manifest");
}

#[tokio::test]
async fn a_retired_file_that_comes_back_is_played_again() {
    let server = MockServer::start().await;
    mount_listing(&server, &FULL).await;
    mount_all_files(&server, DAY_1).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());
    sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("first");
    server.reset().await;
    mount_listing(&server, &[(LOGO, "image/png", "logo.png")]).await;
    mount_all_files(&server, DAY_1).await;
    sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("second");

    // The photo is put back on Drive within the grace period.
    server.reset().await;
    mount_listing(&server, &FULL).await;
    mount_all_files(&server, DAY_1).await;
    let (manifest, report) = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("third");

    assert_eq!(ids(&manifest), [PHOTO, CLIP, LOGO]);
    assert!(manifest.retired.is_empty());
    assert_eq!(report.pruned, 0);
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_cached_copy() {
    let server = MockServer::start().await;
    mount_listing(&server, &FULL).await;
    mount_all_files(&server, DAY_1).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());
    sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("first");

    // The photo changed on Drive, but its download now fails.
    server.reset().await;
    mount_listing(&server, &FULL).await;
    Mock::given(method("HEAD"))
        .and(query_param("id", PHOTO))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/jpeg")
                .insert_header("content-length", "99")
                .insert_header("last-modified", DAY_2),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(query_param("id", PHOTO))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    mount_file(&server, CLIP, "video/mp4", b"mp4-bytes-of-the-clip", DAY_1).await;
    mount_file(&server, LOGO, "image/png", b"png-bytes", DAY_1).await;
    let (manifest, report) = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("second");

    assert_eq!(ids(&manifest), [PHOTO, CLIP, LOGO]);
    assert_eq!((report.kept, report.failed), (3, 0));
    assert_eq!(read(&dir, &format!("{PHOTO}.jpg")), b"jpeg-bytes-1");
    // The old date stays, so the next sync tries again…
    assert_eq!(manifest.items[0].last_modified.as_deref(), Some(DAY_1));
    // …and the failure is recorded with its reason.
    assert_eq!(manifest.failures.len(), 1);
    let error = manifest.last_error.unwrap_or_default();
    assert!(
        error.contains("leto.jpg") && error.contains("HTTP 500"),
        "{error}"
    );
}

/// How many `method` requests for file `id` the mock received.
async fn requests_of(server: &MockServer, method_name: &str, id: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| {
            r.method.as_str() == method_name
                && r.url.path() == "/download"
                && r.url.query_pairs().any(|(k, v)| k == "id" && v == id)
        })
        .count()
}

/// The listing has the photo; its HEAD works, its download answers 500.
async fn mount_failing_photo(server: &MockServer) {
    mount_listing(server, &[(PHOTO, "image/jpeg", "leto.jpg")]).await;
    Mock::given(method("HEAD"))
        .and(query_param("id", PHOTO))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/jpeg")
                .insert_header("content-length", "12"),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/download"))
        .and(query_param("id", PHOTO))
        .respond_with(ResponseTemplate::new(500))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_failing_file_waits_for_its_backoff_and_a_refresh_retries_it() {
    let server = MockServer::start().await;
    mount_failing_photo(&server).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());

    let (first, report) = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("first");
    assert_eq!(report.failed, 1);
    let failure: &FileFailure = first.failures.first().expect("failure recorded");
    assert_eq!((failure.file_id.as_str(), failure.streak), (PHOTO, 1));
    assert!(failure.retry_at > Utc::now() + ChronoDuration::minutes(4));
    let error = first.last_error.clone().unwrap_or_default();
    assert!(
        error.contains("leto.jpg") && error.contains("HTTP 500"),
        "{error}"
    );

    // The next periodic sync does not touch the file before its retry time.
    let (second, report) = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("second");
    assert_eq!(report.backed_off, 1);
    assert_eq!(requests_of(&server, "HEAD", PHOTO).await, 1);
    assert_eq!(requests_of(&server, "GET", PHOTO).await, 1);
    assert_eq!(
        second.failures, first.failures,
        "the failure is carried over"
    );

    // "Obnoviť" retries at once; the streak grows and the backoff doubles.
    let (third, _) = sync_folder(&client, &dir, &folder(), true, &mut unlimited())
        .await
        .expect("forced");
    assert_eq!(requests_of(&server, "GET", PHOTO).await, 2);
    assert_eq!(third.failures.first().map(|f| f.streak), Some(2));
}

#[tokio::test]
async fn a_recovered_file_clears_its_failure() {
    let server = MockServer::start().await;
    mount_failing_photo(&server).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());
    sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("failing");

    server.reset().await;
    mount_listing(&server, &[(PHOTO, "image/jpeg", "leto.jpg")]).await;
    mount_file(&server, PHOTO, "image/jpeg", b"jpeg-bytes-1", DAY_1).await;
    let (manifest, report) = sync_folder(&client, &dir, &folder(), true, &mut unlimited())
        .await
        .expect("recovered");

    assert_eq!(report.downloaded, 1);
    assert!(manifest.failures.is_empty());
    assert_eq!(manifest.last_error, None);
    assert_eq!(ids(&manifest), [PHOTO]);
}

#[tokio::test]
async fn the_budget_refuses_a_download_that_would_overfill_the_mirror() {
    let server = MockServer::start().await;
    mount_listing(&server, &FULL).await;
    mount_all_files(&server, DAY_1).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());
    // 25 bytes: the photo (12) fits, the clip (21) would not, the logo (9) does.
    let mut budget = Budget { limit: 25, used: 0 };

    let (manifest, report) = sync_folder(&client, &dir, &folder(), false, &mut budget)
        .await
        .expect("sync");

    assert_eq!(ids(&manifest), [PHOTO, LOGO]);
    assert_eq!((report.downloaded, report.failed), (2, 1));
    assert_eq!(budget.used, 12 + 9);
    assert_eq!(
        requests_of(&server, "GET", CLIP).await,
        0,
        "refused before downloading"
    );
    let failure = manifest.failures.first().expect("clip failure");
    assert_eq!(failure.file_id, CLIP);
    assert!(
        failure.reason.contains("úložisko prehrávania je plné"),
        "{}",
        failure.reason
    );
}

#[test]
fn the_retry_backoff_doubles_up_to_six_hours() {
    let minutes: Vec<i64> = [1, 2, 3, 4, 7, 8, 40]
        .iter()
        .map(|streak| retry_backoff(*streak).num_minutes())
        .collect();
    assert_eq!(minutes, [5, 10, 20, 40, 320, 360, 360]);
}

#[test]
fn the_budget_comes_from_whole_gib() {
    assert_eq!(budget_gib(None), 20);
    assert_eq!(budget_gib(Some("5")), 5);
    assert_eq!(budget_gib(Some(" 100 ")), 100);
    assert_eq!(budget_gib(Some("0")), 20);
    assert_eq!(budget_gib(Some("lots")), 20);
    let budget = Budget {
        limit: 10,
        used: 12,
    };
    assert_eq!(budget.remaining(), 0);
}

#[test]
fn the_failure_summary_names_the_first_file() {
    assert_eq!(failure_summary(&[]), None);
    let failure = |name: &str| FileFailure {
        file_id: "f".to_string(),
        name: name.to_string(),
        streak: 1,
        retry_at: Utc::now(),
        reason: "Google Drive vrátil HTTP 403".to_string(),
    };
    assert_eq!(
        failure_summary(&[failure("a.jpg"), failure("b.jpg")]).as_deref(),
        Some("2 súbor(ov) sa nepodarilo stiahnuť — a.jpg: Google Drive vrátil HTTP 403")
    );
}

#[tokio::test]
async fn an_empty_folder_empties_the_playlist() {
    let server = MockServer::start().await;
    mount_listing(&server, &FULL).await;
    mount_all_files(&server, DAY_1).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join(FOLDER_ID);
    let client = DriveClient::for_test(&server.uri());
    sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("first");

    server.reset().await;
    mount_listing(&server, &[]).await;
    let (manifest, report) = sync_folder(&client, &dir, &folder(), false, &mut unlimited())
        .await
        .expect("second");

    assert!(manifest.items.is_empty());
    assert_eq!(manifest.retired.len(), 3);
    assert_eq!(report.pruned, 0, "kept for the grace period");
}

fn item(size: Option<u64>, last_modified: Option<&str>) -> ManifestItem {
    ManifestItem {
        file_id: PHOTO.to_string(),
        name: "leto.jpg".to_string(),
        mime: "image/jpeg".to_string(),
        kind: PlaybackItemKind::Image,
        ext: "jpg".to_string(),
        size,
        last_modified: last_modified.map(str::to_string),
    }
}

fn remote(size: Option<u64>, last_modified: Option<&str>) -> RemoteFile {
    RemoteFile {
        size,
        last_modified: last_modified.map(str::to_string),
    }
}

#[test]
fn a_download_is_needed_only_for_a_missing_or_changed_file() {
    let old = item(Some(10), Some(DAY_1));
    assert!(!needs_download(
        Some(&old),
        &remote(Some(10), Some(DAY_1)),
        true
    ));
    assert!(
        needs_download(Some(&old), &remote(Some(10), Some(DAY_1)), false),
        "file gone"
    );
    assert!(
        needs_download(None, &remote(Some(10), Some(DAY_1)), true),
        "no record"
    );
    assert!(
        needs_download(Some(&old), &remote(Some(11), Some(DAY_1)), true),
        "size"
    );
    assert!(
        needs_download(Some(&old), &remote(Some(10), Some(DAY_2)), true),
        "date"
    );
    // A field the HEAD did not report is not compared.
    assert!(!needs_download(Some(&old), &remote(None, None), true));
}

#[test]
fn only_browser_playable_types_are_kept() {
    assert_eq!(
        playable("image/jpeg"),
        Some((PlaybackItemKind::Image, "jpg"))
    );
    assert_eq!(
        playable("image/png"),
        Some((PlaybackItemKind::Image, "png"))
    );
    assert_eq!(
        playable("image/webp"),
        Some((PlaybackItemKind::Image, "webp"))
    );
    assert_eq!(
        playable("video/mp4"),
        Some((PlaybackItemKind::Video, "mp4"))
    );
    assert_eq!(
        playable("video/webm"),
        Some((PlaybackItemKind::Video, "webm"))
    );
    for refused in [
        "video/quicktime",
        "image/heic",
        "image/svg+xml",
        "application/pdf",
        "audio/mpeg",
        "",
    ] {
        assert_eq!(playable(refused), None, "{refused}");
    }
}

#[test]
fn the_version_token_follows_the_bytes() {
    let a = version_token(&item(Some(10), Some(DAY_1)));
    assert_eq!(a.len(), 12);
    assert_eq!(a, version_token(&item(Some(10), Some(DAY_1))));
    assert_ne!(a, version_token(&item(Some(11), Some(DAY_1))));
    assert_ne!(a, version_token(&item(Some(10), Some(DAY_2))));
}

#[test]
fn the_playlist_lists_items_with_their_cache_urls() {
    let manifest = Manifest {
        folder_id: FOLDER_ID.to_string(),
        items: vec![item(Some(10), Some(DAY_1))],
        synced_at: Some(Utc::now()),
        last_error: Some("chyba".to_string()),
        error_at: None,
        failures: Vec::new(),
        retired: Vec::new(),
    };
    let playlist = to_playlist(FOLDER_ID, Some(&manifest), &manifest.items, true);
    assert_eq!(playlist.items.len(), 1);
    assert_eq!(
        playlist.items[0].url,
        format!(
            "/stream/playback/{FOLDER_ID}/{PHOTO}?v={}",
            version_token(&manifest.items[0])
        )
    );
    assert_eq!(playlist.error.as_deref(), Some("chyba"));
    assert!(playlist.syncing);
    let empty = to_playlist(FOLDER_ID, None, &[], false);
    assert!(empty.items.is_empty() && empty.synced_at.is_none() && empty.error.is_none());
}

fn synced(ago_secs: i64) -> Manifest {
    Manifest {
        folder_id: FOLDER_ID.to_string(),
        synced_at: Some(Utc::now() - ChronoDuration::seconds(ago_secs)),
        ..Manifest::default()
    }
}

#[test]
fn a_config_change_resyncs_only_new_or_stale_folders() {
    let now = Utc::now();
    assert!(
        should_sync(SyncTrigger::ConfigChanged, None, now),
        "new folder"
    );
    assert!(!should_sync(
        SyncTrigger::ConfigChanged,
        Some(&synced(5)),
        now
    ));
    assert!(should_sync(
        SyncTrigger::ConfigChanged,
        Some(&synced(120)),
        now
    ));
    assert!(should_sync(SyncTrigger::Periodic, Some(&synced(5)), now));
    assert!(should_sync(SyncTrigger::Forced, Some(&synced(5)), now));
}

#[test]
fn an_unused_mirror_is_pruned_only_after_the_grace_period() {
    let now = Utc::now();
    assert!(!should_prune_unreferenced(Some(&synced(3600)), now));
    assert!(should_prune_unreferenced(Some(&synced(25 * 3600)), now));
    assert!(should_prune_unreferenced(None, now), "crash leftover");
    let failed_recently = Manifest {
        error_at: Some(now),
        ..Manifest::default()
    };
    assert!(!should_prune_unreferenced(Some(&failed_recently), now));
}

#[test]
fn repeated_failures_log_on_powers_of_two() {
    let logged: Vec<u32> = (0..=9).filter(|n| should_log_failure(*n)).collect();
    assert_eq!(logged, [1, 2, 4, 8]);
}
