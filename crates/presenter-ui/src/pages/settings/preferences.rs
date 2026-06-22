//! Operator Preferences card — the #272 line-limit pref persisted in
//! localStorage. Pure client-side; no server round-trip.

use leptos::prelude::*;

use crate::state::session;

#[component]
pub fn PreferencesCard() -> impl IntoView {
    let line_limit = RwSignal::new(
        session::get_persistent("lineLimit")
            .filter(|v| v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty())
            .unwrap_or_else(|| "32".to_string()),
    );

    let on_input = move |ev: leptos::ev::Event| {
        let raw = event_target_value(&ev);
        line_limit.set(raw.clone());
        let trimmed = raw.trim();
        if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(n) = trimmed.parse::<u32>() {
                if (10..=120).contains(&n) {
                    session::set_persistent("lineLimit", &n.to_string());
                }
            }
        }
    };

    view! {
        <section class="settings__card" data-role="preferences-card">
            <header class="settings__card-header">
                <div>
                    <h2>"Preferences"</h2>
                    <p class="settings__card-sub">"Operator-side settings stored in your browser."</p>
                </div>
            </header>
            <form class="settings__form settings__form--compact" autocomplete="off" on:submit=|ev| ev.prevent_default()>
                <div class="settings__form-row settings__form-row--compact settings__form-row--inline">
                    <label class="settings__form-control--tiny">
                        <span>"Line limit (chars per line)"</span>
                        <input
                            type="number"
                            min="10"
                            max="120"
                            step="1"
                            data-role="pref-line-limit"
                            prop:value=move || line_limit.get()
                            on:input=on_input
                        />
                    </label>
                </div>
                <p class="settings__hint">
                    "Slides with longer lines show a warning marker. Reload the operator after changing."
                </p>
            </form>
        </section>
    }
}
