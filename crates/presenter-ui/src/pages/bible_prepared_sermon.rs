//! A sermon in the operator's Prepared tab (#851 H1, spec §8.2): read-only
//! cards, because its slides are generated from its items and the server
//! refuses every slide write (`refuse_if_sermon`). A card click still plays the
//! stored slide; "Upraviť v Štúdiu" leads to where it is edited.

use leptos::prelude::*;

use crate::api::bible::{self, BibleSlideDto};
use crate::pages::bible_slides::slide_body_view;
use crate::state::bible::BibleState;
use crate::state::sermon_list::short_date;
use crate::state::AppContext;

/// `Kázeň · ne 19. 10.`
pub(crate) fn sermon_badge(presentation: &bible::BiblePresentationSummary) -> Option<String> {
    presentation
        .sermon_date
        .map(|date| format!("Kázeň · {}", short_date(date)))
}

/// The active presentation's badge, while it is a sermon.
pub(super) fn active_sermon_badge(bs: &BibleState) -> Memo<Option<String>> {
    let (active, presentations) = (bs.active_presentation_id, bs.presentations);
    Memo::new(move |_| {
        let id = active.get()?;
        presentations.with(|list| {
            list.iter()
                .find(|presentation| presentation.id == id)
                .and_then(sermon_badge)
        })
    })
}

#[component]
pub(super) fn SermonSlides(badge: String) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let slides = bs.active_presentation_slides;
    let active = bs.active_presentation_id;
    let version = ctx.bible_presentations_version;
    // The preacher may still be cutting: a card shows what the wall would get,
    // and only the newest answer may land.
    let asked = StoredValue::new(0u64);
    Effect::new(move |previous: Option<()>| {
        version.track();
        if previous.is_none() {
            return;
        }
        let Some(id) = active.get_untracked() else {
            return;
        };
        let mine = asked.get_value().wrapping_add(1);
        asked.set_value(mine);
        leptos::task::spawn_local(async move {
            let answer = bible::get_presentation(&id).await;
            if asked.try_get_value() != Some(mine) {
                return;
            }
            if active.get_untracked().as_deref() != Some(id.as_str()) {
                return;
            }
            if let Ok(detail) = answer {
                slides.set(detail.slides);
            }
        });
    });
    let link = move || {
        active
            .get()
            .map(|id| format!("/ui/study?view=sermon&id={id}"))
    };
    view! {
        <div class="bible__sermon-bar" data-role="sermon-bar">
            <span class="bible__sermon-badge" data-role="sermon-badge">{badge}</span>
            <a class="bible__sermon-link" data-role="sermon-edit-link" href=link>
                "Upraviť v Štúdiu"
            </a>
        </div>
        <For
            each=move || slides.get()
            key=|slide| {
                (
                    slide.id.clone(),
                    slide.bible_main.clone(),
                    slide.bible_translation.clone(),
                    slide.bible_main_reference.clone(),
                    slide.bible_translation_reference.clone(),
                )
            }
            children=move |slide| view! { <SermonSlideCard slide=slide /> }
        />
    }
}

#[component]
fn SermonSlideCard(slide: BibleSlideDto) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let active = bs.active_presentation_id;
    let limit = bs.character_limit;
    let id = slide.id.clone();
    let on_click = move |_| {
        let Some(presentation) = active.get_untracked() else {
            return;
        };
        let (id, ctx) = (id.clone(), ctx.clone());
        leptos::task::spawn_local(async move {
            match bible::trigger_presentation_slide(&presentation, &id).await {
                Ok(_) => ctx.show_toast("Triggered", "success"),
                Err(err) => ctx.show_toast(&format!("Verš sa nezobrazil: {err}"), "error"),
            }
        });
    };
    view! {
        <div
            class="operator__slide-card operator__slide-card--bible"
            data-role="sermon-slide-card"
            data-slide-id=slide.id.clone()
            on:click=on_click
        >
            {move || {
                slide_body_view(
                    slide.bible_main.clone(),
                    slide.bible_translation.clone(),
                    slide.bible_main_reference.clone(),
                    slide.bible_translation_reference.clone(),
                    limit.get(),
                )
            }}
        </div>
    }
}
