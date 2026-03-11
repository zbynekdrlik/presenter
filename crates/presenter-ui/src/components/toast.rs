use crate::state::AppContext;
use leptos::prelude::*;

/// Toast notification component.
#[component]
pub fn Toast() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");

    view! {
        <div
            data-role="toast"
            attr:data-visible=move || if ctx.toast_message.get().is_some() { "true" } else { "false" }
            attr:data-variant=move || ctx.toast_variant.get()
            class="operator__toast"
            class:operator__toast--visible=move || ctx.toast_message.get().is_some()
        >
            {move || ctx.toast_message.get().unwrap_or_default()}
        </div>
    }
}
