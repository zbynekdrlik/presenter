//! The recorder's files on disk (#852 C1b, spec §5.5, §12) — blocking
//! `std::fs`, run in `spawn_blocking` only.
//!
//! A recording ([`RecordingFile`]) is one continuous stretch of a mic's audio:
//! `<root>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav` (`-2` … `-9` for more stretches
//! started in the same second), 16 kHz mono s16le behind the 44-byte header,
//! whose two sizes are patched after every batch — a crash leaves a file whose
//! header covers the audio up to the last batch. A file is only ever created
//! (`create_new`: an existing name, a symlink included, is never opened) and
//! never inside a linked day folder.
//!
//! Retention deletes only what the recorder makes: day folders named
//! `YYYY-MM-DD` (canonical spelling) and in them the `<mic id>_<HHMMSS>[-n].wav`
//! regular files; an expired folder is removed once it holds nothing (the pass
//! emptied it, or it was empty already) and kept when anything else is in it.
//! Entries are judged by `DirEntry::file_type`, which never follows a symlink,
//! so a linked folder or file is never entered or deleted.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, NaiveDateTime};
use presenter_core::audio::wav::{
    wav_header, wav_riff_size, WAV_DATA_SIZE_AT, WAV_HEADER_LEN, WAV_RIFF_SIZE_AT,
};
use presenter_core::listen::recorder::{
    is_recording_file_name, parse_recording_day, recording_day_dir, recording_expired,
    recording_file_name, RECORDING_NAMES_PER_SECOND,
};

use super::MicFrame;

/// Why a recording could not be written (Slovak: the status shows it).
#[derive(Debug, thiserror::Error)]
pub(crate) enum RecordError {
    #[error("zápis nahrávky {path} zlyhal: {source}")]
    Io { path: String, source: io::Error },
    #[error("priečinok nahrávok {path} je odkaz — nenahrávam doň")]
    Linked { path: String },
    #[error("v {path} už je 9 nahrávok z tej istej sekundy")]
    NamesTaken { path: String },
}

fn io_error(path: &Path, source: io::Error) -> RecordError {
    RecordError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Folders whose recordings refuse an append, as a full disk would (tests only).
#[cfg(test)]
static FAILING_FOLDERS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// From now on every append to a recording under `dir` fails (tests only).
#[cfg(test)]
pub(super) fn fail_appends_under(dir: &Path) {
    FAILING_FOLDERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(dir.to_path_buf());
}

/// The injected full disk, when `path` lies under a folder that fails (tests only).
#[cfg(test)]
fn fail_if_injected(path: &Path) -> Result<(), RecordError> {
    let failing = FAILING_FOLDERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|dir| path.starts_with(dir));
    if failing {
        return Err(io_error(path, io::Error::other("no space left on device")));
    }
    Ok(())
}

/// One continuous stretch of a mic's audio, in its own file.
#[derive(Debug)]
pub(crate) struct RecordingFile {
    path: PathBuf,
    file: File,
    /// Audio bytes written (whole samples). One mic's day is at most 2.76 GB,
    /// far below what the u32 size fields hold: the day ends at midnight.
    data_bytes: u32,
}

impl RecordingFile {
    /// A new file for `mic_id`'s audio whose first sample was heard at
    /// `started` (local time); its header says "no audio yet".
    pub(crate) fn create(
        root: &Path,
        started: NaiveDateTime,
        mic_id: &str,
    ) -> Result<Self, RecordError> {
        let dir = root.join(recording_day_dir(started.date()));
        fs::create_dir_all(&dir).map_err(|err| io_error(&dir, err))?;
        if fs::symlink_metadata(&dir).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(RecordError::Linked {
                path: dir.display().to_string(),
            });
        }
        for attempt in 1..=RECORDING_NAMES_PER_SECOND {
            let path = dir.join(recording_file_name(mic_id, started.time(), attempt));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    let mut recording = Self {
                        path,
                        file,
                        data_bytes: 0,
                    };
                    recording.io(|file| file.write_all(&wav_header(0)))?;
                    return Ok(recording);
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(io_error(&path, err)),
            }
        }
        Err(RecordError::NamesTaken {
            path: dir.display().to_string(),
        })
    }

    /// Append `frames`, then patch the header sizes: a crash after this leaves
    /// a file whose header covers them. A failed write changes no size.
    pub(crate) fn append(&mut self, frames: &[MicFrame]) -> Result<(), RecordError> {
        #[cfg(test)]
        fail_if_injected(&self.path)?;
        let mut bytes = Vec::with_capacity(frames.iter().map(|frame| frame.len() * 2).sum());
        for sample in frames.iter().flat_map(|frame| frame.iter()) {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let at = WAV_HEADER_LEN as u64 + u64::from(self.data_bytes);
        self.io(|file| {
            file.seek(SeekFrom::Start(at))?;
            file.write_all(&bytes)
        })?;
        let added = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        self.data_bytes = self.data_bytes.saturating_add(added);
        self.patch_sizes()
    }

    /// The header and the audio on disk (`fsync`); the audio bytes written.
    pub(crate) fn finish(mut self) -> Result<u32, RecordError> {
        self.patch_sizes()?;
        self.io(|file| file.sync_all())?;
        Ok(self.data_bytes)
    }

    pub(crate) fn data_bytes(&self) -> u32 {
        self.data_bytes
    }

    pub(crate) fn path_text(&self) -> String {
        self.path.display().to_string()
    }

    fn patch_sizes(&mut self) -> Result<(), RecordError> {
        let riff = wav_riff_size(self.data_bytes).to_le_bytes();
        let data = self.data_bytes.to_le_bytes();
        self.io(|file| {
            file.seek(SeekFrom::Start(WAV_RIFF_SIZE_AT))?;
            file.write_all(&riff)?;
            file.seek(SeekFrom::Start(WAV_DATA_SIZE_AT))?;
            file.write_all(&data)
        })
    }

    fn io<T>(&mut self, op: impl FnOnce(&mut File) -> io::Result<T>) -> Result<T, RecordError> {
        op(&mut self.file).map_err(|err| io_error(&self.path, err))
    }
}

/// What a retention pass did.
#[derive(Debug, Default)]
pub(crate) struct PruneReport {
    /// Each deleted file and its size.
    pub(crate) deleted: Vec<(PathBuf, u64)>,
    /// An expired folder left in place: it still holds something the recorder
    /// did not write.
    pub(crate) kept: Vec<PathBuf>,
    /// An expired folder that could not be read or cleaned, and why; an entry the
    /// scan could not read at all is listed against the recordings folder itself.
    pub(crate) failed: Vec<(PathBuf, String)>,
}

/// Delete the day folders under `root` older than the retention: their
/// recordings, then the folder once it holds nothing else. A missing root is
/// nothing to do; one folder's failure never stops the others.
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
/// (a seam for tests: a real directory cannot be made to fail mid-scan). Only
/// opening the folder can fail the pass; an entry that cannot be read or cleaned
/// is a finding in the report and the scan goes on.
fn prune_scan(
    root: &Path,
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
        match entry {
            Ok(entry) => prune_entry(entry, today, &mut report),
            Err(err) => report.failed.push((root.to_path_buf(), err.to_string())),
        }
    }
    Ok(report)
}

/// One entry of the folder: an expired day folder is pruned, anything else is
/// left alone (and, being no day folder, never reported).
fn prune_entry(entry: RootEntry, today: NaiveDate, report: &mut PruneReport) {
    let expired = entry
        .path
        .file_name()
        .and_then(OsStr::to_str)
        .and_then(parse_recording_day)
        .is_some_and(|day| recording_expired(day, today));
    if !expired {
        return;
    }
    match entry.is_dir {
        Ok(true) => {
            if let Err(err) = prune_day(&entry.path, report) {
                report.failed.push((entry.path, err.to_string()));
            }
        }
        Ok(false) => {}
        Err(err) => report.failed.push((entry.path, err.to_string())),
    }
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

    #[cfg(unix)]
    #[test]
    fn an_expired_folder_that_cannot_be_cleaned_is_reported_and_the_others_still_go() {
        use std::os::unix::fs::PermissionsExt;

        /// Gives the folder its write permission back, so the temp dir can be
        /// removed even when an assertion fails.
        struct Writable(PathBuf);
        impl Drop for Writable {
            fn drop(&mut self) {
                let _restored = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
            }
        }

        let root = tempfile::tempdir().unwrap();
        let stuck = put(root.path(), "2026-08-01", "guest_194205.wav", 10);
        let other = put(root.path(), "2026-09-01", "guest_100000.wav", 20);
        let stuck_dir = root.path().join("2026-08-01");
        // read-only: the folder lists, its recording cannot be removed
        fs::set_permissions(&stuck_dir, fs::Permissions::from_mode(0o555)).unwrap();
        let _writable = Writable(stuck_dir.clone());
        let report = prune_recordings(root.path(), day("2026-10-15")).unwrap();
        assert!(stuck.exists(), "the recording behind the failure stays");
        assert!(
            !root.path().join("2026-09-01").exists(),
            "the other expired folder is pruned"
        );
        assert_eq!(report.deleted, vec![(other, 20)]);
        assert_eq!(report.failed.len(), 1, "{report:?}");
        assert_eq!(report.failed[0].0, stuck_dir);
        assert!(
            report.failed[0].1.contains("ermission denied"),
            "{report:?}"
        );
        assert!(report.kept.is_empty(), "{report:?}");
    }

    fn at(text: &str) -> NaiveDateTime {
        text.parse().unwrap()
    }

    fn frame(value: i16) -> MicFrame {
        std::sync::Arc::from(vec![value; 1_600])
    }

    #[test]
    fn every_batch_leaves_a_header_that_covers_it() {
        let root = tempfile::tempdir().unwrap();
        let mut file =
            RecordingFile::create(root.path(), at("2026-10-18T19:42:05"), "guest").unwrap();
        let path = root.path().join("2026-10-18").join("guest_194205.wav");
        assert_eq!(file.path_text(), path.display().to_string());
        assert_eq!(fs::read(&path).unwrap(), wav_header(0));
        file.append(&[frame(1), frame(-2)]).unwrap();
        // no finish: exactly what a crash right now would leave on disk
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + 6_400);
        assert_eq!(&bytes[..44], &wav_header(6_400)[..]);
        assert_eq!(&bytes[44..46], &1i16.to_le_bytes());
        assert_eq!(&bytes[44 + 3_200..44 + 3_202], &(-2i16).to_le_bytes());
        file.append(&[frame(3)]).unwrap();
        assert_eq!(&fs::read(&path).unwrap()[..44], &wav_header(9_600)[..]);
        assert_eq!(file.finish().unwrap(), 9_600);
    }

    #[test]
    fn a_second_file_in_the_same_second_never_touches_the_first() {
        let root = tempfile::tempdir().unwrap();
        let started = at("2026-10-18T19:42:05");
        let mut first = RecordingFile::create(root.path(), started, "guest").unwrap();
        first.append(&[frame(7)]).unwrap();
        assert_eq!(first.finish().unwrap(), 3_200);
        let second = RecordingFile::create(root.path(), started, "guest").unwrap();
        assert!(
            second.path_text().ends_with("guest_194205-2.wav"),
            "{}",
            second.path_text()
        );
        let old = fs::read(root.path().join("2026-10-18").join("guest_194205.wav")).unwrap();
        assert_eq!(old.len(), 44 + 3_200);
        assert_eq!(&old[44..46], &7i16.to_le_bytes());
    }

    #[test]
    fn a_tenth_file_in_one_second_is_refused_not_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let started = at("2026-10-18T19:42:05");
        for _ in 0..RECORDING_NAMES_PER_SECOND {
            RecordingFile::create(root.path(), started, "guest").unwrap();
        }
        let err = RecordingFile::create(root.path(), started, "guest").unwrap_err();
        assert!(matches!(err, RecordError::NamesTaken { .. }), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_never_written_through() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("2026-10-18")).unwrap();
        let err =
            RecordingFile::create(root.path(), at("2026-10-18T19:42:05"), "guest").unwrap_err();
        assert!(matches!(err, RecordError::Linked { .. }), "{err}");
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        let target = outside.path().join("keep.wav");
        fs::write(&target, b"keep").unwrap();
        fs::create_dir_all(root.path().join("2026-10-19")).unwrap();
        let link = root.path().join("2026-10-19").join("guest_194205.wav");
        std::os::unix::fs::symlink(&target, link).unwrap();
        let file = RecordingFile::create(root.path(), at("2026-10-19T19:42:05"), "guest").unwrap();
        assert!(
            file.path_text().ends_with("guest_194205-2.wav"),
            "a link at the name is a taken name"
        );
        assert_eq!(fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn a_failing_write_after_a_good_create_changes_no_size_and_says_why() {
        let root = tempfile::tempdir().unwrap();
        let mut good =
            RecordingFile::create(root.path(), at("2026-10-18T19:42:05"), "guest").unwrap();
        good.append(&[frame(1)]).unwrap();
        // the disk refuses the next write: the same file through a handle that cannot write
        let mut full = RecordingFile {
            path: good.path.clone(),
            file: File::open(&good.path).unwrap(),
            data_bytes: good.data_bytes(),
        };
        let err = full.append(&[frame(2)]).unwrap_err();
        assert!(matches!(err, RecordError::Io { .. }), "{err}");
        assert!(err.to_string().starts_with("zápis nahrávky "), "{err}");
        assert_eq!(
            full.data_bytes(),
            3_200,
            "nothing counted that was not written"
        );
        let bytes = fs::read(&good.path).unwrap();
        assert_eq!(
            &bytes[..44],
            &wav_header(3_200)[..],
            "the header still covers the audio"
        );
        assert_eq!(bytes.len(), 44 + 3_200);
    }

    #[test]
    fn a_folder_that_cannot_be_made_is_a_slovak_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("not-a-folder");
        fs::write(&root, b"x").unwrap();
        let err = RecordingFile::create(&root, at("2026-10-18T19:42:05"), "guest").unwrap_err();
        let text = err.to_string();
        assert!(
            text.starts_with("zápis nahrávky ") && text.contains("not-a-folder"),
            "{text}"
        );
    }
}
