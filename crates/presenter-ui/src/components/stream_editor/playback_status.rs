//! Pure texts of the playback element form (#843): the inline hint for a
//! pasted link and the sync status line. Host-tested; Slovak like the rest of
//! the editor.

use chrono::{DateTime, Utc};
use presenter_core::{parse_drive_folder_url, DriveLinkError, PlaybackPlaylist};

/// Why a non-empty pasted link cannot be used, in plain Slovak; `None` for an
/// empty field (nothing to complain about yet) or a usable folder link.
pub fn link_problem(url: &str) -> Option<&'static str> {
    if url.trim().is_empty() {
        return None;
    }
    let err = parse_drive_folder_url(url).err()?;
    Some(match err {
        DriveLinkError::Empty => return None,
        DriveLinkError::NotDrive => "Toto nie je odkaz na Google Drive (drive.google.com).",
        DriveLinkError::FileLink => "Toto je odkaz na jeden súbor — vlož odkaz na celý priečinok.",
        DriveLinkError::NoFolderId | DriveLinkError::InvalidId => {
            "V odkaze chýba priečinok — skopíruj ho cez Zdieľať › Kopírovať odkaz."
        }
        DriveLinkError::InvalidResourceKey => "Odkaz má poškodený resourcekey — skopíruj ho znova.",
    })
}

/// What the status line shows (`state` is the `data-state` E2E hook).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub state: &'static str,
    pub text: String,
}

/// `N fotiek / M videí` with Slovak plural forms (1 / 2–4 / other).
pub fn counts_text(photos: usize, videos: usize) -> String {
    format!(
        "{} / {}",
        plural(photos, "fotka", "fotky", "fotiek"),
        plural(videos, "video", "videá", "videí")
    )
}

fn plural(n: usize, one: &str, few: &str, many: &str) -> String {
    let word = match n {
        1 => one,
        2..=4 => few,
        _ => many,
    };
    format!("{n} {word}")
}

/// The status line for a valid link. `playlist` is `None` while the first
/// fetch is in flight; `time` formats the sync time (local `HH:MM` in the
/// browser, fixed in tests).
pub fn status_line(
    playlist: Option<&PlaybackPlaylist>,
    time: impl Fn(DateTime<Utc>) -> String,
) -> StatusLine {
    let Some(playlist) = playlist else {
        return StatusLine {
            state: "loading",
            text: "Načítavam stav…".to_string(),
        };
    };
    let (photos, videos) = playlist.counts();
    let counts = counts_text(photos, videos);
    if playlist.syncing {
        let text = if playlist.items.is_empty() {
            "Synchronizujem priečinok…".to_string()
        } else {
            format!("Synchronizujem priečinok… (zatiaľ {counts})")
        };
        return StatusLine {
            state: "syncing",
            text,
        };
    }
    if let Some(error) = &playlist.error {
        let text = if playlist.items.is_empty() {
            format!("Chyba: {error}")
        } else {
            format!("Chyba: {error} — hrá posledný stav: {counts}")
        };
        return StatusLine {
            state: "error",
            text,
        };
    }
    match playlist.synced_at {
        Some(at) => StatusLine {
            state: "synced",
            text: format!("{counts}, synchronizované {}", time(at)),
        },
        None => StatusLine {
            state: "unsynced",
            text: "Zatiaľ nesynchronizované — ulož prvok alebo klikni Obnoviť.".to_string(),
        },
    }
}

/// `true` while the status is still expected to change on its own (a sync
/// queued/running, or a saved link the worker has not reached yet) — the
/// form polls the status meanwhile.
pub fn should_poll(playlist: &PlaybackPlaylist) -> bool {
    playlist.syncing || (playlist.synced_at.is_none() && playlist.error.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::{PlaybackItem, PlaybackItemKind};

    const LINK: &str = "https://drive.google.com/drive/folders/1AbCdEfGhIjKlMnOpQrStUvWxYz012";

    fn playlist(photos: usize, videos: usize) -> PlaybackPlaylist {
        let item = |kind| PlaybackItem {
            file_id: "f".to_string(),
            kind,
            name: "n".to_string(),
            url: "/u".to_string(),
        };
        let mut items: Vec<PlaybackItem> =
            (0..photos).map(|_| item(PlaybackItemKind::Image)).collect();
        items.extend((0..videos).map(|_| item(PlaybackItemKind::Video)));
        PlaybackPlaylist {
            folder_id: "F".to_string(),
            items,
            synced_at: None,
            error: None,
            syncing: false,
        }
    }

    fn fixed(_: DateTime<Utc>) -> String {
        "10:15".to_string()
    }

    #[test]
    fn link_problems_are_explained() {
        assert_eq!(link_problem(""), None);
        assert_eq!(link_problem("   "), None);
        assert_eq!(link_problem(LINK), None);
        assert!(link_problem("https://example.com/x").is_some_and(|m| m.contains("Google Drive")));
        assert!(link_problem(
            "https://drive.google.com/file/d/1AbCdEfGhIjKlMnOpQrStUvWxYz012/view"
        )
        .is_some_and(|m| m.contains("jeden súbor")));
        assert!(link_problem("https://drive.google.com/drive/my-drive")
            .is_some_and(|m| m.contains("chýba priečinok")));
    }

    #[test]
    fn counts_use_slovak_plurals() {
        assert_eq!(counts_text(0, 0), "0 fotiek / 0 videí");
        assert_eq!(counts_text(1, 1), "1 fotka / 1 video");
        assert_eq!(counts_text(3, 2), "3 fotky / 2 videá");
        assert_eq!(counts_text(12, 5), "12 fotiek / 5 videí");
    }

    #[test]
    fn the_status_line_follows_the_sync_state() {
        assert_eq!(status_line(None, fixed).state, "loading");

        let mut synced = playlist(2, 1);
        synced.synced_at = Some(Utc::now());
        assert_eq!(
            status_line(Some(&synced), fixed),
            StatusLine {
                state: "synced",
                text: "2 fotky / 1 video, synchronizované 10:15".to_string(),
            }
        );

        let mut syncing = playlist(0, 0);
        syncing.syncing = true;
        assert_eq!(status_line(Some(&syncing), fixed).state, "syncing");

        let mut failed = synced.clone();
        failed.error = Some("priečinok nie je verejný".to_string());
        let line = status_line(Some(&failed), fixed);
        assert_eq!(line.state, "error");
        assert!(line.text.contains("nie je verejný") && line.text.contains("2 fotky"));

        assert_eq!(status_line(Some(&playlist(0, 0)), fixed).state, "unsynced");
    }

    #[test]
    fn the_form_polls_until_the_sync_settled() {
        let mut list = playlist(0, 0);
        assert!(should_poll(&list), "saved, not reached yet");
        list.syncing = true;
        assert!(should_poll(&list));
        list.syncing = false;
        list.synced_at = Some(Utc::now());
        assert!(!should_poll(&list));
        list.synced_at = None;
        list.error = Some("x".to_string());
        assert!(!should_poll(&list));
    }
}
