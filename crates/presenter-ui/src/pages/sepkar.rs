//! Šepkár page `/ui/sepkar` (#852 spec §9.7): anyone who can open Presenter
//! sends the interpreter a short tip — a word → its translation, or a note.
//! Phone first; on a laptop the "Dnes" list sits beside the form. The server
//! is the only validator: its Slovak 422 / 429 text shows inline and the
//! fields keep their content. The name is remembered per device
//! (`state::session`, non-throwing localStorage). The list refetches on every
//! `InterpreterTips` event and after every (re)connect.
//!
//! Every tip is rendered as a text node, never as HTML (`<b>` shows as `<b>`).

use crate::api::{self, interpreter_tips::TipsScope, ApiError};
use crate::components::version_label::VersionLabel;
use crate::state::interpreter::{
    note_counter, optional_text, sepkar_error_text, sepkar_notice, tip_meta, tip_pair, FetchGuard,
    SepkarRowState,
};
use crate::state::session;
use crate::ws::passive::{use_passive_websocket, LiveEventHandler, PassiveWsState};
use gloo_timers::callback::Timeout;
use leptos::html::Input;
use leptos::prelude::*;
use presenter_core::interpreter_tips::{NewTipRequest, TipDto, TipsList};
use presenter_core::LiveEvent;
use std::rc::Rc;
use web_sys::KeyboardEvent;

/// localStorage key (after the `presenter:` prefix `state::session` adds).
const NAME_KEY: &str = "sepkarName";
/// The "Odoslané 19:42" line stays on screen this long.
const NOTICE_MS: u32 = 4_000;

/// The form and the "Dnes" list. `Copy` (signals only), shared by the page,
/// the socket handler and the send task.
#[derive(Clone, Copy)]
struct SepkarForm {
    name: RwSignal<String>,
    source: RwSignal<String>,
    target: RwSignal<String>,
    note: RwSignal<String>,
    sending: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    notice: RwSignal<Option<String>>,
    notice_ticket: StoredValue<u64>,
    today: RwSignal<TipsList>,
    /// The first "Dnes" list arrived: before that the list is not "empty",
    /// it is not known yet.
    loaded: RwSignal<bool>,
    today_guard: StoredValue<FetchGuard>,
}

/// Empties `field` when it still holds what was sent; false when the šepkár
/// kept typing while the tip was on its way (their text is not ours to wipe).
fn clear_if_unchanged(field: RwSignal<String>, sent: &str) -> bool {
    let unchanged = field.with_untracked(|value| value.as_str() == sent);
    if unchanged {
        field.set(String::new());
    }
    unchanged
}

/// Enter that sends or moves on: not the Enter that confirms an IME / keyboard
/// composition.
fn is_plain_enter(ev: &KeyboardEvent) -> bool {
    ev.key() == "Enter" && !ev.is_composing()
}

fn focus(input: NodeRef<Input>) {
    if let Some(el) = input.get_untracked() {
        let _ = el.focus();
    }
}

impl SepkarForm {
    fn new() -> Self {
        Self {
            name: RwSignal::new(session::get_persistent(NAME_KEY).unwrap_or_default()),
            source: RwSignal::new(String::new()),
            target: RwSignal::new(String::new()),
            note: RwSignal::new(String::new()),
            sending: RwSignal::new(false),
            error: RwSignal::new(None),
            notice: RwSignal::new(None),
            notice_ticket: StoredValue::new(0),
            today: RwSignal::new(TipsList::default()),
            loaded: RwSignal::new(false),
            today_guard: StoredValue::new(FetchGuard::default()),
        }
    }

    /// Refetch today's tips; only the newest request may land.
    fn reload_today(self) {
        let ticket = self
            .today_guard
            .try_update_value(FetchGuard::begin)
            .unwrap_or_default();
        leptos::task::spawn_local(async move {
            match api::interpreter_tips::list(TipsScope::Today).await {
                Ok(list) => {
                    if self.today_guard.with_value(|guard| guard.accepts(ticket)) {
                        self.today.set(list);
                        self.loaded.set(true);
                    }
                }
                Err(err) => leptos::logging::log!("sepkar: today's tips failed: {err}"),
            }
        });
    }

    /// The line stays for `NOTICE_MS`; a newer line owns the timer (ticket).
    fn show_notice(self, text: String) {
        let ticket = self.notice_ticket.get_value().wrapping_add(1);
        self.notice_ticket.set_value(ticket);
        self.notice.set(Some(text));
        Timeout::new(NOTICE_MS, move || {
            if self.notice_ticket.try_get_value() == Some(ticket) {
                let _ = self.notice.try_set(None);
            }
        })
        .forget();
    }

    /// Send the tip. Success: the notice, the tip fields clear (those the
    /// šepkár did not touch meanwhile) and focus goes back to "Slovo".
    /// Refusal / offline: the Slovak error inline, the fields untouched.
    fn send(self, source_input: NodeRef<Input>) {
        if self.sending.get_untracked() {
            return;
        }
        let source = self.source.get_untracked();
        let target = self.target.get_untracked();
        let note = self.note.get_untracked();
        let request = NewTipRequest {
            sender_name: self.name.get_untracked(),
            source_term: optional_text(&source),
            target_term: optional_text(&target),
            note: optional_text(&note),
        };
        self.sending.set(true);
        self.error.set(None);
        self.notice.set(None);
        leptos::task::spawn_local(async move {
            let result = api::interpreter_tips::create(&request).await;
            self.sending.set(false);
            match result {
                Ok(response) => {
                    let cleared = [
                        clear_if_unchanged(self.source, &source),
                        clear_if_unchanged(self.target, &target),
                        clear_if_unchanged(self.note, &note),
                    ];
                    let text = sepkar_notice(response.duplicate, &response.tip, &chrono::Local);
                    self.show_notice(text);
                    if cleared.iter().all(|done| *done) {
                        focus(source_input);
                    }
                    self.reload_today();
                }
                Err(err) => self.fail(&err),
            }
        });
    }

    /// Show the Slovak line for a refused / failed send (the raw error goes
    /// to the console only).
    fn fail(self, err: &ApiError) {
        leptos::logging::log!("sepkar: send failed: {err}");
        let status = match err {
            ApiError::Status(code, detail) => Some((*code, detail.as_str())),
            _ => None,
        };
        self.error.set(Some(sepkar_error_text(status)));
    }
}

#[component]
pub fn SepkarPage() -> impl IntoView {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "sepkar");
    }
    let form = SepkarForm::new();
    let source_input = NodeRef::<Input>::new();
    let target_input = NodeRef::<Input>::new();
    let on_event: LiveEventHandler = Rc::new(move |event| {
        if matches!(event, LiveEvent::InterpreterTips { .. }) {
            form.reload_today();
        }
    });
    let ws = use_passive_websocket("sepkar", on_event);
    // Load on open (connects = 0) and after every (re)connect.
    Effect::new(move |_| {
        let _ = ws.connects.get();
        form.reload_today();
    });
    let connection = move || match ws.state.get() {
        PassiveWsState::Connected => "Spojenie so serverom: v poriadku",
        PassiveWsState::Connecting => "Spojenie so serverom: pripájam…",
        PassiveWsState::Reconnecting => "Spojenie so serverom: obnovujem…",
    };

    view! {
        <div class="sepkar__root" data-role="sepkar">
            <header class="sepkar__header">
                <h1 class="sepkar__title">"Šepkár pre tlmočníka"</h1>
                <span
                    class="sepkar__dot"
                    data-role="sepkar-ws"
                    data-ws=move || ws.state.get().as_str()
                    role="img"
                    aria-label=connection
                    title=connection
                ></span>
            </header>
            <TipForm form=form source_input=source_input target_input=target_input />
            <TodayList form=form />
            <footer class="sepkar__version"><VersionLabel /></footer>
        </div>
    }
}

/// The four fields, the counter, the send button and the two answer lines.
#[component]
fn TipForm(
    form: SepkarForm,
    source_input: NodeRef<Input>,
    target_input: NodeRef<Input>,
) -> impl IntoView {
    let on_name_key = move |ev: KeyboardEvent| {
        if is_plain_enter(&ev) {
            ev.prevent_default();
            focus(source_input);
        }
    };
    let on_source_key = move |ev: KeyboardEvent| {
        if is_plain_enter(&ev) {
            ev.prevent_default();
            focus(target_input);
        }
    };
    let on_send_key = move |ev: KeyboardEvent| {
        if is_plain_enter(&ev) {
            ev.prevent_default();
            form.send(source_input);
        }
    };
    let counter = move || note_counter(&form.note.get());

    view! {
        <section class="sepkar__form" data-role="sepkar-form">
            <label class="sepkar__field">
                <span class="sepkar__label">"Tvoje meno"</span>
                <input
                    class="sepkar__input"
                    data-role="sepkar-name"
                    type="text"
                    autocomplete="name"
                    enterkeyhint="next"
                    prop:value=move || form.name.get()
                    on:input=move |ev| {
                        let value = event_target_value(&ev);
                        session::set_persistent(NAME_KEY, &value);
                        form.name.set(value);
                    }
                    on:keydown=on_name_key
                />
            </label>
            <label class="sepkar__field">
                <span class="sepkar__label">"Slovo (EN)"</span>
                <input
                    class="sepkar__input"
                    data-role="sepkar-source"
                    type="text"
                    autocomplete="off"
                    autocapitalize="off"
                    spellcheck="false"
                    enterkeyhint="next"
                    node_ref=source_input
                    prop:value=move || form.source.get()
                    on:input=move |ev| form.source.set(event_target_value(&ev))
                    on:keydown=on_source_key
                />
            </label>
            <label class="sepkar__field">
                <span class="sepkar__label">"Preklad"</span>
                <input
                    class="sepkar__input"
                    data-role="sepkar-target"
                    type="text"
                    autocomplete="off"
                    enterkeyhint="send"
                    node_ref=target_input
                    prop:value=move || form.target.get()
                    on:input=move |ev| form.target.set(event_target_value(&ev))
                    on:keydown=on_send_key
                />
            </label>
            <label class="sepkar__field">
                <span class="sepkar__label">"Poznámka"</span>
                <input
                    class="sepkar__input"
                    data-role="sepkar-note"
                    type="text"
                    autocomplete="off"
                    enterkeyhint="send"
                    prop:value=move || form.note.get()
                    on:input=move |ev| form.note.set(event_target_value(&ev))
                    on:keydown=on_send_key
                />
            </label>
            <span
                class="sepkar__counter"
                data-role="sepkar-note-counter"
                data-over=move || counter().1.to_string()
            >
                {move || counter().0}
            </span>
            <button
                type="button"
                class="sepkar__send"
                data-role="sepkar-send"
                prop:disabled=move || form.sending.get()
                on:click=move |_| form.send(source_input)
            >
                "POSLAŤ TLMOČNÍKOVI"
            </button>
            <Show when=move || form.error.with(Option::is_some)>
                <p class="sepkar__error" data-role="sepkar-error" role="alert">
                    {move || form.error.get().unwrap_or_default()}
                </p>
            </Show>
            // The status region is always mounted and only its content comes
            // and goes: a region inserted together with its text is often not
            // announced by screen readers.
            <div class="sepkar__status" role="status">
                <Show when=move || form.notice.with(Option::is_some)>
                    <p class="sepkar__notice" data-role="sepkar-notice">
                        {move || form.notice.get().unwrap_or_default()}
                    </p>
                </Show>
            </div>
        </section>
    }
}

/// "Dnes": every tip of today, newest first, with where it stands.
#[component]
fn TodayList(form: SepkarForm) -> impl IntoView {
    let empty = move || form.loaded.get() && form.today.with(|list| list.tips.is_empty());
    let rows = move || {
        let list = form.today.get();
        list.tips
            .into_iter()
            .map(|tip| {
                let state = SepkarRowState::of(&tip, &list.active_ids);
                (tip, state)
            })
            .collect::<Vec<_>>()
    };
    view! {
        <section class="sepkar__today" data-role="sepkar-today">
            <h2 class="sepkar__today-title">"Dnes"</h2>
            <Show when=empty>
                <p class="sepkar__empty">"Dnes ešte žiadne tipy."</p>
            </Show>
            <ul class="sepkar__list">
                <For
                    each=rows
                    key=|(tip, state)| (tip.id, *state)
                    children=|(tip, state)| view! { <SepkarRow tip=tip state=state /> }
                />
            </ul>
        </section>
    }
}

#[component]
fn SepkarRow(tip: TipDto, state: SepkarRowState) -> impl IntoView {
    let text = match (tip_pair(&tip), tip.note.clone()) {
        (Some(pair), Some(note)) => format!("{pair} — {note}"),
        (Some(pair), None) => pair,
        (None, Some(note)) => note,
        (None, None) => String::new(),
    };
    let meta = format!("{} · {}", tip_meta(&tip, &chrono::Local), state.label());
    view! {
        <li
            class="sepkar__row"
            data-role="sepkar-row"
            data-tip-id=tip.id.to_string()
            data-state=state.as_str()
        >
            <div class="sepkar__row-text">{text}</div>
            <div class="sepkar__row-meta">{meta}</div>
        </li>
    }
}
