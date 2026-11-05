use leptos::prelude::*;

/// (data-target, English label, Slovak label, href) of every other surface.
const TARGETS: [(&str, &str, &str, &str); 5] = [
    ("Stage", "Stage", "Pódium", "/stage"),
    ("Camera", "Camera", "Kamera", "/ui/camera"),
    ("Tablet", "Tablet", "Tablet", "/ui/tablet"),
    ("Timer", "Timer", "Časovač", "/stream/timer"),
    ("Stream", "Stream", "Stream", "/ui/stream"),
];

/// Links that open the other surfaces in a new browser tab. On a desktop they
/// are the items of the header's Obrazovky menu (`slovak`, #875 H1); on a phone
/// they sit in the "Viac" sheet with their English labels (the phone shell is
/// unchanged by #875). `data-target` stays the English name in both.
#[component]
pub fn SurfaceNav(#[prop(optional)] slovak: bool) -> impl IntoView {
    view! {
        <nav
            class="operator__surface-nav"
            data-role="surface-nav"
            aria-label="Open other surfaces in a new tab"
        >
            {TARGETS.into_iter().map(|(target, en, sk, href)| view! {
                <a
                    class="operator__surface-nav-link"
                    data-role="surface-nav-link"
                    data-target=target
                    href=href
                    target="_blank"
                    rel="noopener"
                >
                    {if slovak { sk } else { en }}
                    <span class="operator__surface-nav-icon" aria-hidden="true">"\u{2197}"</span>
                </a>
            }).collect_view()}
        </nav>
    }
}
