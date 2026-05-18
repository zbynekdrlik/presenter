use leptos::prelude::*;

/// Surface-nav pills: 4 inline links that open external surfaces in a new
/// browser tab. Lives in the right side of the operator header's slim top
/// row (rendered in `components/header.rs`'s `operator__header-top`).
/// Separate from the main header row so the existing search / view-nav /
/// stage-output controls keep their full width. See spec
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
