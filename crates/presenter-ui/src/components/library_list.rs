use leptos::prelude::*;

use crate::components::modal;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

#[component]
pub fn LibraryList() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let _op = use_context::<OperatorState>().expect("OperatorState");

    let select_library = move |id: String, name: String| {
        ctx.selected_library_id.set(Some(id.clone()));
        ctx.selected_playlist_id.set(None);
        ctx.context_title.set(name);
        crate::state::session::set("activeLibraryId", &id);
        crate::state::session::remove("activePlaylistId");

        let id_clone = id.clone();
        leptos::task::spawn_local(async move {
            if let Ok(presentations) = crate::api::libraries::list_presentations(&id_clone).await {
                let ctx = use_context::<AppContext>().expect("AppContext");
                ctx.presentations.set(presentations);
            }
        });
    };

    let on_create = move |_| {
        let op = use_context::<OperatorState>().expect("OperatorState");
        op.modal_mode.set("create".to_string());
        op.modal_target_id.set(None);
        modal::open_modal(&op, "library-edit");
    };

    let on_more = move |_| {
        let op = use_context::<OperatorState>().expect("OperatorState");
        modal::open_modal(&op, "library");
    };

    let visible_count: usize = 5;

    view! {
        <div class="operator__list-section">
            <div class="operator__list-header">
                <h3 class="operator__list-title">"Libraries"</h3>
                <button data-role="library-more" class="operator__list-more" on:click=on_more>
                    {move || {
                        let total = ctx.libraries.get().len();
                        if total > visible_count { format!("{total}") } else { String::new() }
                    }}
                </button>
                <button data-role="library-create" class="operator__list-create" on:click=on_create>"+"</button>
            </div>
            <ul data-role="library-list" class="operator__list">
                {move || {
                    let libs = ctx.libraries.get();
                    let favs = ctx.favorite_library_ids.get();
                    let active_id = ctx.selected_library_id.get();

                    let mut sorted: Vec<_> = libs.into_iter().collect();
                    sorted.sort_by(|a, b| {
                        let a_fav = favs.contains(&a.id.to_string());
                        let b_fav = favs.contains(&b.id.to_string());
                        b_fav.cmp(&a_fav).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                    });

                    // Show favorites + active + up to visible_count
                    let visible: Vec<_> = sorted.into_iter().take(visible_count).collect();

                    visible.into_iter().map(|lib| {
                        let id = lib.id.to_string();
                        let name = lib.name.clone();
                        let count = lib.presentation_count;
                        let is_active = active_id.as_deref() == Some(&id);
                        let id_for_click = id.clone();
                        let name_for_click = name.clone();
                        let id_for_edit = id.clone();
                        let id_for_row = id.clone();
                        let id_for_btn = id.clone();
                        let id_for_modal = id.clone();

                        view! {
                            <li data-role="library-row" data-library-id=id_for_row class="operator__list-item">
                                <button
                                    data-role="library-item"
                                    data-library-id=id_for_btn
                                    attr:data-active=move || if is_active { "true" } else { "false" }
                                    class="operator__list-btn"
                                    on:click=move |_| {
                                        select_library(id_for_click.clone(), name_for_click.clone());
                                    }
                                >
                                    <span class="operator__list-label">{name}</span>
                                    <span class="operator__list-meta" data-role="library-count">{count}</span>
                                </button>
                                <button
                                    data-action="library-edit"
                                    data-library-id=id_for_edit
                                    class="operator__list-edit"
                                    on:click=move |ev: leptos::ev::MouseEvent| {
                                        ev.stop_propagation();
                                        let op = use_context::<OperatorState>().expect("OperatorState");
                                        op.modal_mode.set("edit".to_string());
                                        op.modal_target_id.set(Some(id_for_modal.clone()));
                                        modal::open_modal(&op, "library-edit");
                                    }
                                >
                                    "\u{270e}"
                                </button>
                            </li>
                        }
                    }).collect_view()
                }}
            </ul>
        </div>
    }
}
