//! Stream-graphics font STORAGE + metadata parsing (#778, epic #718).
//!
//! The metadata ROW is the `stream_fonts` repository; this module owns
//! everything AROUND it: magic-byte format detection (only raw `ttf`/`otf` — a
//! browser cannot `@font-face` a `.ttc` collection and the parser cannot read
//! `woff`/`woff2`, so those are rejected 422), family/weight/italic parsing from
//! the font's `name`/`OS/2` tables via `read-fonts` (Google fontations), and
//! content-addressed byte
//! files under `<stream-assets>/fonts/<sha256>.<ext>` — a `fonts/` subdir of the
//! image asset dir, so it inherits the deploy-survival of `stream-assets/` (the
//! deploy `rsync --delete` is scoped to `libraries/`, never this tree).
//!
//! Path-traversal safety mirrors `stream_assets.rs`: the on-disk name is a
//! stored sha256 (hex WE computed) + a whitelisted ext, never client input.
//! The sha/atomic-write primitives are REUSED from `stream_assets` (no copy).
//!
//! Browser loadability (#778 reopen): [`check_browser_loadable`] mirrors the
//! OTS hard-fails browsers apply to every web font. The upload refuses a font
//! failing it, and [`AppState::loadable_stream_fonts`] hides an already-stored
//! one from the font list + `fonts.css` (verdict cached per content sha256,
//! nothing deleted).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
// `Path` is named only by the `#[cfg(test)]` `dir()` accessor + the test module,
// so a module-level import would be `unused` in the non-test build (`-D warnings`
// on the clippy job — the #616 test-only-import class).
#[cfg(test)]
use std::path::Path;

use presenter_core::stream::StreamFont;
use read_fonts::tables::head::MacStyle;
use read_fonts::tables::os2::SelectionFlags;
use read_fonts::types::NameId;
use read_fonts::{FontRef, TableProvider};

use crate::state::stream_assets::{
    is_valid_sha256, read_content, remove_content, store_content_addressed, sweep_tmp_dir,
};
use crate::state::AppState;

mod browser_check;
pub(crate) use browser_check::check_browser_loadable;

mod weight;

/// Test-only sfnt byte surgery deriving the broken-font fixtures from the OFL
/// fixture (#778 browser-sanitiser check) — shared by the unit + router tests.
#[cfg(test)]
pub(crate) mod test_fonts;

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
/// weight is `OS/2.usWeightClass` (clamped into the valid 1..=1000 range,
/// defaulting to 400 when the `OS/2` table is absent); italic is the `OS/2`
/// `fsSelection` italic bit, falling back to `head.macStyle` when there is no
/// `OS/2` table.
pub(crate) fn parse_font_metadata(bytes: &[u8]) -> Result<FontMeta, FontParseError> {
    let font = FontRef::new(bytes).map_err(|_| FontParseError::Unparseable)?;
    let family = pick_family(&font).ok_or(FontParseError::NoFamily)?;
    let weight = font
        .os2()
        .map(|os2| os2.us_weight_class())
        .unwrap_or(400)
        .clamp(1, 1000);
    let italic = match font.os2() {
        Ok(os2) => os2.fs_selection().contains(SelectionFlags::ITALIC),
        Err(_) => font
            .head()
            .map(|head| head.mac_style().contains(MacStyle::ITALIC))
            .unwrap_or(false),
    };
    Ok(FontMeta {
        family,
        weight,
        italic,
    })
}

/// Preferred family name: typographic family (16) → legacy family (1), first
/// non-empty Unicode/Windows name record.
fn pick_family(font: &FontRef) -> Option<String> {
    name_record(font, NameId::TYPOGRAPHIC_FAMILY_NAME)
        .or_else(|| name_record(font, NameId::FAMILY_NAME))
}

/// First non-empty name-table string for `want`. Iterates the `name` table's
/// records, keeping only Unicode (platform 0) / Windows (platform 3) records —
/// the UTF-16 strings `read-fonts` decodes via `NameString`'s `Display` impl.
fn name_record(font: &FontRef, want: NameId) -> Option<String> {
    let name = font.name().ok()?;
    let data = name.string_data();
    for record in name.name_record() {
        if record.name_id() != want || !matches!(record.platform_id(), 0 | 3) {
            continue;
        }
        let Ok(decoded) = record.string(data) else {
            continue;
        };
        let s = decoded.to_string();
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
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

    /// The stored faces a browser will actually load (#778 reopen): every
    /// `stream_fonts` row minus faces whose bytes fail
    /// [`check_browser_loadable`] or whose file is missing. Feeds BOTH
    /// `GET /stream/api/fonts` (editor picker, font panel, output font-gate
    /// preload) and `GET /stream/fonts.css`, so no page ever fetches a face the
    /// browser's OpenType sanitiser refuses (the console warning SNV/PP logged
    /// for font id 71). Nothing is deleted: the row and the file stay, and an
    /// explicit `DELETE /stream/fonts/{id}` still works.
    pub(crate) async fn loadable_stream_fonts(&self) -> anyhow::Result<Vec<StreamFont>> {
        let fonts = self.repository().list_stream_fonts().await?;
        let store = self.font_store();
        let mut loadable = Vec::with_capacity(fonts.len());
        for font in fonts {
            if self.stored_font_is_loadable(&store, &font).await {
                loadable.push(font);
            }
        }
        Ok(loadable)
    }

    /// Compute every stored face's verdict now (startup warm-up), so the first
    /// `fonts.css` / font list after a restart does not read every stored font
    /// file inline (~72 MB / 397 files on SNV) — possibly past the output
    /// page's 2 s font-wait. Logged; a failure only means the verdicts are
    /// computed on the first request instead.
    pub(crate) async fn warm_stream_font_verdicts(&self) {
        let started = std::time::Instant::now();
        match self.loadable_stream_fonts().await {
            Ok(loadable) => tracing::info!(
                loadable = loadable.len(),
                elapsed = ?started.elapsed(),
                "stream-font browser-loadability verdicts warmed"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                "stream-font verdict warm-up failed — verdicts are computed on first request"
            ),
        }
    }

    /// Run [`AppState::warm_stream_font_verdicts`] in the background (startup,
    /// off the request path). A background task, so it is skipped in validate
    /// (schema-probe) mode like every other one (#771). `pub` because `main.rs`
    /// is a separate crate root (a `pub(crate)` fn called only from there is
    /// dead code to clippy).
    pub fn spawn_stream_font_verdict_warmup(&self) {
        if !self.startup_mode().starts_integrations() {
            tracing::info!("validate startup mode — stream-font verdict warm-up skipped");
            return;
        }
        let state = self.clone();
        tokio::spawn(async move { state.warm_stream_font_verdicts().await });
    }

    /// Cached browser-loadability verdict for one stored face. The verdict is a
    /// pure function of the bytes, which are immutable per sha256, so it is
    /// computed once per process (and logged once when negative). A missing or
    /// unreadable file hides the face without caching, so it reappears as soon
    /// as the file is back.
    async fn stored_font_is_loadable(&self, store: &FontStore, font: &StreamFont) -> bool {
        if let Some(verdict) = self.stream_font_verdicts.get(&font.sha256) {
            return verdict;
        }
        let bytes = match store.read(&font.sha256, &font.format).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                self.report_font_file_problem(font, "missing");
                return false;
            }
            Err(e) => {
                self.report_font_file_problem(font, &format!("unreadable: {e}"));
                return false;
            }
        };
        let loadable = match check_browser_loadable(&bytes) {
            Ok(()) => true,
            Err(defect) => {
                tracing::warn!(
                    font_id = font.id,
                    family = %font.family,
                    sha256 = %font.sha256,
                    %defect,
                    "stored font would be refused by the browser's OpenType sanitiser — \
                     face hidden from fonts.css and the font list (row and file kept)"
                );
                false
            }
        };
        self.stream_font_verdicts
            .record(font.sha256.clone(), loadable);
        loadable
    }

    /// Log a stored face whose file is missing/unreadable: WARN the first time
    /// per sha256, DEBUG after. Such a face is re-checked on EVERY list/css
    /// request (it is not cached, so it comes back with its file), and a WARN
    /// per request would flood the journal (the #484 log-flood rule).
    fn report_font_file_problem(&self, font: &StreamFont, problem: &str) {
        if self.stream_font_verdicts.first_file_problem(&font.sha256) {
            tracing::warn!(
                font_id = font.id,
                family = %font.family,
                sha256 = %font.sha256,
                problem,
                "stored font file unavailable — face hidden from fonts.css and the \
                 font list (warned once per font)"
            );
        } else {
            tracing::debug!(
                font_id = font.id,
                sha256 = %font.sha256,
                problem,
                "stored font file still unavailable — face hidden"
            );
        }
    }
}

/// Per-process cache of [`check_browser_loadable`] verdicts for STORED fonts,
/// keyed by content sha256 (#778 reopen). One instance per `AppState`, shared
/// by every clone through an `Arc` (the `ai_health_cache` pattern). The lock is
/// held only for a map lookup/insert, never across an await; a poisoned lock
/// degrades to a cache miss (the verdict is recomputed), never a panic.
#[derive(Debug, Default)]
pub(crate) struct FontVerdictCache {
    verdicts: Mutex<HashMap<String, bool>>,
    /// Shas whose missing/unreadable file was already WARN-logged.
    file_problem_warned: Mutex<HashSet<String>>,
}

impl FontVerdictCache {
    fn get(&self, sha256: &str) -> Option<bool> {
        self.verdicts.lock().ok()?.get(sha256).copied()
    }

    fn record(&self, sha256: String, loadable: bool) {
        if let Ok(mut verdicts) = self.verdicts.lock() {
            verdicts.insert(sha256, loadable);
        }
    }

    /// `true` only the first time a file problem is reported for `sha256` (a
    /// poisoned lock reports `true`: an extra WARN beats a silent one).
    fn first_file_problem(&self, sha256: &str) -> bool {
        match self.file_problem_warned.lock() {
            Ok(mut warned) => warned.insert(sha256.to_string()),
            Err(_) => true,
        }
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
        let meta = parse_font_metadata(bytes, "Gruppo-Regular.ttf").expect("fixture parses");
        assert_eq!(meta.family, "Gruppo", "fixture family parsed: {meta:?}");
        assert_eq!(meta.weight, 400, "OS/2 400 + style Regular: {meta:?}");
        assert_eq!(meta.style_name.as_deref(), Some("Regular"));
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
        assert!(parse_font_metadata(b"this is definitely not a font", "x.ttf").is_err());
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

    #[tokio::test]
    async fn loadable_verdict_is_computed_once_per_content_sha() {
        use crate::state::stream_fonts::test_fonts::{with_os2_version, GRUPPO_TTF};
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();
        let mut state = AppState::in_memory().await.unwrap();
        state.set_stream_assets_dir(tmp.path().join("stream-assets"));
        // The font-id-71 shape + 8 trailing bytes: a sha (and dedup row) unique
        // to this test in the process-wide shared in-memory DB.
        let mut broken = with_os2_version(GRUPPO_TTF, 5);
        broken.extend_from_slice(&[0; 8]);
        let sha = crate::state::stream_assets::sha256_hex(&broken);
        let path = state
            .font_store()
            .store(&sha, "ttf", &broken)
            .await
            .unwrap();
        let font = state
            .repository()
            .insert_or_get_stream_font(presenter_persistence::NewStreamFont {
                sha256: sha,
                original_filename: "CachedVerdict778.ttf".to_string(),
                family: "CachedVerdictFamily778".to_string(),
                weight: 400,
                italic: false,
                format: "ttf".to_string(),
                size_bytes: broken.len() as i64,
            })
            .await
            .unwrap();
        let listed = |fonts: Vec<StreamFont>| fonts.iter().any(|f| f.id == font.id);

        assert!(
            !listed(state.loadable_stream_fonts().await.unwrap()),
            "a face the browser refuses is hidden"
        );
        // Swap valid bytes in under the same name: the verdict is a pure
        // function of the (immutable-per-sha) content, so it is NOT re-read.
        std::fs::write(&path, GRUPPO_TTF).unwrap();
        assert!(
            !listed(state.loadable_stream_fonts().await.unwrap()),
            "the cached verdict holds for the process (no per-request re-check)"
        );
        // A fresh, empty verdict cache over the SAME database (what a new
        // process sees) evaluates the stored bytes again.
        let mut fresh = state.clone();
        fresh.stream_font_verdicts = Arc::new(FontVerdictCache::default());
        assert!(
            listed(fresh.loadable_stream_fonts().await.unwrap()),
            "a fresh cache re-evaluates the stored bytes"
        );
    }

    #[tokio::test]
    async fn warm_up_caches_stored_verdicts_before_any_request() {
        use crate::state::stream_fonts::test_fonts::{with_os2_version, GRUPPO_TTF};
        let tmp = tempfile::tempdir().unwrap();
        let mut state = AppState::in_memory().await.unwrap();
        state.set_stream_assets_dir(tmp.path().join("stream-assets"));
        // The font-id-71 shape + 12 trailing bytes: unique sha in the shared DB.
        let mut broken = with_os2_version(GRUPPO_TTF, 5);
        broken.extend_from_slice(&[0; 12]);
        let sha = crate::state::stream_assets::sha256_hex(&broken);
        state
            .font_store()
            .store(&sha, "ttf", &broken)
            .await
            .unwrap();
        state
            .repository()
            .insert_or_get_stream_font(presenter_persistence::NewStreamFont {
                sha256: sha.clone(),
                original_filename: "WarmUp778.ttf".to_string(),
                family: "WarmUpFamily778".to_string(),
                weight: 400,
                italic: false,
                format: "ttf".to_string(),
                size_bytes: broken.len() as i64,
            })
            .await
            .unwrap();
        assert_eq!(state.stream_font_verdicts.get(&sha), None, "cold cache");

        state.warm_stream_font_verdicts().await;
        assert_eq!(
            state.stream_font_verdicts.get(&sha),
            Some(false),
            "the warm-up recorded the refused face's verdict"
        );
    }

    #[test]
    fn file_problem_is_reported_once_per_sha() {
        let cache = FontVerdictCache::default();
        assert!(cache.first_file_problem("aa"), "first report warns");
        assert!(!cache.first_file_problem("aa"), "repeat stays quiet");
        assert!(cache.first_file_problem("bb"), "another font warns");
    }

    // ---- #830: weight + italic from the style name -------------------------

    const SUBFAMILY: u16 = 2;
    const FULL_NAME: u16 = 4;
    const TYPO_FAMILY: u16 = 16;
    const TYPO_SUBFAMILY: u16 = 17;

    /// A face of `family` derived from the OFL fixture, styled `style` in name
    /// 17 while its OS/2 weight stays the default 400 — the Nexa mislabelling.
    fn mislabelled_face(family: &str, style: &str) -> Vec<u8> {
        use crate::state::stream_fonts::test_fonts::{with_names, GRUPPO_TTF};
        with_names(
            GRUPPO_TTF,
            &[(TYPO_FAMILY, family), (TYPO_SUBFAMILY, style)],
        )
    }

    #[test]
    fn parse_font_metadata_reads_the_weight_from_a_mislabelled_style_name() {
        for (style, weight) in [
            ("Light", 300),
            ("XBold", 800),
            ("Heavy", 900),
            ("Black", 900),
        ] {
            let meta = parse_font_metadata(&mislabelled_face("Parse830", style), "upload.ttf")
                .expect("derived face parses");
            assert_eq!(meta.family, "Parse830", "{style}");
            assert_eq!(meta.weight, weight, "{style}");
            assert_eq!(meta.style_name.as_deref(), Some(style));
        }
    }

    #[test]
    fn parse_font_metadata_leaves_an_os2_correct_face_alone() {
        use crate::state::stream_fonts::test_fonts::with_os2_weight;
        let bytes = with_os2_weight(&mislabelled_face("Parse830", "Black"), 300);
        let meta = parse_font_metadata(&bytes, "Parse830-Black.ttf").unwrap();
        assert_eq!(meta.weight, 300, "a non-default OS/2 weight is trusted");
    }

    #[test]
    fn parse_font_metadata_falls_back_to_the_full_name_then_the_filename() {
        use crate::state::stream_fonts::test_fonts::{with_names, GRUPPO_TTF};
        let full = with_names(
            GRUPPO_TTF,
            &[(SUBFAMILY, "Italic"), (FULL_NAME, "Gruppo Light Italic")],
        );
        assert_eq!(
            parse_font_metadata(&full, "upload.ttf").unwrap().weight,
            300
        );
        let plain = with_names(
            GRUPPO_TTF,
            &[(SUBFAMILY, "Italic"), (FULL_NAME, "Gruppo Italic")],
        );
        assert_eq!(
            parse_font_metadata(&plain, "Gruppo-Heavy-Italic.ttf")
                .unwrap()
                .weight,
            900
        );
        assert_eq!(
            parse_font_metadata(&plain, "upload.ttf").unwrap().weight,
            400
        );
    }

    #[test]
    fn parse_font_metadata_reads_italic_from_the_flags_or_the_style_name() {
        use crate::state::stream_fonts::test_fonts::{with_names, with_os2_italic, GRUPPO_TTF};
        let flagged = with_os2_italic(&mislabelled_face("Parse830", "Black Italic"));
        let meta = parse_font_metadata(&flagged, "x.ttf").unwrap();
        assert!(meta.italic, "fsSelection italic: {meta:?}");
        assert_eq!(meta.weight, 900);
        // The name says Italic while the flags do not: still an italic face.
        let named = with_names(GRUPPO_TTF, &[(SUBFAMILY, "Italic")]);
        assert!(parse_font_metadata(&named, "x.ttf").unwrap().italic);
        assert!(!parse_font_metadata(GRUPPO_TTF, "x.ttf").unwrap().italic);
    }

    #[test]
    fn derived_faces_stay_browser_loadable() {
        use crate::state::stream_fonts::test_fonts::{with_os2_italic, with_os2_weight};
        // The fixture surgery must not add an OTS defect of its own.
        let face = with_os2_italic(&with_os2_weight(
            &mislabelled_face("Parse830", "Black Italic"),
            400,
        ));
        assert_eq!(check_browser_loadable(&face), Ok(()));
    }

    async fn rederive_state() -> (AppState, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let mut state = AppState::in_memory().await.unwrap();
        state.set_stream_assets_dir(tmp.path().join("stream-assets"));
        (state, tmp)
    }

    /// Store `bytes` as a face uploaded before #830: row metadata from the
    /// OS/2 table alone (weight 400, upright), file on disk.
    async fn seed_old_face(
        state: &AppState,
        family: &str,
        style: &str,
        bytes: &[u8],
    ) -> StreamFont {
        let sha = crate::state::stream_assets::sha256_hex(bytes);
        state.font_store().store(&sha, "ttf", bytes).await.unwrap();
        state
            .repository()
            .insert_or_get_stream_font(presenter_persistence::NewStreamFont {
                sha256: sha,
                original_filename: format!("{family}-{style}.ttf"),
                family: family.to_string(),
                weight: 400,
                italic: false,
                format: "ttf".to_string(),
                size_bytes: bytes.len() as i64,
            })
            .await
            .unwrap()
    }

    async fn stored(state: &AppState, font: &StreamFont) -> (u16, bool) {
        let row = state.repository().get_stream_font(font.id).await.unwrap();
        (row.weight, row.italic)
    }

    #[tokio::test]
    async fn rederive_fixes_stored_mislabelled_faces_and_is_idempotent() {
        let (state, _tmp) = rederive_state().await;
        let family = "Rederive830";
        let light =
            seed_old_face(&state, family, "Light", &mislabelled_face(family, "Light")).await;
        let regular = seed_old_face(
            &state,
            family,
            "Regular",
            &mislabelled_face(family, "Regular"),
        )
        .await;
        // Italic only by name (flags upright): the old row says upright.
        let light_italic = seed_old_face(
            &state,
            family,
            "LightItalic",
            &mislabelled_face(family, "Light Italic"),
        )
        .await;

        let changed = state.rederive_stream_font_faces().await.unwrap();
        assert!(changed >= 2, "both mislabelled faces re-derived: {changed}");
        assert_eq!(stored(&state, &light).await, (300, false));
        assert_eq!(
            stored(&state, &regular).await,
            (400, false),
            "already right"
        );
        assert_eq!(stored(&state, &light_italic).await, (300, true));

        assert_eq!(
            state.rederive_stream_font_family(family).await.unwrap(),
            0,
            "a second pass changes nothing"
        );
        assert_eq!(stored(&state, &light).await, (300, false));
    }

    #[tokio::test]
    async fn rederive_moves_heavy_below_black_in_its_family() {
        let (state, _tmp) = rederive_state().await;
        // Nexa's shape: XBold, Heavy and Black all stored at 400.
        let nexa = "Pair830";
        let xbold = seed_old_face(&state, nexa, "XBold", &mislabelled_face(nexa, "XBold")).await;
        let heavy = seed_old_face(&state, nexa, "Heavy", &mislabelled_face(nexa, "Heavy")).await;
        let black = seed_old_face(&state, nexa, "Black", &mislabelled_face(nexa, "Black")).await;
        // Heavy + Black without an 800 face.
        let duo = "Duo830";
        let duo_heavy = seed_old_face(&state, duo, "Heavy", &mislabelled_face(duo, "Heavy")).await;
        let duo_black = seed_old_face(&state, duo, "Black", &mislabelled_face(duo, "Black")).await;

        state.rederive_stream_font_faces().await.unwrap();
        assert_eq!(stored(&state, &xbold).await.0, 800);
        assert_eq!(stored(&state, &heavy).await.0, 850, "800 is XBold's");
        assert_eq!(stored(&state, &black).await.0, 900);
        assert_eq!(stored(&state, &duo_heavy).await.0, 800);
        assert_eq!(stored(&state, &duo_black).await.0, 900);
    }

    #[tokio::test]
    async fn rederive_skips_a_face_whose_file_is_missing() {
        let (state, _tmp) = rederive_state().await;
        // A row whose bytes are not on disk (never stored): its filename names
        // a Black face, but nothing can be read, so the row stays as it is.
        let bytes = mislabelled_face("Missing830", "Black");
        let row = state
            .repository()
            .insert_or_get_stream_font(presenter_persistence::NewStreamFont {
                sha256: crate::state::stream_assets::sha256_hex(&bytes),
                original_filename: "Missing830-Black.ttf".to_string(),
                family: "Missing830".to_string(),
                weight: 400,
                italic: false,
                format: "ttf".to_string(),
                size_bytes: bytes.len() as i64,
            })
            .await
            .unwrap();
        state.rederive_stream_font_faces().await.unwrap();
        assert_eq!(stored(&state, &row).await, (400, false));
    }
}
