use leptos::prelude::*;

/// Settings page — configuration for integrations and stage design.
#[component]
pub fn SettingsPage() -> impl IntoView {
    view! {
        <div data-role="settings-page" class="settings-layout">
            <header data-role="settings-header">
                <h1>"Settings"</h1>
            </header>
            <main data-role="settings-main">
                <p>"Settings interface — coming soon."</p>
            </main>
        </div>
    }
}
