//! #852 C1b: the recordings folder, the retention pass, the toggle's status and
//! the writers (driven through the real supervisor, frames sent into the mic's
//! channel or, end to end, real VBAN over loopback UDP). Folders are temp
//! dirs; the clock is a settable local time.

use super::*;
use crate::listen::health::ListenHealth;
use crate::tracing_capture::{quoted, Capture};
use presenter_core::audio::level::rms_dbfs;
use presenter_core::audio::vban::VbanSampleType;
use presenter_core::audio::vban_encode::{tone, VbanEncoder};
use presenter_core::audio::wav::wav_header;
use presenter_core::listen::mics::{MicChannel, MicLanguage, MicRole};
use presenter_core::listen::recorder::{MicRecording, RecordingState};
use presenter_core::listen::status::{ListenStatus, SocketState};
use std::path::Path;
use std::sync::Mutex;

fn at(text: &str) -> NaiveDateTime {
    text.parse().unwrap()
}

/// A settable local wall time and the handle that moves it.
fn clock(start: &str) -> (Clock, Arc<Mutex<NaiveDateTime>>) {
    let now = Arc::new(Mutex::new(at(start)));
    let read = Arc::clone(&now);
    let clock: Clock = Arc::new(move || *read.lock().unwrap());
    (clock, now)
}

/// `root/<folder>/<file>` with `len` bytes.
fn put(root: &Path, folder: &str, file: &str, len: usize) -> PathBuf {
    let dir = root.join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(file);
    std::fs::write(&path, vec![0u8; len]).unwrap();
    path
}

#[test]
fn the_recordings_folder_is_the_env_value_or_next_to_the_database() {
    assert_eq!(RECORDINGS_DIR_ENV, "PRESENTER_LISTEN_RECORDINGS_DIR");
    let prod = "sqlite:///opt/presenter/presenter.db";
    assert_eq!(
        recordings_root(Some("/srv/rec".into()), prod),
        PathBuf::from("/srv/rec")
    );
    assert_eq!(
        recordings_root(Some(OsString::new()), prod),
        PathBuf::from("/opt/presenter/recordings"),
        "an empty override is no override"
    );
    assert_eq!(
        recordings_root(None, "sqlite:///opt/presenter-dev/presenter.db?mode=rwc"),
        PathBuf::from("/opt/presenter-dev/recordings")
    );
    assert_eq!(
        recordings_root(None, "sqlite::memory:"),
        PathBuf::from("./recordings")
    );
    assert_eq!(recordings_root(None, ""), PathBuf::from("./recordings"));
    assert!(
        RecorderEnv::from_env()
            .root
            .starts_with(std::env::temp_dir()),
        "a unit test's default folder is never inside the crate"
    );
}

#[tokio::test]
async fn retention_runs_once_per_local_date() {
    let root = tempfile::tempdir().unwrap();
    let (clock, now) = clock("2026-10-15T10:00:00");
    let env = RecorderEnv {
        root: root.path().to_path_buf(),
        clock,
        first_pass_delay: Duration::ZERO,
    };
    let expired = put(root.path(), "2026-09-14", "guest_194205.wav", 10);
    let mut pruned_on = None;
    prune_if_new_day(&env, &mut pruned_on).await;
    assert!(!expired.exists(), "the first pass of the day deletes it");
    let later_today = put(root.path(), "2026-09-01", "guest_194205.wav", 10);
    let thirtieth = put(root.path(), "2026-09-15", "guest_194205.wav", 10);
    prune_if_new_day(&env, &mut pruned_on).await;
    assert!(later_today.exists(), "one pass per local date");
    *now.lock().unwrap() = at("2026-10-16T00:00:30");
    prune_if_new_day(&env, &mut pruned_on).await;
    assert!(
        !later_today.exists() && !thirtieth.exists(),
        "the next day's pass"
    );
}

#[tokio::test]
async fn a_started_service_prunes_at_once_and_logs_each_file() {
    let capture = Capture::install();
    let root = tempfile::tempdir().unwrap();
    let (clock, _) = clock("2026-10-15T10:00:00");
    let expired = put(root.path(), "2026-09-01", "guest_194205.wav", 1_234);
    let young = put(root.path(), "2026-10-14", "guest_194205.wav", 10);
    let service = ListenService::new();
    service.set_recorder_env(RecorderEnv {
        root: root.path().to_path_buf(),
        clock,
        first_pass_delay: Duration::ZERO,
    });
    assert!(service.start("127.0.0.1:0".parse().unwrap()));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while expired.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no retention pass at startup"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(young.exists());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let lines = loop {
        let lines = capture.events_with_message("recording deleted (retention)");
        if !lines.is_empty() || tokio::time::Instant::now() >= deadline {
            break lines;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].field("bytes"), Some("1234"));
}

#[test]
fn a_failure_listed_against_the_folder_itself_does_not_claim_an_expired_day_folder() {
    let capture = Capture::install();
    let root = PathBuf::from("/srv/rec");
    let report = PruneReport {
        failed: vec![
            (root.clone(), "readdir failed".to_string()),
            (root.join("2026-08-01"), "Permission denied".to_string()),
        ],
        ..PruneReport::default()
    };
    log_prune(&report, &root);
    let listing = capture.events_with_message("an entry of the recordings folder");
    assert_eq!(listing.len(), 1, "{listing:?}");
    assert_eq!(listing[0].level, tracing::Level::WARN);
    assert_eq!(listing[0].field("dir"), Some("/srv/rec"));
    assert_eq!(
        listing[0].field("error"),
        Some(quoted("readdir failed").as_str())
    );
    let expired = capture.events_with_message("an expired recordings folder");
    assert_eq!(expired.len(), 1, "{expired:?}");
    assert_eq!(expired[0].level, tracing::Level::WARN);
    assert_eq!(expired[0].field("dir"), Some("/srv/rec/2026-08-01"));
}

#[tokio::test(start_paused = true)]
async fn a_started_supervisor_prunes_nothing_until_the_first_pass_delay_is_over() {
    let root = tempfile::tempdir().unwrap();
    let (clock, _) = clock("2026-10-15T10:00:00");
    let expired = put(root.path(), "2026-09-01", "guest_194205.wav", 10);
    let young = put(root.path(), "2026-10-14", "guest_194205.wav", 10);
    let service = ListenService::new();
    service.set_recorder_env(RecorderEnv {
        root: root.path().to_path_buf(),
        clock,
        first_pass_delay: FIRST_PASS_DELAY,
    });
    assert!(service.start("127.0.0.1:0".parse().unwrap()));
    // The paused clock moves only while every task is idle, and not at all while
    // a `spawn_blocking` task runs (tokio test-util): a pass that started early
    // would have deleted the file before this sleep returns.
    tokio::time::sleep(FIRST_PASS_DELAY - Duration::from_secs(1)).await;
    assert!(expired.exists(), "no pass before the delay is over");
    tokio::time::sleep(Duration::from_secs(2)).await;
    for _ in 0..100 {
        if !expired.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !expired.exists(),
        "the first pass runs once the delay is over"
    );
    assert!(young.exists(), "only the expired folder goes");
}

#[tokio::test(start_paused = true)]
async fn the_first_pass_waits_for_the_delay_then_ticks_every_minute() {
    assert_eq!(
        FIRST_PASS_DELAY,
        Duration::from_secs(600),
        "a booting box gets ten minutes to set its clock"
    );
    let mut tick = retention_tick(FIRST_PASS_DELAY);
    let just_before = FIRST_PASS_DELAY - Duration::from_secs(1);
    assert!(
        tokio::time::timeout(just_before, tick.tick())
            .await
            .is_err(),
        "no pass before the delay is over"
    );
    tokio::time::timeout(Duration::from_secs(2), tick.tick())
        .await
        .unwrap();
    let almost_a_minute = RECONCILE_EVERY - Duration::from_secs(1);
    assert!(
        tokio::time::timeout(almost_a_minute, tick.tick())
            .await
            .is_err(),
        "then one tick a minute"
    );
    tokio::time::timeout(Duration::from_secs(2), tick.tick())
        .await
        .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_folder_that_cannot_be_cleaned_warns_and_a_folder_with_foreign_files_is_kept() {
    use std::os::unix::fs::PermissionsExt;

    /// Gives the folder its write permission back, so the temp dir can be
    /// removed even when an assertion fails.
    struct Writable(PathBuf);
    impl Drop for Writable {
        fn drop(&mut self) {
            let _restored =
                std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    let capture = Capture::install();
    let root = tempfile::tempdir().unwrap();
    let (clock, _) = clock("2026-10-15T10:00:00");
    let env = RecorderEnv {
        root: root.path().to_path_buf(),
        clock,
        first_pass_delay: Duration::ZERO,
    };
    let stuck = put(root.path(), "2026-08-01", "guest_194205.wav", 10);
    let stuck_dir = root.path().join("2026-08-01");
    // read-only: the folder lists, its recording cannot be removed
    std::fs::set_permissions(&stuck_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let _writable = Writable(stuck_dir.clone());
    let foreign = put(root.path(), "2026-08-02", "notes.txt", 5);
    let recording = put(root.path(), "2026-08-02", "guest_100000.wav", 7);
    let mut pruned_on = None;
    prune_if_new_day(&env, &mut pruned_on).await;
    assert!(
        stuck.exists() && foreign.exists(),
        "nothing foreign is lost"
    );
    assert!(
        !recording.exists(),
        "the pass went on past the failed folder"
    );
    let failed = capture.events_with_message("could not be read or cleaned");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].level, tracing::Level::WARN);
    assert_eq!(
        failed[0].field("dir"),
        Some(stuck_dir.display().to_string().as_str())
    );
    assert!(
        failed[0]
            .field("error")
            .is_some_and(|error| error.contains("ermission denied")),
        "{failed:?}"
    );
    let kept = capture.events_with_message("expired recordings folder kept");
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].level, tracing::Level::INFO);
    assert_eq!(
        kept[0].field("dir"),
        Some(
            root.path()
                .join("2026-08-02")
                .display()
                .to_string()
                .as_str()
        )
    );
    let deleted = capture.events_with_message("recording deleted (retention)");
    assert_eq!(deleted.len(), 1, "{deleted:?}");
}

#[tokio::test(start_paused = true)]
async fn without_a_delay_the_first_tick_is_immediate() {
    let mut tick = retention_tick(Duration::ZERO);
    tokio::time::timeout(Duration::from_millis(1), tick.tick())
        .await
        .unwrap();
}

#[test]
fn the_status_says_whether_the_recorder_records_today() {
    let service = ListenService::new();
    let (clock, now) = clock("2026-10-18T19:42:05");
    service.set_recorder_env(RecorderEnv {
        root: PathBuf::from("/srv/recordings"),
        clock,
        first_pass_delay: Duration::ZERO,
    });
    let off = service.status().recorder;
    assert_eq!((off.enabled, off.enabled_for), (false, None));
    assert_eq!(off.root, "/srv/recordings");
    let day: NaiveDate = "2026-10-18".parse().unwrap();
    service.apply_recorder(Some(day));
    let on = service.status().recorder;
    assert_eq!((on.enabled, on.enabled_for), (true, Some(day)));
    *now.lock().unwrap() = at("2026-10-19T00:00:01");
    assert!(!service.status().recorder.enabled, "the day is over");
    assert_eq!(service.today(), "2026-10-19".parse::<NaiveDate>().unwrap());
}

// ---- the writers (C1b-4) ------------------------------------------------------

const DAY: &str = "2026-10-18";

fn day(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

fn mic(id: &str, stream: &str) -> MicConfig {
    MicConfig {
        id: id.to_string(),
        name: id.to_string(),
        stream_name: stream.to_string(),
        sender_ip: None,
        channel: MicChannel::Left,
        language: MicLanguage::En,
        role: MicRole::Guest,
    }
}

fn recording(status: &ListenStatus, id: &str) -> MicRecording {
    status
        .recorder
        .mics
        .iter()
        .find(|m| m.id == id)
        .cloned()
        .unwrap()
}

/// Poll `service.status()` until `done` holds; fails with the last status after 5 s.
async fn wait_for(
    service: &ListenService,
    what: &str,
    done: impl Fn(&ListenStatus) -> bool,
) -> ListenStatus {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let status = service.status();
        if done(&status) {
            return status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}: {status:#?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Poll `done` for at most 5 s.
async fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn frame(value: i16) -> MicFrame {
    Arc::from(vec![value; 1_600])
}

fn send(service: &ListenService, mic_id: &str, value: i16) {
    service.frame_sender(mic_id).send(frame(value)).unwrap();
}

fn day_file(root: &Path, name: &str) -> PathBuf {
    root.join(DAY).join(name)
}

/// The names of the files in the day folder, sorted.
fn day_files(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(DAY))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// A file's header and its samples.
fn read_wav(path: &Path) -> (Vec<u8>, Vec<i16>) {
    let bytes = std::fs::read(path).unwrap();
    let samples = bytes[44..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    (bytes[..44].to_vec(), samples)
}

/// A service recording `mics` into `root` on `start`'s day, its supervisor
/// running (no socket), every writer subscribed; and the clock's handle.
async fn recording_service(
    root: &Path,
    start: &str,
    mics: Vec<MicConfig>,
) -> (ListenService, Arc<Mutex<NaiveDateTime>>) {
    let (clock, now) = clock(start);
    let service = ListenService::new();
    service.set_recorder_env(RecorderEnv {
        root: root.to_path_buf(),
        clock,
        first_pass_delay: Duration::ZERO,
    });
    let ids: Vec<String> = mics.iter().map(|m| m.id.clone()).collect();
    service.apply(mics);
    service.apply_recorder(Some(at(start).date()));
    tokio::spawn(supervise(service.clone()));
    for id in &ids {
        wait_for(&service, "a writer per mic", |s| {
            recording(s, id).state == RecordingState::Waiting
        })
        .await;
    }
    (service, now)
}

/// 75 packets of a 1 kHz half-scale tone = 4 frames of 16 kHz (C1a-3: the 4th
/// completes with the 75th packet).
async fn send_tone(ready: &ListenStatus) {
    let vban: std::net::SocketAddr = ready.socket.local_addr.as_deref().unwrap().parse().unwrap();
    let tx = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut encoder = VbanEncoder::new("GUEST_EN", 48_000, 1, VbanSampleType::Int16);
    for (k, chunk) in tone(48_000, 1_000.0, 0.5, 0, 75 * 256, 1)
        .chunks(256)
        .enumerate()
    {
        tx.send_to(&encoder.packet(chunk).unwrap(), vban)
            .await
            .unwrap();
        if k % 4 == 3 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

#[test]
fn the_recorder_status_lists_every_configured_mic_and_keeps_the_last_file() {
    let mics = vec![mic("guest", "GUEST_EN"), mic("interp", "INTERP_SK")];
    let mut health = ListenHealth::default();
    let file = "/r/2026-10-18/guest_194205.wav";
    health.recorder.writer_started("guest");
    health.recorder.wrote("guest", file, 6_400);
    assert_eq!(
        health.recorder.dropped("guest", 22),
        (1, 22),
        "the first lag event"
    );
    assert_eq!(
        health.recorder.failed("interp", "zápis nahrávky x zlyhal"),
        1
    );
    assert_eq!(
        health.recorder.failed("interp", "zápis nahrávky x zlyhal"),
        2
    );
    health.recorder.writer_started("interp"); // a restart keeps the error on show
    let status = health.snapshot(&mics, Instant::now()).recorder;
    assert_eq!(
        status.mics[0],
        MicRecording {
            id: "guest".into(),
            state: RecordingState::Recording,
            file: Some(file.into()),
            bytes: 6_400,
            dropped_frames: 22,
            error: None,
        }
    );
    assert_eq!(status.mics[1].state, RecordingState::Error);
    health.end_session(); // the socket closed: the recorder's status stays
    health.recorder.all_stopped(); // switched off: every mic off, errors forgotten
    let status = health.snapshot(&mics, Instant::now()).recorder;
    assert_eq!(
        (
            status.mics[0].state,
            status.mics[0].file.as_deref(),
            status.mics[0].bytes
        ),
        (RecordingState::Off, Some(file), 6_400),
        "the last file stays on show"
    );
    assert_eq!(
        (status.mics[1].state, status.mics[1].error.clone()),
        (RecordingState::Off, None)
    );
    health.recorder.retain(&["interp"]);
    let status = health.snapshot(&mics, Instant::now()).recorder;
    assert_eq!(
        status.mics[0],
        MicRecording::off("guest"),
        "a removed mic's entry goes"
    );
}

/// The unit's integration test: real VBAN over loopback UDP → the chain → the
/// frame channel → the writer → a WAV any reader opens.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_vban_lands_in_todays_wav_exactly_as_the_chain_made_it() {
    let root = tempfile::tempdir().unwrap();
    let (clock, _) = clock("2026-10-18T19:42:05");
    let service = ListenService::new();
    service.set_recorder_env(RecorderEnv {
        root: root.path().to_path_buf(),
        clock,
        first_pass_delay: Duration::ZERO,
    });
    service.apply(vec![mic("guest", "GUEST_EN")]);
    service.apply_recorder(Some(day(DAY)));
    assert!(service.start("127.0.0.1:0".parse().unwrap()));
    let ready = wait_for(&service, "bound and waiting", |s| {
        s.socket.state == SocketState::Bound
            && recording(s, "guest").state == RecordingState::Waiting
    })
    .await;
    send_tone(&ready).await;
    let status = wait_for(&service, "4 frames recorded", |s| {
        recording(s, "guest").bytes == 4 * 3_200
    })
    .await;
    // the clock stands at 19:42:05; the 4 frames began at most 0.4 s before it
    let path = day_file(root.path(), "guest_194204.wav");
    assert_eq!(day_files(root.path()), vec!["guest_194204.wav"]);
    let rec = recording(&status, "guest");
    assert_eq!(rec.state, RecordingState::Recording);
    assert_eq!(
        rec.file.as_deref(),
        Some(path.display().to_string().as_str())
    );
    assert!(status.recorder.enabled);
    let (header, samples) = read_wav(&path);
    assert_eq!(
        header,
        wav_header(12_800).to_vec(),
        "the header covers the audio before any close"
    );
    assert_eq!(samples.len(), 6_400);
    let level = rms_dbfs(&samples[4_800..]);
    assert!(
        (-9.6..=-8.5).contains(&level),
        "the 4th frame of a 1 kHz half-scale tone ≈ −9.03 dBFS: {level}"
    );
    assert_eq!(
        status.mics[0].frames_out, 4,
        "every frame the chain made is in the file"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_recorder_is_off_by_default_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (clock, _) = clock("2026-10-18T19:42:05");
    let service = ListenService::new();
    service.set_recorder_env(RecorderEnv {
        root: root.path().to_path_buf(),
        clock,
        first_pass_delay: Duration::ZERO,
    });
    service.apply(vec![mic("guest", "GUEST_EN")]);
    assert!(service.start("127.0.0.1:0".parse().unwrap()));
    let ready = wait_for(&service, "bound", |s| s.socket.state == SocketState::Bound).await;
    send_tone(&ready).await;
    let status = wait_for(&service, "4 frames made", |s| {
        s.mics[0].frames_out == 4 && s.mics[0].packets == 75
    })
    .await;
    assert_eq!(
        (status.recorder.enabled, status.recorder.enabled_for),
        (false, None)
    );
    assert_eq!(recording(&status, "guest").state, RecordingState::Off);
    assert!(!root.path().join(DAY).exists(), "nothing written");
}

#[tokio::test]
async fn a_restart_the_same_day_records_into_a_new_file_and_keeps_the_old_one() {
    let root = tempfile::tempdir().unwrap();
    // one-frame batches begin 0.1 s before the clock: at 19:42:05.000
    let start = "2026-10-18T19:42:05.100";
    let (service, _) = recording_service(root.path(), start, vec![mic("guest", "GUEST_EN")]).await;
    send(&service, "guest", 100);
    wait_for(&service, "first file", |s| {
        recording(s, "guest").bytes == 3_200
    })
    .await;
    service.apply_recorder(None);
    wait_for(&service, "switched off", |s| {
        recording(s, "guest").state == RecordingState::Off
    })
    .await;
    service.apply_recorder(Some(day(DAY))); // a new writer, in the same second
    wait_for(&service, "a new writer", |s| {
        recording(s, "guest").state == RecordingState::Waiting
    })
    .await;
    send(&service, "guest", -100);
    let second = day_file(root.path(), "guest_194205-2.wav")
        .display()
        .to_string();
    wait_for(&service, "second file", |s| {
        recording(s, "guest").file.as_deref() == Some(second.as_str())
    })
    .await;
    let (header, samples) = read_wav(&day_file(root.path(), "guest_194205.wav"));
    assert_eq!(
        (header, samples.len(), samples[0]),
        (wav_header(3_200).to_vec(), 1_600, 100),
        "the first file: closed and untouched"
    );
    let (header, samples) = read_wav(Path::new(&second));
    assert_eq!((header, samples[0]), (wav_header(3_200).to_vec(), -100));
}

#[tokio::test]
async fn a_writer_that_falls_behind_loses_the_oldest_frames_and_counts_them() {
    let capture = Capture::install();
    let root = tempfile::tempdir().unwrap();
    let start = "2026-10-18T19:42:05.100";
    let (service, _) = recording_service(root.path(), start, vec![mic("guest", "GUEST_EN")]).await;
    let frames = service.frame_sender("guest");
    // a disk stall: 15 s of frames while the writer cannot run (nothing is
    // awaited in between on this current-thread runtime); every send returns at once
    for k in 0..150i16 {
        frames.send(frame(k)).unwrap();
    }
    wait_for(
        &service,
        "the channel's 12.8 s written, the rest counted",
        |s| {
            let rec = recording(s, "guest");
            rec.bytes == 128 * 3_200 && rec.dropped_frames == 22
        },
    )
    .await;
    // the 128 frames left began 12.8 s before the clock: 19:41:52.300
    assert_eq!(day_files(root.path()), vec!["guest_194152.wav"]);
    let (_, samples) = read_wav(&day_file(root.path(), "guest_194152.wav"));
    assert_eq!(samples[0], 22, "the oldest 22 frames went");
    assert_eq!(samples[samples.len() - 1], 149, "the newest are all there");
    assert_eq!(capture.events_with_message("recorder fell behind").len(), 1);
}

#[tokio::test]
async fn the_recording_ends_at_midnight_and_no_writer_starts_the_next_day() {
    let root = tempfile::tempdir().unwrap();
    let mics = vec![mic("guest", "GUEST_EN")];
    let start = "2026-10-18T23:59:58.100";
    let (service, now) = recording_service(root.path(), start, mics.clone()).await;
    send(&service, "guest", 1);
    wait_for(&service, "before midnight", |s| {
        recording(s, "guest").bytes == 3_200
    })
    .await;
    *now.lock().unwrap() = at("2026-10-19T00:00:01.100");
    send(&service, "guest", 2);
    wait_for(&service, "the day ended", |s| {
        recording(s, "guest").state == RecordingState::Off
    })
    .await;
    service.apply(mics); // a reconcile on the new day starts nothing
    tokio::time::sleep(Duration::from_millis(100)).await;
    let frames = service.frame_sender("guest");
    assert_eq!(
        frames.receiver_count(),
        0,
        "no writer listens on the next day"
    );
    let _unheard = frames.send(frame(3));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !root.path().join("2026-10-19").exists(),
        "nothing recorded after midnight"
    );
    let (header, samples) = read_wav(&day_file(root.path(), "guest_235958.wav"));
    assert_eq!(
        (header, samples.len()),
        (wav_header(3_200).to_vec(), 1_600),
        "closed with its header; the frame after midnight was not written"
    );
    let status = service.status().recorder;
    assert_eq!(
        (status.enabled, status.enabled_for),
        (false, Some(day(DAY)))
    );
}

#[tokio::test]
async fn a_mic_removed_from_the_list_stops_its_writer_and_leaves_the_status() {
    let root = tempfile::tempdir().unwrap();
    let guest = mic("guest", "GUEST_EN");
    let mics = vec![guest.clone(), mic("interp", "INTERP_SK")];
    let start = "2026-10-18T19:42:05.100";
    let (service, _) = recording_service(root.path(), start, mics).await;
    send(&service, "interp", 5);
    wait_for(&service, "interp records", |s| {
        recording(s, "interp").bytes == 3_200
    })
    .await;
    service.apply(vec![guest]);
    let interp = service.frame_sender("interp");
    wait_until("interp's writer ended", || interp.receiver_count() == 0).await;
    let status = service.status();
    assert_eq!(status.recorder.mics.len(), 1);
    assert_eq!(
        recording(&status, "guest").state,
        RecordingState::Waiting,
        "the other mic records on"
    );
    let (header, _) = read_wav(&day_file(root.path(), "interp_194205.wav"));
    assert_eq!(header, wav_header(3_200).to_vec());
}

/// One failed write in a writer: wait for it to listen, send a frame, wait
/// for it to fail and end.
async fn fail_once(frames: &broadcast::Sender<MicFrame>) {
    wait_until("a writer listens", || frames.receiver_count() == 1).await;
    frames.send(frame(1)).unwrap();
    wait_until("the write failed and the writer ended", || {
        frames.receiver_count() == 0
    })
    .await;
}

/// Let the supervisor collect the ended writer, move the clock past its pause
/// (C1b-5's back-off), and reconcile again.
async fn restart_after(
    service: &ListenService,
    mics: &[MicConfig],
    now: &Mutex<NaiveDateTime>,
    pause: TimeDelta,
) {
    service.apply(mics.to_vec());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let later = *now.lock().unwrap() + pause + TimeDelta::seconds(1);
    *now.lock().unwrap() = later;
    service.apply(mics.to_vec());
}

#[tokio::test]
async fn a_failed_write_is_shown_warned_on_power_of_two_failures_and_restarted() {
    let capture = Capture::install();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("not-a-folder");
    std::fs::write(&root, b"a file where the folder should be").unwrap();
    let mics = vec![mic("guest", "GUEST_EN")];
    let (service, now) = recording_service(&root, "2026-10-18T19:42:05", mics.clone()).await;
    let frames = service.frame_sender("guest");
    fail_once(&frames).await;
    restart_after(&service, &mics, &now, TimeDelta::minutes(1)).await;
    fail_once(&frames).await;
    restart_after(&service, &mics, &now, TimeDelta::minutes(2)).await;
    fail_once(&frames).await;
    let rec = recording(&service.status(), "guest");
    assert_eq!(rec.state, RecordingState::Error);
    assert!(
        rec.error
            .as_deref()
            .is_some_and(|e| e.starts_with("zápis nahrávky ")),
        "{rec:?}"
    );
    let warned: Vec<String> = capture
        .events_with_message("recording write failed")
        .iter()
        .filter_map(|line| line.field("failures").map(str::to_string))
        .collect();
    assert_eq!(
        warned,
        vec!["1", "2"],
        "3 failures in a row: 2 lines, not 3"
    );
    std::fs::remove_file(&root).unwrap(); // the disk is fine again
    restart_after(&service, &mics, &now, TimeDelta::minutes(4)).await;
    wait_until("a writer listens", || frames.receiver_count() == 1).await;
    frames.send(frame(7)).unwrap();
    let status = wait_for(&service, "recording again", |s| {
        recording(s, "guest").state == RecordingState::Recording
    })
    .await;
    assert_eq!(
        recording(&status, "guest").error,
        None,
        "a good write clears the error"
    );
}

/// A restart the same day (an auto-deploy mid-sermon) records on at once: the
/// retention pass waits `FIRST_PASS_DELAY` for NTP, the writers do not.
#[tokio::test(start_paused = true)]
async fn a_same_day_restart_records_at_once_while_the_first_retention_pass_waits() {
    let root = tempfile::tempdir().unwrap();
    let (clock, _) = clock("2026-10-18T19:42:05.100");
    let expired = put(root.path(), "2026-09-01", "guest_194205.wav", 10);
    let service = ListenService::new();
    service.set_recorder_env(RecorderEnv {
        root: root.path().to_path_buf(),
        clock,
        first_pass_delay: FIRST_PASS_DELAY,
    });
    service.apply(vec![mic("guest", "GUEST_EN")]);
    service.apply_recorder(Some(day(DAY)));
    let started = tokio::time::Instant::now();
    tokio::spawn(supervise(service.clone()));
    wait_for(&service, "a writer at once", |s| {
        recording(s, "guest").state == RecordingState::Waiting
    })
    .await;
    send(&service, "guest", 5);
    wait_for(&service, "recording", |s| {
        recording(s, "guest").bytes == 3_200
    })
    .await;
    assert!(
        started.elapsed() < FIRST_PASS_DELAY,
        "the writers never wait for the retention delay"
    );
    assert!(
        expired.exists(),
        "the first retention pass is still waiting"
    );
    tokio::time::sleep(FIRST_PASS_DELAY).await;
    wait_until("the first pass deleted the expired file", || {
        !expired.exists()
    })
    .await;
    assert!(
        day_file(root.path(), "guest_194205.wav").exists(),
        "retention never touches today's recording"
    );
}
