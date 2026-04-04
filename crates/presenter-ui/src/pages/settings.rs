use leptos::prelude::*;

use crate::api::ndi::{self, VideoSourceDto};

/// Settings page — configuration for integrations.
#[component]
pub fn SettingsPage() -> impl IntoView {
    let sources = RwSignal::new(Vec::<VideoSourceDto>::new());
    let ndi_available = RwSignal::new(false);
    let discovered = RwSignal::new(Vec::<String>::new());
    let new_label = RwSignal::new(String::new());
    let new_ndi_name = RwSignal::new(String::new());

    // Load initial data
    {
        let sources = sources;
        let ndi_available = ndi_available;
        leptos::task::spawn_local(async move {
            if let Ok(status) = ndi::get_ndi_status().await {
                ndi_available.set(status.available);
            }
            if let Ok(list) = ndi::list_video_sources().await {
                sources.set(list);
            }
        });
    }

    let refresh = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = ndi::list_video_sources().await {
                sources.set(list);
            }
        });
    };

    let scan_network = move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(found) = ndi::discover_ndi_sources().await {
                discovered.set(found.into_iter().map(|s| s.name).collect());
            }
        });
    };

    let add_source = move |_| {
        let label = new_label.get_untracked();
        let ndi_name = new_ndi_name.get_untracked();
        if label.trim().is_empty() || ndi_name.trim().is_empty() {
            return;
        }
        let refresh = refresh;
        leptos::task::spawn_local(async move {
            if let Ok(_) = ndi::create_video_source(&label, &ndi_name).await {
                new_label.set(String::new());
                new_ndi_name.set(String::new());
                refresh();
            }
        });
    };

    let activate = move |id: String| {
        let refresh = refresh;
        leptos::task::spawn_local(async move {
            let _ = ndi::activate_video_source(&id).await;
            refresh();
        });
    };

    let deactivate = move |_| {
        let refresh = refresh;
        leptos::task::spawn_local(async move {
            let _ = ndi::deactivate_video_sources().await;
            refresh();
        });
    };

    let delete_source = move |id: String| {
        let refresh = refresh;
        leptos::task::spawn_local(async move {
            let _ = ndi::delete_video_source(&id).await;
            refresh();
        });
    };

    view! {
        <div class="settings-layout">
            <header class="settings__header">
                <div class="settings__header-title">
                    <h1>"Settings"</h1>
                </div>
            </header>
            <main class="settings__main">
                // Video Sources card
                <div class="settings__card" data-role="video-sources-card">
                    <div class="settings__card-header">
                        <div>
                            <h2>"Video Sources"</h2>
                            <p>"Configure NDI sources for stage display"</p>
                        </div>
                        <div class="settings__badge-group">
                            <div class={move || {
                                if ndi_available.get() {
                                    "settings__badge settings__badge--ok"
                                } else {
                                    "settings__badge settings__badge--off"
                                }
                            }}>
                                {move || if ndi_available.get() { "NDI Available" } else { "NDI Unavailable" }}
                            </div>
                        </div>
                    </div>

                    // Source list
                    <div class="settings__source-list">
                        <For
                            each=move || sources.get()
                            key=|s| s.id.clone()
                            children=move |source| {
                                let id = source.id.clone();
                                let id_activate = id.clone();
                                let id_delete = id.clone();
                                view! {
                                    <div class="settings__source-item">
                                        <div class={if source.is_active { "settings__source-dot settings__source-dot--active" } else { "settings__source-dot" }}></div>
                                        <div class="settings__source-info">
                                            <div class="settings__source-label">{source.label.clone()}</div>
                                            <div class="settings__source-ndi">"NDI: " {source.ndi_name.clone()}</div>
                                        </div>
                                        {if source.is_active {
                                            view! {
                                                <button class="settings__btn settings__btn--active" on:click=move |_| (deactivate)(web_sys::MouseEvent::new("click").unwrap())>"ACTIVE"</button>
                                            }.into_any()
                                        } else {
                                            let activate = activate.clone();
                                            view! {
                                                <button class="settings__btn settings__btn--activate" on:click=move |_| (activate)(id_activate.clone())>"Activate"</button>
                                            }.into_any()
                                        }}
                                        <button class="settings__btn settings__btn--delete" on:click=move |_| (delete_source)(id_delete.clone())>"Delete"</button>
                                    </div>
                                }
                            }
                        />
                    </div>

                    // Add source form
                    <div class="settings__form">
                        <div class="settings__form-header">
                            <h3>"Add Video Source"</h3>
                        </div>
                        <div class="settings__form-row">
                            <label>"Label"</label>
                            <input
                                type="text"
                                placeholder="Main Camera"
                                class="settings__input"
                                prop:value=move || new_label.get()
                                on:input=move |ev| new_label.set(event_target_value(&ev))
                            />
                        </div>
                        <div class="settings__form-row">
                            <label>"NDI Source"</label>
                            <div class="settings__ndi-select">
                                <input
                                    type="text"
                                    placeholder="CAM1 (usb)"
                                    class="settings__input"
                                    prop:value=move || new_ndi_name.get()
                                    on:input=move |ev| new_ndi_name.set(event_target_value(&ev))
                                    list="ndi-sources"
                                />
                                <datalist id="ndi-sources">
                                    <For
                                        each=move || discovered.get()
                                        key=|s| s.clone()
                                        children=|name| view! { <option value={name.clone()} /> }
                                    />
                                </datalist>
                                <button class="settings__btn settings__btn--scan" on:click=scan_network>"Scan"</button>
                            </div>
                        </div>
                        <button class="settings__btn settings__btn--add" on:click=add_source>"+ Add Source"</button>
                    </div>
                </div>
            </main>
        </div>
    }
}
