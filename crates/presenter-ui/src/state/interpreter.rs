//! Pure rules of the interpreter display and the šepkár page (#852),
//! host-tested with `cargo test --lib` in `crates/presenter-ui`. No clock and
//! no DOM here: `js_sys::Date::now()` panics on the host.

use chrono::TimeZone;
use presenter_core::interpreter_tips::{TipDto, TIP_LIVE_LIST_CAP, TIP_NOTE_MAX_CHARS};
use std::collections::HashSet;

/// `?layout=` of `/ui/interpreter` (spec §8.1). Explicit, so a TV never
/// guesses; anything but `tv` is the tablet layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpreterLayout {
    Tablet,
    Tv,
}

impl InterpreterLayout {
    pub fn from_param(value: Option<&str>) -> Self {
        match value {
            Some("tv") => Self::Tv,
            _ => Self::Tablet,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tablet => "tablet",
            Self::Tv => "tv",
        }
    }
}

/// A leftward swipe this long (px) dismisses a tip card.
pub const SWIPE_DISMISS_PX: f64 = 80.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwipeDecision {
    Dismiss,
    SnapBack,
}

/// The release of a swipe that moved `dx` / `dy` px: a mostly horizontal
/// leftward swipe of at least [`SWIPE_DISMISS_PX`] dismisses.
pub fn swipe_decision(dx: f64, dy: f64) -> SwipeDecision {
    if dx <= -SWIPE_DISMISS_PX && dx.abs() > dy.abs() {
        SwipeDecision::Dismiss
    } else {
        SwipeDecision::SnapBack
    }
}

/// How far a card follows the finger: leftwards only.
pub fn swipe_offset(dx: f64) -> f64 {
    dx.min(0.0)
}

/// Tips to highlight: ids in `incoming` never seen before. The first list a
/// page loads highlights nothing (those tips were already there).
pub fn fresh_ids(known: &HashSet<i64>, incoming: &[TipDto], first_load: bool) -> HashSet<i64> {
    if first_load {
        return HashSet::new();
    }
    incoming
        .iter()
        .map(|tip| tip.id)
        .filter(|id| !known.contains(id))
        .collect()
}

/// "atonement → zmierenie"; `None` for a note-only tip.
pub fn tip_pair(tip: &TipDto) -> Option<String> {
    match (&tip.source_term, &tip.target_term) {
        (Some(source), Some(target)) => Some(format!("{source} → {target}")),
        _ => None,
    }
}

/// "Peter · 19:42" in `zone` (the pages pass `chrono::Local`).
pub fn tip_meta<Tz: TimeZone>(tip: &TipDto, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let time = tip.created_at.with_timezone(zone).format("%H:%M");
    format!("{} · {time}", tip.sender_name)
}

/// A row of the šepkár page's "Dnes" list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SepkarRowState {
    OnScreen,
    Dismissed,
    /// Not hidden, but older than the 4 h window (an earlier service).
    Expired,
}

impl SepkarRowState {
    pub fn of(tip: &TipDto, active_ids: &[i64]) -> Self {
        if tip.dismissed_at.is_some() {
            Self::Dismissed
        } else if active_ids.contains(&tip.id) {
            Self::OnScreen
        } else {
            Self::Expired
        }
    }

    /// The `data-state` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnScreen => "on-screen",
            Self::Dismissed => "dismissed",
            Self::Expired => "expired",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OnScreen => "na obrazovke",
            Self::Dismissed => "skryté",
            Self::Expired => "staršie",
        }
    }
}

/// The list the tips area shows: tips whose dismiss POST is still in flight
/// stay hidden (a list that was published before the dismiss landed must not
/// bring them back for a moment), and at most [`TIP_LIVE_LIST_CAP`] remain,
/// newest first, exactly like the live event (the resync `GET` is not capped).
pub fn visible_tips(mut tips: Vec<TipDto>, pending_dismiss: &HashSet<i64>) -> Vec<TipDto> {
    tips.retain(|tip| !pending_dismiss.contains(&tip.id));
    tips.truncate(TIP_LIVE_LIST_CAP);
    tips
}

/// The Slovak error line of a failed tips write (`what` = "Tip sa nepodarilo
/// skryť"). The server's own message is shown only for its refusals (404 / 409
/// / 422 / 429, Slovak by construction); anything else is a fixed Slovak line,
/// never raw network or proxy text. `status` is `None` when no HTTP answer
/// came at all.
pub fn tip_error_text(what: &str, status: Option<(u16, &str)>) -> String {
    match status {
        Some((404 | 409 | 422 | 429, detail)) if !detail.trim().is_empty() => {
            format!("{what}: {detail}")
        }
        Some((code, _)) => format!("{what} — server odpovedal chybou {code}."),
        None => format!("{what} — server je nedostupný."),
    }
}

/// The Slovak error line of a refused or failed tip SEND (the šepkár page).
/// The server's own message is shown verbatim only for its two refusals —
/// 422 (the rule that was broken) and 429 (how long to wait), Slovak by
/// construction; anything else is a fixed Slovak line, never raw network or
/// proxy text. `status` is `None` when no usable HTTP answer came at all.
/// Every line says the tip stayed in the fields, because it did.
pub fn sepkar_error_text(status: Option<(u16, &str)>) -> String {
    match status {
        Some((422 | 429, detail)) if !detail.trim().is_empty() => detail.to_string(),
        Some((code, _)) => {
            format!("Server odpovedal chybou {code} — tip zostal v poliach, skús znova.")
        }
        None => "Server je nedostupný — tip zostal v poliach, skús znova.".to_string(),
    }
}

/// The line after an accepted send: "Odoslané 19:42" in `zone` (the pages pass
/// `chrono::Local`), or, when the server answered with the earlier identical
/// tip, that the interpreter already has it.
pub fn sepkar_notice<Tz: TimeZone>(duplicate: bool, tip: &TipDto, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if duplicate {
        return "Tento tip už tlmočník má".to_string();
    }
    let time = tip.created_at.with_timezone(zone).format("%H:%M");
    format!("Odoslané {time}")
}

/// A form field as the optional request member: `None` when it is blank (the
/// server trims and validates; this only keeps blank fields out of the
/// request), otherwise the text exactly as typed.
pub fn optional_text(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// "12 / 200" and whether the note is over the limit (chars, not bytes).
pub fn note_counter(note: &str) -> (String, bool) {
    let count = note.chars().count();
    (
        format!("{count} / {TIP_NOTE_MAX_CHARS}"),
        count > TIP_NOTE_MAX_CHARS,
    )
}

/// Drops a resync answer that something newer overtook (the stage's
/// `StageSyncGeneration` idea): [`FetchGuard::begin`] a fetch, call
/// [`FetchGuard::note_newer`] on every live list or own write, and apply the
/// answer only if [`FetchGuard::accepts`] its ticket. Only the newest fetch
/// can land.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FetchGuard {
    generation: u64,
}

impl FetchGuard {
    pub fn begin(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    pub fn note_newer(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn accepts(&self, ticket: u64) -> bool {
        self.generation == ticket
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset, Utc};

    fn tip(id: i64, pair: Option<(&str, &str)>, note: Option<&str>) -> TipDto {
        TipDto {
            id,
            service_date: "2026-10-18".to_string(),
            source_term: pair.map(|(source, _)| source.to_string()),
            target_term: pair.map(|(_, target)| target.to_string()),
            note: note.map(str::to_string),
            sender_name: "Peter".to_string(),
            created_at: DateTime::parse_from_rfc3339("2026-10-18T17:42:05Z")
                .unwrap()
                .with_timezone(&Utc),
            dismissed_at: None,
        }
    }

    #[test]
    fn the_layout_param_defaults_to_tablet() {
        assert_eq!(
            InterpreterLayout::from_param(None),
            InterpreterLayout::Tablet
        );
        assert_eq!(
            InterpreterLayout::from_param(Some("tablet")),
            InterpreterLayout::Tablet
        );
        assert_eq!(
            InterpreterLayout::from_param(Some("TV")),
            InterpreterLayout::Tablet
        );
        assert_eq!(
            InterpreterLayout::from_param(Some("tv")),
            InterpreterLayout::Tv
        );
        assert_eq!(InterpreterLayout::Tv.as_str(), "tv");
        assert_eq!(InterpreterLayout::Tablet.as_str(), "tablet");
    }

    #[test]
    fn a_left_swipe_past_80_px_dismisses_everything_else_snaps_back() {
        assert_eq!(swipe_decision(-81.0, 10.0), SwipeDecision::Dismiss);
        assert_eq!(swipe_decision(-80.0, 0.0), SwipeDecision::Dismiss);
        assert_eq!(swipe_decision(-79.0, 0.0), SwipeDecision::SnapBack);
        assert_eq!(
            swipe_decision(-100.0, 120.0),
            SwipeDecision::SnapBack,
            "a vertical scroll"
        );
        assert_eq!(
            swipe_decision(150.0, 0.0),
            SwipeDecision::SnapBack,
            "a right swipe"
        );
    }

    #[test]
    fn a_card_follows_the_finger_only_leftwards() {
        assert_eq!(swipe_offset(-40.0), -40.0);
        assert_eq!(swipe_offset(25.0), 0.0);
    }

    #[test]
    fn the_first_list_highlights_nothing_later_lists_only_new_ids() {
        let known: HashSet<i64> = [1, 2].into_iter().collect();
        let incoming = vec![tip(3, None, Some("c")), tip(2, None, Some("b"))];
        let only_the_new: HashSet<i64> = [3].into_iter().collect();
        assert!(fresh_ids(&known, &incoming, true).is_empty());
        assert_eq!(fresh_ids(&known, &incoming, false), only_the_new);
    }

    #[test]
    fn labels_show_the_pair_and_sender_with_local_time() {
        let pair = tip(1, Some(("atonement", "zmierenie")), None);
        assert_eq!(tip_pair(&pair).as_deref(), Some("atonement → zmierenie"));
        assert_eq!(tip_pair(&tip(2, None, Some("x"))), None);
        let bratislava = FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(tip_meta(&pair, &bratislava), "Peter · 19:42");
    }

    #[test]
    fn sepkar_rows_are_on_screen_hidden_or_older() {
        let shown = tip(1, None, Some("a"));
        assert_eq!(SepkarRowState::of(&shown, &[1]), SepkarRowState::OnScreen);
        assert_eq!(SepkarRowState::of(&shown, &[]), SepkarRowState::Expired);
        let mut hidden = shown.clone();
        hidden.dismissed_at = Some(shown.created_at);
        assert_eq!(SepkarRowState::of(&hidden, &[1]), SepkarRowState::Dismissed);
        assert_eq!(SepkarRowState::Dismissed.label(), "skryté");
        assert_eq!(SepkarRowState::OnScreen.as_str(), "on-screen");
    }

    #[test]
    fn a_send_error_keeps_the_servers_two_refusals_and_is_otherwise_a_fixed_slovak_line() {
        assert_eq!(
            sepkar_error_text(Some((422, "Poznámka môže mať najviac 200 znakov."))),
            "Poznámka môže mať najviac 200 znakov."
        );
        assert_eq!(
            sepkar_error_text(Some((
                429,
                "Pomaly — najviac 5 tipov za minútu. Skús znova o 12 s."
            ))),
            "Pomaly — najviac 5 tipov za minútu. Skús znova o 12 s."
        );
        assert_eq!(
            sepkar_error_text(Some((502, "<html>Bad Gateway</html>"))),
            "Server odpovedal chybou 502 — tip zostal v poliach, skús znova."
        );
        assert_eq!(
            sepkar_error_text(Some((422, "  "))),
            "Server odpovedal chybou 422 — tip zostal v poliach, skús znova.",
            "an empty refusal body is no message to show"
        );
        assert_eq!(
            sepkar_error_text(None),
            "Server je nedostupný — tip zostal v poliach, skús znova."
        );
    }

    #[test]
    fn the_notice_after_a_send_says_the_time_or_that_the_tip_was_already_there() {
        let sent = tip(1, Some(("atonement", "zmierenie")), None);
        let bratislava = FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(sepkar_notice(false, &sent, &bratislava), "Odoslané 19:42");
        assert_eq!(
            sepkar_notice(true, &sent, &bratislava),
            "Tento tip už tlmočník má"
        );
    }

    #[test]
    fn a_blank_field_is_no_request_member_and_typed_text_is_kept_as_typed() {
        assert_eq!(optional_text(""), None);
        assert_eq!(optional_text("  \t "), None);
        assert_eq!(
            optional_text(" Atonement "),
            Some(" Atonement ".to_string())
        );
        assert_eq!(optional_text("<b>x</b>"), Some("<b>x</b>".to_string()));
    }

    #[test]
    fn the_note_counter_counts_chars_and_flags_over_200() {
        assert_eq!(note_counter(""), ("0 / 200".to_string(), false));
        assert_eq!(
            note_counter(&"ž".repeat(200)),
            ("200 / 200".to_string(), false)
        );
        assert_eq!(
            note_counter(&"ž".repeat(201)),
            ("201 / 200".to_string(), true)
        );
    }

    #[test]
    fn a_pending_dismiss_stays_hidden_and_the_list_is_capped_like_the_live_event() {
        let list: Vec<TipDto> = (1..=25).map(|id| tip(id, None, Some("n"))).collect();
        let pending: HashSet<i64> = [2].into_iter().collect();
        let shown = visible_tips(list, &pending);
        assert_eq!(shown.len(), TIP_LIVE_LIST_CAP);
        assert!(
            shown.iter().all(|tip| tip.id != 2),
            "the pending one is hidden"
        );
        assert_eq!(shown[0].id, 1, "the server's order (newest first) is kept");
        assert_eq!(shown[1].id, 3);
        let few = visible_tips(vec![tip(7, None, Some("n"))], &HashSet::new());
        assert_eq!(few.len(), 1);
    }

    #[test]
    fn error_text_is_slovak_and_only_the_servers_refusals_keep_their_message() {
        assert_eq!(
            tip_error_text(
                "Tip sa nepodarilo vrátiť",
                Some((409, "Tento tip už nie je z dnešnej bohoslužby."))
            ),
            "Tip sa nepodarilo vrátiť: Tento tip už nie je z dnešnej bohoslužby."
        );
        assert_eq!(
            tip_error_text("Tip sa nepodarilo skryť", Some((404, "Tip neexistuje."))),
            "Tip sa nepodarilo skryť: Tip neexistuje."
        );
        assert_eq!(
            tip_error_text(
                "Tip sa nepodarilo skryť",
                Some((502, "<html>Bad Gateway</html>"))
            ),
            "Tip sa nepodarilo skryť — server odpovedal chybou 502."
        );
        assert_eq!(
            tip_error_text("Tip sa nepodarilo skryť", Some((404, "  "))),
            "Tip sa nepodarilo skryť — server odpovedal chybou 404."
        );
        assert_eq!(
            tip_error_text("Tip sa nepodarilo skryť", None),
            "Tip sa nepodarilo skryť — server je nedostupný."
        );
    }

    #[test]
    fn fetch_guard_drops_an_answer_overtaken_by_a_live_list() {
        let mut guard = FetchGuard::default();
        let first = guard.begin();
        assert!(guard.accepts(first));
        guard.note_newer();
        assert!(
            !guard.accepts(first),
            "a live list arrived while it was in flight"
        );
        let second = guard.begin();
        let third = guard.begin();
        assert!(!guard.accepts(second), "only the newest resync may land");
        assert!(guard.accepts(third));
    }
}
