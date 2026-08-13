//! Tablet slide list rendering (#693).
//!
//! Split out of `pages/tablet.rs` to keep that file under the size cap. The
//! list is a keyed `<For>` (keyed by the stable `slide.id`) so a live content
//! edit reconciles in place instead of tearing down the `.tablet-main` scroll
//! subtree — preserving the user's scroll position and the marked slide. Per
//! the keyed-`<For>` gotcha (`.claude/skills/ui/SKILL.md` #496 / #693), every
//! mutable value is read REACTIVELY by id inside `children`.

use leptos::prelude::*;

use super::{html_escape_multiline, is_slide_active, trigger_slide};
use crate::api::bible::BibleSlideDto;
use crate::state::tablet::TabletContext;

/// #693: per-row striping metadata derived once from the ordered slide list.
/// Kept SEPARATE from slide content so a pure text edit (same ids, same
/// striping) yields an equal `Vec` — the backing `Memo` then never notifies and
/// the keyed `<For>` below stays completely untouched, preserving scroll.
#[derive(Clone, PartialEq)]
struct SlideRowMeta {
    id: String,
    is_light: bool,
    is_group_start: bool,
}

fn compute_row_meta(slides: &[BibleSlideDto]) -> Vec<SlideRowMeta> {
    let mut last_reference: Option<String> = None;
    let mut group_index: usize = 0;
    let mut rows = Vec::with_capacity(slides.len());

    for slide in slides {
        let effective_ref = if slide.bible_main_reference.is_empty() {
            None
        } else {
            Some(slide.bible_main_reference.clone())
        };
        let is_new_group = effective_ref.as_deref() != last_reference.as_deref();
        if is_new_group && last_reference.is_some() {
            group_index += 1;
        }
        last_reference = effective_ref;

        rows.push(SlideRowMeta {
            id: slide.id.clone(),
            is_light: group_index % 2 == 0,
            is_group_start: is_new_group && group_index > 0,
        });
    }

    rows
}

/// Reactive reader for a string field of the slide identified by `id`. Returns
/// a `Fn` closure so the value is RE-READ from the source signal on every edit
/// and patched in place — a keyed `<For>` does not re-run `children` for an
/// unchanged key (ui skill, #496), so captured-once text would go stale.
fn slide_field(
    slides: RwSignal<Vec<BibleSlideDto>>,
    id: String,
    pick: impl Fn(&BibleSlideDto) -> String + 'static,
) -> impl Fn() -> String {
    move || {
        slides.with(|list| {
            list.iter()
                .find(|slide| slide.id == id)
                .map(&pick)
                .unwrap_or_default()
        })
    }
}

/// Reactive reader for a boolean striping flag of the row identified by `id`.
fn row_flag(
    row_meta: Memo<Vec<SlideRowMeta>>,
    id: String,
    pick: impl Fn(&SlideRowMeta) -> bool + 'static,
    default: bool,
) -> impl Fn() -> bool {
    move || {
        row_meta.with(|rows| {
            rows.iter()
                .find(|row| row.id == id)
                .map(&pick)
                .unwrap_or(default)
        })
    }
}

#[component]
pub(super) fn SlideList() -> impl IntoView {
    let ctx = use_ctx!(TabletContext);
    let slides = ctx.slides;
    let current_pid = ctx.current_presentation_id;
    let row_meta = Memo::new(move |_| compute_row_meta(&slides.get()));

    view! {
        <Show
            when=move || current_pid.get().is_some()
            fallback=|| view! {
                <p class="tablet-slides__empty">"Select a presentation to view slides."</p>
            }
        >
            <Show
                when=move || !row_meta.get().is_empty()
                fallback=|| view! {
                    <p class="tablet-slides__empty">"No slides in this presentation."</p>
                }
            >
                <For
                    each=move || row_meta.get()
                    key=|meta| meta.id.clone()
                    children=move |meta: SlideRowMeta| {
                        view! { <TabletSlideCard slide_id=meta.id row_meta=row_meta /> }
                    }
                />
            </Show>
        </Show>
    }
}

#[component]
fn TabletSlideCard(slide_id: String, row_meta: Memo<Vec<SlideRowMeta>>) -> impl IntoView {
    let ctx = use_ctx!(TabletContext);
    let slides = ctx.slides;
    let active_broadcast = ctx.active_broadcast;
    let active_slide_id = ctx.active_slide_id;
    let is_loading = RwSignal::new(false);

    // #693: EVERY mutable value is read reactively by id (see `slide_field` /
    // `row_flag`) so a live content edit patches this card in place instead of
    // tearing down and rebuilding it — which is what reset the user's scroll.
    let main_ref = slide_field(slides, slide_id.clone(), |s| s.bible_main_reference.clone());
    let main_text = slide_field(slides, slide_id.clone(), |s| s.bible_main.clone());
    let translation_text = slide_field(slides, slide_id.clone(), |s| s.bible_translation.clone());

    let is_light = row_flag(row_meta, slide_id.clone(), |m| m.is_light, true);
    let is_dark = row_flag(row_meta, slide_id.clone(), |m| !m.is_light, false);
    let is_group_start = row_flag(row_meta, slide_id.clone(), |m| m.is_group_start, false);

    let is_active = {
        let id = slide_id.clone();
        move || {
            slides.with(|list| {
                list.iter()
                    .find(|s| s.id == id)
                    .is_some_and(|s| is_slide_active(s, &active_broadcast.get()))
            }) || active_slide_id.get().as_deref() == Some(id.as_str())
        }
    };

    let on_click = {
        let ctx = ctx.clone();
        let id = slide_id.clone();
        move |_| {
            let Some(slide) = ctx.slides.get_untracked().into_iter().find(|s| s.id == id) else {
                return;
            };
            let ctx = ctx.clone();
            let loading = is_loading;
            loading.set(true);
            leptos::task::spawn_local(async move {
                trigger_slide(&ctx, &slide).await;
                loading.set(false);
            });
        }
    };

    let ref_view = move || {
        let value = main_ref();
        (!value.is_empty()).then(move || {
            view! { <header class="tablet-slide__ref">{value}</header> }
        })
    };
    let main_view = move || {
        let value = main_text();
        (!value.is_empty())
            .then(move || view! { <p class="tablet-slide__main" inner_html=html_escape_multiline(&value) /> })
    };
    let translation_view = move || {
        let value = translation_text();
        (!value.is_empty())
            .then(move || view! { <p class="tablet-slide__translation" inner_html=html_escape_multiline(&value) /> })
    };

    view! {
        <article
            class="tablet-slide"
            class:tablet-slide--light=is_light
            class:tablet-slide--dark=is_dark
            class:tablet-slide--group-start=is_group_start
            class:is-active=is_active
            class:is-loading=move || is_loading.get()
            data-role="tablet-slide"
            data-slide-id=slide_id
            on:click=on_click
        >
            {ref_view}
            <section class="tablet-slide__body">
                {main_view}
                {translation_view}
            </section>
        </article>
    }
}
