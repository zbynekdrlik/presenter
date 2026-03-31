use leptos::prelude::*;
use presenter_core::BibleSlideOutput;

#[component]
pub fn BibleOverlay(overlay: RwSignal<Option<BibleSlideOutput>>) -> impl IntoView {
    let data_visible = move || if overlay.get().is_some() { "true" } else { "false" };

    view! {
        <div class="stage__bible-overlay" data-visible=data_visible>
            {move || {
                overlay.get().map(|output| {
                    let has_secondary = !output.secondary_text.is_empty();
                    let secondary_visible = if has_secondary { "true" } else { "false" };

                    view! {
                        <div class="stage__bible-text">{output.main_text.clone()}</div>
                        <div class="stage__bible-reference">{output.main_reference.clone()}</div>

                        <div class="stage__bible-secondary" data-visible=secondary_visible>
                            <div class="stage__bible-secondary-text">
                                {output.secondary_text.clone()}
                            </div>
                            <div class="stage__bible-secondary-ref">
                                {output.secondary_reference.clone()}
                            </div>
                        </div>
                    }
                })
            }}
        </div>
    }
}
