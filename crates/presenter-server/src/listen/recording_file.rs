//! The recorder's files on disk (#852 C1b, spec §5.5, §12) — blocking
//! `std::fs`, run in `spawn_blocking` only.
//!
//! Retention deletes only what the recorder makes: day folders named
//! `YYYY-MM-DD` (canonical spelling) and in them the `<mic id>_<HHMMSS>[-n].wav`
//! regular files; a folder is removed only when that emptied it. Entries are
//! judged by `DirEntry::file_type`, which never follows a symlink, so a linked
//! folder or file is never entered or deleted.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use presenter_core::listen::recorder::{
    is_recording_file_name, parse_recording_day, recording_expired,
};

/// What a retention pass did.
#[derive(Debug, Default)]
pub(crate) struct PruneReport {
    /// Each deleted file and its size.
    pub(crate) deleted: Vec<(PathBuf, u64)>,
    /// An expired folder left in place: it still holds something the recorder
    /// did not write.
    pub(crate) kept: Vec<PathBuf>,
    /// An expired folder that could not be cleaned, and why.
    pub(crate) failed: Vec<(PathBuf, String)>,
}

/// Delete the day folders under `root` older than the retention: their
/// recordings, then the folder when that emptied it. A missing root is nothing
/// to do; one folder's failure never stops the others.
pub(crate) fn prune_recordings(root: &Path, today: NaiveDate) -> io::Result<PruneReport> {
    let opened = fs::read_dir(root).map(|entries| entries.map(root_entry));
    prune_scan(root, opened, today)
}

/// One entry of the recordings folder as the scan sees it: its path and whether
/// it is a directory.
struct RootEntry {
    path: PathBuf,
    is_dir: io::Result<bool>,
}

/// A directory entry as the scan sees it; a failed read of the entry itself
/// stays an error item of the iterator.
fn root_entry(entry: io::Result<fs::DirEntry>) -> io::Result<RootEntry> {
    let entry = entry?;
    Ok(RootEntry {
        is_dir: entry.file_type().map(|kind| kind.is_dir()),
        path: entry.path(),
    })
}

/// The retention pass over the entries of `root`, given the result of opening it
/// (a seam for tests: a real directory cannot be made to fail mid-scan).
fn prune_scan(
    _root: &Path,
    opened: io::Result<impl Iterator<Item = io::Result<RootEntry>>>,
    today: NaiveDate,
) -> io::Result<PruneReport> {
    let entries = match opened {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(PruneReport::default()),
        Err(err) => return Err(err),
    };
    let mut report = PruneReport::default();
    for entry in entries {
        let entry = entry?;
        let expired = entry.is_dir?
            && entry
                .path
                .file_name()
                .and_then(OsStr::to_str)
                .and_then(parse_recording_day)
                .is_some_and(|day| recording_expired(day, today));
        if expired {
            let dir = entry.path;
            if let Err(err) = prune_day(&dir, &mut report) {
                report.failed.push((dir, err.to_string()));
            }
        }
    }
    Ok(report)
}

/// One expired day folder: its recordings, then the folder if nothing else is in it.
fn prune_day(dir: &Path, report: &mut PruneReport) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let recording = entry.file_type()?.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(is_recording_file_name);
        if recording {
            let bytes = entry.metadata()?.len();
            fs::remove_file(entry.path())?;
            report.deleted.push((entry.path(), bytes));
        }
    }
    match fs::remove_dir(dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => {
            report.kept.push(dir.to_path_buf());
            Ok(())
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    /// `root/<folder>/<file>` with `len` bytes.
    fn put(root: &Path, folder: &str, file: &str, len: usize) -> PathBuf {
        let dir = root.join(folder);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(file);
        fs::write(&path, vec![0u8; len]).unwrap();
        path
    }

    #[test]
    fn old_day_folders_go_file_by_file_and_the_30th_day_stays() {
        let root = tempfile::tempdir().unwrap();
        let guest = put(root.path(), "2026-09-14", "guest_194205.wav", 100);
        let interp = put(root.path(), "2026-09-14", "interp_194206-2.wav", 300);
        let kept = put(root.path(), "2026-09-15", "guest_100000.wav", 50);
        let report = prune_recordings(root.path(), day("2026-10-15")).unwrap();
        let mut deleted = report.deleted.clone();
        deleted.sort();
        assert_eq!(deleted, vec![(guest, 100), (interp, 300)]);
        assert!(
            !root.path().join("2026-09-14").exists(),
            "the emptied folder goes"
        );
        assert!(kept.exists(), "30 days old: kept");
        assert!(
            report.kept.is_empty() && report.failed.is_empty(),
            "{report:?}"
        );
    }

    #[test]
    fn a_folder_with_other_files_or_another_name_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let ours = put(root.path(), "2026-09-01", "guest_194205.wav", 10);
        let notes = put(root.path(), "2026-09-01", "notes.txt", 10);
        let renamed = put(root.path(), "2026-09-01", "guest.wav", 10);
        let elsewhere = put(root.path(), "old-stuff", "guest_194205.wav", 10);
        let loose = put(root.path(), ".", "2026-09-02", 10);
        let report = prune_recordings(root.path(), day("2026-10-15")).unwrap();
        assert!(!ours.exists());
        assert!(
            notes.exists() && renamed.exists(),
            "not the recorder's files"
        );
        assert_eq!(report.kept, vec![root.path().join("2026-09-01")]);
        assert!(elsewhere.exists(), "a folder not named like a day");
        assert!(loose.exists(), "a FILE named like a day");
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_folder_is_never_followed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = put(outside.path(), "2026-09-01", "guest_194205.wav", 10);
        std::os::unix::fs::symlink(
            outside.path().join("2026-09-01"),
            root.path().join("2026-09-01"),
        )
        .unwrap();
        let linked_file = put(root.path(), "2026-09-02", "notes.txt", 1);
        std::os::unix::fs::symlink(&target, linked_file.with_file_name("guest_100000.wav"))
            .unwrap();
        let report = prune_recordings(root.path(), day("2026-10-15")).unwrap();
        assert!(target.exists(), "nothing behind a symlink is deleted");
        assert!(report.deleted.is_empty(), "{report:?}");
        assert!(
            root.path()
                .join("2026-09-02")
                .join("guest_100000.wav")
                .symlink_metadata()
                .is_ok(),
            "a link named like a recording is not a regular file: left"
        );
    }

    #[test]
    fn a_missing_root_is_nothing_to_do() {
        let root = tempfile::tempdir().unwrap();
        let report = prune_recordings(&root.path().join("never-made"), day("2026-10-15")).unwrap();
        assert!(report.deleted.is_empty() && report.kept.is_empty() && report.failed.is_empty());
    }

    /// A folder entry whose type could not be read, as the scan sees it.
    fn unreadable(root: &Path, name: &str, why: &'static str) -> io::Result<RootEntry> {
        Ok(RootEntry {
            path: root.join(name),
            is_dir: Err(io::Error::other(why)),
        })
    }

    fn folder(root: &Path, name: &str) -> io::Result<RootEntry> {
        Ok(RootEntry {
            path: root.join(name),
            is_dir: Ok(true),
        })
    }

    #[test]
    fn an_entry_that_cannot_be_read_never_stops_the_rest_of_the_pass() {
        let root = tempfile::tempdir().unwrap();
        let before = put(root.path(), "2026-08-01", "guest_194205.wav", 10);
        let after = put(root.path(), "2026-09-01", "interp_100000.wav", 20);
        let entries = vec![
            folder(root.path(), "2026-08-01"),
            Err(io::Error::other("readdir failed")),
            unreadable(root.path(), "2026-08-15", "stat failed"),
            unreadable(root.path(), "notes", "not a day folder"),
            unreadable(root.path(), "2026-10-01", "not expired"),
            folder(root.path(), "2026-09-01"),
        ];
        let report = prune_scan(root.path(), Ok(entries.into_iter()), day("2026-10-15"))
            .expect("one bad entry is a finding of the pass, not the end of it");
        let mut deleted = report.deleted.clone();
        deleted.sort();
        assert_eq!(
            deleted,
            vec![(before, 10), (after, 20)],
            "the folders before AND after the bad entries are pruned and reported"
        );
        assert!(!root.path().join("2026-08-01").exists());
        assert!(!root.path().join("2026-09-01").exists());
        let mut failed = report.failed.clone();
        failed.sort();
        assert_eq!(
            failed,
            vec![
                (root.path().to_path_buf(), "readdir failed".to_string()),
                (root.path().join("2026-08-15"), "stat failed".to_string()),
            ],
            "the unreadable item is reported against the folder, an unreadable expired \
             day against its path; an entry that is no expired day folder is not reported"
        );
    }
}
