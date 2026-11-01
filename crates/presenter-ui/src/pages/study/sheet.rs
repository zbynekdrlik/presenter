//! The bottom sheet every study sheet sits in (a side panel at ≥ 1024 px). The
//! ✕ and the backdrop close it with a Back, like the system gesture.

use leptos::prelude::*;

use super::{nav, Study};

/// `modal` dims the page. The verse sheet is not modal: the text stays
/// tappable, so another verse number extends the selection.
#[component]
pub(super) fn Sheet(
    s: Study,
    role: &'static str,
    title: Signal<String>,
    #[prop(optional)] modal: bool,
    children: Children,
) -> impl IntoView {
    // Only a press that STARTED here closes: the finger of the long press that
    // opened this sheet lifts over the backdrop, and that click is not a tap.
    let pressed_here = StoredValue::new(false);
    view! {
        {modal
            .then(|| {
                view! {
                    <div
                        class="study-backdrop"
                        data-role="study-backdrop"
                        on:pointerdown=move |_| pressed_here.set_value(true)
                        on:click=move |_| {
                            if pressed_here.get_value() {
                                pressed_here.set_value(false);
                                nav::close_sheet(s);
                            }
                        }
                    ></div>
                }
            })}
        <section class="study-sheet" data-role=role role="dialog" aria-label=move || title.get()>
            <div class="study-sheet__head">
                <h2 class="study-sheet__title">{move || title.get()}</h2>
                <button
                    type="button"
                    class="study-icon"
                    data-role="study-sheet-close"
                    aria-label="Zavrieť"
                    on:click=move |_| nav::close_sheet(s)
                >
                    "\u{2715}"
                </button>
            </div>
            <div class="study-sheet__body">{children()}</div>
        </section>
    }
}
