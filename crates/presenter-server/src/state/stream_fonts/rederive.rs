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

use super::weight::family_weights;
use super::{parse_font_metadata, FontMeta};
use crate::state::AppState;

impl AppState {
    /// Re-derive every stored family (startup). Returns how many faces changed.
    pub(crate) async fn rederive_stream_font_faces(&self) -> anyhow::Result<usize> {
        let mut changed = 0;
        for family in self.repository().distinct_font_families().await? {
            changed += self.rederive_stream_font_family(&family).await?;
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
    /// row (logged). Returns how many faces changed.
    pub(crate) async fn rederive_stream_font_family(&self, family: &str) -> anyhow::Result<usize> {
        let _serial = self.stream_font_rederive_lock.lock().await;
        let faces = self.repository().stream_fonts_of_family(family).await?;
        let derived = self.derive_stored_faces(faces).await;
        let inputs: Vec<_> = derived.iter().map(|(_, meta)| meta.face_weight()).collect();
        let mut changed = 0;
        for ((font, meta), weight) in derived.iter().zip(family_weights(&inputs)) {
            if font.weight == weight && font.italic == meta.italic {
                continue;
            }
            self.repository()
                .update_stream_font_face(font.id, weight, meta.italic)
                .await?;
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
            if self.stream_font_verdicts.face(&font.sha256).is_none() {
                self.evaluate_face(&font, &bytes);
            }
            match parse_font_metadata(&bytes, &font.original_filename) {
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
