//! NDI video-sources card for the settings page (#347).

use leptos::prelude::*;

use super::ToastHandle;
use crate::api::ndi::{self, VideoSourceDto};
use crate::components::modal::confirm;

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
