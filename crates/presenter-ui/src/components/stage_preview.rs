use crate::state::AppContext;
use leptos::prelude::*;

/// Stage status and preview panel.
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
            data-role="stage-status"
            attr:data-active=move || {
                let snap = ctx.stage_snapshot.get();
                if snap.as_ref().and_then(|s| s.current_slide_id.as_ref()).is_some() { "true" } else { "false" }
            }
            class="operator__stage-status"
        >
            <div class="operator__stage-preview">
                <div data-role="stage-current" class="operator__stage-current">
                    {move || {
                        ctx.stage_snapshot.get()
                            .and_then(|s| s.current.as_ref().map(|slide| slide.main.clone()))
                            .unwrap_or_default()
                    }}
                </div>
                <div data-role="stage-next" class="operator__stage-next">
                    {move || {
                        ctx.stage_snapshot.get()
                            .and_then(|s| s.next.as_ref().map(|slide| slide.main.clone()))
                            .unwrap_or_default()
                    }}
                </div>
                <div data-role="stage-song-line" class="operator__stage-song">
                    {move || {
                        ctx.stage_snapshot.get()
                            .and_then(|s| s.presentation_name.clone())
                            .unwrap_or_default()
                    }}
                </div>
            </div>

            <div class="operator__stage-controls">
                <button data-role="clear-slide" class="operator__clear-btn" on:click=on_clear>
                    "Clear"
                </button>
            </div>

            <div class="operator__ableset-controls">
                <button
                    data-role="ableset-enable"
                    attr:data-state=move || {
                        if ctx.ableset_status.get().map(|s| s.enabled).unwrap_or(false) { "on" } else { "off" }
                    }
                    class="operator__ableset-btn"
                    on:click=on_ableset_enable
                >
                    {move || {
                        if ctx.ableset_status.get().map(|s| s.enabled).unwrap_or(false) {
                            "AbleSet ON"
                        } else {
                            "AbleSet OFF"
                        }
                    }}
                </button>
                <button
                    data-role="ableset-follow"
                    attr:data-state=move || {
                        if ctx.ableset_status.get().map(|s| s.follow_enabled).unwrap_or(false) { "on" } else { "off" }
                    }
                    class="operator__ableset-btn"
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

            <div
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
                class="operator__stage-monitor"
            >
                <span data-role="stage-monitor-connected">
                    {move || {
                        let conns = ctx.stage_connections.get();
                        conns.iter().filter(|c| c.status == presenter_core::StageClientStatus::Connected).count().to_string()
                    }}
                </span>
                " / "
                <span data-role="stage-monitor-issues">
                    {move || {
                        let conns = ctx.stage_connections.get();
                        conns.iter().filter(|c| c.status != presenter_core::StageClientStatus::Connected).count().to_string()
                    }}
                </span>
            </div>
        </div>
    }
}
