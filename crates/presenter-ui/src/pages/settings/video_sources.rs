//! NDI video-sources card for the settings page (#347).

use leptos::prelude::*;

use super::ToastHandle;
use crate::api::ndi::{self, VideoSourceDto};
use crate::components::modal::confirm;

/// The badge text for a source state (#546).
///
/// Plain operator language, not protocol language: the person reading this is standing
/// in a church about to run a service, and the words have to tell them what to DO.
/// An unrecognised state degrades to the honest "NDI unavailable" rather than panicking
/// or rendering a raw wire token.
pub(crate) fn status_label(state: &str) -> &'static str {
    // [red] stub — the real copy lands in the GREEN commit.
    let _ = state;
    ""
}

/// The CSS class pair for a source state (#546).
pub(crate) fn status_class(state: &str) -> String {
    // [red] stub.
    let _ = state;
    String::new()
}

/// What the operator should DO about this state — shown under the row. `None` when the
/// state needs no action (live / ready / connecting).
pub(crate) fn status_hint(state: &str) -> Option<&'static str> {
    // [red] stub.
    let _ = state;
    None
}

#[component]
pub fn VideoSourcesCard(toast: ToastHandle) -> impl IntoView {
    let sources = RwSignal::new(Vec::<VideoSourceDto>::new());
    let ndi_available = RwSignal::new(false);
    let discovered = RwSignal::new(Vec::<String>::new());
    let new_label = RwSignal::new(String::new());
    let new_ndi_name = RwSignal::new(String::new());

    leptos::task::spawn_local(async move {
        if let Ok(status) = ndi::get_ndi_status().await {
            ndi_available.set(status.available);
        }
        if let Ok(list) = ndi::list_video_sources().await {
            sources.set(list);
        }
    });

    let refresh = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = ndi::list_video_sources().await {
                sources.set(list);
            }
        });
    };

    let scan = move |_| {
        leptos::task::spawn_local(async move {
            match ndi::discover_ndi_sources().await {
                Ok(found) => {
                    let count = found.len();
                    discovered.set(found.into_iter().map(|s| s.name).collect());
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
                    key=|s: &VideoSourceDto| format!("{}-{}", s.id, s.is_active)
                    children=move |source: VideoSourceDto| {
                        let dot_class = if source.is_active {
                            "settings__source-dot settings__source-dot--active"
                        } else {
                            "settings__source-dot"
                        };
                        let id_activate = source.id.clone();
                        let id_delete = source.id.clone();
                        let is_active = source.is_active;
                        view! {
                            <div class="settings__source-item" data-source-id=source.id.clone()>
                                <div class=dot_class></div>
                                <div class="settings__source-info">
                                    <div class="settings__source-label">{source.label.clone()}</div>
                                    <div class="settings__source-ndi">"NDI: " {source.ndi_name.clone()}</div>
                                </div>
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
        assert_eq!(status_label("wat"), "NDI unavailable");
        assert_eq!(status_label(""), "NDI unavailable");
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
            "settings__status settings__status--unknown",
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
