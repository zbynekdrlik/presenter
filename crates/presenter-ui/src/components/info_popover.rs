use leptos::prelude::*;

#[component]
pub fn InfoPopover(
    /// "local" / "remote" / empty if not yet fetched.
    network_mode: ReadSignal<String>,
) -> impl IntoView {
    let (open, set_open) = signal(false);

    // Captured once at mount; these don't change during the session.
    let version = env!("CARGO_PKG_VERSION");
    let channel = option_env!("PRESENTER_BUILD_CHANNEL").unwrap_or("dev");
    let hostname = web_sys::window()
        .and_then(|w| w.location().host().ok())
        .unwrap_or_default();

    let reload = move |_| {
        if let Some(w) = web_sys::window() {
            let _ = w.location().reload();
        }
    };

    view! {
        <div class="info-popover-wrap">
            <button
                type="button"
                class="info-button"
                aria-label="Info"
                data-role="info-button"
                on:click=move |_| { let _ = set_open.try_update(|v| *v = !*v); }
            >"\u{24D8}"</button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="info-popover" data-role="info-popover">
                    <dl>
                        <dt>"Version"</dt>
                        <dd>{format!("{version} ({channel})")}</dd>
                        <dt>"Host"</dt>
                        <dd>{hostname.clone()}</dd>
                        <dt>"Network"</dt>
                        <dd>{move || {
                            let m = network_mode.get();
                            if m == "local" { "LAN".to_string() }
                            else if m == "remote" { "WAN".to_string() }
                            else { "unknown".to_string() }
                        }}</dd>
                    </dl>
                    <button type="button" class="info-popover__reload" on:click=reload>"Reload"</button>
                </div>
            </Show>
        </div>
    }
}
