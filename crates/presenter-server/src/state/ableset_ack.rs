//! AbleSet<->Presenter mismatch acknowledgement store (#601), split out of
//! `ableset_mismatch.rs` (#655) — pure code motion in this commit, no
//! behavior change. F4's domain refusal enum and F9's mutex/prune/cap/
//! validation land in a follow-up commit on this same file, which is why
//! the split happens first: it keeps that follow-up diff readable instead
//! of interleaving "moved" and "changed" lines.

use anyhow::Context;
use std::collections::HashMap;
use tracing::warn;

use super::AppState;
use crate::ableset::AbleSetStatusSnapshot;

const ABLESET_MISMATCH_ACK_SETTING_KEY: &str = "ableset_mismatch_acks";

/// An operator's explicit "yes, these two titles are the same song"
/// acknowledgement for one song number. The settled design rejected a
/// similarity threshold (a genuinely wrong pair could easily look similar,
/// and a deliberate variant like "Alive with you KIDS" can look very
/// different) — only an explicit human call is safe here. Bound to the
/// EXACT title pair it was granted for: if either side's title later
/// changes, the ack no longer matches and the warning returns (a later
/// renumbering must never silently inherit an old "this is fine").
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbleSetMismatchAck {
    pub(crate) ableset_title: String,
    pub(crate) presenter_title: String,
}

pub(crate) type AckMap = HashMap<String, AbleSetMismatchAck>;

impl AppState {
    /// Load the persisted acknowledgement map from the generic `app_settings`
    /// key-value store — no schema migration needed, this is a JSON blob
    /// under one well-known key. A corrupt/unparseable blob degrades to
    /// "no acknowledgements" (loud rebuild warnings) rather than failing the
    /// whole cache rebuild.
    pub(super) async fn load_ableset_mismatch_acks(&self) -> anyhow::Result<AckMap> {
        let Some(raw) = self
            .repository
            .get_app_setting(ABLESET_MISMATCH_ACK_SETTING_KEY)
            .await?
        else {
            return Ok(AckMap::new());
        };
        Ok(serde_json::from_str(&raw).unwrap_or_else(|err| {
            warn!(
                ?err,
                "corrupt AbleSet mismatch acknowledgement store — treating as empty (#601)"
            );
            AckMap::new()
        }))
    }

    async fn save_ableset_mismatch_acks(&self, acks: &AckMap) -> anyhow::Result<()> {
        let raw = serde_json::to_string(acks)
            .context("failed to serialize AbleSet mismatch acknowledgements")?;
        self.repository
            .set_app_setting(ABLESET_MISMATCH_ACK_SETTING_KEY, &raw)
            .await
    }

    async fn refresh_current_ableset_cache(&self) -> anyhow::Result<()> {
        let settings = self.ableset_bridge.status_snapshot().await;
        self.refresh_ableset_cache(&settings).await
    }

    /// Record (or overwrite) the operator's acknowledgement that `number`'s
    /// two CURRENT titles are deliberately different names for the same
    /// song, then rebuild the cache immediately so the mismatch report drops
    /// it right away rather than waiting for the next unrelated rebuild.
    pub async fn acknowledge_ableset_mismatch(
        &self,
        number: &str,
        ableset_title: &str,
        presenter_title: &str,
    ) -> anyhow::Result<AbleSetStatusSnapshot> {
        let mut acks = self.load_ableset_mismatch_acks().await.unwrap_or_default();
        acks.insert(
            number.to_string(),
            AbleSetMismatchAck {
                ableset_title: ableset_title.to_string(),
                presenter_title: presenter_title.to_string(),
            },
        );
        self.save_ableset_mismatch_acks(&acks).await?;
        self.refresh_current_ableset_cache().await?;
        Ok(self.ableset_status_snapshot().await)
    }

    /// Revoke a prior acknowledgement (the report is "visible/revocable" per
    /// the settled design) — the warning returns on the immediate rebuild
    /// triggered here if the titles still disagree.
    pub async fn unacknowledge_ableset_mismatch(
        &self,
        number: &str,
    ) -> anyhow::Result<AbleSetStatusSnapshot> {
        let mut acks = self.load_ableset_mismatch_acks().await.unwrap_or_default();
        acks.remove(number);
        self.save_ableset_mismatch_acks(&acks).await?;
        self.refresh_current_ableset_cache().await?;
        Ok(self.ableset_status_snapshot().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // #655 F9d — RED (this commit): `validate_ableset_ack_number` does not
    // exist yet, so this fails to compile. GREEN adds it and wires it into
    // `acknowledge_ableset_mismatch`/`unacknowledge_ableset_mismatch`.

    #[test]
    fn validate_ableset_ack_number_accepts_exact_length_all_digits() {
        assert!(validate_ableset_ack_number("017", 3).is_ok());
    }

    #[test]
    fn validate_ableset_ack_number_rejects_empty() {
        assert!(validate_ableset_ack_number("", 3).is_err());
    }

    #[test]
    fn validate_ableset_ack_number_rejects_wrong_length() {
        assert!(validate_ableset_ack_number("17", 3).is_err());
        assert!(validate_ableset_ack_number("0017", 3).is_err());
    }

    #[test]
    fn validate_ableset_ack_number_rejects_non_digits() {
        assert!(validate_ableset_ack_number("01a", 3).is_err());
    }
}
