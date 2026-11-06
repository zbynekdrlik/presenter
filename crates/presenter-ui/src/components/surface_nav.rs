use leptos::prelude::*;

/// (data-target, label, href) of every other surface.
const TARGETS: [(&str, &str, &str); 5] = [
    ("Stage", "Pódium", "/stage"),
    ("Camera", "Kamera", "/ui/camera"),
    ("Tablet", "Tablet", "/ui/tablet"),
    ("Timer", "Časovač", "/stream/timer"),
    ("Stream", "Stream", "/ui/stream"),
];

/// Links that open the other surfaces in a new browser tab. On a desktop they
/// are the items of the header's Obrazovky menu (#875 H1); on a phone they sit
/// in the "Viac" sheet. Slovak labels in both (#875 H3); `data-target` stays
/// the English name.
#[component]
pub fn SurfaceNav() -> impl IntoView {
    view! {
        <nav
            class="operator__surface-nav"
            data-role="surface-nav"
            aria-label="Otvoriť ďalšie obrazovky na novej karte"
        >
            {TARGETS.into_iter().map(|(target, label, href)| view! {
                <a
                    class="operator__surface-nav-link"
                    data-role="surface-nav-link"
                    data-target=target
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
