//! Re-derive stored faces' weight + italic from their bytes (#830).
//!
//! Faces uploaded before #830 were stored with the OS/2 weight alone, so
//! Nexa's Light, Heavy, Black and XBold sit at 400 on SNV/PP. This pass
//! re-reads each face's bytes, derives the weight/italic the way an upload
//! now does ([`parse_font_metadata`] + [`family_weights`]) and updates the
//! rows that differ. It needs no schema change, is idempotent, and logs every
//! change. It runs once at startup for every family, and for one family after
//! each upload, since a new Black face moves a stored Heavy below it.
//!
//! One lock serialises the passes: two uploads of the same family each
//! re-derive it, and without the lock the older pass could write a weight it
//! computed before the newer face existed.

use presenter_core::stream::StreamFont;

use super::weight::{colliding_styles, family_weights};
use super::{parse_font_metadata, FontMeta};
use crate::state::AppState;

impl AppState {
    /// Re-derive every stored family (startup). A family whose pass fails is
    /// logged and skipped, so one bad family never holds back the others.
    /// Returns how many faces changed.
    pub(crate) async fn rederive_stream_font_faces(&self) -> anyhow::Result<usize> {
        let mut changed = 0;
        for family in self.repository().distinct_font_families().await? {
            match self.rederive_stream_font_family(&family).await {
                Ok(count) => changed += count,
                Err(e) => tracing::warn!(
                    family = %family,
                    error = %e,
                    "stream-font family re-derive failed — its stored weights are unchanged"
                ),
            }
        }
        Ok(changed)
    }

    /// [`AppState::rederive_stream_font_faces`], logged: one INFO line with the
    /// count, or a WARN when the pass failed (the next startup retries it).
    pub(crate) async fn rederive_stream_font_faces_at_startup(&self) {
        let started = std::time::Instant::now();
        match self.rederive_stream_font_faces().await {
            Ok(changed) => tracing::info!(
                changed,
                elapsed = ?started.elapsed(),
                "stream-font faces re-derived from their style names"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                "stream-font re-derive failed — stored weights unchanged, retried on the next start"
            ),
        }
    }

    /// Re-derive one family's faces from their bytes and update the rows that
    /// differ. A face whose file is missing, unreadable or unparseable keeps its
    /// row (logged), and so does one deleted while the pass runs. Returns how
    /// many faces changed.
    pub(crate) async fn rederive_stream_font_family(&self, family: &str) -> anyhow::Result<usize> {
        let _serial = self.stream_font_rederive_lock.lock().await;
        let faces = self.repository().stream_fonts_of_family(family).await?;
        let derived = self.derive_stored_faces(faces).await;
        let inputs: Vec<_> = derived.iter().map(|(_, meta)| meta.face_weight()).collect();
        let weights = family_weights(&inputs);
        warn_on_collisions(family, &derived, &weights);
        let mut changed = 0;
        for ((font, meta), &weight) in derived.iter().zip(&weights) {
            if font.weight == weight && font.italic == meta.italic {
                continue;
            }
            let updated = self
                .repository()
                .update_stream_font_face(font.id, weight, meta.italic)
                .await?;
            if !updated {
                tracing::debug!(
                    font_id = font.id,
                    "stream font face deleted during its re-derive — skipped"
                );
                continue;
            }
            tracing::info!(
                font_id = font.id,
                family = %font.family,
                file = %font.original_filename,
                style = meta.style_name.as_deref().unwrap_or("-"),
                from_weight = font.weight,
                to_weight = weight,
                from_italic = font.italic,
                to_italic = meta.italic,
                "stream font face re-derived from its style name"
            );
            changed += 1;
        }
        Ok(changed)
    }

    /// Read + parse each stored face. The bytes also warm the verdict cache
    /// (the list/css read the same files right after the startup pass).
    async fn derive_stored_faces(&self, faces: Vec<StreamFont>) -> Vec<(StreamFont, FontMeta)> {
        let store = self.font_store();
        let mut derived = Vec::with_capacity(faces.len());
        for font in faces {
            let Some(bytes) = self.read_stored_font(&store, &font).await else {
                continue;
            };
            let parsed = parse_font_metadata(&bytes, &font.original_filename);
            if self.stream_font_verdicts.face(&font.sha256).is_none() {
                let style_name = parsed
                    .as_ref()
                    .ok()
                    .and_then(|meta| meta.style_name.clone());
                self.evaluate_face(&font, &bytes, style_name);
            }
            match parsed {
                Ok(meta) => derived.push((font, meta)),
                Err(e) => tracing::warn!(
                    font_id = font.id,
                    family = %font.family,
                    error = %e,
                    "stored font metadata unreadable — its weight is not re-derived"
                ),
            }
        }
        derived
    }
}

/// WARN when faces of `family` still share a weight + style after the
/// re-derive: `/stream/fonts.css` then serves only one of each pair.
fn warn_on_collisions(family: &str, derived: &[(StreamFont, FontMeta)], weights: &[u16]) {
    let styles: Vec<(u16, bool)> = derived
        .iter()
        .zip(weights)
        .map(|((_, meta), &weight)| (weight, meta.italic))
        .collect();
    let collisions = colliding_styles(&styles);
    if !collisions.is_empty() {
        tracing::warn!(
            family,
            ?collisions,
            "stream font faces still share a weight and style — fonts.css serves only one of each"
        );
    }
}
