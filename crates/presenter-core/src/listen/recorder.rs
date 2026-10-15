//! The recorder's contract (#852 C1b, spec §5.5, §12): its `app_settings` row,
//! the toggle's request, its part of `GET /listen/status`, and the file layout
//! `<root>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav` with its 30-day retention.
//!
//! The toggle is a per-DAY switch: switching it on stores the server's local
//! date (`enabledFor`); the recorder records while that date is today, so it
//! goes off by itself at midnight and a restart the same day resumes it. Every
//! file is one continuous stretch of a mic's audio, named by the local time of
//! its first frame — the bake-off places the show-log times by it (spec §13).

use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::listen::mics::is_valid_mic_id;

/// The `app_settings` key of the toggle.
pub const LISTEN_RECORDER_KEY: &str = "listen.recorder";
/// Day folders older than this many days are deleted (spec §5.5).
pub const RECORDING_RETENTION_DAYS: i64 = 30;
/// The most files one mic can start within one second (`_HHMMSS`, `-2` … `-9`).
pub const RECORDING_NAMES_PER_SECOND: u32 = 9;
const RECORDING_EXTENSION: &str = ".wav";

/// The stored `listen.recorder` row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderSetting {
    /// The local date the recorder was switched on for; `None` = off.
    #[serde(default)]
    pub enabled_for: Option<NaiveDate>,
}

impl RecorderSetting {
    /// Whether the recorder records on `today`.
    pub fn records_on(&self, today: NaiveDate) -> bool {
        self.enabled_for == Some(today)
    }
}

/// `PUT /listen/recorder`: on = today, off = not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecorderToggle {
    pub enabled: bool,
}

/// One mic's recorder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingState {
    /// Not recording: switched off, the day is over, or never on.
    #[default]
    Off,
    /// Switched on, no audio from the mic yet: a file opens with its first frame.
    Waiting,
    Recording,
    /// A write failed; the writer is started again after a pause.
    Error,
}

/// One mic in the recorder's status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicRecording {
    pub id: String,
    pub state: RecordingState,
    /// The file being written, or the last one.
    pub file: Option<String>,
    /// Audio bytes in that file (16 kHz mono s16: 32 000 per second).
    pub bytes: u64,
    /// Frames lost because the writer fell 12.8 s behind (since the server started).
    pub dropped_frames: u64,
    /// Slovak, for the operator.
    pub error: Option<String>,
}

impl MicRecording {
    /// A mic no writer ran for.
    pub fn off(id: &str) -> Self {
        Self {
            id: id.to_string(),
            state: RecordingState::Off,
            file: None,
            bytes: 0,
            dropped_frames: 0,
            error: None,
        }
    }
}

/// The recorder's part of `GET /listen/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    /// Recording today.
    pub enabled: bool,
    /// The day it was switched on for (possibly over).
    pub enabled_for: Option<NaiveDate>,
    /// Where the day folders go.
    pub root: String,
    /// Every configured mic.
    pub mics: Vec<MicRecording>,
}

/// A day folder's name: `YYYY-MM-DD`.
pub fn recording_day_dir(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

/// The day of a folder the recorder made; `None` for any other name (only the
/// canonical `YYYY-MM-DD` spelling counts).
pub fn parse_recording_day(name: &str) -> Option<NaiveDate> {
    let day = NaiveDate::parse_from_str(name, "%Y-%m-%d").ok()?;
    (recording_day_dir(day) == name).then_some(day)
}

/// A day folder is deleted once it is more than [`RECORDING_RETENTION_DAYS`] old.
pub fn recording_expired(day: NaiveDate, today: NaiveDate) -> bool {
    today.signed_duration_since(day).num_days() > RECORDING_RETENTION_DAYS
}

/// The file of a stretch of `mic_id`'s audio whose first frame came at
/// `started`; `attempt` 2 … 9 names a further stretch started in the same second.
pub fn recording_file_name(mic_id: &str, started: NaiveTime, attempt: u32) -> String {
    let time = started.format("%H%M%S");
    match attempt {
        0 | 1 => format!("{mic_id}_{time}{RECORDING_EXTENSION}"),
        n => format!("{mic_id}_{time}-{n}{RECORDING_EXTENSION}"),
    }
}

/// Whether `name` is a file the recorder writes: `<mic id>_<HHMMSS>[-2…9].wav`
/// (a mic id never holds `_`). Retention deletes nothing else.
pub fn is_recording_file_name(name: &str) -> bool {
    let Some((mic_id, stamp)) = name
        .strip_suffix(RECORDING_EXTENSION)
        .and_then(|stem| stem.rsplit_once('_'))
    else {
        return false;
    };
    let (time, attempt) = match stamp.split_once('-') {
        Some((time, attempt)) => (time, Some(attempt)),
        None => (stamp, None),
    };
    let attempt_ok = matches!(
        attempt,
        None | Some("2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
    );
    let time_ok = time.len() == 6
        && time.bytes().all(|b| b.is_ascii_digit())
        && NaiveTime::parse_from_str(time, "%H%M%S").is_ok();
    attempt_ok && time_ok && is_valid_mic_id(mic_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn time(text: &str) -> NaiveTime {
        text.parse().unwrap()
    }

    #[test]
    fn the_setting_is_off_by_default_and_on_only_for_its_day() {
        assert_eq!(RecorderSetting::default().enabled_for, None);
        let empty: RecorderSetting = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, RecorderSetting::default(), "an empty row = off");
        let on = RecorderSetting {
            enabled_for: Some(day("2026-10-18")),
        };
        assert_eq!(
            serde_json::to_value(on).unwrap(),
            json!({"enabledFor": "2026-10-18"})
        );
        assert!(on.records_on(day("2026-10-18")));
        assert!(
            !on.records_on(day("2026-10-19")),
            "off by itself after midnight"
        );
        assert!(!RecorderSetting::default().records_on(day("2026-10-18")));
        let toggle: RecorderToggle = serde_json::from_value(json!({"enabled": true})).unwrap();
        assert!(toggle.enabled);
    }

    #[test]
    fn the_status_wire_shape_is_exact() {
        let status = RecorderStatus {
            enabled: true,
            enabled_for: Some(day("2026-10-18")),
            root: "/opt/presenter/recordings".into(),
            mics: vec![MicRecording {
                id: "guest".into(),
                state: RecordingState::Recording,
                file: Some("/opt/presenter/recordings/2026-10-18/guest_194205.wav".into()),
                bytes: 32_000,
                dropped_frames: 0,
                error: None,
            }],
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            json!({
                "enabled": true, "enabledFor": "2026-10-18", "root": "/opt/presenter/recordings",
                "mics": [{"id": "guest", "state": "recording",
                    "file": "/opt/presenter/recordings/2026-10-18/guest_194205.wav",
                    "bytes": 32000, "droppedFrames": 0, "error": null}]
            })
        );
        for (state, wire) in [
            (RecordingState::Off, "off"),
            (RecordingState::Waiting, "waiting"),
            (RecordingState::Error, "error"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), json!(wire));
        }
        assert_eq!(RecordingState::default(), RecordingState::Off);
    }

    #[test]
    fn only_a_canonical_date_names_a_day_folder() {
        assert_eq!(recording_day_dir(day("2026-01-05")), "2026-01-05");
        assert_eq!(parse_recording_day("2026-10-18"), Some(day("2026-10-18")));
        for name in [
            "2026-1-5",
            "2026-02-30",
            "recordings",
            "2026-10-18x",
            "",
            "+2026-10-18",
            "2026-10-18 ",
        ] {
            assert_eq!(parse_recording_day(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_day_is_kept_30_days_and_deleted_on_the_31st() {
        let today = day("2026-10-15");
        assert!(
            !recording_expired(day("2026-09-15"), today),
            "30 days old: kept"
        );
        assert!(
            recording_expired(day("2026-09-14"), today),
            "31 days old: deleted"
        );
        assert!(!recording_expired(today, today));
        assert!(
            !recording_expired(day("2026-10-16"), today),
            "a folder dated in the future is never deleted"
        );
    }

    #[test]
    fn a_files_name_carries_its_mic_and_start_time() {
        assert_eq!(
            recording_file_name("guest", time("19:42:05"), 1),
            "guest_194205.wav"
        );
        assert_eq!(
            recording_file_name("host-2", time("07:05:09"), 3),
            "host-2_070509-3.wav"
        );
        for attempt in 1..=RECORDING_NAMES_PER_SECOND {
            let name = recording_file_name("guest", time("19:42:05"), attempt);
            assert!(is_recording_file_name(&name), "{name}");
        }
    }

    #[test]
    fn retention_recognises_only_the_recorders_own_file_names() {
        for name in [
            "guest.wav",
            "Guest_194205.wav",
            "../guest_194205.wav",
            "guest_194205.wav.tmp",
            "guest_194205-1.wav",
            "guest_194205-10.wav",
            "guest_246000.wav",
            "guest_19420.wav",
            "_194205.wav",
            "guest_194205.WAV",
            "notes.txt",
        ] {
            assert!(!is_recording_file_name(name), "{name:?}");
        }
    }
}
