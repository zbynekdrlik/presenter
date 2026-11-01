//! Náhľad (spec §7.6): the slides exactly as stored, what the wall would get,
//! with the character badges of the operator's list and the secondary line's
//! state. Nothing here reaches a wall (T11).

use leptos::prelude::*;

use super::sermon_editor::{write, Editor};
use super::sheet::Sheet;
use crate::api::bible as bible_api;
use crate::api::sermons::{self, SermonSlide};
use crate::state::bible::slide_char_count;

const SECONDARY_MISSING: &str = "Druhý preklad sa nepodarilo načítať — slajdy ho zatiaľ nemajú";

fn char_badge(field: &'static str, text: &str, limit: Option<u32>) -> impl IntoView {
    let (count, over) = slide_char_count(text, limit.unwrap_or(u32::MAX));
    view! {
        <span
            class="sermon-slide__chars"
            data-role="slide-char-count"
            data-field=field
            data-over=if over { "true" } else { "false" }
        >
            {count}
        </span>
    }
}

fn slide_view(slide: SermonSlide, limit: Option<u32>) -> impl IntoView {
    let has_secondary = !slide.secondary_text.is_empty();
    view! {
        <article class="sermon-slide" data-role="sermon-slide" data-slide-id=slide.id.clone()>
            <div class="sermon-slide__head">
                <h3 class="sermon-slide__label" data-role="sermon-slide-label">
                    {slide.main_reference.clone()}
                </h3>
                {char_badge("main", &slide.main_text, limit)}
                {has_secondary.then(|| char_badge("translation", &slide.secondary_text, limit))}
            </div>
            <p class="sermon-slide__text" data-role="sermon-slide-main">{slide.main_text.clone()}</p>
            {has_secondary
                .then(|| {
                    view! {
                        <p class="sermon-slide__text sermon-slide__text--secondary">
                            {slide.secondary_text.clone()}
                        </p>
                        <p class="sermon-slide__label">{slide.secondary_reference.clone()}</p>
                    }
                })}
        </article>
    }
}

#[component]
pub(super) fn PreviewSheet(e: Editor) -> impl IntoView {
    let limit = RwSignal::new(None);
    leptos::task::spawn_local(async move {
        // Without the preference the badges still count, none reads as over.
        if let Ok(preferences) = bible_api::get_preferences().await {
            limit.set(Some(preferences.character_limit));
        }
    });
    let slides = move || {
        e.detail.with(|detail| {
            detail
                .as_ref()
                .map(|held| held.slides.clone())
                .unwrap_or_default()
        })
    };
    let warning = move || {
        e.detail.with(|detail| {
            let held = detail.as_ref().filter(|held| held.secondary_missing)?;
            Some(
                held.warnings
                    .first()
                    .cloned()
                    .unwrap_or_else(|| SECONDARY_MISSING.to_string()),
            )
        })
    };
    let regenerate = move |_| {
        write(
            e,
            "Kázeň sa medzitým zmenila — skús znova",
            move |id, revision| async move { sermons::regenerate(&id, revision).await },
        );
    };
    view! {
        <Sheet s=e.s role="sermon-preview" title=Signal::derive(|| "Náhľad".to_string())>
            {move || {
                warning()
                    .map(|message| {
                        view! {
                            <div class="sermon-preview__warning" data-role="sermon-preview-warning">
                                <p>{message}</p>
                                <button
                                    type="button"
                                    class="study-button"
                                    data-role="sermon-regenerate"
                                    on:click=regenerate
                                >
                                    "Obnoviť slajdy"
                                </button>
                            </div>
                        }
                    })
            }}
            <div class="sermon-preview">
                {move || {
                    let shown = slides();
                    let empty = shown.is_empty();
                    let limit = limit.get();
                    view! {
                        {empty
                            .then(|| {
                                view! {
                                    <p class="study-empty" data-role="sermon-preview-empty">
                                        "Zatiaľ žiadne slajdy"
                                    </p>
                                }
                            })}
                        {shown
                            .into_iter()
                            .map(|slide| slide_view(slide, limit))
                            .collect_view()}
                    }
                }}
            </div>
        </Sheet>
    }
}
