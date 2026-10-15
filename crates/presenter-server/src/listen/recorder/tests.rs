//! #852 C1b-2: the recordings folder and the daily retention pass. Folders are
//! temp dirs; the clock is a settable local time.

use super::*;
use crate::tracing_capture::Capture;
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
