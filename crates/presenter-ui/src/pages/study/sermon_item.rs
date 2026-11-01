//! One sermon item's card (spec §7.6): the words, the tools and the item menu.
//! The card listens once for all its words: a chapter has thousands of them.

use leptos::prelude::*;
use presenter_core::bible::sermon::{token_texts, translation_short_code};
use wasm_bindgen::JsCast;

use super::long_press::LongPress;
use super::sermon_editor::{delete_item, write, Editor};
use crate::api::sermons;
use crate::state::sermon_edit::{ItemDraft, TapMode, TokenState};

/// The word under an event: its verse and index.
fn token_at(target: Option<web_sys::EventTarget>) -> Option<(u16, u16)> {
    let element = target?.dyn_into::<web_sys::Element>().ok()?;
    let token = element.closest("[data-role=\"sermon-token\"]").ok()??;
    Some((
        token.get_attribute("data-verse")?.parse().ok()?,
        token.get_attribute("data-index")?.parse().ok()?,
    ))
}

#[component]
fn Words(e: Editor, id: String) -> impl IntoView {
    let press = LongPress::new();
    let on_press = {
        let id = id.clone();
        move |ev: web_sys::PointerEvent| {
            if let Some((verse, index)) = token_at(ev.target()) {
                let id = id.clone();
                press.begin(&ev, move || {
                    let mode = e.mode.get_untracked().other();
                    e.edit(&id, |draft| draft.tap(mode, verse, index));
                });
            }
        }
    };
    let on_tap = {
        let id = id.clone();
        move |ev: web_sys::MouseEvent| {
            if press.take_fired() {
                return;
            }
            if let Some((verse, index)) = token_at(ev.target()) {
                let mode = e.mode.get_untracked();
                e.edit(&id, |draft| draft.tap(mode, verse, index));
            }
        }
    };
    let on_menu = {
        let id = id.clone();
        move |ev: web_sys::MouseEvent| {
            let Some((verse, index)) = token_at(ev.target()) else {
                return;
            };
            press.context_menu(&ev, || {
                let mode = e.mode.get_untracked().other();
                e.edit(&id, |draft| draft.tap(mode, verse, index));
            });
        }
    };
    let words = move || {
        e.drafts.with(|drafts| {
            let Some(draft) = drafts.get(&id) else {
                return Vec::new();
            };
            draft
                .source
                .iter()
                .map(|verse| verse_words(draft, verse.verse, &verse.text))
                .collect()
        })
    };
    view! {
        <div
            class="sermon-words"
            data-role="sermon-words"
            on:pointerdown=on_press
            on:pointermove=move |ev| press.moved(&ev)
            on:pointerup=move |_| press.cancel()
            on:pointercancel=move |_| press.cancel()
            on:click=on_tap
            on:contextmenu=on_menu
        >
            {words}
        </div>
    }
}

fn verse_words(draft: &ItemDraft, verse: u16, text: &str) -> AnyView {
    let tokens = token_texts(text)
        .into_iter()
        .enumerate()
        .map(|(index, token)| {
            let index = u16::try_from(index).unwrap_or(u16::MAX);
            let state = draft.token_state(verse, index);
            let shown = if state == TokenState::Emphasized {
                token.to_uppercase()
            } else {
                token.to_string()
            };
            view! {
                <span
                    class="sermon-token"
                    data-role="sermon-token"
                    data-verse=verse
                    data-index=index
                    data-state=state.key()
                >
                    {shown}
                </span>
                " "
            }
        })
        .collect_view();
    view! {
        <p class="sermon-verse">
            <span class="sermon-verse__number">{verse}</span>
            {tokens}
        </p>
    }
    .into_any()
}

#[component]
fn Tools(e: Editor, id: String) -> impl IntoView {
    let reads = id.clone();
    let can = move |read: fn(&ItemDraft) -> bool| {
        let id = reads.clone();
        move || e.drafts.with(|drafts| drafts.get(&id).is_some_and(read))
    };
    let can_cut = can(ItemDraft::can_cut);
    let can_cut_too = can(ItemDraft::can_cut);
    let can_undo = can(ItemDraft::can_undo);
    let mode_button = move |mode: TapMode, label: &'static str, key: &'static str| {
        view! {
            <button
                type="button"
                class="sermon-tool"
                data-mode=key
                aria-pressed=move || if e.mode.get() == mode { "true" } else { "false" }
                on:click=move |_| e.mode.set(mode)
            >
                {label}
            </button>
        }
    };
    let cut = id.clone();
    let cut_start = id.clone();
    view! {
        <div class="sermon-tools">
            <div class="sermon-tools__modes" data-role="sermon-mode">
                {mode_button(TapMode::Strike, "Škrtať", "strike")}
                {mode_button(TapMode::Emphasize, "Zvýrazniť", "emphasize")}
            </div>
            <button
                type="button"
                class="sermon-tool"
                data-role="sermon-cut-to-end"
                disabled=move || !can_cut()
                on:click=move |_| e.edit(&cut, ItemDraft::cut_to_end)
            >
                "Do konca verša"
            </button>
            <button
                type="button"
                class="sermon-tool"
                data-role="sermon-cut-from-start"
                disabled=move || !can_cut_too()
                on:click=move |_| e.edit(&cut_start, ItemDraft::cut_from_start)
            >
                "Od začiatku verša"
            </button>
            <button
                type="button"
                class="sermon-tool"
                data-role="sermon-undo"
                disabled=move || !can_undo()
                on:click=move |_| e.edit(&id, ItemDraft::undo)
            >
                "Späť"
            </button>
        </div>
    }
}

fn change_translation(e: Editor, id: String, code: String) {
    let Some(item) = e.item(&id) else {
        return;
    };
    if item.translation == code {
        return;
    }
    let edited = e.drafts.with_untracked(|drafts| {
        drafts
            .get(&id)
            .is_some_and(|draft| !draft.edits.0.is_empty())
    });
    let confirmed = !edited
        || crate::utils::window::window()
            .confirm_with_message("Úpravy tejto položky sa zrušia")
            .unwrap_or(false);
    if confirmed {
        write(
            e,
            "Položka sa medzitým zmenila — skús znova",
            move |sermon, revision| async move {
                sermons::change_translation(&sermon, &id, revision, &code).await
            },
        );
    }
}

#[component]
fn ItemMenu(e: Editor, id: String) -> impl IntoView {
    let open = RwSignal::new(false);
    let shows_refresh = {
        let id = id.clone();
        move || {
            e.detail.with(|detail| {
                detail
                    .as_ref()
                    .is_some_and(|held| held.items.iter().any(|i| i.id == id && i.source_changed))
            })
        }
    };
    let translation = {
        let id = id.clone();
        move || e.item(&id).map(|item| item.translation).unwrap_or_default()
    };
    let for_select = id.clone();
    let for_refresh = id.clone();
    let for_delete = id.clone();
    let translations = move || {
        e.s.translations
            .get()
            .into_iter()
            .filter(|t| t.code != crate::pages::study::REMOTE_TRANSLATION)
            .collect::<Vec<_>>()
    };
    view! {
        <button
            type="button"
            class="study-icon"
            data-role="sermon-item-menu"
            aria-label="Možnosti položky"
            aria-expanded=move || if open.get() { "true" } else { "false" }
            on:click=move |_| open.update(|shown| *shown = !*shown)
        >
            "\u{22EF}"
        </button>
        {move || {
            let for_select = for_select.clone();
            let for_refresh = for_refresh.clone();
            let for_delete = for_delete.clone();
            let shown_translation = translation();
            open.get()
                .then(|| {
                    view! {
                        <div class="sermon-menu" data-role="sermon-item-actions">
                            <label class="sermon-new__field">
                                "Zmeniť preklad"
                                <select
                                    class="study-input"
                                    data-role="sermon-item-translation"
                                    on:change=move |ev| {
                                        change_translation(
                                            e,
                                            for_select.clone(),
                                            event_target_value(&ev),
                                        )
                                    }
                                >
                                    {translations()
                                        .into_iter()
                                        .map(|t| {
                                            let selected = t.code == shown_translation;
                                            view! {
                                                <option value=t.code.clone() prop:selected=selected>
                                                    {t.name.clone()}
                                                </option>
                                            }
                                        })
                                        .collect_view()}
                                </select>
                            </label>
                            {shows_refresh()
                                .then(|| {
                                    view! {
                                        <button
                                            type="button"
                                            class="study-button"
                                            data-role="sermon-item-refresh"
                                            on:click=move |_| {
                                                let item = for_refresh.clone();
                                                write(
                                                    e,
                                                    "Položka sa medzitým zmenila — skús znova",
                                                    move |sermon, revision| async move {
                                                        sermons::refresh_source(&sermon, &item, revision).await
                                                    },
                                                );
                                            }
                                        >
                                            "Obnoviť text"
                                        </button>
                                    }
                                })}
                            <button
                                type="button"
                                class="study-button"
                                data-role="sermon-item-delete"
                                on:click=move |_| {
                                    if let Some(item) = e.item(&for_delete) {
                                        delete_item(e, item);
                                    }
                                }
                            >
                                "Odstrániť"
                            </button>
                        </div>
                    }
                })
        }}
    }
}

#[component]
pub(super) fn SermonItemCard(e: Editor, id: String) -> impl IntoView {
    let for_label = id.clone();
    let shown_id = id.clone();
    let menu_id = id.clone();
    let words_id = id.clone();
    let label = Memo::new(move |_| {
        let (book, chapter, translation) = e.detail.with(|detail| {
            detail
                .as_ref()
                .and_then(|held| held.items.iter().find(|item| item.id == for_label))
                .map(|item| (item.book.clone(), item.chapter, item.translation.clone()))
                .unwrap_or_default()
        });
        let shown = e.drafts.with(|drafts| {
            drafts.get(&for_label).map(|draft| {
                (
                    draft.label(&book, chapter, &translation),
                    draft.is_all_struck(),
                )
            })
        });
        let (text, struck) = shown.unwrap_or_default();
        (text, struck, translation)
    });
    view! {
        <article
            class="sermon-item"
            data-role="sermon-item"
            data-item-id=shown_id
            data-label=move || label.with(|held| held.0.clone())
            data-translation=move || label.with(|held| held.2.clone())
        >
            <div class="sermon-item__head">
                <h3 class="sermon-item__label">{move || label.with(|held| held.0.clone())}</h3>
                <span class="study-chip sermon-item__chip">
                    {move || label.with(|held| translation_short_code(&held.2))}
                </span>
                <ItemMenu e=e id=menu_id />
            </div>
            {move || {
                label
                    .with(|held| held.1)
                    .then(|| {
                        view! {
                            <p class="sermon-item__warning" data-role="sermon-item-struck">
                                "Položka je celá vyškrtnutá — nezobrazí sa"
                            </p>
                        }
                    })
            }}
            <Words e=e id=words_id />
            <Tools e=e id=id />
        </article>
    }
}
