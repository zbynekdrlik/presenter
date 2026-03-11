use leptos::prelude::*;

use crate::state::AppContext;

/// Toast notification component.
#[component]
pub fn Toast(ctx: AppContext) -> impl IntoView {
    let msg = ctx.toast_message;
    view! {
        <div
            class="operator__toast"
            data-role="toast"
            class:operator__toast--visible=move || msg.get().is_some()
        >
            {move || msg.get().unwrap_or_default()}
        </div>
    }
}
