//! Stream-graphics font STORAGE + metadata parsing (#778, epic #718).
//!
//! The metadata ROW is the `stream_fonts` repository; this module owns
//! everything AROUND it: magic-byte format detection (only raw `ttf`/`otf` — a
//! browser cannot `@font-face` a `.ttc` collection and `ttf-parser` cannot read
//! `woff`/`woff2`, so those are rejected 422), family/weight/italic parsing from
//! the font's `name`/`OS/2` tables via `ttf-parser`, and content-addressed byte
//! files under `<stream-assets>/fonts/<sha256>.<ext>` — a `fonts/` subdir of the
//! image asset dir, so it inherits the deploy-survival of `stream-assets/` (the
//! deploy `rsync --delete` is scoped to `libraries/`, never this tree).
//!
//! Path-traversal safety mirrors `stream_assets.rs`: the on-disk name is a
//! stored sha256 (hex WE computed) + a whitelisted ext, never client input.
//! The sha/atomic-write primitives are REUSED from `stream_assets` (no copy).

use std::path::{Path, PathBuf};

use crate::state::stream_assets::{
    is_valid_sha256, read_content, remove_content, store_content_addressed, sweep_tmp_dir,
};
use crate::state::AppState;

/// Business cap on a single uploaded font file (5 MiB). A ttf/otf face is well
/// under this; the route's `DefaultBodyLimit` sits a little higher (a DoS
/// ceiling on the raw multipart body), this is the precise user-facing `413`.
pub(crate) const MAX_FONT_BYTES: usize = 5 * 1024 * 1024;

/// An accepted font container, detected by MAGIC BYTES (never the client
/// content-type). `ext` names the on-disk file + the DB `format` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetectedFont {
    pub ext: &'static str,
}

/// Detect a raw TrueType/OpenType font from the leading 4-byte sfnt version
/// tag. Returns `None` for anything else — `.ttc` collections (`ttcf`),
/// `woff`/`woff2` (`wOFF`/`wOF2`), Type1, etc. — which the caller maps to `422`.
pub(crate) fn detect_font(bytes: &[u8]) -> Option<DetectedFont> {
    if bytes.len() < 4 {
        return None;
    }
    let sig = [bytes[0], bytes[1], bytes[2], bytes[3]];
    match sig {
        // 0x00010000 — TrueType outlines (the common .ttf sfnt version).
        [0x00, 0x01, 0x00, 0x00] => Some(DetectedFont { ext: "ttf" }),
        // "true" — legacy Apple TrueType.
        [0x74, 0x72, 0x75, 0x65] => Some(DetectedFont { ext: "ttf" }),
        // "OTTO" — OpenType with CFF outlines (.otf).
        [0x4F, 0x54, 0x54, 0x4F] => Some(DetectedFont { ext: "otf" }),
        _ => None,
    }
}

/// A face's parsed metadata: the family name shown in the picker + the weight
/// and italic flag used to build the `@font-face` rule and limit the editor's
/// weight options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FontMeta {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
}

/// Why a font's metadata could not be read — mapped to `422` by the router.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FontParseError {
    #[error("unsupported or corrupt font file")]
    Unparseable,
    #[error("font has no usable family name")]
    NoFamily,
}

/// Parse family / weight / italic from a raw ttf/otf byte buffer. Family is the
/// typographic family (name id 16) falling back to the legacy family (id 1);
/// weight is `OS/2.usWeightClass` (clamped into the valid 1..=1000 range);
/// italic is the `fsSelection`/`macStyle` italic bit.
pub(crate) fn parse_font_metadata(bytes: &[u8]) -> Result<FontMeta, FontParseError> {
    let face = ttf_parser::Face::parse(bytes, 0).map_err(|_| FontParseError::Unparseable)?;
    let family = pick_family(&face).ok_or(FontParseError::NoFamily)?;
    let weight = face.weight().to_number().clamp(1, 1000);
    let italic = face.is_italic();
    Ok(FontMeta {
        family,
        weight,
        italic,
    })
}

/// Preferred family name: typographic family (16) → legacy family (1), first
/// non-empty Unicode name record.
fn pick_family(face: &ttf_parser::Face) -> Option<String> {
    name_record(face, ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
        .or_else(|| name_record(face, ttf_parser::name_id::FAMILY))
}

/// First non-empty Unicode name-table string for `want`. Iterated by index
/// (`Names::len`/`get`) — the stable API surface — filtering to Unicode records
/// (`to_string` only decodes those).
fn name_record(face: &ttf_parser::Face, want: u16) -> Option<String> {
    let names = face.names();
    for i in 0..names.len() {
        let Some(name) = names.get(i) else {
            continue;
        };
        if name.name_id != want || !name.is_unicode() {
            continue;
        }
        if let Some(s) = name.to_string() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// The content-addressed font-file store over the `fonts/` directory. A thin
/// sibling of [`crate::state::stream_assets::AssetStore`] that REUSES the same
/// sha/atomic-write primitives, differing only in the `ttf`/`otf` ext whitelist.
#[derive(Clone)]
pub(crate) struct FontStore {
    dir: PathBuf,
}

impl FontStore {
    pub(crate) fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    #[cfg(test)]
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) async fn ensure_dir(&self) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.dir).await
    }

    /// Absolute path for a stored font, or `None` when the name is not a bare
    /// sha256 + whitelisted ext (the traversal guard — never joins client
    /// segments).
    pub(crate) fn path_for(&self, sha256: &str, ext: &str) -> Option<PathBuf> {
        if !is_valid_sha256(sha256) {
            return None;
        }
        if !matches!(ext, "ttf" | "otf") {
            return None;
        }
        Some(self.dir.join(format!("{sha256}.{ext}")))
    }

    pub(crate) async fn store(
        &self,
        sha256: &str,
        ext: &str,
        bytes: &[u8],
    ) -> std::io::Result<PathBuf> {
        let final_path = self.path_for(sha256, ext).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid font name")
        })?;
        store_content_addressed(&self.dir, &final_path, bytes).await
    }

    pub(crate) async fn read(&self, sha256: &str, ext: &str) -> std::io::Result<Option<Vec<u8>>> {
        let Some(path) = self.path_for(sha256, ext) else {
            return Ok(None);
        };
        read_content(&path).await
    }

    pub(crate) async fn remove(&self, sha256: &str, ext: &str) -> std::io::Result<()> {
        let Some(path) = self.path_for(sha256, ext) else {
            return Ok(());
        };
        remove_content(&path).await
    }

    pub(crate) async fn sweep_tmp(&self) -> std::io::Result<()> {
        sweep_tmp_dir(&self.dir).await
    }
}

impl AppState {
    /// A [`FontStore`] over the `fonts/` subdir of the stream-assets dir.
    pub(crate) fn font_store(&self) -> FontStore {
        FontStore::new(self.stream_assets_dir.join("fonts"))
    }

    /// Ensure the stream-fonts directory exists and clear any stale upload tmp
    /// files (startup hook, idempotent, logged). `pub` for the same reason as
    /// [`AppState::ensure_stream_assets_dir`]: `main.rs` is a separate crate root.
    pub async fn ensure_stream_fonts_dir(&self) -> std::io::Result<()> {
        let store = self.font_store();
        tracing::info!(dir = %store.dir.display(), "ensuring stream-fonts directory exists");
        store.ensure_dir().await?;
        if let Err(e) = store.sweep_tmp().await {
            tracing::warn!(error = %e, "stream-fonts tmp sweep failed (non-fatal)");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_font_accepts_ttf_otf_and_rejects_others() {
        assert_eq!(
            detect_font(&[0x00, 0x01, 0x00, 0x00, 0, 0]).unwrap().ext,
            "ttf"
        );
        assert_eq!(detect_font(b"true....").unwrap().ext, "ttf");
        assert_eq!(detect_font(b"OTTO....").unwrap().ext, "otf");
        // Rejected: TrueType collection, woff, woff2, garbage, too short.
        assert!(detect_font(b"ttcf....").is_none());
        assert!(detect_font(b"wOFF....").is_none());
        assert!(detect_font(b"wOF2....").is_none());
        assert!(detect_font(b"not a font").is_none());
        assert!(detect_font(&[0x00, 0x01]).is_none());
    }

    #[test]
    fn path_for_rejects_traversal_and_bad_ext() {
        let store = FontStore::new("/tmp/does-not-matter");
        let good = "a".repeat(64);
        assert!(store.path_for(&good, "ttf").is_some());
        assert!(store.path_for(&good, "otf").is_some());
        assert!(store.path_for("../../etc/passwd", "ttf").is_none());
        assert!(store.path_for(&good, "woff2").is_none());
        assert!(store.path_for(&good, "exe").is_none());
        let p = store.path_for(&good, "ttf").unwrap();
        assert_eq!(p.parent().unwrap(), Path::new("/tmp/does-not-matter"));
    }

    #[test]
    fn parse_font_metadata_reads_the_ofl_fixture() {
        // The committed OFL fixture font (tests/e2e/fixtures) doubles as the Rust
        // metadata-parse fixture — one licence-clean font for both test layers.
        let bytes = include_bytes!("../../../../tests/e2e/fixtures/fonts/Gruppo-Regular.ttf");
        let meta = parse_font_metadata(bytes).expect("fixture parses");
        assert_eq!(meta.family, "Gruppo", "fixture family parsed: {meta:?}");
        assert!(
            !meta.family.trim().is_empty(),
            "fixture has a family name: {meta:?}"
        );
        assert!(
            (1..=1000).contains(&meta.weight),
            "weight in range: {}",
            meta.weight
        );
    }

    #[test]
    fn parse_font_metadata_rejects_non_font_bytes() {
        assert!(parse_font_metadata(b"this is definitely not a font").is_err());
    }

    #[tokio::test]
    async fn store_writes_reads_dedups_and_removes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FontStore::new(tmp.path().join("fonts"));
        let bytes = b"\x00\x01\x00\x00 fake ttf body".to_vec();
        let sha = crate::state::stream_assets::sha256_hex(&bytes);

        let p1 = store.store(&sha, "ttf", &bytes).await.unwrap();
        assert!(p1.exists());
        assert_eq!(store.read(&sha, "ttf").await.unwrap(), Some(bytes.clone()));
        // Dedup: identical bytes → same path, one file.
        let p2 = store.store(&sha, "ttf", &bytes).await.unwrap();
        assert_eq!(p1, p2);
        let count = std::fs::read_dir(store.dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".ttf"))
            .count();
        assert_eq!(count, 1);
        store.remove(&sha, "ttf").await.unwrap();
        assert_eq!(store.read(&sha, "ttf").await.unwrap(), None);
        store.remove(&sha, "ttf").await.unwrap(); // idempotent
    }
}
