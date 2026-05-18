use leptos::prelude::*;

/// Surface-nav strip: 4 pill links that open external surfaces in a new
/// browser tab. Lives on the operator chrome only (rendered in
/// `pages/operator.rs`). See spec
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
            <span class="operator__surface-nav-label">"Open in new tab:"</span>
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
