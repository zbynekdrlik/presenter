//! The interpreter's EN column (#852 C3-9, spec §8.2, plan Decision 15): the
//! guest's live English transcript from `GET /listen/ws`. Closed sentences
//! newest at the bottom (older ones dimmed, the newest 300 kept), then the open
//! sentence: its final words normal, the live (non-final) rest lighter. The
//! column follows the bottom while the reader is there; scrolling up pauses it
//! and shows "▼ Naživo · N nové" (N = sentences closed since), a tap jumps
//! back. A banner says when the transcript cannot come (C-S7) or the socket is
//! down. The column shows only once there is something to show — a sentence, a
//! session that runs, or a reason worth a banner — so a box without
//! transcription keeps the tips area full width (`data-transcript="absent"`).
//!
//! Every (re)connect starts with the server's snapshot, which REPLACES what the
//! column held; the messages after it are applied in arrival order, in place.
//! [`TranscriptModel`] holds sentences, tails and sessions in their own signals
//! and changes them only through the pure functions below (host-tested), so a
//! tail every ~0.2 s neither copies nor re-diffs the sentence list.

use crate::ws::passive::PassiveWsState;
use leptos::prelude::*;
use presenter_core::listen::transcription::{MicTranscription, OffReason, TranscriptionState};
use presenter_core::listen::ws::{
    ListenWsMessage, TranscriptSentence, TranscriptSnapshot, TranscriptTail,
};

/// Sentences kept (and in the DOM).
pub const MAX_SENTENCES: usize = 300;
/// The reader is "at the bottom" within this many pixels.
const BOTTOM_SLACK_PX: i32 = 24;
/// The banner while `/listen/ws` itself is down (no other banner applies).
const SOCKET_DOWN: &str = "Prepis: bez spojenia so serverom — obnovujem…";

/// What a snapshot leaves: its newest [`MAX_SENTENCES`] sentences (oldest
/// first), its non-empty tails and its sessions.
type Parts = (
    Vec<TranscriptSentence>,
    Vec<TranscriptTail>,
    Vec<MicTranscription>,
);

fn snapshot_parts(snapshot: TranscriptSnapshot) -> Parts {
    let mut sentences = snapshot.sentences;
    sentences.sort_by_key(|s| s.id);
    let cut = sentences.len().saturating_sub(MAX_SENTENCES);
    let tails = snapshot
        .tails
        .into_iter()
        .filter(|tail| !tail_is_empty(tail))
        .collect();
    (sentences.split_off(cut), tails, snapshot.sessions)
}

/// Append a sentence newer than every one held (a repeat or an older one is
/// ignored → `false`); the oldest goes past [`MAX_SENTENCES`].
fn push_sentence(sentences: &mut Vec<TranscriptSentence>, sentence: TranscriptSentence) -> bool {
    if !is_newer(sentences, &sentence) {
        return false;
    }
    sentences.push(sentence);
    if sentences.len() > MAX_SENTENCES {
        sentences.remove(0);
    }
    true
}

fn is_newer(sentences: &[TranscriptSentence], sentence: &TranscriptSentence) -> bool {
    sentences.last().is_none_or(|last| last.id < sentence.id)
}

/// The "N nové" count after a sentence arrived: counted only while the reader
/// looks back.
fn unseen_after(unseen: usize, following: bool) -> usize {
    if following {
        unseen
    } else {
        unseen + 1
    }
}

/// A mic's tail replaces its previous one; an empty one clears it.
fn apply_tail(tails: &mut Vec<TranscriptTail>, tail: TranscriptTail) {
    tails.retain(|t| t.mic_id != tail.mic_id);
    if !tail_is_empty(&tail) {
        tails.push(tail);
    }
}

/// A mic's session replaces its previous one (the newest last).
fn apply_session(sessions: &mut Vec<MicTranscription>, session: MicTranscription) {
    sessions.retain(|s| s.mic_id != session.mic_id);
    sessions.push(session);
}

/// The session the column speaks about: one that runs, else the newest one
/// received (a replaced guest mic comes once as `off`).
fn current_session(sessions: &[MicTranscription]) -> Option<&MicTranscription> {
    sessions
        .iter()
        .rev()
        .find(|s| s.state != TranscriptionState::Off)
        .or_else(|| sessions.last())
}

/// The open sentence shown under the closed ones: the newest non-empty tail.
fn newest_tail(tails: &[TranscriptTail]) -> Option<&TranscriptTail> {
    tails.iter().max_by_key(|tail| tail.updated_at)
}

/// The column shows once it has text, a session that runs, or a reason worth
/// a banner — never for a plain "stopped today".
fn shows(has_sentences: bool, has_tail: bool, session: Option<&MicTranscription>) -> bool {
    has_sentences
        || has_tail
        || session.is_some_and(|s| s.state != TranscriptionState::Off || banner_text(s).is_some())
}

fn tail_is_empty(tail: &TranscriptTail) -> bool {
    tail.final_text.trim().is_empty() && tail.partial_text.trim().is_empty()
}

/// The banner (C-S7) for a session, or none when the transcript runs or waits
/// for the next words (a gated or parked session is a normal pause — the
/// "no sound from the guest mic" banner needs the mic's packet age, which the
/// session does not carry).
fn banner_text(session: &MicTranscription) -> Option<&'static str> {
    match (session.state, session.reason) {
        (TranscriptionState::Reconnecting, _) => Some("Služba prepisu nedostupná — skúšam znova"),
        (_, Some(OffReason::Auth | OffReason::Payment)) => {
            Some("Prepis vypnutý — problém s účtom služby")
        }
        (_, Some(OffReason::CapReached)) => Some("Denný limit prepisu dosiahnutý"),
        (_, Some(reason @ (OffReason::Refused | OffReason::TooManyConnections))) => {
            Some(reason.slovak())
        }
        _ => None,
    }
}

/// The banner shown: the session's, else the socket's own when it is down.
fn shown_banner(session: Option<&MicTranscription>, ws: PassiveWsState) -> Option<&'static str> {
    session
        .and_then(banner_text)
        .or((ws == PassiveWsState::Reconnecting).then_some(SOCKET_DOWN))
}

/// The "▼ Naživo" button's text.
fn live_button_text(unseen: usize) -> String {
    if unseen == 0 {
        "▼ Naživo".to_string()
    } else {
        format!("▼ Naživo · {unseen} nové")
    }
}

/// The server's token of a state (`data-state`).
fn state_token(state: TranscriptionState) -> Option<String> {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
}

/// Client state of the column. `Copy` (signals only).
#[derive(Clone, Copy)]
pub struct TranscriptModel {
    sentences: RwSignal<Vec<TranscriptSentence>>,
    tails: RwSignal<Vec<TranscriptTail>>,
    sessions: RwSignal<Vec<MicTranscription>>,
    /// The reader is at the bottom: new text scrolls into view.
    following: RwSignal<bool>,
    /// Sentences closed while the reader looked back.
    unseen: RwSignal<usize>,
}

impl TranscriptModel {
    pub fn new() -> Self {
        Self {
            sentences: RwSignal::new(Vec::new()),
            tails: RwSignal::new(Vec::new()),
            sessions: RwSignal::new(Vec::new()),
            following: RwSignal::new(true),
            unseen: RwSignal::new(0),
        }
    }

    /// One `/listen/ws` message (the socket's handler), applied in place.
    pub fn apply(self, message: ListenWsMessage) {
        match message {
            ListenWsMessage::Snapshot { snapshot } => {
                let (sentences, tails, sessions) = snapshot_parts(snapshot);
                self.sentences.set(sentences);
                self.tails.set(tails);
                self.sessions.set(sessions);
            }
            ListenWsMessage::Sentence { sentence } => {
                if self.sentences.with_untracked(|s| is_newer(s, &sentence)) {
                    self.sentences.update(|s| {
                        push_sentence(s, sentence);
                    });
                    let following = self.following.get_untracked();
                    self.unseen.update(|n| *n = unseen_after(*n, following));
                }
            }
            ListenWsMessage::Tail { tail } => self.tails.update(|t| apply_tail(t, tail)),
            ListenWsMessage::Session { session } => {
                self.sessions.update(|s| apply_session(s, session));
            }
        }
    }

    /// `data-transcript`: the column has something to show.
    pub fn present(self) -> Memo<bool> {
        Memo::new(move |_| {
            let has_sentences = self.sentences.with(|s| !s.is_empty());
            let has_tail = self.tails.with(|t| !t.is_empty());
            self.sessions
                .with(|s| shows(has_sentences, has_tail, current_session(s)))
        })
    }
}

impl Default for TranscriptModel {
    fn default() -> Self {
        Self::new()
    }
}

fn at_bottom(el: &web_sys::Element) -> bool {
    el.scroll_height() - el.scroll_top() - el.client_height() <= BOTTOM_SLACK_PX
}

fn scroll_to_bottom(el: &web_sys::Element) {
    el.set_scroll_top(el.scroll_height());
}

/// The EN column (spec §8.2); `ws` is the `/listen/ws` socket's state.
#[component]
pub fn TranscriptColumn(model: TranscriptModel, ws: ReadSignal<PassiveWsState>) -> impl IntoView {
    let list = NodeRef::<leptos::html::Div>::new();
    let session = Memo::new(move |_| model.sessions.with(|s| current_session(s).cloned()));
    let banner = Memo::new(move |_| session.with(|s| shown_banner(s.as_ref(), ws.get())));
    let tail = Memo::new(move |_| model.tails.with(|t| newest_tail(t).cloned()));
    // New text while following: scroll it into view once it is rendered —
    // unless the reader scrolled up between the change and the frame.
    Effect::new(move |_| {
        model.sentences.track();
        tail.track();
        if model.following.get_untracked() {
            request_animation_frame(move || {
                if !model.following.get_untracked() {
                    return;
                }
                if let Some(el) = list.get_untracked() {
                    scroll_to_bottom(&el);
                }
            });
        }
    });
    let on_scroll = move |_| {
        if let Some(el) = list.get_untracked() {
            let bottom = at_bottom(&el);
            if bottom != model.following.get_untracked() {
                model.following.set(bottom);
            }
            if bottom {
                model.unseen.set(0);
            }
        }
    };
    let jump = move |_| {
        model.following.set(true);
        model.unseen.set(0);
        if let Some(el) = list.get_untracked() {
            scroll_to_bottom(&el);
        }
    };
    view! {
        <section class="interpreter-transcript" data-role="transcript"
            data-ws=move || ws.get().as_str()
            data-state=move || session.with(|s| s.as_ref().and_then(|s| state_token(s.state)))>
            <Show when=move || banner.with(Option::is_some)>
                <p class="interpreter-transcript__banner" data-role="transcript-banner" role="status">
                    {move || banner.get().unwrap_or_default()}
                </p>
            </Show>
            <div class="interpreter-transcript__list" data-role="transcript-list" node_ref=list on:scroll=on_scroll>
                <For each=move || model.sentences.get() key=|sentence| sentence.id let:sentence>
                    <p class="interpreter-transcript__sentence" data-role="transcript-sentence"
                        data-id=sentence.id.to_string()>{sentence.text}</p>
                </For>
                <Show when=move || tail.with(Option::is_some)>
                    <p class="interpreter-transcript__tail" data-role="transcript-tail">
                        <span>{move || tail.with(|t| t.as_ref().map(|t| t.final_text.clone()).unwrap_or_default())}</span>
                        <span class="interpreter-transcript__partial" data-partial="true">
                            {move || tail.with(|t| t.as_ref().map(|t| t.partial_text.clone()).unwrap_or_default())}
                        </span>
                    </p>
                </Show>
            </div>
            <Show when=move || !model.following.get()>
                <button type="button" class="interpreter-transcript__live" data-role="transcript-live"
                    on:click=jump>{move || live_button_text(model.unseen.get())}</button>
            </Show>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 22, 9, 30, second)
            .single()
            .unwrap()
    }

    fn sentence(id: u64) -> TranscriptSentence {
        TranscriptSentence {
            id,
            mic_id: "guest".into(),
            text: format!("Sentence {id}."),
            start_ms: id * 1_000,
            end_ms: id * 1_000 + 900,
            closed_at: at(0),
            language: Some("en".into()),
        }
    }

    fn tail(final_text: &str, partial: &str, second: u32) -> TranscriptTail {
        TranscriptTail {
            mic_id: "guest".into(),
            final_text: final_text.into(),
            partial_text: partial.into(),
            updated_at: at(second),
        }
    }

    fn session(state: TranscriptionState, reason: Option<OffReason>) -> MicTranscription {
        MicTranscription {
            mic_id: "guest".into(),
            state,
            reason,
            ..MicTranscription::default()
        }
    }

    #[test]
    fn a_snapshot_keeps_its_newest_300_in_order_and_drops_empty_tails() {
        let snapshot = TranscriptSnapshot {
            sentences: (10..=320).rev().map(sentence).collect(),
            tails: vec![tail("", " ", 1), tail("For God", " so", 2)],
            sessions: vec![session(TranscriptionState::Streaming, None)],
        };
        let (sentences, tails, sessions) = snapshot_parts(snapshot);
        assert_eq!(sentences.len(), MAX_SENTENCES);
        assert_eq!(sentences[0].id, 21, "the oldest beyond 300 are gone");
        assert_eq!(sentences.last().map(|s| s.id), Some(320));
        assert_eq!(tails.len(), 1, "an empty tail is not shown");
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn sentences_append_once_and_the_oldest_goes_past_300() {
        let mut sentences = Vec::new();
        assert!(push_sentence(&mut sentences, sentence(5)));
        assert!(!push_sentence(&mut sentences, sentence(5)), "a repeat");
        assert!(!push_sentence(&mut sentences, sentence(4)), "an older one");
        for id in 6..=305 {
            assert!(push_sentence(&mut sentences, sentence(id)));
        }
        assert_eq!(sentences.len(), MAX_SENTENCES);
        assert_eq!(sentences[0].id, 6);
    }

    #[test]
    fn only_a_reader_looking_back_counts_new_sentences() {
        assert_eq!(unseen_after(0, true), 0, "following: nothing to count");
        assert_eq!(unseen_after(2, false), 3);
        assert_eq!(live_button_text(0), "▼ Naživo");
        assert_eq!(live_button_text(3), "▼ Naživo · 3 nové");
    }

    #[test]
    fn a_tail_replaces_its_mics_tail_and_an_empty_one_clears_it() {
        let mut tails = Vec::new();
        apply_tail(&mut tails, tail("For God", " so lo", 1));
        apply_tail(&mut tails, tail("For God so", " loved", 2));
        assert_eq!(
            newest_tail(&tails).map(|t| t.partial_text.as_str()),
            Some(" loved")
        );
        assert_eq!(tails.len(), 1);
        apply_tail(&mut tails, tail("", "", 3));
        assert_eq!(
            newest_tail(&tails),
            None,
            "an empty tail clears the open sentence"
        );
    }

    #[test]
    fn the_column_shows_only_when_there_is_something_to_show() {
        assert!(
            !shows(false, false, None),
            "no session, no text: the tips stay full width"
        );
        let waiting = session(TranscriptionState::Off, Some(OffReason::OverrideOff));
        assert!(
            !shows(false, false, Some(&waiting)),
            "stopped for today is not worth a column"
        );
        let cap = session(TranscriptionState::Off, Some(OffReason::CapReached));
        assert!(shows(false, false, Some(&cap)), "the cap is worth a banner");
        let gated = session(TranscriptionState::Gated, None);
        assert!(shows(false, false, Some(&gated)));
        assert!(shows(true, false, None), "text from before stays readable");
        assert!(shows(false, true, None));
    }

    #[test]
    fn the_running_session_wins_over_a_replaced_mics_off() {
        let mut sessions = Vec::new();
        apply_session(&mut sessions, session(TranscriptionState::Streaming, None));
        let mut old = session(TranscriptionState::Off, Some(OffReason::NoGuestMic));
        old.mic_id = "old".into();
        apply_session(&mut sessions, old);
        assert_eq!(
            current_session(&sessions).map(|s| s.mic_id.as_str()),
            Some("guest")
        );
        apply_session(&mut sessions, session(TranscriptionState::Gated, None));
        assert_eq!(sessions.len(), 2, "a mic's session replaces its own");
    }

    #[test]
    fn the_banners_follow_c_s7_and_name_a_dead_socket() {
        let banner = |state, reason| banner_text(&session(state, reason));
        assert_eq!(
            banner(TranscriptionState::Reconnecting, None),
            Some("Služba prepisu nedostupná — skúšam znova")
        );
        assert_eq!(
            banner(TranscriptionState::Stopped, Some(OffReason::Payment)),
            Some("Prepis vypnutý — problém s účtom služby")
        );
        assert_eq!(
            banner(TranscriptionState::Off, Some(OffReason::CapReached)),
            Some("Denný limit prepisu dosiahnutý")
        );
        assert_eq!(banner(TranscriptionState::Streaming, None), None);
        assert_eq!(
            banner(TranscriptionState::Gated, None),
            None,
            "a pause is normal"
        );
        assert_eq!(
            banner(TranscriptionState::Parked, None),
            None,
            "five quiet minutes (a song, a prayer) are normal too"
        );
        let streaming = session(TranscriptionState::Streaming, None);
        assert_eq!(
            shown_banner(Some(&streaming), PassiveWsState::Reconnecting),
            Some(SOCKET_DOWN)
        );
        assert_eq!(
            shown_banner(Some(&streaming), PassiveWsState::Connected),
            None
        );
        let outage = session(TranscriptionState::Reconnecting, None);
        assert_eq!(
            shown_banner(Some(&outage), PassiveWsState::Reconnecting),
            banner_text(&outage),
            "the service's own reason first"
        );
        assert_eq!(
            state_token(TranscriptionState::Reconnecting).as_deref(),
            Some("reconnecting")
        );
    }
}
