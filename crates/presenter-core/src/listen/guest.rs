//! The guests' live translation wire (#999): what `GET /listen/guest/languages`
//! answers and what `GET /listen/guest/ws?lang=xx` sends as TEXT (the speech
//! itself goes as BINARY s16le frames at [`GuestWsMessage::Hello`]'s rate).
//! Shared by the server and the guest page `/poslech`, camelCase with a
//! `type` tag like `/listen/ws`.

use serde::{Deserialize, Serialize};

/// The languages a guest can pick: a BCP-47 code Gemini Live Translate takes as
/// `targetLanguageCode`, and the language's name IN that language — a guest
/// who does not read Slovak finds their own. The owner's list (EN, DE, HU, UK,
/// PL, ES, FR, RO, RU) first, then a few more European languages.
pub const GUEST_LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("de", "Deutsch"),
    ("hu", "Magyar"),
    ("uk", "Українська"),
    ("pl", "Polski"),
    ("es", "Español"),
    ("fr", "Français"),
    ("ro", "Română"),
    ("ru", "Русский"),
    ("it", "Italiano"),
    ("pt", "Português"),
    ("nl", "Nederlands"),
];

/// The offered language with this code (exact, lowercase): `(code, name)`.
pub fn guest_language(code: &str) -> Option<(&'static str, &'static str)> {
    GUEST_LANGUAGES.iter().copied().find(|(c, _)| *c == code)
}

/// Why a language's session stopped for good.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuestStopReason {
    /// The translation service refused the key (or none is set).
    Key,
    /// The translation service does not know the model.
    Model,
    /// The day's minute cap is used up.
    DailyCap,
}

/// A language session's state, as its listeners see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum GuestSessionState {
    Connecting,
    /// Ready, the preacher is quiet: nothing is sent, nothing costs.
    Waiting,
    /// Ready, the preacher speaks.
    Live,
    Reconnecting,
    Stopped {
        reason: GuestStopReason,
    },
}

impl GuestSessionState {
    /// Ready (the preacher speaks or not).
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Live | Self::Waiting)
    }
}

/// Why a guest cannot listen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuestRefusal {
    UnknownLanguage,
    /// No translation key on this box.
    NoKey,
    /// The preacher's mic is not on this box.
    NoSource,
    /// The most languages run at once already.
    Full,
    DailyCap,
}

/// A TEXT message of `GET /listen/guest/ws`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GuestWsMessage {
    /// First: the language and the rate of the BINARY speech frames.
    Hello {
        language: String,
        name: String,
        #[serde(rename = "sampleRate")]
        sample_rate: u32,
    },
    /// The session's state, at the start and on every change.
    State { session: GuestSessionState },
    /// Translated text as it is spoken (a fragment to append).
    Text { text: String },
    /// The Slovak original (a fragment to append).
    Original { text: String },
    /// The model ended a turn.
    Turn,
    /// Nobody can listen to this language now; the socket closes.
    Refused { reason: GuestRefusal },
}

/// One offered language on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestLanguageInfo {
    pub code: String,
    pub name: String,
}

/// `GET /listen/guest/languages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestLanguages {
    pub languages: Vec<GuestLanguageInfo>,
    /// Whether a guest can start a language now.
    pub available: bool,
    pub refusal: Option<GuestRefusal>,
    /// The languages translated right now (a guest joins them even when full).
    pub active: Vec<String>,
    pub max_languages: usize,
}

impl GuestLanguages {
    /// The offered list with this box's status.
    pub fn offered(refusal: Option<GuestRefusal>, active: Vec<String>, max: usize) -> Self {
        Self {
            languages: GUEST_LANGUAGES
                .iter()
                .map(|(code, name)| GuestLanguageInfo {
                    code: (*code).to_string(),
                    name: (*name).to_string(),
                })
                .collect(),
            available: refusal.is_none(),
            refusal,
            active,
            max_languages: max,
        }
    }

    /// Whether a guest can pick `code` now: it runs already, or the box can
    /// start one more.
    pub fn can_pick(&self, code: &str) -> bool {
        self.active.iter().any(|c| c == code)
            || (self.available && self.active.len() < self.max_languages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_owners_languages_are_offered_in_their_own_names() {
        for (code, name) in [
            ("en", "English"),
            ("de", "Deutsch"),
            ("hu", "Magyar"),
            ("uk", "Українська"),
            ("pl", "Polski"),
            ("es", "Español"),
            ("fr", "Français"),
            ("ro", "Română"),
            ("ru", "Русский"),
        ] {
            assert_eq!(guest_language(code), Some((code, name)), "{code}");
        }
        assert_eq!(GUEST_LANGUAGES[0].0, "en");
        assert_eq!(
            guest_language("sk"),
            None,
            "Slovak is the source, never a target"
        );
        assert_eq!(guest_language("EN"), None);
        let mut codes: Vec<&str> = GUEST_LANGUAGES.iter().map(|(c, _)| *c).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), GUEST_LANGUAGES.len());
    }

    #[test]
    fn the_ws_messages_keep_their_wire_shape() {
        let hello = GuestWsMessage::Hello {
            language: "de".into(),
            name: "Deutsch".into(),
            sample_rate: 24_000,
        };
        assert_eq!(
            serde_json::to_value(&hello).unwrap(),
            serde_json::json!({"type":"hello","language":"de","name":"Deutsch","sampleRate":24000})
        );
        let stopped = GuestWsMessage::State {
            session: GuestSessionState::Stopped {
                reason: GuestStopReason::DailyCap,
            },
        };
        assert_eq!(
            serde_json::to_value(&stopped).unwrap(),
            serde_json::json!({"type":"state","session":{"state":"stopped","reason":"dailyCap"}})
        );
        assert_eq!(
            serde_json::to_value(GuestWsMessage::Refused {
                reason: GuestRefusal::NoSource
            })
            .unwrap(),
            serde_json::json!({"type":"refused","reason":"noSource"})
        );
        for message in [
            hello,
            stopped,
            GuestWsMessage::Text {
                text: " they".into(),
            },
            GuestWsMessage::Original { text: "oni".into() },
            GuestWsMessage::Turn,
        ] {
            let text = serde_json::to_string(&message).unwrap();
            assert_eq!(
                serde_json::from_str::<GuestWsMessage>(&text).unwrap(),
                message
            );
        }
        assert!(GuestSessionState::Live.is_ready());
        assert!(GuestSessionState::Waiting.is_ready());
        assert!(!GuestSessionState::Reconnecting.is_ready());
    }

    #[test]
    fn a_full_box_still_offers_the_running_languages() {
        let full = GuestLanguages::offered(
            None,
            vec!["de".into(), "en".into(), "hu".into(), "uk".into()],
            4,
        );
        assert!(full.can_pick("en"));
        assert!(!full.can_pick("pl"));
        let open = GuestLanguages::offered(None, vec!["en".into()], 4);
        assert!(open.can_pick("pl"));
        let no_key = GuestLanguages::offered(Some(GuestRefusal::NoKey), Vec::new(), 4);
        assert!(!no_key.available);
        assert!(!no_key.can_pick("en"));
        assert_eq!(no_key.languages.len(), GUEST_LANGUAGES.len());
        let json = serde_json::to_value(&no_key).unwrap();
        assert_eq!(json["refusal"], "noKey");
        assert_eq!(json["maxLanguages"], 4);
        assert_eq!(json["languages"][3]["name"], "Українська");
    }
}
