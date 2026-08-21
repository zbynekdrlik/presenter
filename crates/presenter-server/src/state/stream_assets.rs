//! Stream-graphics asset STORAGE — the on-disk, content-addressed byte layer
//! beneath the #705 metadata-row repository (#708, epic #718 §5 / arch dec. #4).
//!
//! The repository (`presenter-persistence::repository::stream_assets`) records
//! only the `stream_assets` metadata ROW. This module owns everything AROUND
//! the row: magic-byte validation, sha256 hashing, and content-addressed files
//! written to `<workdir>/stream-assets/<sha256>.<ext>` so identical bytes dedup
//! to one file and can be cached forever (the name for a given content can
//! never change). The HTTP surface that drives it lives in
//! `router/stream_assets.rs`.
//!
//! Path-traversal safety (arch / issue acceptance): the served/deleted file
//! name is derived ONLY from a stored sha256 (hex we computed) plus a
//! whitelisted extension — never from client input. [`AssetStore::path_for`]
//! additionally refuses any name that is not a bare `^[0-9a-f]{64}$` sha with a
//! whitelisted ext, so no `/` or `..` can ever reach a path join.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::state::AppState;

/// Business cap on a single uploaded asset (20 MiB). The upload route's
/// `DefaultBodyLimit` is set a little higher (a DoS ceiling on the raw body);
/// this is the precise, user-facing `413` boundary enforced on the decoded
/// image bytes.
pub(crate) const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;

/// An accepted image kind, detected by MAGIC BYTES (never the client-declared
/// content-type, which can lie). `mime` is stored on the row; `ext` names the
/// on-disk file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetectedImage {
    pub mime: &'static str,
    pub ext: &'static str,
}

/// Detect the image kind from the leading magic bytes. Returns `None` for
/// anything not in the accepted set (png / jpeg / webp) — the caller maps that
/// to `422`.
pub(crate) fn detect_image(bytes: &[u8]) -> Option<DetectedImage> {
    const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() >= 8 && bytes[..8] == PNG_MAGIC {
        return Some(DetectedImage {
            mime: "image/png",
            ext: "png",
        });
    }
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some(DetectedImage {
            mime: "image/jpeg",
            ext: "jpg",
        });
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(DetectedImage {
            mime: "image/webp",
            ext: "webp",
        });
    }
    None
}

/// Extension used to name the on-disk file for a stored `mime` (serve/delete
/// path). Mirrors [`detect_image`]'s `ext` choices. `None` for an unknown mime.
pub(crate) fn ext_for_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// Lowercase-hex sha256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        // Infallible: writing to a String never errors.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// True iff `s` is a bare lowercase-hex sha256 (64 hex chars). The
/// content-addressed-name traversal guard.
pub(crate) fn is_valid_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Best-effort image dimensions from the header alone — no decode, no
/// dependency. Returns `None` (never an error) for any unparseable header;
/// `width`/`height` on the row are `Option`.
pub(crate) fn image_dims(bytes: &[u8]) -> Option<(u32, u32)> {
    match detect_image(bytes)?.mime {
        "image/png" => png_dims(bytes),
        "image/jpeg" => jpeg_dims(bytes),
        "image/webp" => webp_dims(bytes),
        _ => None,
    }
}

fn png_dims(b: &[u8]) -> Option<(u32, u32)> {
    // 8-byte signature, then the IHDR chunk: length(4) + "IHDR"(4) + width(4) +
    // height(4). Width at offset 16, height at 20 (big-endian).
    if b.len() < 24 || &b[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    if w == 0 || h == 0 {
        None
    } else {
        Some((w, h))
    }
}

fn jpeg_dims(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 3 < b.len() {
        if b[i] != 0xFF {
            return None; // not aligned on a marker
        }
        let marker = b[i + 1];
        if marker == 0xFF {
            i += 1; // padding fill byte
            continue;
        }
        // Standalone markers carry no length: RSTn (D0-D7), SOI(D8), EOI(D9),
        // TEM(01).
        if (0xD0..=0xD9).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        // SOF markers carry the frame dimensions — all of C0..CF EXCEPT C4
        // (DHT), C8 (JPG) and CC (DAC).
        let is_sof = matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF);
        if is_sof {
            // payload: precision(1) height(2) width(2), starting at i+4.
            if i + 8 >= b.len() {
                return None;
            }
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            if w == 0 || h == 0 {
                return None;
            }
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

fn webp_dims(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 16 {
        return None;
    }
    match &b[12..16] {
        b"VP8 " => {
            // Lossy: frame tag at 20, start code 9d 01 2a at 23..26, then 14-bit
            // width and height (little-endian) at 26 and 28.
            if b.len() < 30 || b[23] != 0x9d || b[24] != 0x01 || b[25] != 0x2a {
                return None;
            }
            let w = (u16::from_le_bytes([b[26], b[27]]) & 0x3FFF) as u32;
            let h = (u16::from_le_bytes([b[28], b[29]]) & 0x3FFF) as u32;
            non_zero(w, h)
        }
        b"VP8L" => {
            // Lossless: signature 0x2f at 20, then 14-bit (width-1) and
            // (height-1) packed little-endian.
            if b.len() < 25 || b[20] != 0x2f {
                return None;
            }
            let bits = u32::from_le_bytes([b[21], b[22], b[23], b[24]]);
            let w = (bits & 0x3FFF) + 1;
            let h = ((bits >> 14) & 0x3FFF) + 1;
            non_zero(w, h)
        }
        b"VP8X" => {
            // Extended: canvas (width-1) as 3 bytes LE at 24, (height-1) at 27.
            if b.len() < 30 {
                return None;
            }
            let w = (u32::from(b[24]) | (u32::from(b[25]) << 8) | (u32::from(b[26]) << 16)) + 1;
            let h = (u32::from(b[27]) | (u32::from(b[28]) << 8) | (u32::from(b[29]) << 16)) + 1;
            non_zero(w, h)
        }
        _ => None,
    }
}

fn non_zero(w: u32, h: u32) -> Option<(u32, u32)> {
    if w == 0 || h == 0 {
        None
    } else {
        Some((w, h))
    }
}

/// The content-addressed file store over a single directory.
#[derive(Clone)]
pub(crate) struct AssetStore {
    dir: PathBuf,
}

impl AssetStore {
    pub(crate) fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    #[cfg(test)]
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// Create the store directory (idempotent). Called at startup and
    /// defensively before every write.
    pub(crate) async fn ensure_dir(&self) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.dir).await
    }

    /// Absolute path for a stored asset, or `None` when the name components are
    /// not a bare sha256 + whitelisted ext (traversal guard — never joins
    /// client-controlled path segments).
    pub(crate) fn path_for(&self, sha256: &str, ext: &str) -> Option<PathBuf> {
        if !is_valid_sha256(sha256) {
            return None;
        }
        if !matches!(ext, "png" | "jpg" | "jpeg" | "webp") {
            return None;
        }
        Some(self.dir.join(format!("{sha256}.{ext}")))
    }

    /// Write `bytes` content-addressed (unique tmp file + atomic rename;
    /// idempotent when identical bytes already exist). Returns the final path.
    pub(crate) async fn store(
        &self,
        sha256: &str,
        ext: &str,
        bytes: &[u8],
    ) -> std::io::Result<PathBuf> {
        self.ensure_dir().await?;
        let final_path = self.path_for(sha256, ext).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid asset name")
        })?;
        // Dedup: identical content is already on disk under this name.
        if tokio::fs::metadata(&final_path).await.is_ok() {
            return Ok(final_path);
        }
        let tmp = self
            .dir
            .join(format!(".upload-{}.tmp", uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp, bytes).await?;
        // Atomic on the same filesystem; overwriting identical content on a
        // concurrent-upload race is harmless (same bytes).
        tokio::fs::rename(&tmp, &final_path).await?;
        Ok(final_path)
    }

    /// Read the stored bytes, or `None` when the row's file is missing / the
    /// name is invalid (the caller maps that to `404`).
    pub(crate) async fn read(&self, sha256: &str, ext: &str) -> std::io::Result<Option<Vec<u8>>> {
        let Some(path) = self.path_for(sha256, ext) else {
            return Ok(None);
        };
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Remove the stored file (idempotent — a missing file is success).
    pub(crate) async fn remove(&self, sha256: &str, ext: &str) -> std::io::Result<()> {
        let Some(path) = self.path_for(sha256, ext) else {
            return Ok(());
        };
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Remove stale `.upload-<uuid>.tmp` files left by a crash between the write
    /// and the rename in [`Self::store`]. Such files are inert (never served —
    /// `path_for` only accepts a bare sha256 + whitelisted ext) but would
    /// otherwise accumulate disk; swept once at startup.
    pub(crate) async fn sweep_tmp(&self) -> std::io::Result<()> {
        let mut entries = match tokio::fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".upload-") && name.ends_with(".tmp") {
                // Best-effort: a concurrent upload's rename may already have
                // moved it, or a permission quirk — never fail startup over it.
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
        Ok(())
    }
}

/// Resolve the stream-assets directory: an explicit `PRESENTER_STREAM_ASSETS_DIR`
/// override, else the `stream-assets` sibling of the sqlite db named in
/// `PRESENTER_DB_URL`, else `./stream-assets` (cwd). For prod/dev all three
/// resolve to `<deploy-dir>/stream-assets` (the unit sets both
/// `WorkingDirectory` and `PRESENTER_DB_URL`).
pub(crate) fn resolve_dir() -> PathBuf {
    if let Some(explicit) = std::env::var_os("PRESENTER_STREAM_ASSETS_DIR") {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    let db_url = std::env::var("PRESENTER_DB_URL").unwrap_or_default();
    stream_assets_dir_from_db_url(&db_url)
}

/// Pure: the `stream-assets` dir that sits next to the sqlite file named in
/// `db_url`. An in-memory / empty / relative-filename url falls back to
/// `./stream-assets`.
pub(crate) fn stream_assets_dir_from_db_url(db_url: &str) -> PathBuf {
    let parent = sqlite_file_path(db_url)
        .as_deref()
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    parent.join("stream-assets")
}

fn sqlite_file_path(db_url: &str) -> Option<PathBuf> {
    let s = db_url.strip_prefix("sqlite://").unwrap_or(db_url);
    let s = s.strip_prefix("sqlite:").unwrap_or(s);
    let s = s.split('?').next().unwrap_or(s);
    if s.is_empty() || s.contains(":memory:") {
        return None;
    }
    Some(PathBuf::from(s))
}

impl AppState {
    /// Directory holding content-addressed stream-graphics asset files (next to
    /// `presenter.db`). See #708 / arch dec. #4. (Read by tests + the #709
    /// editor def-assembly; the handlers go through [`Self::asset_store`].)
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn stream_assets_dir(&self) -> &Path {
        &self.stream_assets_dir
    }

    /// An [`AssetStore`] over [`Self::stream_assets_dir`].
    pub(crate) fn asset_store(&self) -> AssetStore {
        AssetStore::new(self.stream_assets_dir.clone())
    }

    /// Ensure the stream-assets directory exists and clear any stale upload
    /// tmp files (startup hook, idempotent, logged).
    ///
    /// `pub`, not `pub(crate)`: `main.rs` is a SEPARATE crate root from this
    /// lib (see `lib.rs`'s doc comment / `.claude/rules/ai-eval-harness.md`)
    /// and calls this at startup — a `pub(crate)` method is invisible across
    /// that crate boundary, which is also exactly why clippy's `dead_code`
    /// lint flagged this (and transitively [`AssetStore::sweep_tmp`], reached
    /// only through this method in the non-test build) as unreachable: within
    /// the lib crate itself, nothing else calls it.
    pub async fn ensure_stream_assets_dir(&self) -> std::io::Result<()> {
        tracing::info!(
            dir = %self.stream_assets_dir.display(),
            "ensuring stream-assets directory exists"
        );
        let store = self.asset_store();
        store.ensure_dir().await?;
        if let Err(e) = store.sweep_tmp().await {
            tracing::warn!(error = %e, "stream-assets tmp sweep failed (non-fatal)");
        }
        Ok(())
    }

    /// Point the store at a specific directory (test isolation only).
    #[cfg(test)]
    pub(crate) fn set_stream_assets_dir(&mut self, dir: impl Into<PathBuf>) {
        self.stream_assets_dir = dir.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_png(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(&[0, 0, 0, 13]); // IHDR length
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]); // bit depth, color, etc.
        v
    }

    fn minimal_jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8]; // SOI
        v.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]); // APP0 (len 4)
        v.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]); // SOF0, len 17, precision 8
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&[0x03, 0x01, 0x22, 0x00]); // components (partial, enough)
        v
    }

    fn minimal_webp_vp8x(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&[0, 0, 0, 0]); // file size (ignored by parser)
        v.extend_from_slice(b"WEBP");
        v.extend_from_slice(b"VP8X");
        v.extend_from_slice(&[0, 0, 0, 10]); // chunk size
        v.extend_from_slice(&[0, 0, 0, 0]); // flags + reserved
        let wm1 = w - 1;
        let hm1 = h - 1;
        v.extend_from_slice(&[
            (wm1 & 0xFF) as u8,
            ((wm1 >> 8) & 0xFF) as u8,
            ((wm1 >> 16) & 0xFF) as u8,
        ]);
        v.extend_from_slice(&[
            (hm1 & 0xFF) as u8,
            ((hm1 >> 8) & 0xFF) as u8,
            ((hm1 >> 16) & 0xFF) as u8,
        ]);
        v
    }

    #[test]
    fn detect_image_recognizes_accepted_kinds_and_rejects_others() {
        assert_eq!(detect_image(&minimal_png(1, 1)).unwrap().mime, "image/png");
        assert_eq!(
            detect_image(&minimal_jpeg(1, 1)).unwrap().mime,
            "image/jpeg"
        );
        assert_eq!(
            detect_image(&minimal_webp_vp8x(1, 1)).unwrap().mime,
            "image/webp"
        );
        assert!(detect_image(b"GIF89a...........").is_none());
        assert!(detect_image(b"not an image at all").is_none());
        assert!(detect_image(&[]).is_none());
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        // sha256("") — a stable, checkable vector.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(sha256_hex(b"").len(), 64);
        // Determinism: identical bytes hash identically (dedup foundation).
        assert_eq!(sha256_hex(b"abc"), sha256_hex(b"abc"));
        assert_ne!(sha256_hex(b"abc"), sha256_hex(b"abd"));
    }

    #[test]
    fn is_valid_sha256_accepts_only_bare_lowercase_hex_64() {
        assert!(is_valid_sha256(&"a".repeat(64)));
        assert!(is_valid_sha256(&sha256_hex(b"x")));
        assert!(!is_valid_sha256(&"a".repeat(63)));
        assert!(!is_valid_sha256(&"a".repeat(65)));
        assert!(!is_valid_sha256(&"A".repeat(64))); // uppercase not allowed
        assert!(!is_valid_sha256("../../etc/passwd"));
        assert!(!is_valid_sha256(""));
    }

    #[test]
    fn image_dims_parses_each_format() {
        assert_eq!(image_dims(&minimal_png(1920, 1080)), Some((1920, 1080)));
        assert_eq!(image_dims(&minimal_jpeg(640, 480)), Some((640, 480)));
        assert_eq!(image_dims(&minimal_webp_vp8x(800, 600)), Some((800, 600)));
        assert_eq!(image_dims(b"not an image"), None);
    }

    #[test]
    fn path_for_rejects_traversal_and_bad_ext() {
        let store = AssetStore::new("/tmp/does-not-matter");
        let good = "a".repeat(64);
        assert!(store.path_for(&good, "png").is_some());
        // Traversal / non-hex names are refused outright.
        assert!(store.path_for("../../etc/passwd", "png").is_none());
        assert!(store.path_for("../secret", "png").is_none());
        assert!(store.path_for(&good, "exe").is_none());
        assert!(store.path_for(&good, "php").is_none());
        // A valid name never escapes the store directory.
        let p = store.path_for(&good, "png").unwrap();
        assert_eq!(p.parent().unwrap(), Path::new("/tmp/does-not-matter"));
    }

    #[tokio::test]
    async fn store_writes_reads_dedups_and_removes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = AssetStore::new(tmp.path().join("stream-assets"));
        let bytes = minimal_png(4, 4);
        let sha = sha256_hex(&bytes);

        let path1 = store.store(&sha, "png", &bytes).await.unwrap();
        assert!(path1.exists(), "file written to disk");
        assert_eq!(store.read(&sha, "png").await.unwrap(), Some(bytes.clone()));

        // Dedup: storing identical bytes yields the same path, one file.
        let path2 = store.store(&sha, "png", &bytes).await.unwrap();
        assert_eq!(path1, path2);
        let entries: Vec<_> = std::fs::read_dir(store.dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".png"))
            .collect();
        assert_eq!(entries.len(), 1, "identical bytes dedup to one file");

        // Remove is real then idempotent.
        store.remove(&sha, "png").await.unwrap();
        assert_eq!(store.read(&sha, "png").await.unwrap(), None);
        store.remove(&sha, "png").await.unwrap(); // no error on missing
    }

    #[tokio::test]
    async fn sweep_tmp_removes_stale_upload_files_but_keeps_assets() {
        let tmp = tempfile::tempdir().unwrap();
        let store = AssetStore::new(tmp.path().join("stream-assets"));
        let bytes = minimal_png(4, 4);
        let sha = sha256_hex(&bytes);
        store.store(&sha, "png", &bytes).await.unwrap();

        // Simulate a crash-orphaned tmp file.
        let orphan = store.dir().join(".upload-deadbeef.tmp");
        std::fs::write(&orphan, b"partial").unwrap();
        assert!(orphan.exists());

        store.sweep_tmp().await.unwrap();

        assert!(!orphan.exists(), "stale .tmp swept");
        assert_eq!(
            store.read(&sha, "png").await.unwrap(),
            Some(bytes),
            "real asset untouched by the sweep"
        );
        // Idempotent on an empty/clean dir.
        store.sweep_tmp().await.unwrap();
    }

    #[test]
    fn dir_from_db_url_resolves_sibling_of_the_db_file() {
        assert_eq!(
            stream_assets_dir_from_db_url("sqlite:///opt/presenter/presenter.db"),
            PathBuf::from("/opt/presenter/stream-assets")
        );
        assert_eq!(
            stream_assets_dir_from_db_url("sqlite://presenter_dev.db"),
            PathBuf::from("./stream-assets")
        );
        assert_eq!(
            stream_assets_dir_from_db_url("sqlite::memory:?cache=shared"),
            PathBuf::from("./stream-assets")
        );
        assert_eq!(
            stream_assets_dir_from_db_url(""),
            PathBuf::from("./stream-assets")
        );
    }
}
