//! #843: the PLAYBACK element — a Google Drive folder of photos and videos
//! played in a loop inside the element's frame.
//!
//! This module holds the pure pieces both sides share:
//! - [`parse_drive_folder_url`]: the operator pastes a Drive folder link; the
//!   server (validation + sync) and the editor (inline hint) read the folder id
//!   and the optional `resourcekey` from it the same way.
//! - the playlist DTOs the server answers on `GET /stream/api/playback` and the
//!   output page plays ([`PlaybackPlaylist`], [`PlaybackItem`]).
//! - the playback-specific validation, called from `validate_props`.
//!
//! The Drive access itself (listing, download, cache) is server-only
//! (`presenter-server/src/drive`, `.../playback`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{ContentTransition, Frame, StreamElementProps, StreamValidationError};

/// Seconds a photo stays on screen when the element does not say otherwise.
pub const STREAM_PLAYBACK_DEFAULT_IMAGE_SECONDS: u32 = 5;
/// Shortest photo duration, in seconds.
pub const STREAM_PLAYBACK_IMAGE_SECONDS_MIN: u32 = 1;
/// Longest photo duration (10 minutes) — anything above is a typo.
pub const STREAM_PLAYBACK_IMAGE_SECONDS_MAX: u32 = 600;

/// The only host a folder link may name.
const DRIVE_HOST: &str = "drive.google.com";
/// Drive ids are at least 25 characters of `[-\w]` (live-checked on #843).
const DRIVE_FOLDER_ID_MIN_LEN: usize = 25;
/// Upper bound for any Drive id or resource key — a sanity cap, real ids are
/// 28–44 characters.
const DRIVE_ID_MAX_LEN: usize = 128;

/// serde default of `Playback.image_seconds`.
pub(super) fn default_image_seconds() -> u32 {
    STREAM_PLAYBACK_DEFAULT_IMAGE_SECONDS
}

/// serde default of `Playback.muted` — a stream graphic is silent unless the
/// operator turns the sound on.
pub(super) fn default_muted() -> bool {
    true
}

/// A Drive folder named by a pasted link: its id plus the `resourcekey` an
/// older shared folder needs (without it the listing answers 401).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveFolder {
    pub id: String,
    pub resource_key: Option<String>,
}

/// Why a pasted text is not a usable Drive folder link.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DriveLinkError {
    #[error("the link is empty")]
    Empty,
    #[error("not a drive.google.com link")]
    NotDrive,
    #[error("the link points to a single file, not a folder")]
    FileLink,
    #[error("the link names no folder")]
    NoFolderId,
    #[error("the folder id is not a Drive id")]
    InvalidId,
    #[error("the resourcekey is malformed")]
    InvalidResourceKey,
}

/// `true` for a `[-\w]{min_len,128}` Drive id / resource key.
fn is_drive_token(token: &str, min_len: usize) -> bool {
    (min_len..=DRIVE_ID_MAX_LEN).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `true` for a Drive FILE id as the server addresses cached files with
/// (`/stream/playback/{folder_id}/{file_id}`): `[-\w]{1,128}`. Path segments
/// that fail this never reach the filesystem.
pub fn is_valid_drive_file_id(id: &str) -> bool {
    is_drive_token(id, 1)
}

/// `true` for a Drive FOLDER id: `[-\w]{25,128}`.
pub fn is_valid_drive_folder_id(id: &str) -> bool {
    is_drive_token(id, DRIVE_FOLDER_ID_MIN_LEN)
}

/// Read the folder id (and `resourcekey`) from a pasted Google Drive link.
///
/// Accepted (scheme optional, `http`/`https`, host `drive.google.com`):
/// - `/drive/folders/<ID>`, `/drive/u/<N>/folders/<ID>` and the older
///   `/drive/mobile/folders/<ID>`;
/// - `/open?id=<ID>`, `/folderview?id=<ID>`, `/embeddedfolderview?id=<ID>`.
///
/// `resourcekey` comes from the query string; any other query parameter
/// (`usp=sharing`, …) and a `#fragment` are ignored.
pub fn parse_drive_folder_url(url: &str) -> Result<DriveFolder, DriveLinkError> {
    let text = url.trim();
    if text.is_empty() {
        return Err(DriveLinkError::Empty);
    }
    let rest = strip_scheme(text);
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    if !is_drive_host(authority) {
        return Err(DriveLinkError::NotDrive);
    }
    let tail = tail.split('#').next().unwrap_or_default();
    let (path, query) = match tail.split_once('?') {
        Some((path, query)) => (path, query),
        None => (tail, ""),
    };
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let id = folder_id_from(&segments, query)?;
    if !is_valid_drive_folder_id(id) {
        return Err(DriveLinkError::InvalidId);
    }
    let resource_key = match query_param(query, "resourcekey") {
        None | Some("") => None,
        Some(key) if is_drive_token(key, 1) => Some(key.to_string()),
        Some(_) => return Err(DriveLinkError::InvalidResourceKey),
    };
    Ok(DriveFolder {
        id: id.to_string(),
        resource_key,
    })
}

/// The text after an optional `http://` / `https://` (case-insensitive).
fn strip_scheme(text: &str) -> &str {
    for scheme in ["https://", "http://"] {
        // `get` (not `[..]`): a pasted text may start with a multi-byte char.
        let matches = text
            .get(..scheme.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme));
        if matches {
            return text.get(scheme.len()..).unwrap_or_default();
        }
    }
    text
}

/// `true` when the authority is `drive.google.com` (an optional `:port`
/// allowed, credentials not).
fn is_drive_host(authority: &str) -> bool {
    if authority.contains('@') {
        return false;
    }
    let host = authority.split(':').next().unwrap_or_default();
    host.eq_ignore_ascii_case(DRIVE_HOST)
}

/// The folder id named by the path segments (or the `id` query parameter).
fn folder_id_from<'a>(segments: &[&'a str], query: &'a str) -> Result<&'a str, DriveLinkError> {
    match segments {
        ["drive", "folders", id, ..]
        | ["drive", "u", _, "folders", id, ..]
        | ["drive", "mobile", "folders", id, ..] => Ok(*id),
        ["open"] | ["folderview"] | ["embeddedfolderview"] => {
            query_param(query, "id").ok_or(DriveLinkError::NoFolderId)
        }
        ["file", ..] | ["drive", "u", _, "file", ..] => Err(DriveLinkError::FileLink),
        _ => Err(DriveLinkError::NoFolderId),
    }
}

/// The first value of query parameter `name` (raw — Drive ids and resource keys
/// never need percent-decoding).
fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// Validate the playback (#843) fields. An EMPTY link is allowed — a freshly
/// added element has none yet and plays nothing until the operator pastes one;
/// a non-empty link must name a Drive folder.
pub(super) fn validate_playback(props: &StreamElementProps) -> Result<(), StreamValidationError> {
    let StreamElementProps::Playback {
        source_url,
        frame,
        opacity,
        image_seconds,
        content_transition,
        ..
    } = props
    else {
        return Ok(());
    };
    if !source_url.trim().is_empty() {
        parse_drive_folder_url(source_url)
            .map_err(|reason| StreamValidationError::InvalidDriveLink { reason })?;
    }
    validate_shared(frame, *opacity, content_transition)?;
    if !(STREAM_PLAYBACK_IMAGE_SECONDS_MIN..=STREAM_PLAYBACK_IMAGE_SECONDS_MAX)
        .contains(image_seconds)
    {
        return Err(StreamValidationError::ImageSecondsOutOfRange {
            value: *image_seconds,
        });
    }
    Ok(())
}

/// The frame / opacity / transition checks every visual kind shares.
fn validate_shared(
    frame: &Frame,
    opacity: f32,
    transition: &ContentTransition,
) -> Result<(), StreamValidationError> {
    super::validate_frame(frame)?;
    super::validate_opacity(opacity)?;
    super::validate_transition(transition)
}

// ---------------------------------------------------------------------------
// Playlist DTOs — `GET /stream/api/playback?url=<link>`.
// ---------------------------------------------------------------------------

/// What a playlist item is, from the MIME type the Drive listing reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackItemKind {
    Image,
    Video,
}

/// One cached file of the folder, in Drive's listing order. `url` is the
/// server's id-addressed cache URL (`/stream/playback/{folder}/{file}?v=…`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackItem {
    pub file_id: String,
    pub kind: PlaybackItemKind,
    pub name: String,
    pub url: String,
}

/// The playlist of one Drive folder: the cached items, when they were last
/// synced, the last sync error (the last good items keep playing), and whether
/// a sync of this folder is queued or running right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPlaylist {
    pub folder_id: String,
    pub items: Vec<PlaybackItem>,
    #[serde(default)]
    pub synced_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub syncing: bool,
}

impl PlaybackPlaylist {
    /// `(photos, videos)` in the playlist — the editor's status line.
    pub fn counts(&self) -> (usize, usize) {
        let videos = self
            .items
            .iter()
            .filter(|item| item.kind == PlaybackItemKind::Video)
            .count();
        (self.items.len() - videos, videos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{validate_props, ImageFit};
    use serde_json::json;

    const ID: &str = "1AbCdEfGhIjKlMnOpQrStUvWxYz_-012";

    fn ok(url: &str) -> DriveFolder {
        parse_drive_folder_url(url).unwrap_or_else(|e| panic!("{url}: {e}"))
    }

    #[test]
    fn folder_paths_are_parsed() {
        for url in [
            format!("https://drive.google.com/drive/folders/{ID}"),
            format!("https://drive.google.com/drive/folders/{ID}?usp=sharing"),
            format!("https://drive.google.com/drive/u/1/folders/{ID}"),
            // Older mobile share links.
            format!("https://drive.google.com/drive/mobile/folders/{ID}?usp=sharing"),
            format!("http://DRIVE.google.com/drive/folders/{ID}/"),
            format!("drive.google.com/drive/folders/{ID}#grid"),
            format!("  https://drive.google.com/drive/folders/{ID}  "),
            format!("https://drive.google.com:443/drive/folders/{ID}"),
        ] {
            assert_eq!(
                ok(&url),
                DriveFolder {
                    id: ID.to_string(),
                    resource_key: None
                },
                "{url}"
            );
        }
    }

    #[test]
    fn id_query_paths_are_parsed() {
        for path in ["open", "folderview", "embeddedfolderview"] {
            let url = format!("https://drive.google.com/{path}?id={ID}");
            assert_eq!(ok(&url).id, ID, "{url}");
        }
        // `id` is found among other parameters, in any position.
        let url = format!("https://drive.google.com/open?usp=x&id={ID}&authuser=0");
        assert_eq!(ok(&url).id, ID);
    }

    #[test]
    fn resource_key_comes_from_the_query() {
        let url = format!("https://drive.google.com/drive/folders/{ID}?resourcekey=0-SynthKey_AAA");
        assert_eq!(ok(&url).resource_key.as_deref(), Some("0-SynthKey_AAA"));
        let url =
            format!("https://drive.google.com/embeddedfolderview?id={ID}&resourcekey=0-abc_DEF");
        assert_eq!(ok(&url).resource_key.as_deref(), Some("0-abc_DEF"));
        // An empty key is the same as none.
        let url = format!("https://drive.google.com/drive/folders/{ID}?resourcekey=");
        assert_eq!(ok(&url).resource_key, None);
    }

    #[test]
    fn junk_links_are_refused_with_the_reason() {
        let cases = [
            ("", DriveLinkError::Empty),
            ("   ", DriveLinkError::Empty),
            (
                "https://example.com/drive/folders/x",
                DriveLinkError::NotDrive,
            ),
            (
                "https://docs.google.com/drive/folders/x",
                DriveLinkError::NotDrive,
            ),
            (
                "https://user@drive.google.com/drive/folders/x",
                DriveLinkError::NotDrive,
            ),
            ("https://drive.google.com/", DriveLinkError::NoFolderId),
            ("https://drive.google.com/open", DriveLinkError::NoFolderId),
            (
                "https://drive.google.com/drive/my-drive",
                DriveLinkError::NoFolderId,
            ),
            (
                "https://drive.google.com/drive/folders/short",
                DriveLinkError::InvalidId,
            ),
            (
                "https://drive.google.com/drive/folders/1AbCdEfGhIjKlMnOpQrStUvWx%20z",
                DriveLinkError::InvalidId,
            ),
        ];
        for (url, want) in cases {
            assert_eq!(parse_drive_folder_url(url), Err(want), "{url:?}");
        }
        let file = format!("https://drive.google.com/file/d/{ID}/view?usp=sharing");
        assert_eq!(parse_drive_folder_url(&file), Err(DriveLinkError::FileLink));
        let bad_key = format!("https://drive.google.com/drive/folders/{ID}?resourcekey=a%2Fb");
        assert_eq!(
            parse_drive_folder_url(&bad_key),
            Err(DriveLinkError::InvalidResourceKey)
        );
    }

    #[test]
    fn id_validators() {
        assert!(is_valid_drive_folder_id(ID));
        assert!(!is_valid_drive_folder_id("short_id"));
        assert!(!is_valid_drive_folder_id(&"a".repeat(129)));
        assert!(is_valid_drive_file_id("x"));
        assert!(is_valid_drive_file_id("1SynthFile-id_AAAAAAAAAAAAAAAAAA"));
        assert!(!is_valid_drive_file_id(""));
        assert!(!is_valid_drive_file_id("../etc"));
        assert!(!is_valid_drive_file_id("a/b"));
        assert!(!is_valid_drive_file_id("a.mp4"));
    }

    fn frame() -> Frame {
        Frame {
            x_pct: 10.0,
            y_pct: 10.0,
            w_pct: 40.0,
            h_pct: 40.0,
        }
    }

    fn playback(url: &str) -> StreamElementProps {
        StreamElementProps::Playback {
            source_url: url.to_string(),
            fit: ImageFit::Cover,
            frame: frame(),
            opacity: 1.0,
            image_seconds: 7,
            muted: true,
            content_transition: ContentTransition::Fade { duration_ms: 400 },
        }
    }

    #[test]
    fn playback_round_trips_with_the_kind_tag() {
        let props = playback(&format!("https://drive.google.com/drive/folders/{ID}"));
        let value = serde_json::to_value(&props).expect("serialize");
        assert_eq!(value["kind"], json!("playback"));
        assert_eq!(value["image_seconds"], json!(7));
        assert_eq!(value["source_url"], json!(props_url(&props)));
        assert_eq!(value["frame"]["wPct"], json!(40.0));
        let parsed: StreamElementProps = serde_json::from_value(value).expect("deserialize");
        assert_eq!(parsed, props);
        assert_eq!(parsed.kind_str(), "playback");
        assert!(parsed.font_families().is_empty());
    }

    fn props_url(props: &StreamElementProps) -> String {
        match props {
            StreamElementProps::Playback { source_url, .. } => source_url.clone(),
            other => panic!("expected playback, got {other:?}"),
        }
    }

    #[test]
    fn omitted_seconds_muted_and_transition_take_their_defaults() {
        let wire = json!({
            "kind": "playback",
            "source_url": "",
            "fit": "contain",
            "frame": { "xPct": 0, "yPct": 0, "wPct": 100, "hPct": 100 },
            "opacity": 1.0
        });
        match serde_json::from_value::<StreamElementProps>(wire).expect("parse") {
            StreamElementProps::Playback {
                image_seconds,
                muted,
                content_transition,
                ..
            } => {
                assert_eq!(image_seconds, STREAM_PLAYBACK_DEFAULT_IMAGE_SECONDS);
                assert!(muted);
                assert_eq!(content_transition, ContentTransition::default());
            }
            other => panic!("expected playback, got {other:?}"),
        }
    }

    #[test]
    fn validation_accepts_an_empty_or_a_folder_link() {
        assert_eq!(validate_props(&playback(""), &[]), Ok(()));
        assert_eq!(validate_props(&playback("  "), &[]), Ok(()));
        let url = format!("https://drive.google.com/drive/folders/{ID}");
        assert_eq!(validate_props(&playback(&url), &[]), Ok(()));
    }

    #[test]
    fn validation_refuses_junk() {
        assert_eq!(
            validate_props(&playback("https://example.com/x"), &[]),
            Err(StreamValidationError::InvalidDriveLink {
                reason: DriveLinkError::NotDrive
            })
        );
        let url = format!("https://drive.google.com/drive/folders/{ID}");
        let with = |f: &dyn Fn(&mut StreamElementProps)| {
            let mut props = playback(&url);
            f(&mut props);
            validate_props(&props, &[])
        };
        let set_seconds = |value: u32| {
            move |p: &mut StreamElementProps| {
                if let StreamElementProps::Playback { image_seconds, .. } = p {
                    *image_seconds = value;
                }
            }
        };
        assert_eq!(
            with(&set_seconds(0)),
            Err(StreamValidationError::ImageSecondsOutOfRange { value: 0 })
        );
        assert_eq!(
            with(&set_seconds(601)),
            Err(StreamValidationError::ImageSecondsOutOfRange { value: 601 })
        );
        assert_eq!(with(&set_seconds(1)), Ok(()));
        assert_eq!(with(&set_seconds(600)), Ok(()));
        assert!(matches!(
            with(&|p: &mut StreamElementProps| {
                if let StreamElementProps::Playback { opacity, .. } = p {
                    *opacity = 1.5;
                }
            }),
            Err(StreamValidationError::OpacityOutOfRange { .. })
        ));
        assert!(matches!(
            with(&|p: &mut StreamElementProps| {
                if let StreamElementProps::Playback { frame, .. } = p {
                    frame.w_pct = 0.0;
                }
            }),
            Err(StreamValidationError::NonPositiveFrameSize { .. })
        ));
        assert!(matches!(
            with(&|p: &mut StreamElementProps| {
                if let StreamElementProps::Playback {
                    content_transition, ..
                } = p
                {
                    *content_transition = ContentTransition::FadeThrough {
                        duration_ms: 20_000,
                    };
                }
            }),
            Err(StreamValidationError::TransitionTooLong { .. })
        ));
    }

    #[test]
    fn playlist_counts_photos_and_videos() {
        let item = |kind| PlaybackItem {
            file_id: "f".to_string(),
            kind,
            name: "n".to_string(),
            url: "/u".to_string(),
        };
        let playlist = PlaybackPlaylist {
            folder_id: ID.to_string(),
            items: vec![
                item(PlaybackItemKind::Image),
                item(PlaybackItemKind::Video),
                item(PlaybackItemKind::Image),
            ],
            synced_at: None,
            error: None,
            syncing: false,
        };
        assert_eq!(playlist.counts(), (2, 1));
        let value = serde_json::to_value(&playlist).expect("serialize");
        assert_eq!(value["folderId"], json!(ID));
        assert_eq!(value["items"][1]["kind"], json!("video"));
        assert_eq!(value["items"][0]["fileId"], json!("f"));
        assert_eq!(value["syncedAt"], json!(null));
        assert_eq!(value["syncing"], json!(false));
    }
}
