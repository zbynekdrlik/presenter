use leptos::prelude::*;

/// Shared version label component. Fetches `/healthz` once on mount and
/// displays `v{version} ({channel})` for dev builds, or `v{version}` for
/// release builds.
///
/// Tagged with `data-testid="version"` so Playwright can target it
/// consistently across all UI routes.
#[component]
pub fn VersionLabel() -> impl IntoView {
    let version_text = RwSignal::new(String::new());
    leptos::task::spawn_local(async move {
        if let Ok(health) = crate::api::get_json::<crate::api::HealthzResponse>("/healthz").await {
            let text = if health.channel.is_empty() || health.channel == "release" {
                format!("v{}", health.version)
            } else {
                format!("v{} ({})", health.version, health.channel)
            };
            version_text.set(text);
        }
    });
    view! {
        <span data-testid="version">{move || version_text.get()}</span>
    }
}
