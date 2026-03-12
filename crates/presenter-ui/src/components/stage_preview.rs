use crate::state::AppContext;
use leptos::prelude::*;

/// Stage preview panel rendered inside the header.
#[component]
pub fn StagePreview() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");

    let on_clear = move |_| {
        leptos::task::spawn_local(async move {
            let _ = crate::api::stage::clear().await;
        });
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
        leptos::task::spawn_local(async move {
            if let Ok(s) = crate::api::settings::set_ableset_follow(!currently_following).await {
                ableset_status.set(Some(s));
            }
        });
    };

    view! {
        <div
            class="operator__stage-preview"
            data-role="stage-status"
            attr:data-active=move || {
                let snap = ctx.stage_snapshot.get();
                if snap.as_ref().and_then(|s| s.current_slide_id.as_ref()).is_some() { "true" } else { "false" }
            }
        >
            <div data-role="worship-preview" class="operator__worship-preview-wrap">
                <div class="operator__stage-preview-stack">
                    <div class="operator__stage-preview-panel operator__stage-preview-panel--next" data-role="stage-next">
                        {move || {
                            ctx.stage_snapshot.get()
                                .and_then(|s| s.next.as_ref().map(|slide| slide.main.clone()))
                                .unwrap_or_else(|| "\u{2014}".to_string())
                        }}
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
                            attr:data-state=move || {
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
                            attr:data-state=move || {
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
                    {move || {
                        ctx.stage_snapshot.get()
                            .and_then(|s| s.current.as_ref().map(|slide| slide.main.clone()))
                            .unwrap_or_else(|| "\u{2014}".to_string())
                    }}
                </div>
            </div>
            <div class="operator__bible-preview" data-role="bible-preview" style="display:none">
                <span class="operator__bible-preview-empty">"No active passage"</span>
            </div>
            <button
                type="button"
                class="operator__stage-monitor"
                data-role="stage-monitor"
                attr:data-connected=move || {
                    let conns = ctx.stage_connections.get();
                    let connected = conns.iter().filter(|c| c.status == presenter_core::StageClientStatus::Connected).count();
                    connected.to_string()
                }
                attr:data-issues=move || {
                    let conns = ctx.stage_connections.get();
                    let issues = conns.iter().filter(|c| c.status != presenter_core::StageClientStatus::Connected).count();
                    issues.to_string()
                }
                aria-label="Stage display health"
                title="Stage displays"
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
