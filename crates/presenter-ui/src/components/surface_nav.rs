use leptos::prelude::*;

/// Surface-nav pills: 4 inline links that open external surfaces in a new
/// browser tab. Lives inside the operator header's brand row (rendered in
/// `components/header.rs`'s `operator__header-brand`, immediately after the
/// version badge). Header uses `align-items: flex-start` so brand+pills sit
/// flush at the top border; the existing Worship/Bible/Timers/AI/Settings
/// tabs and stage-output controls keep their original positions. See spec
/// `docs/superpowers/specs/2026-05-18-operator-surface-nav-design.md`.
#[component]
pub fn SurfaceNav() -> impl IntoView {
    let targets = [
        ("Stage", "/stage"),
        ("Camera", "/ui/camera"),
        ("Tablet", "/ui/tablet"),
        ("Timer", "/overlays/timer"),
    ];

    view! {
        <nav
            class="operator__surface-nav"
            data-role="surface-nav"
            aria-label="Open other surfaces in a new tab"
        >
            {targets.into_iter().map(|(label, href)| view! {
                <a
                    class="operator__surface-nav-link"
                    data-role="surface-nav-link"
                    data-target=label
                    href=href
                    target="_blank"
                    rel="noopener"
                >
                    {label}
                    <span class="operator__surface-nav-icon" aria-hidden="true">"\u{2197}"</span>
                </a>
            }).collect_view()}
        </nav>
    }
}
