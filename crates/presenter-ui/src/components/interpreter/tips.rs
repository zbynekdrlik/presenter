//! The šepkár tips area (#852 spec §9.6): cards newest on top (pair large,
//! note smaller, "Peter · 19:42"), a 2 s highlight for a new card, ✕ or a
//! left swipe hides a card with a 5 s "Skryté — Späť" offer, "Skryť všetky"
//! after a confirm. Tips are immutable, so a keyed `<For>` by id is enough.

use crate::api::{self, interpreter_tips::TipsScope, ApiError};
use crate::state::interpreter::{
    fresh_ids, swipe_decision, swipe_offset, tip_meta, tip_pair, FetchGuard, SwipeDecision,
};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::interpreter_tips::TipDto;
use std::collections::HashSet;
use wasm_bindgen::JsCast;
use web_sys::PointerEvent;

const UNDO_MS: u32 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UndoOffer {
    id: i64,
    ticket: u64,
}

/// Client state of the tips area. `Copy` (signals only), shared by the page,
/// the socket handler and the cards.
#[derive(Clone, Copy)]
pub struct TipsModel {
    tips: RwSignal<Vec<TipDto>>,
    /// The first list arrived (`data-ready`, the E2E settle signal).
    ready: RwSignal<bool>,
    known: StoredValue<HashSet<i64>>,
    fresh: StoredValue<HashSet<i64>>,
    guard: StoredValue<FetchGuard>,
    undo_offer: RwSignal<Option<UndoOffer>>,
    undo_ticket: StoredValue<u64>,
    error: RwSignal<Option<String>>,
}

impl TipsModel {
    pub fn new() -> Self {
        Self {
            tips: RwSignal::new(Vec::new()),
            ready: RwSignal::new(false),
            known: StoredValue::new(HashSet::new()),
            fresh: StoredValue::new(HashSet::new()),
            guard: StoredValue::new(FetchGuard::default()),
            undo_offer: RwSignal::new(None),
            undo_ticket: StoredValue::new(0),
            error: RwSignal::new(None),
        }
    }

    /// Install a full active list (live event, resync or own write).
    fn install(self, list: Vec<TipDto>) {
        let first = !self.ready.get_untracked();
        let fresh = self
            .known
            .with_value(|known| fresh_ids(known, &list, first));
        self.known
            .update_value(|known| known.extend(list.iter().map(|tip| tip.id)));
        self.fresh.set_value(fresh);
        self.error.set(None);
        self.tips.set(list);
        self.ready.set(true);
    }

    /// A live `InterpreterTips` list: newer than any resync in flight.
    pub fn apply_live(self, list: Vec<TipDto>) {
        self.guard.update_value(FetchGuard::note_newer);
        self.install(list);
    }

    /// `GET /interpreter/tips` on load and after every (re)connect; dropped
    /// when a live list or an own write overtook it.
    pub fn resync(self) {
        let ticket = self
            .guard
            .try_update_value(FetchGuard::begin)
            .unwrap_or_default();
        leptos::task::spawn_local(async move {
            match api::interpreter_tips::list(TipsScope::Active).await {
                Ok(list) => {
                    if self.guard.with_value(|guard| guard.accepts(ticket)) {
                        self.install(list.tips);
                    }
                }
                Err(err) => leptos::logging::log!("interpreter: tips resync failed: {err}"),
            }
        });
    }

    fn is_fresh(self, id: i64) -> bool {
        self.fresh.with_value(|fresh| fresh.contains(&id))
    }

    /// ✕ / swipe: hide at once, tell the server, offer "Späť" for 5 s.
    fn dismiss(self, id: i64) {
        self.guard.update_value(FetchGuard::note_newer);
        self.tips.update(|tips| tips.retain(|tip| tip.id != id));
        let ticket = self.undo_ticket.get_value().wrapping_add(1);
        self.undo_ticket.set_value(ticket);
        self.undo_offer.set(Some(UndoOffer { id, ticket }));
        Timeout::new(UNDO_MS, move || {
            let current = self.undo_offer.try_get_untracked().flatten();
            if current.is_some_and(|offer| offer.ticket == ticket) {
                let _ = self.undo_offer.try_set(None);
            }
        })
        .forget();
        leptos::task::spawn_local(async move {
            if let Err(err) = api::interpreter_tips::dismiss(id).await {
                self.fail("Tip sa nepodarilo skryť", err);
            }
        });
    }

    fn undo_dismiss(self) {
        let Some(offer) = self.undo_offer.get_untracked() else {
            return;
        };
        self.undo_offer.set(None);
        leptos::task::spawn_local(async move {
            match api::interpreter_tips::restore(offer.id).await {
                Ok(_) => self.resync(),
                Err(err) => self.fail("Tip sa nepodarilo vrátiť", err),
            }
        });
    }

    fn dismiss_all(self) {
        let confirmed = crate::utils::window::window()
            .confirm_with_message("Skryť všetky tipy?")
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        leptos::task::spawn_local(async move {
            if let Err(err) = api::interpreter_tips::dismiss_all().await {
                self.fail("Tipy sa nepodarilo skryť", err);
            }
        });
    }

    fn fail(self, what: &str, err: ApiError) {
        let message = match err {
            ApiError::Status(_, detail) => format!("{what}: {detail}"),
            other => format!("{what} — server je nedostupný ({other})."),
        };
        self.error.set(Some(message));
        self.resync();
    }
}

impl Default for TipsModel {
    fn default() -> Self {
        Self::new()
    }
}

#[component]
pub fn TipsArea(model: TipsModel) -> impl IntoView {
    let tips = move || model.tips.get();
    let empty = move || model.ready.get() && model.tips.with(Vec::is_empty);
    view! {
        <section
            class="interpreter-tips"
            data-role="interpreter-tips"
            data-ready=move || model.ready.get().to_string()
        >
            <header class="interpreter-tips__header">
                <h2 class="interpreter-tips__title">"TIPY OD ŠEPKÁRA"</h2>
                <button
                    type="button"
                    class="interpreter-tips__clear"
                    data-role="interpreter-tips-dismiss-all"
                    prop:disabled=move || model.tips.with(Vec::is_empty)
                    on:click=move |_| model.dismiss_all()
                >
                    "Skryť všetky"
                </button>
            </header>
            <Show when=move || model.error.with(Option::is_some)>
                <p class="interpreter-tips__error" data-role="interpreter-tips-error" role="alert">
                    {move || model.error.get().unwrap_or_default()}
                </p>
            </Show>
            <Show when=empty>
                <p class="interpreter-tips__empty" data-role="interpreter-tips-empty">
                    "Zatiaľ žiadne tipy. Šepkár ich posiela zo stránky /ui/sepkar."
                </p>
            </Show>
            <ul class="interpreter-tips__list" data-role="interpreter-tips-list">
                <For
                    each=tips
                    key=|tip| tip.id
                    children=move |tip| view! { <TipCard tip=tip model=model /> }
                />
            </ul>
            <Show when=move || model.undo_offer.with(Option::is_some)>
                <div class="interpreter-undo" data-role="interpreter-undo" role="status">
                    <span>"Skryté"</span>
                    <button
                        type="button"
                        class="interpreter-undo__button"
                        data-role="interpreter-undo-button"
                        on:click=move |_| model.undo_dismiss()
                    >
                        "Späť"
                    </button>
                </div>
            </Show>
        </section>
    }
}

/// True when the pointer went down on the card's ✕ — that is a tap on the
/// button, never the start of a swipe.
fn starts_on_dismiss_button(ev: &PointerEvent) -> bool {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| {
            el.closest("[data-role=\"interpreter-tip-dismiss\"]")
                .ok()
                .flatten()
        })
        .is_some()
}

#[component]
fn TipCard(tip: TipDto, model: TipsModel) -> impl IntoView {
    let id = tip.id;
    let pair = tip_pair(&tip);
    let note = tip.note.clone();
    let meta = tip_meta(&tip, &chrono::Local);
    let is_new = model.is_fresh(id);
    // Swipe: where the pointer went down, and how far the card follows it.
    // No pointer capture: it would retarget the ✕ button's click.
    let start = StoredValue::new(None::<(f64, f64)>);
    let offset = RwSignal::new(0.0_f64);
    let on_down = move |ev: PointerEvent| {
        if !starts_on_dismiss_button(&ev) {
            start.set_value(Some((f64::from(ev.client_x()), f64::from(ev.client_y()))));
        }
    };
    // `try_get_value`: a pointer event can still reach a card that was just
    // removed (its owner is disposed); that must be a no-op, never a panic.
    let on_move = move |ev: PointerEvent| {
        if let Some((x, _)) = start.try_get_value().flatten() {
            offset.set(swipe_offset(f64::from(ev.client_x()) - x));
        }
    };
    let on_up = move |ev: PointerEvent| {
        let Some((x, y)) = start.try_get_value().flatten() else {
            return;
        };
        start.set_value(None);
        let dx = f64::from(ev.client_x()) - x;
        let dy = f64::from(ev.client_y()) - y;
        match swipe_decision(dx, dy) {
            SwipeDecision::Dismiss => model.dismiss(id),
            SwipeDecision::SnapBack => offset.set(0.0),
        }
    };
    let on_cancel = move |_: PointerEvent| {
        start.set_value(None);
        offset.set(0.0);
    };
    let style = move || format!("transform: translateX({}px)", offset.get());
    view! {
        <li
            class="interpreter-tip"
            data-role="interpreter-tip"
            data-tip-id=id.to_string()
            data-new=is_new.to_string()
            style=style
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:pointerleave=on_cancel
        >
            {pair.map(|text| view! {
                <span class="interpreter-tip__pair" data-role="interpreter-tip-pair">{text}</span>
            })}
            {note.map(|text| view! {
                <span class="interpreter-tip__note" data-role="interpreter-tip-note">{text}</span>
            })}
            <span class="interpreter-tip__meta" data-role="interpreter-tip-meta">{meta}</span>
            <button
                type="button"
                class="interpreter-tip__dismiss"
                data-role="interpreter-tip-dismiss"
                aria-label="Skryť tip"
                on:click=move |_| model.dismiss(id)
            >
                "✕"
            </button>
        </li>
    }
}
