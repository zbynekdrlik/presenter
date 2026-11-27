//! The guest page (#999 L3) at the ROOT of the guest-only listener
//! (`PRESENTER_GUEST_PORT`; the guests' public hostname points there, ROZHODNUTÉ 27.11): a guest who does not understand Slovak hears the
//! sermon live in their own language, with the text on screen. Phone first,
//! no login. Screen 1 lists the languages in their own names (`?lang=xx` keeps
//! the pick over a reload); screen 2 has ONE big Play — the tap that unlocks
//! audio on iOS — the session's state, the transcript and the optional Slovak
//! original. While playing, a screen wake lock keeps the phone awake.
//! The UI copy is English (owner, 27.11: the page is for foreigners), the
//! language names are their own.
//! `/ui/guest-qr` (a normal host) is the operator's QR code of it (`qr.rs`).

mod audio;
mod qr;

pub use qr::GuestQrPage;

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use presenter_core::listen::guest::{
    guest_language, GuestLanguages, GuestRefusal, GuestSessionState, GuestWsMessage,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use self::audio::{AudioOut, PlayerReport};
use crate::state::guest::{refusal_text, state_text, Transcript};
use crate::ws::guest::{GuestFrame, GuestStream};

/// The server's speech rate until its `hello` says otherwise.
const DEFAULT_RATE: u32 = 24_000;

/// What runs while the guest listens (not reactive: browser objects).
#[derive(Default)]
struct Listening {
    audio: Option<AudioOut>,
    stream: Option<GuestStream>,
    wake_lock: Option<JsValue>,
    rate: u32,
    chunks: u64,
    /// Bumped by every Play and Stop: an audio / wake-lock start that
    /// finishes after a newer Play or a Stop drops what it made.
    start: u64,
}

impl Listening {
    fn release_wake_lock(&mut self) {
        if let Some(lock) = self.wake_lock.take() {
            audio::release_wake_lock(&lock);
        }
    }
}

/// The page's reactive state.
#[derive(Clone, Copy)]
struct Page {
    chosen: RwSignal<Option<String>>,
    playing: RwSignal<bool>,
    session: RwSignal<Option<GuestSessionState>>,
    refused: RwSignal<Option<GuestRefusal>>,
    audio_failed: RwSignal<bool>,
    transcript: RwSignal<Transcript>,
}

fn set_body(name: &str, value: &str) {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute(name, value);
    }
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// The offered languages and whether the box can translate now.
fn load_languages(languages: RwSignal<Option<GuestLanguages>>, load_failed: RwSignal<bool>) {
    leptos::task::spawn_local(async move {
        match crate::api::guest::languages().await {
            Ok(list) => {
                load_failed.set(false);
                languages.set(Some(list));
            }
            Err(err) => {
                leptos::logging::log!("guest: languages not loaded: {err:?}");
                load_failed.set(true);
            }
        }
    });
}

#[component]
pub fn GuestPage() -> impl IntoView {
    install_pwa();
    let languages = RwSignal::new(None::<GuestLanguages>);
    let load_failed = RwSignal::new(false);
    let version = RwSignal::new(String::new());
    load_languages(languages, load_failed);
    leptos::task::spawn_local(async move {
        if let Ok(health) = crate::api::get_json::<crate::api::HealthzResponse>("/healthz").await {
            version.set(format!("v{}", health.version));
        }
    });
    let chosen = crate::utils::window::url_param("lang").filter(|c| guest_language(c).is_some());
    let page = Page {
        chosen: RwSignal::new(chosen),
        playing: RwSignal::new(false),
        session: RwSignal::new(None),
        refused: RwSignal::new(None),
        audio_failed: RwSignal::new(false),
        transcript: RwSignal::new(Transcript::default()),
    };
    let listening = Rc::new(RefCell::new(Listening::default()));
    watch_visibility(page, Rc::clone(&listening));
    let show_original = RwSignal::new(false);
    Effect::new(move |_| {
        set_body(
            "data-original",
            if show_original.get() { "true" } else { "false" },
        )
    });
    let screen = move || {
        if page.chosen.get().is_some() {
            "player"
        } else {
            "picker"
        }
    };

    let stop_on_change = Rc::clone(&listening);
    let pick = move |code: Option<String>| {
        stop(page, &stop_on_change);
        page.transcript.set(Transcript::default());
        crate::utils::window::replace_url_param("lang", code.as_deref().unwrap_or(""));
        if code.is_none() {
            // Back on the list: which languages run now (●) has changed.
            load_languages(languages, load_failed);
        }
        page.chosen.set(code);
    };
    let pick = StoredValue::new_local(pick);
    let held = StoredValue::new_local(Rc::clone(&listening));
    let toggle_play = move |_| {
        held.with_value(|listening| {
            if page.playing.get_untracked() {
                stop(page, listening);
            } else {
                start(page, listening);
            }
        });
    };

    view! {
        <div class="guest" data-role="guest-page" data-screen=screen>
            <header class="guest__head">
                <img src="/icon-192.png" alt="" class="guest__logo" />
                <h1>"Sermon translation"</h1>
                <span class="guest__lang" data-role="guest-current-language">
                    {move || page.chosen.get().and_then(|c| guest_language(&c)).map(|(_, name)| name).unwrap_or("")}
                </span>
            </header>
            <Show when=move || page.chosen.get().is_none() fallback=|| ()>
                <section class="guest__picker">
                    <p class="guest__hint">"Put on your headphones and pick your language. You will hear the Slovak sermon translated live, with the text on screen."</p>
                    <Show when=move || load_failed.get() fallback=|| ()>
                        <p class="guest__error" data-role="guest-load-error">"The languages did not load — please reload the page."</p>
                    </Show>
                    <div class="guest__languages" data-role="guest-languages">
                        {move || languages.get().map(|list| language_buttons(&list, move |code| pick.with_value(|f| f(Some(code)))))}
                    </div>
                    <p class="guest__error" data-role="guest-unavailable">
                        {move || languages.get().and_then(|l| l.refusal).map(refusal_text).unwrap_or("")}
                    </p>
                </section>
            </Show>
            <Show when=move || page.chosen.get().is_some() fallback=|| ()>
                <section class="guest__player">
                    <div class="guest__controls">
                        <button
                            type="button"
                            class="guest__play"
                            data-role="guest-play"
                            data-playing=move || if page.playing.get() { "true" } else { "false" }
                            aria-label=move || if page.playing.get() { "Stop" } else { "Play" }
                            on:click=toggle_play
                        >{move || if page.playing.get() { "■" } else { "▶" }}</button>
                        <div class="guest__status">
                            <span class="guest__state" data-role="guest-state" data-state=move || status_line(page).0>
                                {move || status_line(page).1}
                            </span>
                            <span class="guest__sub">"You hear the translation about 3 seconds after the preacher."</span>
                        </div>
                    </div>
                    <div class="guest__row">
                        <button type="button" class="guest__small" data-role="guest-change-language" on:click=move |_| pick.with_value(|f| f(None))>
                            "Change language"
                        </button>
                        <label class="guest__small">
                            <input type="checkbox" data-role="guest-original-toggle" on:change=move |ev| show_original.set(event_target_checked(&ev)) />
                            "Slovak original"
                        </label>
                    </div>
                    <TranscriptView transcript=page.transcript />
                </section>
            </Show>
            <footer class="guest__foot" data-role="guest-version">
                "Live machine translation · " {move || version.get()}
            </footer>
        </div>
    }
}

fn language_buttons(
    list: &GuestLanguages,
    on_pick: impl Fn(String) + Copy + 'static,
) -> impl IntoView {
    list.languages
        .iter()
        .map(|language| {
            let code = language.code.clone();
            let active = list.active.contains(&code);
            let disabled = !list.can_pick(&code);
            let pick_code = code.clone();
            view! {
                <button
                    type="button"
                    class="guest__language"
                    data-role="guest-language"
                    data-code=code
                    data-active=if active { "true" } else { "false" }
                    disabled=disabled
                    on:click=move |_| on_pick(pick_code.clone())
                >{language.name.clone()}</button>
            }
        })
        .collect_view()
}

/// `data-state` + the line under Play.
fn status_line(page: Page) -> (&'static str, &'static str) {
    if let Some(reason) = page.refused.get() {
        return ("refused", refusal_text(reason));
    }
    if page.audio_failed.get() {
        return ("refused", "Audio could not start — tap ▶ again");
    }
    if !page.playing.get() {
        return ("idle", "Tap ▶ to listen");
    }
    match page.session.get() {
        Some(state) => state_text(&state),
        None => ("connecting", "Connecting…"),
    }
}

/// Play: the audio context in the tap itself, then the stream.
fn start(page: Page, listening: &Rc<RefCell<Listening>>) {
    let Some(code) = page.chosen.get_untracked() else {
        return;
    };
    let context = match audio::new_context(DEFAULT_RATE) {
        Ok(context) => context,
        Err(err) => {
            leptos::logging::warn!("guest: no audio context: {err:?}");
            page.audio_failed.set(true);
            return;
        }
    };
    // iOS unlocks audio only inside the tap: resume here, synchronously.
    let _ = context.resume();
    page.audio_failed.set(false);
    page.refused.set(None);
    page.session.set(None);
    page.playing.set(true);
    let start = {
        let mut state = listening.borrow_mut();
        state.rate = DEFAULT_RATE;
        state.chunks = 0;
        state.start += 1;
        state.start
    };
    set_body("data-guest-chunks", "0");
    set_body("data-guest-played", "0");
    let for_audio = Rc::clone(listening);
    leptos::task::spawn_local(async move {
        let report = |report: PlayerReport| {
            set_body("data-guest-played", &format!("{}", report.played));
            set_body("data-guest-underruns", &format!("{}", report.underruns));
        };
        let current = |state: &Listening| state.start == start;
        match AudioOut::start(context, report).await {
            Ok(out) if current(&for_audio.borrow()) => {
                leptos::logging::log!("guest: audio running at {} Hz", out.rate());
                for_audio.borrow_mut().audio = Some(out);
            }
            Ok(_) => {}
            Err(err) => {
                leptos::logging::warn!("guest: audio did not start: {err:?}");
                page.audio_failed.set(true);
            }
        }
        let lock = audio::request_wake_lock().await;
        let mut state = for_audio.borrow_mut();
        if current(&state) {
            state.release_wake_lock();
            state.wake_lock = lock;
        } else if let Some(lock) = lock {
            audio::release_wake_lock(&lock);
        }
    });
    let for_frames = Rc::clone(listening);
    let stream = GuestStream::open(
        &code,
        Rc::new(move |frame| on_frame(page, &for_frames, frame)),
    );
    listening.borrow_mut().stream = Some(stream);
}

fn stop(page: Page, listening: &Rc<RefCell<Listening>>) {
    page.playing.set(false);
    page.session.set(None);
    let mut state = listening.borrow_mut();
    state.start += 1;
    state.stream = None;
    state.audio = None;
    state.release_wake_lock();
}

fn on_frame(page: Page, listening: &Rc<RefCell<Listening>>, frame: GuestFrame) {
    match frame {
        GuestFrame::Speech(bytes) => {
            let mut state = listening.borrow_mut();
            state.chunks += 1;
            let chunks = state.chunks;
            if let Some(audio) = &state.audio {
                audio.push(&bytes, state.rate);
            }
            drop(state);
            set_body("data-guest-chunks", &chunks.to_string());
        }
        GuestFrame::Message(message) => on_message(page, listening, message),
        GuestFrame::Lost { retry_ms } => {
            leptos::logging::log!("guest: stream lost, again in {retry_ms} ms");
            page.session.set(Some(GuestSessionState::Reconnecting));
            if let Some(audio) = &listening.borrow().audio {
                audio.reset();
            }
        }
    }
}

fn on_message(page: Page, listening: &Rc<RefCell<Listening>>, message: GuestWsMessage) {
    match message {
        GuestWsMessage::Hello { sample_rate, .. } => listening.borrow_mut().rate = sample_rate,
        GuestWsMessage::State { session } => {
            // A language stopped for good: the screen may sleep again.
            if matches!(session, GuestSessionState::Stopped { .. }) {
                listening.borrow_mut().release_wake_lock();
            }
            page.session.set(Some(session));
        }
        GuestWsMessage::Text { text } => page.transcript.update(|t| t.push_text(&text, now_ms())),
        GuestWsMessage::Original { text } => page.transcript.update(|t| t.push_original(&text)),
        GuestWsMessage::Turn => page.transcript.update(Transcript::end_turn),
        GuestWsMessage::Refused { reason } => {
            page.refused.set(Some(reason));
            let mut state = listening.borrow_mut();
            state.start += 1;
            state.stream = None;
            state.audio = None;
            state.release_wake_lock();
            drop(state);
            page.playing.set(false);
        }
    }
}

/// The phone woke up again: resume the audio and take the wake lock again
/// (a browser releases it when the page is hidden).
fn watch_visibility(page: Page, listening: Rc<RefCell<Listening>>) {
    let document = crate::utils::window::document();
    let on_change = Closure::<dyn FnMut()>::new(move || {
        let visible = !crate::utils::window::document().hidden();
        if !visible || !page.playing.get_untracked() {
            return;
        }
        if let Some(audio) = &listening.borrow().audio {
            audio.resume();
        }
        let listening = Rc::clone(&listening);
        leptos::task::spawn_local(async move {
            if let Some(lock) = audio::request_wake_lock().await {
                let mut state = listening.borrow_mut();
                if page.playing.get_untracked() {
                    state.release_wake_lock();
                    state.wake_lock = Some(lock);
                } else {
                    audio::release_wake_lock(&lock);
                }
            }
        });
    });
    let _ = document
        .add_event_listener_with_callback("visibilitychange", on_change.as_ref().unchecked_ref());
    on_change.forget();
}

#[component]
fn TranscriptView(transcript: RwSignal<Transcript>) -> impl IntoView {
    let node = NodeRef::<leptos::html::Div>::new();
    let follow = RwSignal::new(true);
    Effect::new(move |_| {
        transcript.track();
        if follow.get_untracked() {
            if let Some(el) = node.get_untracked() {
                el.set_scroll_top(el.scroll_height());
            }
        }
    });
    let on_scroll = move |_| {
        if let Some(el) = node.get_untracked() {
            follow.set(el.scroll_top() + el.client_height() >= el.scroll_height() - 40);
        }
    };
    view! {
        <div class="guest__transcript" data-role="guest-transcript" node_ref=node on:scroll=on_scroll aria-live="polite">
            <Show when=move || transcript.with(|t| t.paragraphs.is_empty()) fallback=|| ()>
                <p class="guest__empty">"The translated text appears here."</p>
            </Show>
            <For
                each=move || transcript.with(|t| t.paragraphs.clone())
                key=|p| (p.id, p.text.len(), p.original.len())
                let:paragraph
            >
                <div class="guest__paragraph" data-role="guest-paragraph">
                    <p class="guest__text">{paragraph.text}</p>
                    <p class="guest__original" data-role="guest-original">{paragraph.original}</p>
                </div>
            </For>
        </div>
    }
}

/// Install-as-an-app tags, as the study page does. No credentials: the page
/// is public (the guest listener has no login in front of it).
fn install_pwa() {
    let document = crate::utils::window::document();
    document.set_title("Sermon translation");
    let Some(head) = document.head() else {
        return;
    };
    let tags: [(&str, &[(&str, &str)]); 6] = [
        ("link", &[("rel", "manifest"), ("href", "/manifest.json")]),
        (
            "link",
            &[("rel", "apple-touch-icon"), ("href", "/icon-192.png")],
        ),
        (
            "meta",
            &[("name", "mobile-web-app-capable"), ("content", "yes")],
        ),
        (
            "meta",
            &[("name", "apple-mobile-web-app-capable"), ("content", "yes")],
        ),
        (
            "meta",
            &[
                ("name", "apple-mobile-web-app-title"),
                ("content", "Translation"),
            ],
        ),
        ("meta", &[("name", "theme-color"), ("content", "#0f172a")]),
    ];
    for (tag, attrs) in tags {
        if let Ok(element) = document.create_element(tag) {
            for (name, value) in attrs {
                let _ = element.set_attribute(name, value);
            }
            let _ = head.append_child(&element);
        }
    }
    let _ = js_sys::eval(
        "if ('serviceWorker' in navigator) { navigator.serviceWorker.register('/sw.js').catch(function () {}); }",
    );
}
