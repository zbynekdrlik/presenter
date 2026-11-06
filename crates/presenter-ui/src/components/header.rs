use leptos::prelude::*;

use crate::components::header_menus::{HeaderMenus, DIRECT_VIEWS, SETTINGS_VIEW};
use crate::components::more_sheet::open_more_sheet;
use crate::components::stage_layout_select::StageLayoutSelect;
use crate::components::stage_preview::{desktop_place, PreviewPlace, StagePreview};
use crate::components::stage_text_mode_picker::StageTextModePicker;
use crate::state::operator::OperatorState;
use crate::state::view_nav::{set_view, VIEWS};
use crate::state::AppContext;

/// #640: what a `/healthz` version-poll observation should do to this tab's
/// captured baseline. Pure decision logic (no I/O — no `web_sys`/fetch), so
/// it's testable on the HOST target without a real browser/WASM runtime (run
/// via `cd crates/presenter-ui && cargo test --lib`).
///
/// Before this, `known_version` was set ONLY by the one-shot mount fetch — a
/// single failed `/healthz` call (a transient blip) permanently disabled
/// version-drift detection for the tab's whole lifetime, because the
/// periodic poll only ever COMPARED against the baseline, it never seeded
/// it. Routing every poll observation (mount AND periodic) through this
/// function means the baseline self-heals on the NEXT successful call,
/// whenever that happens — not just the very first one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionPollOutcome {
    /// No baseline yet — seed it. Not a mismatch: this IS the first real
    /// observation, there is nothing to compare it against.
    SeedBaseline,
    /// Baseline known and this observation differs — the server was
    /// redeployed underneath this open tab.
    Mismatch,
    /// Baseline known and matches — nothing changed.
    Unchanged,
}

fn classify_version_poll(observed: &str, baseline: &str) -> VersionPollOutcome {
    if baseline.is_empty() {
        VersionPollOutcome::SeedBaseline
    } else if observed != baseline {
        VersionPollOutcome::Mismatch
    } else {
        VersionPollOutcome::Unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_baseline_seeds_regardless_of_observed_value() {
        assert_eq!(
            classify_version_poll("1.2.3", ""),
            VersionPollOutcome::SeedBaseline
        );
    }

    #[test]
    fn known_baseline_matching_observed_is_unchanged() {
        assert_eq!(
            classify_version_poll("1.2.3", "1.2.3"),
            VersionPollOutcome::Unchanged
        );
    }

    #[test]
    fn known_baseline_differing_observed_is_mismatch() {
        assert_eq!(
            classify_version_poll("1.2.4", "1.2.3"),
            VersionPollOutcome::Mismatch
        );
    }
}

#[component]
pub fn Header() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);

    // Search form handlers (songs and libraries; Biblia has no header
    // search since #850 B2c, the verse console searches the text).
    let on_search_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
    };

    let on_search_input = move |ev| {
        let val: String = event_target_value(&ev);
        op.search_query.set(val.clone());
        op.search_open.set(!val.is_empty());
    };

    let on_search_clear = move |_| {
        op.search_query.set(String::new());
        op.search_open.set(false);
        ctx.search_results.set(Vec::new());
    };

    // Mode toggle
    let set_mode = move |mode: &str| {
        let m = mode.to_string();
        ctx.mode.set(m.clone());
        crate::state::session::set("mode", &m);
    };

    // #850 §6.3: on a phone the chips, surfaces and Stage Output live in the
    // "Viac" sheet; mounting them here too would double their 5 s polls.
    let phone = ctx.phone_shell;
    let settings_label = VIEWS
        .into_iter()
        .find(|(id, _)| *id == SETTINGS_VIEW)
        .map_or("Nastavenia", |(_, label)| label);
    let on_more = {
        let op = op.clone();
        move |_| open_more_sheet(&op)
    };

    // #574: version-drift detection. An operator tab left open across a
    // deploy must recover on its own instead of silently misbehaving with
    // stale WASM until someone manually reloads (2026-07-24 SNV incident).
    //
    // NOTE: `presenter-ui` is workspace-EXCLUDED with its OWN unrelated
    // Cargo.toml version (0.1.x — see the deploy skill) — `env!("CARGO_PKG_VERSION")`
    // would therefore NEVER match the real server version and would show
    // this banner on every single page load. The only baseline that is
    // actually comparable to a later poll is the version THIS tab's own
    // first `/healthz` response reported.
    const VERSION_POLL_INTERVAL_MS: u32 = 15_000;

    let version_text = RwSignal::new(String::new());
    let known_version = RwSignal::new(String::new());
    let new_version_available = RwSignal::new(false);
    {
        leptos::task::spawn_local(async move {
            if let Ok(health) =
                crate::api::get_json::<crate::api::HealthzResponse>("/healthz").await
            {
                let text = if health.channel.is_empty() || health.channel == "release" {
                    format!("v{}", health.version)
                } else {
                    format!("v{} ({})", health.version, health.channel)
                };
                version_text.set(text);
                // #640 review: route through the SAME classifier as the
                // periodic poll below — normally this is always
                // `SeedBaseline` (baseline is empty at mount), but if the
                // periodic poll's tick already seeded a DIFFERENT baseline
                // before an unusually slow mount fetch finally resolves,
                // this must classify the observation too, not blindly
                // overwrite `known_version`.
                let baseline = known_version.get_untracked();
                match classify_version_poll(&health.version, &baseline) {
                    VersionPollOutcome::SeedBaseline => known_version.set(health.version),
                    VersionPollOutcome::Mismatch => new_version_available.set(true),
                    VersionPollOutcome::Unchanged => {}
                }
            }
        });
    }

    // #640: expose whether this tab has captured its version-poll baseline
    // yet, so E2E can gate the "start simulating a new version" switch on
    // the app's OWN state instead of a wall-clock guess — a slow WASM boot
    // (up to 30s) could otherwise race a hardcoded few-second grace window.
    // Mirrors the existing `__presenterLiveConnected`/`__presenterTabletReady`
    // test-helper pattern already used elsewhere in this crate.
    {
        use wasm_bindgen::prelude::*;

        let getter = Closure::wrap(Box::new(move || -> JsValue {
            JsValue::from_bool(!known_version.get_untracked().is_empty())
        }) as Box<dyn Fn() -> JsValue>);
        let _ = js_sys::Reflect::set(
            &crate::utils::window::window(),
            &JsValue::from_str("__presenterVersionBaselineKnown"),
            getter.as_ref(),
        );
        getter.forget();
    }

    // Poll periodically. #640: every observation — including this one, not
    // just the mount fetch above — routes through `classify_version_poll` so
    // an empty baseline (the mount fetch failed, or hasn't resolved yet)
    // self-heals here instead of silently skipping forever.
    {
        let interval = gloo_timers::callback::Interval::new(VERSION_POLL_INTERVAL_MS, move || {
            leptos::task::spawn_local(async move {
                if let Ok(health) =
                    crate::api::get_json::<crate::api::HealthzResponse>("/healthz").await
                {
                    let baseline = known_version.get_untracked();
                    match classify_version_poll(&health.version, &baseline) {
                        VersionPollOutcome::SeedBaseline => known_version.set(health.version),
                        VersionPollOutcome::Mismatch => new_version_available.set(true),
                        VersionPollOutcome::Unchanged => {}
                    }
                }
            });
        });
        interval.forget();
    }

    let reload_now = move |_| {
        if let Some(w) = web_sys::window() {
            let _ = w.location().reload();
        }
    };

    view! {
        // #574: prominent, dismiss-free "new version" banner. Never yanks
        // the page mid-action on its own — the operator clicks Reload when
        // ready.
        <Show when=move || new_version_available.get() fallback=|| ()>
            <div class="operator__version-banner" data-role="version-update-banner">
                <span>"Je k dispozícii nová verzia."</span>
                <button
                    type="button"
                    data-role="version-update-reload"
                    on:click=reload_now
                >"Načítať znova"</button>
            </div>
        </Show>
        <header class="operator__header">
            <div class="operator__header-left">
                <div class="operator__header-brand">
                    <h1>"Presenter"</h1>
                    // #850 §6.3: the live connection at a glance (phone shell).
                    <span
                        class="operator__ws-dot"
                        data-role="ws-dot"
                        data-connected=move || if ctx.ws_connected.get() { "true" } else { "false" }
                        title=move || if ctx.ws_connected.get() { "Pripojené" } else { "Nepripojené" }
                    ></span>
                    <span class="operator__version-badge">{move || version_text.get()}</span>
                </div>
                // #850 B2c: on a phone Biblia has no header search (the verse
                // console's field searches the text). The desktop keeps the
                // song search on every section, Biblia too (mockup v3, #875).
                <Show when=move || !phone.get() || ctx.view.get() != "bible">
                    <form class="operator__search" data-role="global-search-form" role="search" autocomplete="off"
                        on:submit=on_search_submit
                    >
                        <span class="operator__search-icon" aria-hidden="true"></span>
                        <input
                            type="search"
                            placeholder="Hľadať knižnice, piesne, slajdy"
                            data-role="global-search-query"
                            aria-label="Hľadať v Presenteri"
                            autocomplete="off"
                            prop:value=move || op.search_query.get()
                            on:input=on_search_input
                        />
                        <button
                            type="button"
                            data-role="global-search-clear"
                            aria-label="Vymazať hľadanie"
                            on:click=on_search_clear
                        >
                            <span aria-hidden="true">{"\u{00d7}"}</span>
                            <span class="sr-only">"Vymazať hľadanie"</span>
                        </button>
                    </form>
                </Show>
            </div>
            // #875 H1: Piesne and Biblia keep their own buttons; the other
            // sections are in the Stránky menu, Nastavenia behind ⚙.
            <nav class="operator__view-nav">
                {VIEWS.into_iter().filter(|(id, _)| DIRECT_VIEWS.contains(id)).map(|(id, label)| {
                    view! {
                        <button
                            type="button"
                            data-role="view-toggle"
                            data-view=id
                            data-active=move || if ctx.view.get() == id { "true" } else { "false" }
                            on:click=move |_| set_view(ctx.view, id)
                        >
                            {label}
                        </button>
                    }
                }).collect_view()}
            </nav>
            // On a phone the surfaces and chips live in the "Viac" sheet;
            // mounting them here too would double their 5 s polls (#850 §6.3).
            <Show when=move || !phone.get()>
                <HeaderMenus version=version_text />
            </Show>
            <div class="operator__header-right">
                <Show when=move || !phone.get()>
                    <StageLayoutSelect />
                    // #799: text mode for the API layouts, only while one is selected.
                    <StageTextModePicker />
                </Show>
                // #875 H2: one stage preview at a time. A phone keeps it here
                // (no mirror); a desktop shows it under the Piesne / Biblia
                // left column and here only on the other sections (counter +
                // broom).
                <Show when=move || phone.get()>
                    <StagePreview place=PreviewPlace::Phone />
                </Show>
                <Show when=move || {
                    !phone.get() && ctx.view.with(|v| desktop_place(v) == PreviewPlace::Header)
                }>
                    <StagePreview place=PreviewPlace::Header />
                </Show>
                <div class="operator__mode-toggle">
                    {["live", "edit"].into_iter().map(|m| {
                        let mode_name = m.to_string();
                        let label = match m { "live" => "Naživo", "edit" => "Úprava", _ => m };
                        let mn = mode_name.clone();
                        view! {
                            <button
                                type="button"
                                data-role="mode-toggle"
                                data-mode=mode_name.clone()
                                data-active=move || if ctx.mode.get() == mn { "true" } else { "false" }
                                on:click={
                                    let mn2 = mode_name.clone();
                                    move |_| set_mode(&mn2)
                                }
                            >
                                {label}
                            </button>
                        }
                    }).collect_view()}
                </div>
                <Show when=move || !phone.get()>
                    <button
                        type="button"
                        class="operator__settings-button"
                        data-role="view-toggle"
                        data-view=SETTINGS_VIEW
                        data-active=move || if ctx.view.get() == SETTINGS_VIEW { "true" } else { "false" }
                        aria-label=settings_label
                        title=settings_label
                        on:click=move |_| set_view(ctx.view, SETTINGS_VIEW)
                    >
                        // The gear is drawn by CSS: the text stays the label.
                        <span class="sr-only">{settings_label}</span>
                    </button>
                </Show>
                <Show when=move || phone.get()>
                    <button
                        type="button"
                        class="operator__more"
                        data-role="more-toggle"
                        aria-haspopup="dialog"
                        on:click=on_more.clone()
                    >
                        "Viac"
                    </button>
                </Show>
            </div>
        </header>
    }
}
