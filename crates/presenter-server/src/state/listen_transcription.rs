//! #852 C3-7: the guest transcription's three `app_settings` rows ↔ the
//! supervisor. `listen.transcription` (the paid switch, the guest mic, the
//! daily cap) and `listen.session` (today's choice) are written only by
//! `PUT /listen/transcription` / `PUT /listen/session`, under the listen save
//! lock, each with one best-effort `settings_audit` row (the `listen.recorder`
//! pattern); `listen.usage` is the meter's own row, written by the supervisor
//! through [`TranscriptionStore`]. All three are read at startup under the
//! same lock, before the supervisor starts (never in validate mode). The
//! store also gives the supervisor today's prepared references and active
//! šepkár terms for the Soniox context. A PUT answers after the supervisor
//! decided on the new rows (bounded), so its status shows what it caused.

use async_trait::async_trait;
use chrono::NaiveDate;
use presenter_core::listen::context::{reference_term, ContextInputs};
use presenter_core::listen::transcription::{
    next_override, validate_transcription_change, SessionMode, SessionOverride,
    TranscriptionSetting, TranscriptionStatus, UsageRow, SESSION_KEY, TRANSCRIPTION_KEY, USAGE_KEY,
};
use presenter_persistence::SettingsAuditSource;
use serde::de::DeserializeOwned;

use super::interpreter_tips::TipScope;
use super::AppState;
use crate::listen::transcription::{TranscriptionStore, ASR_TARGET};

impl AppState {
    /// A stored row; absent or corrupt (WARN) = `None`.
    async fn transcription_row<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let raw = match self.repository.get_app_setting(key).await {
            Ok(raw) => raw?,
            Err(err) => {
                tracing::warn!(target: ASR_TARGET, key, error = %err, "a transcription row cannot be read — the default applies");
                return None;
            }
        };
        serde_json::from_str(&raw)
            .map_err(|err| {
                tracing::warn!(target: ASR_TARGET, key, error = %err, "a stored transcription row is corrupt — the default applies");
            })
            .ok()
    }

    /// Load the three rows into the service (the caller holds the save lock).
    pub(super) async fn load_transcription_rows(&self) {
        let setting = self
            .transcription_row::<TranscriptionSetting>(TRANSCRIPTION_KEY)
            .await
            .unwrap_or_default();
        let over = self.transcription_row::<SessionOverride>(SESSION_KEY).await;
        let usage = self.transcription_row::<UsageRow>(USAGE_KEY).await;
        let transcription = self.listen.transcription();
        transcription.load(setting, over);
        transcription.load_usage(usage, self.listen.today());
    }

    /// `PUT /listen/transcription`: validated against the stored row and the
    /// published mics (→ `TranscriptionValidationError`, 422), stored,
    /// applied, audited.
    pub(crate) async fn set_listen_transcription(
        &self,
        setting: TranscriptionSetting,
        actor: &str,
    ) -> anyhow::Result<TranscriptionStatus> {
        let save = self.listen.save_guard().await;
        let before = self.repository.get_app_setting(TRANSCRIPTION_KEY).await?;
        let stored = match before
            .as_deref()
            .map(serde_json::from_str::<TranscriptionSetting>)
        {
            Some(Ok(stored)) => stored,
            Some(Err(err)) => {
                // The row in use is the one loaded (or the default).
                tracing::warn!(target: ASR_TARGET, error = %err, "the stored listen.transcription is corrupt — validated against the setting in use");
                self.listen.transcription().rows().setting
            }
            None => self.listen.transcription().rows().setting,
        };
        validate_transcription_change(&setting, Some(&stored), &self.listen.mics())?;
        self.repository
            .set_app_setting(TRANSCRIPTION_KEY, &serde_json::to_string(&setting)?)
            .await?;
        let generation = self.listen.transcription().set_setting(setting.clone());
        tracing::info!(
            target: ASR_TARGET,
            enabled = setting.enabled,
            guest_mic = setting.guest_mic.as_deref().unwrap_or(""),
            daily_cap_minutes = setting.daily_cap_minutes,
            actor,
            "guest transcription setting saved"
        );
        let after = serde_json::to_value(&setting).unwrap_or_default();
        self.audit_listen_setting(
            TRANSCRIPTION_KEY,
            SettingsAuditSource::HttpSetter,
            before,
            after,
            actor,
        )
        .await;
        drop(save);
        self.listen.transcription_decided_on(generation).await;
        Ok(self.listen_status().transcription)
    }

    /// `PUT /listen/session`: today's choice (a start after the cap adds an
    /// hour to today's cap), stored, applied, audited.
    pub(crate) async fn set_listen_session(
        &self,
        mode: SessionMode,
        actor: &str,
    ) -> anyhow::Result<TranscriptionStatus> {
        let save = self.listen.save_guard().await;
        let today = self.listen.today();
        let transcription = self.listen.transcription();
        let current = transcription.rows().over;
        let row = next_override(
            current.as_ref(),
            today,
            mode,
            transcription.cap_reached(today),
        );
        let before = self.repository.get_app_setting(SESSION_KEY).await?;
        self.repository
            .set_app_setting(SESSION_KEY, &serde_json::to_string(&row)?)
            .await?;
        let generation = transcription.set_override(row.clone());
        tracing::info!(
            target: ASR_TARGET,
            mode = ?mode,
            extra_minutes = row.extra_minutes,
            actor,
            "guest transcription: today's choice saved"
        );
        let after = serde_json::to_value(&row).unwrap_or_default();
        self.audit_listen_setting(
            SESSION_KEY,
            SettingsAuditSource::HttpSetter,
            before,
            after,
            actor,
        )
        .await;
        drop(save);
        self.listen.transcription_decided_on(generation).await;
        Ok(self.listen_status().transcription)
    }

    /// The references of today's sermons, in sermon and item order.
    async fn todays_references(&self, today: NaiveDate) -> anyhow::Result<Vec<String>> {
        let mut refs = Vec::new();
        for id in self.repository.sermon_ids_on(today).await? {
            let Some(sermon) = self.repository.fetch_sermon(id).await? else {
                continue;
            };
            for item in sermon.items {
                let index = u8::try_from(item.book_number.saturating_sub(1)).unwrap_or(u8::MAX);
                let term = reference_term(
                    index,
                    item.chapter,
                    Some(item.verse_start),
                    Some(item.verse_end),
                );
                if !term.is_empty() {
                    refs.push(term);
                }
            }
        }
        Ok(refs)
    }
}

#[allow(clippy::double_must_use)]
#[async_trait]
impl TranscriptionStore for AppState {
    async fn context_inputs(&self, today: NaiveDate) -> anyhow::Result<ContextInputs> {
        let prepared_refs = self.todays_references(today).await?;
        let tip_terms = self
            .interpreter_tips_list(TipScope::Active)
            .await?
            .tips
            .into_iter()
            .filter_map(|tip| tip.source_term)
            .collect();
        Ok(ContextInputs {
            prepared_refs,
            tip_terms,
        })
    }

    async fn save_usage(&self, row: UsageRow) -> anyhow::Result<()> {
        self.repository
            .set_app_setting(USAGE_KEY, &serde_json::to_string(&row)?)
            .await
    }
}

#[cfg(test)]
mod tests;
