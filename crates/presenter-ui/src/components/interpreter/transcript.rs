//! The interpreter's EN column (#852 C3-9, spec §8.2, plan Decision 15): the
//! guest's live English transcript from `GET /listen/ws`. Closed sentences
//! newest at the bottom (older ones dimmed, the newest 300 kept), then the open
//! sentence: its final words normal, the live (non-final) rest lighter. The
//! column follows the bottom while the reader is there; scrolling up pauses it
//! and shows "▼ Naživo · N nové" (N = sentences closed since), a tap jumps
//! back. A banner says when the transcript cannot come (C-S7). The column shows
//! only once there is something to show — a sentence, a session that runs, or
//! a reason worth a banner — so a box without transcription keeps the tips area
//! full width (`data-transcript="absent"`).
//!
//! Every (re)connect starts with the server's snapshot, which REPLACES the
//! model; the messages after it are applied in arrival order. The pure half
//! ([`Transcript`]) is host-tested; [`TranscriptModel`] holds it in signals.

use leptos::prelude::*;
use presenter_core::listen::transcription::{MicTranscription, OffReason, TranscriptionState};
use presenter_core::listen::ws::{ListenWsMessage, TranscriptSentence, TranscriptTail};

/// Sentences kept (and in the DOM).
pub const MAX_SENTENCES: usize = 300;
/// The reader is "at the bottom" within this many pixels.
const BOTTOM_SLACK_PX: i32 = 24;

/// The client half of the transcript: what `/listen/ws` delivered so far.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub sentences: Vec<TranscriptSentence>,
    pub tails: Vec<TranscriptTail>,
    pub sessions: Vec<MicTranscription>,
}

impl Transcript {
    /// Apply one message; returns how many sentences it added (the "N nové"
    /// count while the reader looks back). A snapshot replaces everything and
    /// adds nothing new to count.
    pub fn apply(&mut self, message: ListenWsMessage) -> usize {
        match message {
            ListenWsMessage::Snapshot { snapshot } => {
                *self = Self {
                    sentences: newest(snapshot.sentences),
                    tails: snapshot
                        .tails
                        .into_iter()
                        .filter(|tail| !tail_is_empty(tail))
                        .collect(),
                    sessions: snapshot.sessions,
                };
                0
            }
            ListenWsMessage::Sentence { sentence } => usize::from(self.push_sentence(sentence)),
            ListenWsMessage::Tail { tail } => {
                self.tails.retain(|t| t.mic_id != tail.mic_id);
                if !tail_is_empty(&tail) {
                    self.tails.push(tail);
                }
                0
            }
            ListenWsMessage::Session { session } => {
                self.sessions.retain(|s| s.mic_id != session.mic_id);
                self.sessions.push(session);
                0
            }
        }
    }

    /// A sentence newer than every one held (a repeat is ignored); the oldest
    /// goes past [`MAX_SENTENCES`].
    fn push_sentence(&mut self, sentence: TranscriptSentence) -> bool {
        if self
            .sentences
            .last()
            .is_some_and(|last| last.id >= sentence.id)
        {
            return false;
        }
        self.sentences.push(sentence);
        if self.sentences.len() > MAX_SENTENCES {
            self.sentences.remove(0);
        }
        true
    }

    /// The session the column speaks about: one that runs, else the newest
    /// one received (a replaced guest mic comes once as `off`).
    pub fn session(&self) -> Option<&MicTranscription> {
        self.sessions
            .iter()
            .rev()
            .find(|s| s.state != TranscriptionState::Off)
            .or_else(|| self.sessions.last())
    }

    /// The open sentence shown under the closed ones: the newest non-empty tail.
    pub fn tail(&self) -> Option<&TranscriptTail> {
        self.tails.iter().max_by_key(|tail| tail.updated_at)
    }

    /// Whether the column has anything to show (`data-transcript`).
    pub fn present(&self) -> bool {
        !self.sentences.is_empty()
            || self.tail().is_some()
            || self
                .session()
                .is_some_and(|s| s.state != TranscriptionState::Off || banner_text(s).is_some())
    }
}

/// The newest [`MAX_SENTENCES`] of a snapshot, oldest first.
fn newest(mut sentences: Vec<TranscriptSentence>) -> Vec<TranscriptSentence> {
    sentences.sort_by_key(|s| s.id);
    let cut = sentences.len().saturating_sub(MAX_SENTENCES);
    sentences.split_off(cut)
}

fn tail_is_empty(tail: &TranscriptTail) -> bool {
    tail.final_text.trim().is_empty() && tail.partial_text.trim().is_empty()
}

/// The banner (C-S7) for a session, or none when the transcript just runs /
/// waits for the next words.
pub fn banner_text(session: &MicTranscription) -> Option<&'static str> {
    match (session.state, session.reason) {
        (TranscriptionState::Reconnecting, _) => Some("Služba prepisu nedostupná — skúšam znova"),
        (TranscriptionState::Parked, _) => Some("Nie je zvuk z mikrofónu hosťa"),
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

/// The "▼ Naživo" button's text.
pub fn live_button_text(unseen: usize) -> String {
    if unseen == 0 {
        "▼ Naživo".to_string()
    } else {
        format!("▼ Naživo · {unseen} nové")
    }
}

/// Client state of the column. `Copy` (signals only): sentences, tails and
/// sessions in their own signals, so a tail every ~0.2 s never re-diffs the
/// sentence list.
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

    /// One `/listen/ws` message (the socket's handler).
    pub fn apply(self, message: ListenWsMessage) {
        let snapshot = matches!(message, ListenWsMessage::Snapshot { .. });
        let mut model = Transcript {
            sentences: self.sentences.get_untracked(),
            tails: self.tails.get_untracked(),
            sessions: self.sessions.get_untracked(),
        };
        let added = model.apply(message);
        if snapshot || added > 0 {
            self.sentences.set(model.sentences);
        }
        self.tails.set(model.tails);
        self.sessions.set(model.sessions);
        if !self.following.get_untracked() {
            self.unseen.update(|n| *n += added);
        }
    }

    fn view(self) -> Transcript {
        Transcript {
            sentences: self.sentences.get(),
            tails: self.tails.get(),
            sessions: self.sessions.get(),
        }
    }

    /// `data-transcript`: the column has something to show.
    pub fn present(self) -> Memo<bool> {
        Memo::new(move |_| self.view().present())
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

/// The EN column (spec §8.2).
#[component]
pub fn TranscriptColumn(model: TranscriptModel) -> impl IntoView {
    let list = NodeRef::<leptos::html::Div>::new();
    let session = Memo::new(move |_| {
        model.sessions.with(|s| {
            Transcript {
                sessions: s.clone(),
                ..Transcript::default()
            }
            .session()
            .cloned()
        })
    });
    let banner = Memo::new(move |_| session.with(|s| s.as_ref().and_then(banner_text)));
    let tail = Memo::new(move |_| {
        model.tails.with(|t| {
            Transcript {
                tails: t.clone(),
                ..Transcript::default()
            }
            .tail()
            .cloned()
        })
    });
    // New text while following: scroll it into view once it is rendered.
    Effect::new(move |_| {
        model.sentences.track();
        tail.track();
        if model.following.get_untracked() {
            request_animation_frame(move || {
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
            data-state=move || session.with(|s| s.as_ref().map(|s| format!("{:?}", s.state).to_lowercase()))>
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
    use presenter_core::listen::ws::TranscriptSnapshot;

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
    fn a_snapshot_replaces_everything_and_keeps_the_newest_300_in_order() {
        let mut model = Transcript::default();
        assert_eq!(
            model.apply(ListenWsMessage::Sentence {
                sentence: sentence(1)
            }),
            1
        );
        let snapshot = TranscriptSnapshot {
            sentences: (10..=320).rev().map(sentence).collect(),
            tails: vec![tail("", " ", 1)],
            sessions: vec![session(TranscriptionState::Streaming, None)],
        };
        assert_eq!(
            model.apply(ListenWsMessage::Snapshot { snapshot }),
            0,
            "nothing counted as new"
        );
        assert_eq!(model.sentences.len(), MAX_SENTENCES);
        assert_eq!(model.sentences[0].id, 21, "the oldest beyond 300 are gone");
        assert_eq!(model.sentences.last().map(|s| s.id), Some(320));
        assert!(model.tails.is_empty(), "an empty tail is not shown");
        assert!(model.present());
    }

    #[test]
    fn sentences_append_once_and_a_tail_replaces_its_mics_tail() {
        let mut model = Transcript::default();
        model.apply(ListenWsMessage::Tail {
            tail: tail("For God", " so lo", 1),
        });
        model.apply(ListenWsMessage::Tail {
            tail: tail("For God so", " loved", 2),
        });
        assert_eq!(
            model.tail().map(|t| t.partial_text.as_str()),
            Some(" loved")
        );
        assert_eq!(
            model.apply(ListenWsMessage::Sentence {
                sentence: sentence(5)
            }),
            1
        );
        assert_eq!(
            model.apply(ListenWsMessage::Sentence {
                sentence: sentence(5)
            }),
            0,
            "a repeat"
        );
        assert_eq!(
            model.apply(ListenWsMessage::Sentence {
                sentence: sentence(4)
            }),
            0,
            "an older one"
        );
        model.apply(ListenWsMessage::Tail {
            tail: tail("", "", 3),
        });
        assert_eq!(model.tail(), None, "an empty tail clears the open sentence");
        assert_eq!(model.sentences.len(), 1);
    }

    #[test]
    fn the_column_shows_only_when_there_is_something_to_show() {
        let mut model = Transcript::default();
        assert!(
            !model.present(),
            "no session, no text: the tips stay full width"
        );
        model.apply(ListenWsMessage::Session {
            session: session(TranscriptionState::Off, Some(OffReason::NotPreach)),
        });
        assert!(
            !model.present(),
            "waiting for the Kázeň layout is not worth a column"
        );
        model.apply(ListenWsMessage::Session {
            session: session(TranscriptionState::Off, Some(OffReason::CapReached)),
        });
        assert!(model.present(), "the cap is worth a banner");
        model.apply(ListenWsMessage::Session {
            session: session(TranscriptionState::Gated, None),
        });
        assert!(model.present());
    }

    #[test]
    fn the_running_session_wins_over_a_replaced_mics_off() {
        let mut model = Transcript::default();
        model.apply(ListenWsMessage::Session {
            session: session(TranscriptionState::Streaming, None),
        });
        let mut old = session(TranscriptionState::Off, Some(OffReason::NoGuestMic));
        old.mic_id = "old".into();
        model.apply(ListenWsMessage::Session { session: old });
        assert_eq!(model.session().map(|s| s.mic_id.as_str()), Some("guest"));
    }

    #[test]
    fn the_banners_follow_c_s7() {
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
        assert_eq!(
            banner(TranscriptionState::Parked, None),
            Some("Nie je zvuk z mikrofónu hosťa")
        );
        assert_eq!(banner(TranscriptionState::Streaming, None), None);
        assert_eq!(
            banner(TranscriptionState::Gated, None),
            None,
            "a pause is normal"
        );
        assert_eq!(live_button_text(0), "▼ Naživo");
        assert_eq!(live_button_text(3), "▼ Naživo · 3 nové");
    }
}
