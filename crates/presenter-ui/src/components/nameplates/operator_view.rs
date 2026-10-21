//! The operator's „Menovky" view (#871): the `stream` output's plates in
//! Stream Deck grid order, next to Worship / Bible / Timers / AI / Settings.
//!
//! The operator who runs the Stream Deck also keeps the plates, so he gets them
//! here and not only in the stream editor. The view is the shared plate list
//! ([`super::plate_list`]) with each row's button number (#870: 1 = the song,
//! then the person plates in list order), a line that explains the numbers, a
//! note once a plate falls outside the 4 × 3 grid, and a hint when `stream` has
//! no lower-third layer to show a plate on (no create button here; the layer
//! is designed in the editor).
//!
//! It mounts the first time the view opens and then stays mounted, like the
//! other views, so a half-typed plate survives a view switch and an operator
//! who never opens it costs no request. It keeps itself current from the
//! operator socket (plate list, plate on air, song, layer) and reloads after a
//! socket reconnect, when events may have been missed (a deploy restart drops
//! the plate on air).

use leptos::prelude::*;
use presenter_core::LiveEvent;

use super::numbering::any_beyond_grid;
use super::plate_list::{NameplateAddForm, NameplateRows, NameplateSongRow, RowOptions};
use super::{def_has_lower_third, NameplateStore};
use crate::state::AppContext;
use crate::ws::WsState;

/// The operator view id: `/ui/operator/menovky`, `body[data-view]`, the tab.
pub const MENOVKY_VIEW: &str = "menovky";

/// The output the view lists. Companion's grid serves only `stream` (#870:
/// only it has slot variables).
const OPERATOR_OUTPUT: &str = "stream";

/// Numbered rows, no editor extras (no Prehrať, nothing after Skryť).
const OPTIONS: RowOptions = RowOptions {
    preview: None,
    after_hide: None,
    numbered: true,
};

/// The Menovky panel body, mounted by `pages/operator.rs`.
#[component]
pub fn OperatorNameplates(
    last_event: ReadSignal<Option<LiveEvent>>,
    ws_state: ReadSignal<WsState>,
) -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let current_view = ctx.view;
    let (toast_message, toast_variant) = (ctx.toast_message, ctx.toast_variant);
    // Latches: false until the view first opens, then true for good.
    let opened = Memo::new(move |was: Option<&bool>| {
        was.copied().unwrap_or(false) || current_view.get() == MENOVKY_VIEW
    });
    view! {
        <Show when=move || opened.get()>
            <NameplateList
                last_event=last_event
                ws_state=ws_state
                toast_message=toast_message
                toast_variant=toast_variant
            />
        </Show>
    }
}

/// Load the `stream` def and record whether it has a lower-third layer (and
/// the revision it was read at). Two loads can overlap (two config events in a
/// row); a def older than the one already applied is dropped.
fn load_layer(has_layer: RwSignal<Option<bool>>, revision: StoredValue<u64>) {
    leptos::task::spawn_local(async move {
        match crate::api::stream::get_output_def(OPERATOR_OUTPUT).await {
            Ok(def) if def.config_revision >= revision.get_value() => {
                revision.set_value(def.config_revision);
                has_layer.set(Some(def_has_lower_third(&def)));
            }
            Ok(def) => leptos::logging::log!(
                "operator menovky: dropping a stale def (revision {})",
                def.config_revision
            ),
            Err(e) => {
                leptos::logging::log!("operator menovky: loading {OPERATOR_OUTPUT:?} failed: {e}")
            }
        }
    });
}

#[component]
fn NameplateList(
    last_event: ReadSignal<Option<LiveEvent>>,
    ws_state: ReadSignal<WsState>,
    toast_message: RwSignal<Option<String>>,
    toast_variant: RwSignal<String>,
) -> impl IntoView {
    let toast = Callback::new(move |(msg, variant): (String, &'static str)| {
        crate::state::show_toast_on(toast_message, toast_variant, &msg, variant);
    });
    let store = NameplateStore::new(RwSignal::new(OPERATOR_OUTPUT.to_string()), toast);
    // `None` until the def has loaded: no hint before we know.
    let has_layer = RwSignal::new(None::<bool>);
    let layer_revision = StoredValue::new(0_u64);
    store.load();
    load_layer(has_layer, layer_revision);

    // Live: plate list / plate on air / song through the store; a newer
    // `stream` config can add or remove the lower-third layer.
    Effect::new(move |_| {
        let Some(event) = last_event.get() else {
            return;
        };
        if let LiveEvent::StreamConfigChanged {
            output,
            config_revision,
        } = &event
        {
            if output == OPERATOR_OUTPUT && *config_revision > layer_revision.get_value() {
                load_layer(has_layer, layer_revision);
            }
        }
        store.apply_event(&event);
    });

    // A reconnect may have missed events: read everything again.
    let was_connected = std::cell::Cell::new(ws_state.get_untracked() == WsState::Connected);
    Effect::new(move |_| {
        let connected = ws_state.get() == WsState::Connected;
        if connected && !was_connected.get() {
            store.load();
            load_layer(has_layer, layer_revision);
        }
        was_connected.set(connected);
    });

    let beyond_grid = move || store.plates.with(|list| any_beyond_grid(list.len()));
    view! {
        <section class="nameplates-view" data-role="operator-nameplates">
            <header class="nameplates-view__head">
                <h2 class="nameplates-view__title">"Menovky"</h2>
                <p class="nameplates-view__help" data-role="operator-nameplates-help">
                    "Číslo pri riadku je tlačidlo na Stream Decku: 1 je pieseň, potom menovky v poradí tohto zoznamu. Šípkami ↑ ↓ presuniete menovku na iné tlačidlo."
                </p>
            </header>
            <Show when=move || has_layer.get() == Some(false)>
                <p class="nameplates-view__hint" data-role="operator-nameplates-no-layer">
                    "Výstup \u{201e}stream\u{201c} zatiaľ nemá menovkovú vrstvu, takže menovka sa v streame neukáže. Vytvoríte ju v Stream Graphics, záložka Menovky: "
                    <a href="/ui/stream?output=stream&tab=nameplates">"otvoriť Stream Graphics"</a>
                    "."
                </p>
            </Show>
            <NameplateSongRow store=store options=OPTIONS />
            <NameplateRows store=store options=OPTIONS />
            <Show when=beyond_grid>
                <p class="nameplates-view__note" data-role="operator-nameplates-grid-note">
                    "Mriežka 4 × 3 na Stream Decku má 12 tlačidiel: pieseň a 11 menoviek. Menovky od čísla 13 tam tlačidlo nemajú, pustíte ich len odtiaľto."
                </p>
            </Show>
            <NameplateAddForm store=store />
        </section>
    }
}
