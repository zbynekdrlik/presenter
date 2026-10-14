//! Šepkár (prompter) tips for the interpreter (#852, spec §9): the DTOs the
//! server, the šepkár page and the interpreter page share, the limits, and
//! the pure rules — text cleaning, validation, which tips are active and
//! which new tip repeats a recent one. No I/O and no clock: callers pass
//! `now` and the service date, so every rule is unit-tested here.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A word or phrase as heard, and its suggested translation: 1–60 chars each.
pub const TIP_TERM_MAX_CHARS: usize = 60;
/// A free note: at most 200 chars (it fits a tablet card).
pub const TIP_NOTE_MAX_CHARS: usize = 200;
/// The sender's typed name: 1–40 chars.
pub const TIP_SENDER_MAX_CHARS: usize = 40;
/// The audit user agent is cut to this many chars.
pub const TIP_USER_AGENT_MAX_CHARS: usize = 200;
/// A tip stays on the interpreter's screen this long; it separates the
/// morning and the evening service of one day.
pub const TIP_ACTIVE_WINDOW_HOURS: i64 = 4;
/// The live event carries at most this many active tips, newest first.
pub const TIP_LIVE_LIST_CAP: usize = 20;
/// The same tip sent again within this many seconds is a duplicate.
pub const TIP_DUPLICATE_WINDOW_SECS: i64 = 120;
/// Stored tips older than this many days are deleted.
pub const TIP_RETENTION_DAYS: i64 = 30;
/// Accepted tips per (sender IP, sender name) per rate window.
pub const TIP_RATE_PER_SENDER: usize = 5;
/// Accepted tips overall per rate window.
pub const TIP_RATE_OVERALL: usize = 30;
/// The rate window, in seconds.
pub const TIP_RATE_WINDOW_SECS: u64 = 60;

/// One stored tip as every client sees it. The audit columns (`sender_ip`,
/// `user_agent`) never leave the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TipDto {
    pub id: i64,
    /// The server's local date at creation, `YYYY-MM-DD`.
    pub service_date: String,
    pub source_term: Option<String>,
    pub target_term: Option<String>,
    pub note: Option<String>,
    pub sender_name: String,
    pub created_at: DateTime<Utc>,
    pub dismissed_at: Option<DateTime<Utc>>,
}

/// `POST /interpreter/tips` body. Every field defaults, so a missing field
/// reaches validation (a Slovak 422) instead of axum's English rejection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewTipRequest {
    pub sender_name: String,
    pub source_term: Option<String>,
    pub target_term: Option<String>,
    pub note: Option<String>,
}

/// A request that passed [`validate_tip`]: cleaned, within the limits, both
/// pair terms or neither, and at least a pair or a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidTip {
    pub sender_name: String,
    pub source_term: Option<String>,
    pub target_term: Option<String>,
    pub note: Option<String>,
}

/// `GET /interpreter/tips` answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TipsList {
    pub service_date: String,
    /// Newest first: the active tips (`scope=active`) or all of today's.
    pub tips: Vec<TipDto>,
    /// Ids of the tips on the interpreter's screen right now, so the šepkár
    /// page can tell "na obrazovke" from a tip older than the 4 h window.
    pub active_ids: Vec<i64>,
}

/// `POST /interpreter/tips` answer: the new tip, or the earlier one it
/// repeats (`duplicate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTipResponse {
    #[serde(flatten)]
    pub tip: TipDto,
    pub duplicate: bool,
}

/// `POST /interpreter/tips/dismiss-all` answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DismissAllResponse {
    pub dismissed: u64,
}

/// Why a tip was refused. The text is shown to the šepkár as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TipValidationError {
    #[error("Napíš svoje meno.")]
    MissingSender,
    #[error("Meno môže mať najviac 40 znakov.")]
    SenderTooLong,
    #[error("Napíš slovo s prekladom alebo poznámku.")]
    Empty,
    #[error("Vyplň slovo aj preklad, alebo ani jedno.")]
    HalfPair,
    #[error("Slovo aj preklad môžu mať najviac 60 znakov.")]
    TermTooLong,
    #[error("Poznámka môže mať najviac 200 znakov.")]
    NoteTooLong,
}

/// Invisible format characters `char::is_control` does not cover (Unicode
/// `Cf` and the line / paragraph separators), as an explicit set because std
/// has no general-category lookup: zero-width space / non-joiner / joiner and
/// the LRM / RLM marks (U+200B-U+200F), the line and paragraph separators and
/// the bidi embeddings and overrides (U+2028-U+202E), the word joiner and
/// invisible operators (U+2060-U+2064), the bidi isolates and the deprecated
/// format controls (U+2066-U+206F) and the byte order mark (U+FEFF). Left in,
/// a name of one zero-width space would pass as "filled in" and a
/// right-to-left override would flip a card.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

/// Control and invisible format characters become spaces, then the text is
/// trimmed: a pasted tab, newline, escape sequence, zero-width space or
/// bidi override never reaches a card or the log.
pub fn clean_text(raw: &str) -> String {
    let replaced: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || is_invisible_format(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    replaced.trim().to_string()
}

/// The audit user agent: cleaned, cut to [`TIP_USER_AGENT_MAX_CHARS`].
pub fn clean_user_agent(raw: &str) -> Option<String> {
    let cleaned = clean_text(raw);
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned.chars().take(TIP_USER_AGENT_MAX_CHARS).collect())
}

fn cleaned(raw: Option<&str>) -> Option<String> {
    raw.map(clean_text).filter(|text| !text.is_empty())
}

fn too_long(text: Option<&str>, max: usize) -> bool {
    text.is_some_and(|text| text.chars().count() > max)
}

/// Spec §9.2 / §9.8: name 1–40, both terms (1–60 each) or neither, note
/// ≤ 200, at least a pair or a note. Lengths count characters, not bytes.
pub fn validate_tip(request: &NewTipRequest) -> Result<ValidTip, TipValidationError> {
    let sender_name = clean_text(&request.sender_name);
    if sender_name.is_empty() {
        return Err(TipValidationError::MissingSender);
    }
    if too_long(Some(sender_name.as_str()), TIP_SENDER_MAX_CHARS) {
        return Err(TipValidationError::SenderTooLong);
    }
    let source_term = cleaned(request.source_term.as_deref());
    let target_term = cleaned(request.target_term.as_deref());
    let note = cleaned(request.note.as_deref());
    if source_term.is_some() != target_term.is_some() {
        return Err(TipValidationError::HalfPair);
    }
    if source_term.is_none() && note.is_none() {
        return Err(TipValidationError::Empty);
    }
    if too_long(source_term.as_deref(), TIP_TERM_MAX_CHARS)
        || too_long(target_term.as_deref(), TIP_TERM_MAX_CHARS)
    {
        return Err(TipValidationError::TermTooLong);
    }
    if too_long(note.as_deref(), TIP_NOTE_MAX_CHARS) {
        return Err(TipValidationError::NoteTooLong);
    }
    Ok(ValidTip {
        sender_name,
        source_term,
        target_term,
        note,
    })
}

fn normalized(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a tip says, for the duplicate check — lower-cased, whitespace
/// collapsed. A pair keys on the pair AND its note (a pair alone has an empty
/// note part), a note alone keys on the note. So a double tap or the same
/// word from two šepkárs is caught, but a resend with a corrected or added
/// note is a new tip and must reach the interpreter.
pub fn duplicate_key(source: Option<&str>, target: Option<&str>, note: Option<&str>) -> String {
    let note = normalized(note.unwrap_or_default());
    match (source, target) {
        (Some(source), Some(target)) => {
            format!(
                "pair\u{1f}{}\u{1f}{}\u{1f}{note}",
                normalized(source),
                normalized(target)
            )
        }
        _ => format!("note\u{1f}{note}"),
    }
}

fn text_chars(parts: [Option<&str>; 3]) -> usize {
    parts
        .iter()
        .flatten()
        .map(|text| text.chars().count())
        .sum()
}

impl ValidTip {
    pub fn duplicate_key(&self) -> String {
        duplicate_key(
            self.source_term.as_deref(),
            self.target_term.as_deref(),
            self.note.as_deref(),
        )
    }
}

impl TipDto {
    pub fn duplicate_key(&self) -> String {
        duplicate_key(
            self.source_term.as_deref(),
            self.target_term.as_deref(),
            self.note.as_deref(),
        )
    }

    /// Characters of tip text (pair + note), for the INFO line.
    pub fn chars(&self) -> usize {
        text_chars([
            self.source_term.as_deref(),
            self.target_term.as_deref(),
            self.note.as_deref(),
        ])
    }
}

/// The recent tip `candidate` repeats: same [`duplicate_key`], sent LESS than
/// [`TIP_DUPLICATE_WINDOW_SECS`] ago (exactly that old is no longer recent),
/// by anyone, and NOT dismissed (a tip the interpreter hid may be sent
/// again).
pub fn find_recent_duplicate<'a>(
    tips: &'a [TipDto],
    candidate: &ValidTip,
    now: DateTime<Utc>,
) -> Option<&'a TipDto> {
    let since = now.checked_sub_signed(Duration::seconds(TIP_DUPLICATE_WINDOW_SECS))?;
    let key = candidate.duplicate_key();
    tips.iter().find(|tip| {
        tip.dismissed_at.is_none() && tip.created_at > since && tip.duplicate_key() == key
    })
}

/// A service date as stored and sent: `YYYY-MM-DD`.
pub fn service_date_string(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Spec §9.3: not dismissed, of today's service date, younger than
/// [`TIP_ACTIVE_WINDOW_HOURS`].
pub fn is_active(tip: &TipDto, today: NaiveDate, now: DateTime<Utc>) -> bool {
    let Some(since) = now.checked_sub_signed(Duration::hours(TIP_ACTIVE_WINDOW_HOURS)) else {
        return false;
    };
    tip.dismissed_at.is_none()
        && tip.service_date == service_date_string(today)
        && tip.created_at > since
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn request(
        sender: &str,
        source: Option<&str>,
        target: Option<&str>,
        note: Option<&str>,
    ) -> NewTipRequest {
        NewTipRequest {
            sender_name: sender.to_string(),
            source_term: source.map(str::to_string),
            target_term: target.map(str::to_string),
            note: note.map(str::to_string),
        }
    }

    fn tip(
        id: i64,
        created: &str,
        source: Option<&str>,
        target: Option<&str>,
        note: Option<&str>,
    ) -> TipDto {
        TipDto {
            id,
            service_date: "2026-10-18".to_string(),
            source_term: source.map(str::to_string),
            target_term: target.map(str::to_string),
            note: note.map(str::to_string),
            sender_name: "Peter".to_string(),
            created_at: at(created),
            dismissed_at: None,
        }
    }

    #[test]
    fn a_pair_a_note_or_both_are_valid_and_cleaned() {
        let pair = validate_tip(&request(
            "  Peter ",
            Some(" atonement\t"),
            Some("zmierenie"),
            None,
        ))
        .unwrap();
        assert_eq!(
            pair,
            ValidTip {
                sender_name: "Peter".to_string(),
                source_term: Some("atonement".to_string()),
                target_term: Some("zmierenie".to_string()),
                note: None,
            }
        );
        let note = validate_tip(&request("Jana", None, None, Some("hovorí o milosti"))).unwrap();
        assert_eq!(note.note.as_deref(), Some("hovorí o milosti"));
        let both = validate_tip(&request(
            "Eva",
            Some("grace"),
            Some("milosť"),
            Some("nie láskavosť"),
        ));
        assert!(both.is_ok(), "{both:?}");
    }

    #[test]
    fn each_rule_has_its_own_refusal() {
        let long = |n: usize| "ž".repeat(n);
        let cases = [
            (
                request("", Some("a"), Some("b"), None),
                TipValidationError::MissingSender,
            ),
            (
                request(" \u{7} ", None, None, Some("x")),
                TipValidationError::MissingSender,
            ),
            (
                request("\u{200B}\u{202E}\u{FEFF}", None, None, Some("x")),
                TipValidationError::MissingSender,
            ),
            (
                request("Peter", None, None, Some("\u{200B}\u{2028}\u{2066}")),
                TipValidationError::Empty,
            ),
            (
                request("Peter", Some("\u{200D}"), Some("b"), None),
                TipValidationError::HalfPair,
            ),
            (
                request(&long(41), None, None, Some("x")),
                TipValidationError::SenderTooLong,
            ),
            (
                request("Peter", None, None, None),
                TipValidationError::Empty,
            ),
            (
                request("Peter", None, None, Some("\u{7}\t\n ")),
                TipValidationError::Empty,
            ),
            (
                request("Peter", Some("atonement"), None, None),
                TipValidationError::HalfPair,
            ),
            (
                request("Peter", None, Some("zmierenie"), Some("x")),
                TipValidationError::HalfPair,
            ),
            (
                request("Peter", Some(long(61).as_str()), Some("b"), None),
                TipValidationError::TermTooLong,
            ),
            (
                request("Peter", Some("a"), Some(long(61).as_str()), None),
                TipValidationError::TermTooLong,
            ),
            (
                request("Peter", None, None, Some(long(201).as_str())),
                TipValidationError::NoteTooLong,
            ),
        ];
        for (request, expected) in cases {
            assert_eq!(validate_tip(&request), Err(expected), "{request:?}");
        }
    }

    #[test]
    fn exactly_at_the_limits_is_valid_counting_chars_not_bytes() {
        let ok = validate_tip(&request(
            &"ž".repeat(40),
            Some("ž".repeat(60).as_str()),
            Some("š".repeat(60).as_str()),
            Some("č".repeat(200).as_str()),
        ));
        assert!(ok.is_ok(), "{ok:?}");
    }

    #[test]
    fn the_messages_name_the_limits() {
        let pairs = [
            (TipValidationError::SenderTooLong, TIP_SENDER_MAX_CHARS),
            (TipValidationError::TermTooLong, TIP_TERM_MAX_CHARS),
            (TipValidationError::NoteTooLong, TIP_NOTE_MAX_CHARS),
        ];
        for (error, limit) in pairs {
            assert!(error.to_string().contains(&limit.to_string()), "{error}");
        }
    }

    #[test]
    fn control_characters_never_survive_cleaning() {
        assert_eq!(clean_text("\u{1b}[31matonement\u{0}"), "[31matonement");
        assert_eq!(clean_text("a\tb\nc"), "a b c");
        assert_eq!(clean_text("\u{7f}"), "");
    }

    #[test]
    fn invisible_format_characters_never_survive_cleaning() {
        // zero-width space / non-joiner / joiner, word joiner, byte order mark
        assert_eq!(clean_text("\u{200B}"), "");
        assert_eq!(clean_text("\u{FEFF}hi\u{2060}"), "hi");
        assert_eq!(clean_text("a\u{200B}\u{200C}\u{200D}b"), "a   b");
        // line / paragraph separators
        assert_eq!(clean_text("x\u{2028}y\u{2029}z"), "x y z");
        // bidi overrides, embeddings, isolates, marks: a card cannot be flipped
        assert_eq!(clean_text("a\u{202E}b\u{202A}c\u{202C}d"), "a b c d");
        assert_eq!(
            clean_text("a\u{2066}b\u{2067}c\u{2068}d\u{2069}e"),
            "a b c d e"
        );
        assert_eq!(clean_text("a\u{200E}b\u{200F}c"), "a b c");
        // the neighbours of each range stay: a no-break space, a hyphen, text
        assert_eq!(
            clean_text("a\u{200A}b"),
            "a\u{200A}b",
            "hair space is visible spacing"
        );
        assert_eq!(
            clean_text("a\u{202F}b"),
            "a\u{202F}b",
            "narrow no-break space"
        );
        assert_eq!(
            clean_text("a\u{2065}b"),
            "a\u{2065}b",
            "unassigned code point"
        );
        assert_eq!(clean_text("zmierenie — milosť"), "zmierenie — milosť");
        assert!(clean_user_agent("\u{200B}\u{202E}").is_none());
    }

    #[test]
    fn the_user_agent_is_cleaned_and_cut() {
        assert_eq!(clean_user_agent("   "), None);
        let cut = clean_user_agent(&"a".repeat(250)).unwrap();
        assert_eq!(cut.chars().count(), TIP_USER_AGENT_MAX_CHARS);
    }

    #[test]
    fn active_means_today_not_dismissed_and_younger_than_four_hours() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
        let now = at("2026-10-18T19:00:00Z");
        let fresh = tip(1, "2026-10-18T15:00:01Z", None, None, Some("a"));
        assert!(is_active(&fresh, today, now));
        let old = tip(2, "2026-10-18T15:00:00Z", None, None, Some("a"));
        assert!(!is_active(&old, today, now), "exactly 4 h old has left");
        let mut dismissed = fresh.clone();
        dismissed.dismissed_at = Some(now);
        assert!(!is_active(&dismissed, today, now));
        let mut yesterday = fresh;
        yesterday.service_date = "2026-10-17".to_string();
        assert!(!is_active(&yesterday, today, now));
    }

    #[test]
    fn a_duplicate_is_the_same_pair_or_note_within_two_minutes_and_not_dismissed() {
        let now = at("2026-10-18T19:42:00Z");
        let tips = vec![
            tip(
                3,
                "2026-10-18T19:41:00Z",
                None,
                None,
                Some("Hovorí  o MILOSTI"),
            ),
            tip(
                2,
                "2026-10-18T19:40:30Z",
                Some("atonement"),
                Some("zmierenie"),
                Some("x"),
            ),
        ];
        let pair = validate_tip(&request(
            "Jana",
            Some(" Atonement "),
            Some("ZMIERENIE"),
            Some(" X"),
        ))
        .unwrap();
        assert_eq!(
            find_recent_duplicate(&tips, &pair, now).map(|t| t.id),
            Some(2),
            "the same pair and note from another sender, case and spacing aside"
        );
        let note = validate_tip(&request("Eva", None, None, Some("hovorí o milosti"))).unwrap();
        assert_eq!(
            find_recent_duplicate(&tips, &note, now).map(|t| t.id),
            Some(3)
        );
        let other = validate_tip(&request("Eva", Some("grace"), Some("milosť"), None)).unwrap();
        assert_eq!(find_recent_duplicate(&tips, &other, now), None);
        let later = at("2026-10-18T19:42:31Z");
        assert_eq!(
            find_recent_duplicate(&tips, &pair, later),
            None,
            "the pair is 2 min 1 s old"
        );
        let mut dismissed = tips.clone();
        dismissed[1].dismissed_at = Some(now);
        assert_eq!(
            find_recent_duplicate(&dismissed, &pair, now),
            None,
            "a tip the interpreter hid may be sent again"
        );
    }

    #[test]
    fn the_duplicate_window_closes_at_exactly_two_minutes() {
        let tips = vec![tip(
            2,
            "2026-10-18T19:40:30Z",
            Some("atonement"),
            Some("zmierenie"),
            None,
        )];
        let pair =
            validate_tip(&request("Jana", Some("atonement"), Some("zmierenie"), None)).unwrap();
        let just_inside = at("2026-10-18T19:42:29Z");
        assert_eq!(
            find_recent_duplicate(&tips, &pair, just_inside).map(|t| t.id),
            Some(2),
            "119 s old is still a duplicate"
        );
        let exactly = at("2026-10-18T19:42:30Z");
        assert_eq!(
            find_recent_duplicate(&tips, &pair, exactly),
            None,
            "exactly 120 s old has left the window (> not >=)"
        );
    }

    #[test]
    fn a_resend_with_a_new_note_is_not_a_duplicate() {
        let now = at("2026-10-18T19:42:00Z");
        let tips = vec![
            tip(
                5,
                "2026-10-18T19:41:30Z",
                Some("atonement"),
                Some("zmierenie"),
                Some("nie zmier"),
            ),
            tip(
                4,
                "2026-10-18T19:41:20Z",
                Some("grace"),
                Some("milosť"),
                None,
            ),
        ];
        let corrected = validate_tip(&request(
            "Peter",
            Some("atonement"),
            Some("zmierenie"),
            Some("nie zmierlivosť"),
        ))
        .unwrap();
        assert_eq!(
            find_recent_duplicate(&tips, &corrected, now),
            None,
            "the same pair with a corrected note reaches the interpreter"
        );
        let dropped_note = validate_tip(&request(
            "Peter",
            Some("atonement"),
            Some("zmierenie"),
            None,
        ))
        .unwrap();
        assert_eq!(
            find_recent_duplicate(&tips, &dropped_note, now),
            None,
            "the pair alone is not the pair with a note"
        );
        let added_note = validate_tip(&request(
            "Peter",
            Some("grace"),
            Some("milosť"),
            Some("nie láskavosť"),
        ))
        .unwrap();
        assert_eq!(
            find_recent_duplicate(&tips, &added_note, now),
            None,
            "a note added to an earlier bare pair is a new tip"
        );
        let bare = validate_tip(&request("Peter", Some("GRACE"), Some(" milosť "), None)).unwrap();
        assert_eq!(
            find_recent_duplicate(&tips, &bare, now).map(|t| t.id),
            Some(4),
            "a bare pair repeated is a double tap"
        );
        let double_tap = validate_tip(&request(
            "Peter",
            Some("atonement"),
            Some("zmierenie"),
            Some("Nie  ZMIER"),
        ))
        .unwrap();
        assert_eq!(
            find_recent_duplicate(&tips, &double_tap, now).map(|t| t.id),
            Some(5),
            "the same pair and note again is a double tap"
        );
    }

    #[test]
    fn chars_count_every_text_field() {
        let both = tip(
            1,
            "2026-10-18T19:00:00Z",
            Some("grace"),
            Some("milosť"),
            Some("nie"),
        );
        assert_eq!(both.chars(), 5 + 6 + 3);
    }

    #[test]
    fn the_service_date_is_iso() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        assert_eq!(service_date_string(date), "2026-01-05");
    }

    #[test]
    fn the_wire_shape_is_camel_case_with_explicit_nulls() {
        let dto = tip(
            17,
            "2026-10-18T17:42:05Z",
            Some("atonement"),
            Some("zmierenie"),
            None,
        );
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({
                "id": 17, "serviceDate": "2026-10-18", "sourceTerm": "atonement",
                "targetTerm": "zmierenie", "note": null, "senderName": "Peter",
                "createdAt": "2026-10-18T17:42:05Z", "dismissedAt": null
            })
        );
        let created = CreateTipResponse {
            tip: dto,
            duplicate: true,
        };
        let wire = serde_json::to_value(&created).unwrap();
        assert_eq!(wire["id"], json!(17));
        assert_eq!(wire["duplicate"], json!(true));
        assert_eq!(
            serde_json::from_value::<CreateTipResponse>(wire).unwrap(),
            created
        );
        let list = TipsList {
            service_date: "2026-10-18".to_string(),
            tips: vec![],
            active_ids: vec![17],
        };
        assert_eq!(
            serde_json::to_value(&list).unwrap(),
            json!({"serviceDate": "2026-10-18", "tips": [], "activeIds": [17]})
        );
    }

    #[test]
    fn a_request_with_missing_fields_reaches_validation() {
        let parsed: NewTipRequest = serde_json::from_value(json!({"note": "x"})).unwrap();
        assert_eq!(
            validate_tip(&parsed),
            Err(TipValidationError::MissingSender)
        );
        let empty: NewTipRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(empty, NewTipRequest::default());
    }
}
