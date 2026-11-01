//! Text size: 3 steps, kept per device (spec §6.1).

use leptos::prelude::*;

use super::sheet::Sheet;
use super::Study;
use crate::state::session;
use crate::state::study::{TextSize, KEY_TEXT_SIZE};

#[component]
pub(super) fn TextSizeSheet(s: Study) -> impl IntoView {
    view! {
        <Sheet
            s=s
            role="study-text-sheet"
            title=Signal::derive(|| "Veľkosť písma".to_string())
            modal=true
        >
            <div class="study-sizes">
                {TextSize::ALL
                    .into_iter()
                    .map(|size| {
                        view! {
                            <button
                                type="button"
                                class="study-size"
                                data-role="study-text-option"
                                data-size=size.key()
                                aria-pressed=move || {
                                    if s.text_size.get() == size { "true" } else { "false" }
                                }
                                on:click=move |_| {
                                    session::set_persistent(KEY_TEXT_SIZE, size.key());
                                    s.text_size.set(size);
                                }
                            >
                                <span class="study-size__sample" aria-hidden="true">"A"</span>
                                <span>{size.label()}</span>
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
        </Sheet>
    }
}
