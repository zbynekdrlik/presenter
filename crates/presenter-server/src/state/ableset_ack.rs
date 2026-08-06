//! AbleSet<->Presenter mismatch acknowledgement store (#601), split out of
//! `ableset_mismatch.rs` (#655 — F4's domain refusal enum plus F9's mutex,
//! prune, cap, and validation grew the ack-store side enough to push both
//! concerns over the repo's per-file size cap; `ableset_mismatch.rs` keeps
//! only the pure comparison logic). This file owns everything HTTP- and
//! persistence-facing about acknowledgements.

use std::collections::HashMap;

use anyhow::Context;
use thiserror::Error;
use tracing::{info, warn};

use super::AppState;
use crate::ableset::AbleSetStatusSnapshot;

const ABLESET_MISMATCH_ACK_SETTING_KEY: &str = "ableset_mismatch_acks";

/// Cap on the number of acknowledgements retained (#655 F9c) — an unbounded
/// store on a long-running instance could otherwise grow forever across many
/// library/setlist changes with no pruning trigger.
const MAX_ABLESET_ACKS: usize = 500;

/// An operator's explicit "yes, these two titles are the same song"
/// acknowledgement for one song number. The settled design rejected a
/// similarity threshold (a genuinely wrong pair could easily look similar,
/// and a deliberate variant like "Alive with you KIDS" can look very
/// different) — only an explicit human call is safe here. Bound to the
/// title pair it was granted for (compared normalised — #655 F9e): a purely
/// cosmetic edit on either side keeps the ack valid, but a genuine title
/// change re-raises the warning.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbleSetMismatchAck {
    pub(crate) ableset_title: String,
    pub(crate) presenter_title: String,
}

pub(crate) type AckMap = HashMap<String, AbleSetMismatchAck>;

/// A `POST /integrations/ableset/mismatches/{ack,unack}` request refused for
/// a DOMAIN reason (#655 F4/F9) — never a blanket 200/500. The router
/// (`router/integrations/ableset.rs`) downcasts on this TYPED error per
/// `.claude/rules/repository-error-pattern.md`'s domain-error pattern.
#[derive(Debug, Error)]
pub(crate) enum AbleSetAckRefusal {
    /// #655 F4: a one-sided numbering gap is a structural problem, not a
    /// naming choice — it can never be silenced by an acknowledgement.
    #[error(
        "a one-sided numbering gap cannot be acknowledged — add or remove the number on the missing side"
    )]
    StructuralGap,
    /// #655 F9d: `number` must be non-empty, all ASCII digits, and exactly
    /// `song_prefix_length` digits long — anything else can never match a
    /// real mismatch entry.
    #[error(
        "acknowledgement number must be {expected} digit(s), all ASCII digits (got {actual:?})"
    )]
    InvalidNumber { expected: u8, actual: String },
    /// #655 F9c: bounds the store so a long-running instance cannot grow it
    /// unbounded across many library/setlist changes.
    #[error(
        "acknowledgement store is full (500 entries max) — remove an old acknowledgement before adding a new one"
    )]
    StoreFull,
}

/// #655 F9d: `number` must be non-empty, all ASCII digits, and exactly
/// `expected_len` digits long — the same shape `extract_song_prefix`
/// produces, so a validated number can always be looked up against both
/// sides of the mismatch report.
fn validate_ableset_ack_number(number: &str, expected_len: u8) -> Result<(), AbleSetAckRefusal> {
    let valid = !number.is_empty()
        && number.len() == expected_len as usize
        && number.chars().all(|c| c.is_ascii_digit());
    if valid {
        Ok(())
    } else {
        Err(AbleSetAckRefusal::InvalidNumber {
            expected: expected_len,
            actual: number.to_string(),
        })
    }
}

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

    pub(super) async fn save_ableset_mismatch_acks(&self, acks: &AckMap) -> anyhow::Result<()> {
        let raw = serde_json::to_string(acks)
            .context("failed to serialize AbleSet mismatch acknowledgements")?;
        self.repository
            .set_app_setting(ABLESET_MISMATCH_ACK_SETTING_KEY, &raw)
            .await
    }

    /// Record (or overwrite) the operator's acknowledgement that `number`'s
    /// two CURRENT titles are deliberately different names for the same
    /// song. #655 F9a serializes the load->mutate->save cycle behind
    /// `ableset_ack_lock` so two concurrent ack/unack calls can never lose
    /// one's update to the other. #655 F5c: rebuilds the mismatch report
    /// from the ALREADY-CACHED `presenter_titles` instead of forcing a full
    /// DB rescan on every ack click (that used to be the log-spam
    /// amplifier — an ack POST forcing a full rescan + WARN burst).
    pub async fn acknowledge_ableset_mismatch(
        &self,
        number: &str,
        ableset_title: &str,
        presenter_title: &str,
    ) -> anyhow::Result<AbleSetStatusSnapshot> {
        let song_prefix_length = self
            .ableset_bridge
            .status_snapshot()
            .await
            .song_prefix_length;
        validate_ableset_ack_number(number, song_prefix_length)?;
        // #655 F4: a one-sided gap can never be acknowledged away — it is a
        // structural problem (missing number), not a naming disagreement.
        if ableset_title.is_empty() || presenter_title.is_empty() {
            return Err(AbleSetAckRefusal::StructuralGap.into());
        }

        let _guard = self.ableset_ack_lock.lock().await;
        let mut acks = self.load_ableset_mismatch_acks().await.unwrap_or_default();
        if !acks.contains_key(number) && acks.len() >= MAX_ABLESET_ACKS {
            return Err(AbleSetAckRefusal::StoreFull.into());
        }
        acks.insert(
            number.to_string(),
            AbleSetMismatchAck {
                ableset_title: ableset_title.to_string(),
                presenter_title: presenter_title.to_string(),
            },
        );
        self.save_ableset_mismatch_acks(&acks).await?;
        self.sync_ableset_ack_cache_and_recompute(acks).await;
        Ok(self.ableset_status_snapshot().await)
    }

    /// Revoke a prior acknowledgement (the report is "visible/revocable" per
    /// the settled design) — the warning returns on the immediate lightweight
    /// recompute triggered here if the titles still disagree. Same
    /// mutex/no-full-rescan treatment as `acknowledge_ableset_mismatch`
    /// (#655 F9a/F5c).
    pub async fn unacknowledge_ableset_mismatch(
        &self,
        number: &str,
    ) -> anyhow::Result<AbleSetStatusSnapshot> {
        let song_prefix_length = self
            .ableset_bridge
            .status_snapshot()
            .await
            .song_prefix_length;
        validate_ableset_ack_number(number, song_prefix_length)?;

        let _guard = self.ableset_ack_lock.lock().await;
        let mut acks = self.load_ableset_mismatch_acks().await.unwrap_or_default();
        acks.remove(number);
        self.save_ableset_mismatch_acks(&acks).await?;
        self.sync_ableset_ack_cache_and_recompute(acks).await;
        Ok(self.ableset_status_snapshot().await)
    }

    /// #655 F8 deviation 4: lazily prime the in-memory ack mirror from the
    /// persisted store EXACTLY ONCE per process lifetime — the mirror starts
    /// empty on every boot (`#[derive(Default)]`) and, absent this, is only
    /// ever populated as a side effect of `acknowledge_ableset_mismatch` /
    /// `unacknowledge_ableset_mismatch`. Without a boot-time load, a freshly
    /// restarted server silences NO mismatch until an operator happens to
    /// touch an ack, re-reporting every already-acknowledged mismatch on
    /// every restart — defeating the whole point of persisting acks (#601).
    /// Called from `refresh_ableset_cache_once` before it reads the mirror.
    /// Serialized under `ableset_ack_lock` (same lock/order as ack/unack) so
    /// two concurrent callers can never both load; the double-checked read
    /// keeps the common (already-initialized) case lock-free.
    pub(super) async fn ensure_ableset_acks_loaded(&self) {
        if self.caches.ableset.read().await.acks_initialized {
            return;
        }
        let _guard = self.ableset_ack_lock.lock().await;
        if self.caches.ableset.read().await.acks_initialized {
            return; // a concurrent caller already won the race and loaded it
        }
        let acks = self.load_ableset_mismatch_acks().await.unwrap_or_default();
        let mut cache = self.caches.ableset.write().await;
        cache.acks = acks;
        cache.acks_initialized = true;
    }

    /// #655 F8: mirror the freshly-persisted ack map into the in-memory
    /// cache — `refresh_ableset_cache`'s heavy DB rescan no longer reloads
    /// acks from the DB on every rebuild, it reads this mirror instead,
    /// which is what narrows its #639 CAS window down to the one remaining
    /// unavoidable DB fetch (the library summaries). #655 F5c: recomputes
    /// the mismatch report from the cache's own `presenter_titles` snapshot
    /// (populated by the last full rebuild) rather than re-fetching the
    /// library from the DB — an ack/unack click is common enough during
    /// pre-service review that forcing a full rescan on every one was the
    /// log-spam amplifier this finding fixes.
    async fn sync_ableset_ack_cache_and_recompute(&self, acks: AckMap) {
        let settings = self.ableset_bridge.status_snapshot().await;
        let ableset_songs = self.ableset_bridge.setlist_song_titles().await;
        let mut cache = self.caches.ableset.write().await;
        cache.acks = acks;
        // The load just above (in `acknowledge_ableset_mismatch` /
        // `unacknowledge_ableset_mismatch`) always reads the FULL persisted
        // map, so the mirror is now fully in sync with the DB — mark it
        // initialized so `ensure_ableset_acks_loaded` never redundantly
        // reloads it on the next rebuild (#655 F8's "exactly one load per
        // process lifetime" guarantee).
        cache.acks_initialized = true;
        let mismatches = super::ableset_mismatch::compute_ableset_mismatches(
            cache.presenter_titles(),
            &ableset_songs,
            settings.song_prefix_length,
            cache.acks(),
        );
        cache.mismatches = mismatches;
    }

    /// #655 F9b: drop acknowledgements whose number is present on NEITHER
    /// side any more — an ack for a song that has since been removed from
    /// both the AbleSet setlist and the Presenter library is dead weight
    /// that can never match anything again. Runs ONLY on a "real comparison"
    /// (both `presenter_titles` and `ableset_songs` non-empty — never when
    /// nothing is being compared, where "absent" carries no information
    /// about which numbers genuinely still exist). Uses its own
    /// load->mutate->save under the SAME mutex as ack/unack so a prune can
    /// never race a concurrent acknowledgement into a lost update.
    pub(super) async fn prune_stale_ableset_acks(
        &self,
        presenter_titles: &HashMap<String, String>,
        ableset_songs: &[(String, String)],
    ) {
        let _guard = self.ableset_ack_lock.lock().await;
        let mut acks = match self.load_ableset_mismatch_acks().await {
            Ok(acks) => acks,
            Err(err) => {
                warn!(
                    ?err,
                    "failed to load AbleSet mismatch acknowledgements for pruning — skipping (#655 F9)"
                );
                return;
            }
        };
        let before = acks.len();
        acks.retain(|number, _| {
            presenter_titles.contains_key(number) || ableset_songs.iter().any(|(n, _)| n == number)
        });
        let pruned = before - acks.len();
        if pruned == 0 {
            return;
        }
        if let Err(err) = self.save_ableset_mismatch_acks(&acks).await {
            warn!(
                ?err,
                "failed to persist pruned AbleSet mismatch acknowledgements (#655 F9)"
            );
            return;
        }
        info!(
            pruned,
            "pruned stale AbleSet mismatch acknowledgements no longer present on either side (#655 F9)"
        );
        self.caches.ableset.write().await.acks = acks;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
