//! NDI video-sources card for the settings page (#347).

use leptos::prelude::*;

use super::ToastHandle;
use crate::api::ndi::{self, VideoSourceDto, VideoSourceStatusDto};
use crate::components::modal::confirm;

/// The badge text for a source state (#546).
///
/// Plain operator language, not protocol language: the person reading this is standing
/// in a church about to run a service, and the words have to tell them what to DO.
///
/// The empty string means "this row has no status yet" — the first paint, or a row added
/// since the last poll. That is NOT the same as `unknown` ("this server cannot see the
/// NDI network"), and printing the latter for the former would tell an operator with a
/// perfectly healthy server that NDI is unavailable. Anything unrecognised degrades to
/// the same honest "Checking…" rather than rendering a raw wire token.
pub(crate) fn status_label(state: &str) -> &'static str {
    match state {
        "live" => "Live",
        "not-broadcasting" => "Not broadcasting",
        "not-found" => "Not found on the network",
        "ready" => "Ready",
        "connecting" => "Connecting…",
        "unknown" => "NDI unavailable",
        _ => "Checking…",
    }
}

/// The CSS class pair for a source state (#546). An unrecognised (or not-yet-loaded)
/// state falls back to `--checking` rather than injecting whatever the server said into
/// a class name.
pub(crate) fn status_class(state: &str) -> String {
    let known = matches!(
        state,
        "live" | "not-broadcasting" | "not-found" | "ready" | "connecting" | "unknown"
    );
    let modifier = if known { state } else { "checking" };
    format!("settings__status settings__status--{modifier}")
}

/// What the operator should DO about this state — shown under the row. `None` when the
/// state needs no action (live / ready / connecting).
///
/// These two sentences are the entire point of #546. At PP the operator had a blank
/// stage and no idea whether to look at the server, the network, the TV, or the sending
/// machine. The answer was the sending machine — and now the page says so.
pub(crate) fn status_hint(state: &str) -> Option<&'static str> {
    match state {
        "not-found" => Some(
            "This NDI name is not on the network. Check the sending machine is on and its \
             NDI output is switched on — or pick one of the names listed below.",
        ),
        "not-broadcasting" => Some(
            "On the network, but sending no video. Start the NDI output on the sending \
             machine.",
        ),
        _ => None,
    }
}

#[component]
pub fn VideoSourcesCard(toast: ToastHandle) -> impl IntoView {
    let sources = RwSignal::new(Vec::<VideoSourceDto>::new());
    let ndi_available = RwSignal::new(false);
    let discovered = RwSignal::new(Vec::<String>::new());
    let statuses = RwSignal::new(Vec::<VideoSourceStatusDto>::new());
    let new_label = RwSignal::new(String::new());
    let new_ndi_name = RwSignal::new(String::new());

    // #546: one call answers all three questions the operator has — can this server see
    // the NDI network, what is ON it, and is each mapped source actually working. Polled
    // on the same 5s cadence as every other live-status card on this page, because a
    // source can go silent at any moment, including mid-service.
    let poll_status = move || {
        leptos::task::spawn_local(async move {
            match ndi::get_video_source_status().await {
                Ok(status) => {
                    ndi_available.set(status.ndi_available);
                    discovered.set(status.discovered);
                    statuses.set(status.sources);
                }
                // Never swallow this: a failing poll means the badges are stale, and the
                // whole point of the card is that the operator can trust what it says.
                Err(err) => leptos::logging::warn!("video-source status poll failed: {err}"),
            }
        });
    };

    leptos::task::spawn_local(async move {
        if let Ok(list) = ndi::list_video_sources().await {
            sources.set(list);
        }
    });
    poll_status();

    // `forget()`, like every other poller on this page (see `settings/mod.rs`): the card
    // mounts once per page load and there is no client-side router, so the timer dies with
    // the navigation. It is NOT `on_cleanup` because gloo's `Interval` is not `Send`, and
    // leptos' `on_cleanup` requires it for the non-wasm (host) test build of this crate.
    let interval = gloo_timers::callback::Interval::new(super::STATUS_REFRESH_MS, move || {
        poll_status();
    });
    interval.forget();

    let refresh = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = ndi::list_video_sources().await {
                sources.set(list);
            }
        });
        // Don't make the operator wait up to 5s to see what their click did.
        poll_status();
    };

    // The status poll already carries the discovery list, so "Scan" is just "don't make
    // me wait for the next tick" — it must not write `discovered` from a second, separate
    // endpoint that the poll would overwrite 5s later anyway.
    let scan = move |_| {
        leptos::task::spawn_local(async move {
            match ndi::get_video_source_status().await {
                Ok(status) => {
                    let count = status.discovered.len();
                    ndi_available.set(status.ndi_available);
                    discovered.set(status.discovered);
                    statuses.set(status.sources);
                    toast.show(&format!("Found {count} NDI source(s)"), "info");
                }
                Err(err) => toast.show(&format!("Scan failed: {err}"), "error"),
            }
        });
    };

    let add_source = move |_| {
        let label = new_label.get_untracked().trim().to_string();
        let ndi_name = new_ndi_name.get_untracked().trim().to_string();
        if label.is_empty() || ndi_name.is_empty() {
            toast.show("Label and NDI name required", "error");
            return;
        }
        leptos::task::spawn_local(async move {
            match ndi::create_video_source(&label, &ndi_name).await {
                Ok(_) => {
                    new_label.set(String::new());
                    new_ndi_name.set(String::new());
                    refresh();
                    toast.show("Source added", "success");
                }
                Err(err) => toast.show(&format!("Failed to add source. {err}"), "error"),
            }
        });
    };

    let activate = move |id: String| {
        leptos::task::spawn_local(async move {
            match ndi::activate_video_source(&id).await {
                Ok(_) => {
                    refresh();
                    toast.show("Source activated", "success");
                }
                Err(err) => toast.show(&format!("Error: {err}"), "error"),
            }
        });
    };

    let deactivate = move |_| {
        leptos::task::spawn_local(async move {
            match ndi::deactivate_video_sources().await {
                Ok(()) => {
                    refresh();
                    toast.show("Sources deactivated", "success");
                }
                Err(err) => toast.show(&format!("Error: {err}"), "error"),
            }
        });
    };

    let delete_source = move |id: String| {
        if !confirm("Delete this video source?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match ndi::delete_video_source(&id).await {
                Ok(()) => {
                    refresh();
                    toast.show("Source deleted", "success");
                }
                Err(err) => toast.show(&format!("Error: {err}"), "error"),
            }
        });
    };

    view! {
        <section class="settings__card" data-role="video-sources-card">
            <header class="settings__card-header">
                <div>
                    <h2>"Video Sources"</h2>
                    <p>"Configure NDI sources for stage display"</p>
                </div>
                <div class="settings__badge-group">
                    <span class=move || if ndi_available.get() {
                        "settings__badge settings__badge--ok"
                    } else {
                        "settings__badge settings__badge--off"
                    }>
                        {move || if ndi_available.get() { "NDI Available" } else { "NDI Unavailable" }}
                    </span>
                </div>
            </header>
            <div class="settings__source-list" data-role="video-source-list">
                <For
                    each=move || sources.get()
                    // Keyed on identity, NOT on the live state: the badge, hint and detail
                    // below read `statuses` through reactive closures, so they update on
                    // every poll without destroying and rebuilding the row (and its buttons).
                    key=|s: &VideoSourceDto| format!("{}-{}", s.id, s.is_active)
                    children=move |source: VideoSourceDto| {
                        let dot_class = if source.is_active {
                            "settings__source-dot settings__source-dot--active"
                        } else {
                            "settings__source-dot"
                        };
                        let id_activate = source.id.clone();
                        let id_delete = source.id.clone();
                        let id_status = source.id.clone();
                        let id_hint = source.id.clone();
                        let is_active = source.is_active;

                        // This row's live status, re-read on every poll. A `Memo` (not a plain
                        // closure) because it is `Copy`, so the badge, the class, the hint and
                        // the detail can each hold it. An id with no entry yet yields "" —
                        // rendered as "Checking…", never as "NDI unavailable".
                        let row_id = source.id.clone();
                        let status = Memo::new(move |_| {
                            statuses.with(|list| list.iter().find(|st| st.id == row_id).cloned())
                        });
                        let state_str = move || status.get().map(|st| st.state).unwrap_or_default();
                        let detail = move || status.get().and_then(|st| st.detail);
                        view! {
                            <div class="settings__source-item" data-role="video-source-row"
                                data-source-id=source.id.clone()>
                                <div class=dot_class></div>
                                <div class="settings__source-info">
                                    <div class="settings__source-label">{source.label.clone()}</div>
                                    <div class="settings__source-ndi">"NDI: " {source.ndi_name.clone()}</div>
                                    {move || status_hint(&state_str()).map(|text| view! {
                                        <div class="settings__source-hint"
                                            data-role="video-source-hint"
                                            data-source-id=id_hint.clone()>{text}</div>
                                    })}
                                    // The pipeline's own error text, when it has one — the most
                                    // specific "why" the server can give.
                                    {move || detail().map(|text| view! {
                                        <div class="settings__source-detail"
                                            data-role="video-source-detail">{text}</div>
                                    })}
                                </div>
                                <span class=move || status_class(&state_str())
                                    data-role="video-source-status"
                                    data-source-id=id_status.clone()
                                    data-state=state_str>
                                    {move || status_label(&state_str())}
                                </span>
                                {if is_active {
                                    view! {
                                        <button class="settings__btn settings__btn--active"
                                            on:click=move |_| deactivate(())>"ACTIVE"</button>
                                    }.into_any()
                                } else {
                                    view! {
                                        <button class="settings__btn settings__btn--activate"
                                            on:click=move |_| activate(id_activate.clone())>"Activate"</button>
                                    }.into_any()
                                }}
                                <button class="settings__btn settings__btn--delete"
                                    on:click=move |_| delete_source(id_delete.clone())>"Delete"</button>
                            </div>
                        }
                    }
                />
            </div>
            // What is ACTUALLY on the network, right next to what is mapped. This one line
            // is what turns "RESOLUME-PP (cg-obs) vs STREAM-PP (stream)" from a two-hour
            // investigation into something the operator spots at a glance (#546).
            <Show when=move || ndi_available.get()>
                <div class="settings__discovered" data-role="ndi-discovered">
                    <span class="settings__discovered-title">"On the network now: "</span>
                    <Show
                        when=move || !discovered.get().is_empty()
                        fallback=|| view! {
                            <span class="settings__discovered-empty">
                                "nothing — no NDI source is broadcasting."
                            </span>
                        }
                    >
                        <For
                            each=move || discovered.get()
                            key=|name: &String| name.clone()
                            children=|name: String| view! {
                                <span class="settings__discovered-name"
                                    data-role="ndi-discovered-name">{name}</span>
                            }
                        />
                    </Show>
                </div>
            </Show>
            <div class="settings__form" data-role="add-video-source-form">
                <div class="settings__form-header">
                    <h3>"Add Video Source"</h3>
                </div>
                <div class="settings__form-row">
                    <label>"Label"</label>
                    <input type="text" placeholder="Main Camera" class="settings__input"
                        data-role="video-source-label"
                        aria-required="true"
                        prop:value=move || new_label.get()
                        on:input=move |ev| new_label.set(event_target_value(&ev)) />
                </div>
                <div class="settings__form-row">
                    <label>"NDI Source"</label>
                    <div class="settings__ndi-select">
                        <input type="text" placeholder="CAM1 (usb)" class="settings__input"
                            data-role="video-source-ndi-name" list="ndi-sources"
                            aria-required="true"
                            prop:value=move || new_ndi_name.get()
                            on:input=move |ev| new_ndi_name.set(event_target_value(&ev)) />
                        <datalist id="ndi-sources">
                            <For
                                each=move || discovered.get()
                                key=|s: &String| s.clone()
                                children=|name: String| view! { <option value=name.clone()></option> }
                            />
                        </datalist>
                        <button class="settings__btn settings__btn--scan" on:click=scan>"Scan"</button>
                    </div>
                </div>
                <button class="settings__btn settings__btn--add" on:click=add_source>"+ Add Source"</button>
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words the operator reads. The PP outage was not caused by a missing feature —
    /// it was caused by the UI saying NOTHING while the server knew the answer. So the
    /// copy is the deliverable, and it is pinned.
    #[test]
    fn every_state_has_plain_operator_copy() {
        assert_eq!(status_label("live"), "Live");
        assert_eq!(status_label("not-broadcasting"), "Not broadcasting");
        assert_eq!(status_label("not-found"), "Not found on the network");
        assert_eq!(status_label("ready"), "Ready");
        assert_eq!(status_label("connecting"), "Connecting…");
        assert_eq!(status_label("unknown"), "NDI unavailable");
    }

    /// A state we do not know must not render a raw wire token at the operator, and must
    /// not panic the WASM app.
    #[test]
    fn an_unknown_state_degrades_instead_of_leaking_or_panicking() {
        assert_eq!(status_label("wat"), "Checking…");
    }

    /// A row whose status has not arrived yet (first paint, or a source added since the
    /// last poll) is NOT a server without NDI. Printing "NDI unavailable" there tells an
    /// operator with a perfectly healthy server to go hunt a phantom fault.
    #[test]
    fn a_not_yet_loaded_status_reads_checking_not_ndi_unavailable() {
        assert_eq!(status_label(""), "Checking…");
        assert_eq!(
            status_class(""),
            "settings__status settings__status--checking"
        );
    }

    /// The card HEADER badge told the same lie the row badges no longer tell: it is painted
    /// from a plain `bool` initialised to `false`, so every load of a perfectly healthy
    /// server flashed "NDI Unavailable" (and hid the "on the network now" list) until the
    /// first poll landed — and stayed there forever if that poll errored.
    #[test]
    fn the_header_badge_says_checking_until_the_first_poll_answers() {
        assert_eq!(header_badge_label(None), "Checking…");
        assert_eq!(header_badge_label(Some(true)), "NDI Available");
        assert_eq!(header_badge_label(Some(false)), "NDI Unavailable");
    }

    /// …and it must not wear the RED "off" class while it is merely waiting.
    #[test]
    fn the_header_badge_is_not_red_while_it_is_still_checking() {
        assert_eq!(
            header_badge_class(None),
            "settings__badge settings__badge--checking"
        );
        assert_eq!(
            header_badge_class(Some(true)),
            "settings__badge settings__badge--ok"
        );
        assert_eq!(
            header_badge_class(Some(false)),
            "settings__badge settings__badge--off"
        );
    }

    #[test]
    fn class_is_built_from_the_state() {
        assert_eq!(
            status_class("not-found"),
            "settings__status settings__status--not-found"
        );
        assert_eq!(
            status_class("live"),
            "settings__status settings__status--live"
        );
        assert_eq!(
            status_class("nonsense"),
            "settings__status settings__status--checking",
            "an unrecognised state must not inject an arbitrary class name"
        );
    }

    /// Only the two BROKEN states get a hint — a hint under a working source is noise.
    #[test]
    fn only_the_broken_states_tell_the_operator_what_to_do() {
        let not_found = status_hint("not-found").expect("not-found must explain itself");
        assert!(
            not_found.contains("not on the network"),
            "the hint must name the actual problem: {not_found}"
        );

        let silent = status_hint("not-broadcasting").expect("not-broadcasting must explain itself");
        assert!(
            silent.contains("sending machine"),
            "the hint must point at the sending machine: {silent}"
        );

        assert_eq!(status_hint("live"), None);
        assert_eq!(status_hint("ready"), None);
        assert_eq!(status_hint("connecting"), None);
        assert_eq!(status_hint("unknown"), None);
    }
}
