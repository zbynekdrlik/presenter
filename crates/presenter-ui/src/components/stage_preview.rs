use leptos::prelude::*;

use crate::state::AppContext;

/// Stage preview panel in the header.
#[component]
pub fn StagePreview(ctx: AppContext) -> impl IntoView {
    let snapshot = ctx.stage_snapshot;
    let connections = ctx.stage_connections;

    let is_active = move || snapshot.get().and_then(|s| s.current_slide_id).is_some();

    let current_text = move || {
        snapshot
            .get()
            .and_then(|s| s.current.map(|c| c.main))
            .unwrap_or_else(|| "\u{2014}".to_string())
    };

    let next_text = move || {
        snapshot
            .get()
            .and_then(|s| s.next.map(|n| n.main))
            .unwrap_or_else(|| "\u{2014}".to_string())
    };

    let song_line = move || {
        snapshot
            .get()
            .and_then(|s| s.song_name)
            .unwrap_or_else(|| "\u{2014}".to_string())
    };

    let connected_count = move || {
        connections
            .get()
            .iter()
            .filter(|c| c.status == presenter_core::StageClientStatus::Connected)
            .count()
    };

    let issues_count = move || {
        connections
            .get()
            .iter()
            .filter(|c| c.status != presenter_core::StageClientStatus::Connected)
            .count()
    };

    let on_clear = move |_| {
        leptos::task::spawn_local(async move {
            let _ = crate::api::stage::clear().await;
        });
    };

    view! {
        <div class="operator__stage-preview" data-role="stage-status" data-active=move || if is_active() { "true" } else { "false" }>
            <div data-role="worship-preview" class="operator__worship-preview-wrap">
                <div class="operator__stage-preview-stack">
                    <div class="operator__stage-preview-panel operator__stage-preview-panel--next" data-role="stage-next">
                        {next_text}
                    </div>
                    <div class="operator__stage-preview-song" data-role="stage-song-line">
                        {song_line}
                    </div>
                    <div class="operator__stage-preview-actions">
                        <button
                            type="button"
                            class="operator__stage-toggle"
                            data-role="ableset-enable"
                            data-state="off"
                        >"Ableton OFF"</button>
                        <button
                            type="button"
                            class="operator__stage-toggle"
                            data-role="ableset-follow"
                            data-state="off"
                        >"Follow OFF"</button>
                    </div>
                </div>
                <div class="operator__stage-preview-panel operator__stage-preview-panel--current" data-role="stage-current">
                    {current_text}
                </div>
            </div>
            <div class="operator__bible-preview" data-role="bible-preview" style="display:none">
                <span class="operator__bible-preview-empty">"No active passage"</span>
            </div>
            <button
                type="button"
                class="operator__stage-monitor"
                data-role="stage-monitor"
                data-connected=move || connected_count().to_string()
                data-issues=move || issues_count().to_string()
                aria-label="Stage display health"
                title="Stage displays"
            >
                <span data-role="stage-monitor-connected" class="operator__stage-monitor-count operator__stage-monitor-count--connected">
                    {move || connected_count().to_string()}
                </span>
                <span class="operator__stage-monitor-separator">"/"</span>
                <span data-role="stage-monitor-issues" class="operator__stage-monitor-count operator__stage-monitor-count--issues">
                    {move || issues_count().to_string()}
                </span>
            </button>
            <button
                type="button"
                class="operator__clear-button"
                data-role="clear-slide"
                aria-label="Clear live outputs"
                on:click=on_clear
            >{"\u{1F9F9}"}</button>
        </div>
    }
}
