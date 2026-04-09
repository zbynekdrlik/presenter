use crate::state::{save_baseline_to_storage, AppContext};
use crate::utils::autofit::autofit_effect;
use leptos::prelude::*;

const PREVIEW_CURRENT_MAX_FONT: f64 = 40.0;
const PREVIEW_NEXT_MAX_FONT: f64 = 18.0;

/// Stage preview panel rendered inside the header.
#[component]
pub fn StagePreview() -> impl IntoView {
    let ctx = use_ctx!(AppContext);

    let current_ref = NodeRef::<leptos::html::Div>::new();
    let next_ref = NodeRef::<leptos::html::Div>::new();

    // Compute alert state: connected < baseline connected
    let has_alert = move || {
        let baseline = ctx.stage_monitor_baseline.get();
        let conns = ctx.stage_connections.get();
        let connected = conns
            .iter()
            .filter(|c| c.status == presenter_core::StageClientStatus::Connected)
            .count();
        match baseline {
            Some((baseline_connected, _)) => connected < baseline_connected,
            None => false,
        }
    };

    // Handler to reset baseline to current counts
    let on_reset_baseline = {
        let stage_connections = ctx.stage_connections;
        let stage_monitor_baseline = ctx.stage_monitor_baseline;
        move |_| {
            let conns = stage_connections.get_untracked();
            let connected = conns
                .iter()
                .filter(|c| c.status == presenter_core::StageClientStatus::Connected)
                .count();
            let issues = conns
                .iter()
                .filter(|c| c.status != presenter_core::StageClientStatus::Connected)
                .count();
            stage_monitor_baseline.set(Some((connected, issues)));
            save_baseline_to_storage(connected, issues);
        }
    };

    let on_clear = {
        let stage_snapshot = ctx.stage_snapshot;
        let active_bible_broadcast = ctx.active_bible_broadcast;
        let toast_message = ctx.toast_message;
        let toast_variant = ctx.toast_variant;
        move |_| {
            leptos::task::spawn_local(async move {
                let _ = crate::api::stage::clear().await;
                let _ = crate::api::bible::clear_broadcast().await;
                stage_snapshot.set(None);
                active_bible_broadcast.set(None);
                toast_variant.set("info".to_string());
                toast_message.set(Some("Slide outputs cleared".to_string()));
            });
        }
    };

    let on_ableset_enable = move |_| {
        let status = ctx.ableset_status.get_untracked();
        let currently_enabled = status.map(|s| s.enabled).unwrap_or(false);
        let ableset_status = ctx.ableset_status;
        leptos::task::spawn_local(async move {
            if let Ok(settings) = crate::api::settings::get_ableset_settings().await {
                let draft = presenter_core::AbleSetSettingsDraft {
                    enabled: !currently_enabled,
                    host: settings.host,
                    osc_port: settings.osc_port,
                    http_port: settings.http_port,
                    library_name: settings.library_name,
                    song_prefix_length: settings.song_prefix_length,
                };
                let _ = crate::api::settings::update_ableset_settings(&draft).await;
                if let Ok(s) = crate::api::settings::get_ableset_status().await {
                    ableset_status.set(Some(s));
                }
            }
        });
    };

    let on_ableset_follow = move |_| {
        let status = ctx.ableset_status.get_untracked();
        let currently_following = status.map(|s| s.follow_enabled).unwrap_or(false);
        let ableset_status = ctx.ableset_status;
        let stage_snapshot = ctx.stage_snapshot;
        let selected_presentation_id = ctx.selected_presentation_id;
        let selected_presentation = ctx.selected_presentation;
        let selected_library_id = ctx.selected_library_id;
        let selected_playlist_id = ctx.selected_playlist_id;
        let selected_playlist = ctx.selected_playlist;
        let presentations = ctx.presentations;
        let slides_cache = ctx.slides_cache;
        leptos::task::spawn_local(async move {
            if let Ok(s) = crate::api::settings::set_ableset_follow(!currently_following).await {
                let now_following = s.follow_enabled;
                ableset_status.set(Some(s));

                // When follow is toggled ON, immediately sync to current stage state
                if now_following {
                    if let Some(snap) = stage_snapshot.get_untracked() {
                        if let Some(pres_id) = snap.presentation_id.map(|id| id.to_string()) {
                            let current = selected_presentation_id.get_untracked();
                            if Some(&pres_id) != current.as_ref() {
                                selected_presentation_id.set(Some(pres_id.clone()));
                                crate::state::session::set("currentPresentationId", &pres_id);
                                if let Ok(detail) =
                                    crate::api::presentations::get_presentation(&pres_id).await
                                {
                                    // Switch to the library containing this presentation
                                    let lib_id = detail.library_id.to_string();
                                    selected_library_id.set(Some(lib_id.clone()));
                                    selected_playlist_id.set(None);
                                    selected_playlist.set(None);
                                    crate::state::session::set("activeLibraryId", &lib_id);
                                    // Load the library's presentation list
                                    if let Ok(pres_list) =
                                        crate::api::libraries::list_presentations(&lib_id).await
                                    {
                                        presentations.set(pres_list);
                                    }
                                    slides_cache.update(|cache| {
                                        cache.insert(pres_id, detail.presentation.slides.clone());
                                    });
                                    selected_presentation.set(Some(detail.presentation));
                                }
                            }
                        }
                    }
                }
            }
        });
    };

    let current_text = move || {
        ctx.stage_snapshot
            .get()
            .and_then(|s| s.current.as_ref().map(|slide| slide.main.clone()))
            .unwrap_or_else(|| "\u{2014}".to_string())
    };
    let next_text = move || {
        ctx.stage_snapshot
            .get()
            .and_then(|s| s.next.as_ref().map(|slide| slide.main.clone()))
            .unwrap_or_else(|| "\u{2014}".to_string())
    };

    autofit_effect(current_ref, PREVIEW_CURRENT_MAX_FONT, current_text.clone());
    autofit_effect(next_ref, PREVIEW_NEXT_MAX_FONT, next_text.clone());

    view! {
        <div
            class="operator__stage-preview"
            data-role="stage-status"
            data-active=move || {
                let snap = ctx.stage_snapshot.get();
                if snap.as_ref().and_then(|s| s.current_slide_id.as_ref()).is_some() { "true" } else { "false" }
            }
        >
            <div
                data-role="worship-preview"
                class="operator__worship-preview-wrap"
                style=move || {
                    if ctx.view.get() == "bible" { "display:none" } else { "" }
                }
            >
                <div class="operator__stage-preview-stack">
                    <div class="operator__stage-preview-panel operator__stage-preview-panel--next" data-role="stage-next">
                        <div node_ref=next_ref class="operator__stage-preview-text">
                            {next_text}
                        </div>
                    </div>
                    <div class="operator__stage-preview-song" data-role="stage-song-line">
                        {move || {
                            ctx.stage_snapshot.get()
                                .and_then(|s| s.presentation_name.clone())
                                .unwrap_or_else(|| "\u{2014}".to_string())
                        }}
                    </div>
                    <div class="operator__stage-preview-actions">
                        <button
                            type="button"
                            class="operator__stage-toggle"
                            data-role="ableset-enable"
                            data-state=move || {
                                if ctx.ableset_status.get().map(|s| s.enabled).unwrap_or(false) { "on" } else { "off" }
                            }
                            on:click=on_ableset_enable
                        >
                            {move || {
                                if ctx.ableset_status.get().map(|s| s.enabled).unwrap_or(false) {
                                    "Ableton ON"
                                } else {
                                    "Ableton OFF"
                                }
                            }}
                        </button>
                        <button
                            type="button"
                            class="operator__stage-toggle"
                            data-role="ableset-follow"
                            data-state=move || {
                                if ctx.ableset_status.get().map(|s| s.follow_enabled).unwrap_or(false) { "on" } else { "off" }
                            }
                            on:click=on_ableset_follow
                        >
                            {move || {
                                if ctx.ableset_status.get().map(|s| s.follow_enabled).unwrap_or(false) {
                                    "Follow ON"
                                } else {
                                    "Follow OFF"
                                }
                            }}
                        </button>
                    </div>
                </div>
                <div class="operator__stage-preview-panel operator__stage-preview-panel--current" data-role="stage-current">
                    <div node_ref=current_ref class="operator__stage-preview-text">
                        {current_text}
                    </div>
                </div>
            </div>
            <div
                class="operator__bible-preview"
                data-role="bible-preview"
                style=move || {
                    if ctx.view.get() == "bible" { "" } else { "display:none" }
                }
                data-active=move || {
                    if ctx.active_bible_broadcast.get().is_some() { "true" } else { "false" }
                }
            >
                {move || {
                    match ctx.active_bible_broadcast.get() {
                        Some(broadcast) => {
                            let text = &broadcast.passage.text;
                            let truncated = if text.chars().count() > 100 {
                                let end: String = text.chars().take(100).collect();
                                format!("{end}…")
                            } else {
                                text.clone()
                            };
                            let reference = broadcast
                                .reference_label
                                .clone()
                                .unwrap_or_else(|| {
                                    format!(
                                        "{} ({})",
                                        broadcast.passage.reference.to_human_readable(),
                                        broadcast.passage.translation.code
                                    )
                                });
                            leptos::prelude::view! {
                                <span class="operator__bible-preview-text" data-role="bible-preview-text">{truncated}</span>
                                <span class="operator__bible-preview-ref" data-role="bible-preview-ref">{reference}</span>
                            }.into_any()
                        }
                        None => {
                            leptos::prelude::view! {
                                <span class="operator__bible-preview-empty">"No active passage"</span>
                            }.into_any()
                        }
                    }
                }}
            </div>
            <button
                type="button"
                class=move || {
                    let base = "operator__stage-monitor";
                    if has_alert() {
                        format!("{} operator__stage-monitor--alert", base)
                    } else {
                        base.to_string()
                    }
                }
                data-role="stage-monitor"
                data-alert=move || if has_alert() { "true" } else { "false" }
                data-connected=move || {
                    let conns = ctx.stage_connections.get();
                    let connected = conns.iter().filter(|c| c.status == presenter_core::StageClientStatus::Connected).count();
                    connected.to_string()
                }
                data-issues=move || {
                    let conns = ctx.stage_connections.get();
                    let issues = conns.iter().filter(|c| c.status != presenter_core::StageClientStatus::Connected).count();
                    issues.to_string()
                }
                aria-label="Stage display health"
                title="Stage displays"
                on:click=on_reset_baseline
            >
                <span data-role="stage-monitor-connected" class="operator__stage-monitor-count operator__stage-monitor-count--connected">
                    {move || {
                        let conns = ctx.stage_connections.get();
                        conns.iter().filter(|c| c.status == presenter_core::StageClientStatus::Connected).count().to_string()
                    }}
                </span>
                <span class="operator__stage-monitor-separator">"/"</span>
                <span data-role="stage-monitor-issues" class="operator__stage-monitor-count operator__stage-monitor-count--issues">
                    {move || {
                        let conns = ctx.stage_connections.get();
                        conns.iter().filter(|c| c.status != presenter_core::StageClientStatus::Connected).count().to_string()
                    }}
                </span>
            </button>
            <button
                type="button"
                class="operator__clear-button"
                data-role="clear-slide"
                aria-label="Clear live outputs"
                on:click=on_clear
            >
                "\u{1f9f9}"
            </button>
        </div>
    }
}
