//! Biblia → Nastavenie → „Preferované preklady" (#921): which translations
//! get a chip in the console's two rows. One checkbox per installed
//! translation and row; a tick adds the translation at the end of the row's
//! list, an untick removes it. Saved at once with the Bible preferences on
//! the server (`BiblePreferencesDraft::preferred_*`), the shared default every
//! console reads on its next start.
//!
//! Saves go one at a time and always carry BOTH lists: two quick ticks never
//! race (only the newest waiting save follows the one on its way), and a
//! refused save puts the lists back to what the server holds.

use leptos::prelude::*;
use presenter_core::bible::sermon::translation_short_code;
use presenter_core::BiblePreferencesDraft;

use crate::api::bible;
use crate::state::bible::BibleState;
use crate::state::bible_translations::{
    console_chip_cap, offered_as_main, translation_groups, translation_name, TranslationGroup,
    TranslationRow as Row,
};
use crate::state::AppContext;

/// One save on its way at a time; the newest change waits for it.
#[derive(Default)]
struct SaveQueue {
    sending: bool,
    waiting: Option<BiblePreferencesDraft>,
}

impl SaveQueue {
    /// `Some(draft)` = send it now; `None` = it waits behind the one on its way.
    fn offer(&mut self, draft: BiblePreferencesDraft) -> Option<BiblePreferencesDraft> {
        if self.sending {
            self.waiting = Some(draft);
            None
        } else {
            self.sending = true;
            Some(draft)
        }
    }

    /// The save on its way answered: the next one to send, if any.
    fn answered(&mut self) -> Option<BiblePreferencesDraft> {
        let next = self.waiting.take();
        self.sending = next.is_some();
        next
    }
}

fn both_lists(bs: BibleState) -> BiblePreferencesDraft {
    BiblePreferencesDraft {
        preferred_main: Some(bs.preferred_main.get_untracked()),
        preferred_secondary: Some(bs.preferred_secondary.get_untracked()),
        ..Default::default()
    }
}

/// Send `draft`, then whatever waited behind it. A refused save toasts and
/// re-reads the lists the server holds, so no box claims a save that did not
/// happen.
fn send(
    bs: BibleState,
    ctx: AppContext,
    queue: StoredValue<SaveQueue>,
    draft: BiblePreferencesDraft,
) {
    leptos::task::spawn_local(async move {
        let mut next = Some(draft);
        while let Some(draft) = next {
            if let Err(e) = bible::update_preferences(&draft).await {
                ctx.show_toast(&format!("Uloženie zlyhalo: {e}"), "error");
                if let Ok(stored) = bible::get_preferences().await {
                    bs.preferred_main.set(stored.preferred_main_or_default());
                    bs.preferred_secondary
                        .set(stored.preferred_secondary_or_default());
                }
            }
            next = queue.try_update_value(SaveQueue::answered).flatten();
        }
    });
}

/// The row's list after a tick (`on`) or an untick of `code`.
fn toggled(list: &[String], code: &str, on: bool) -> Vec<String> {
    let mut next: Vec<String> = list.iter().filter(|c| *c != code).cloned().collect();
    if on {
        next.push(code.to_string());
    }
    next
}

#[component]
pub fn PreferredTranslations() -> impl IntoView {
    let queue = StoredValue::new(SaveQueue::default());
    view! {
        <div class="bible-preferred" data-role="preferred-translations">
            <h3 class="bible-preferred__title">"Preferované preklady"</h3>
            <p class="bible-preferred__hint">
                {format!(
                    "Tlačidlá v riadku prekladov na karte Naživo. Zmestí sa ich {} (na telefóne {}), \
                     v druhom riadku o jedno menej; všetky ostatné preklady sú v zozname \u{25BE}.",
                    console_chip_cap(false, false),
                    console_chip_cap(true, false),
                )}
            </p>
            <RowChoices row=Row::Main queue=queue />
            <RowChoices row=Row::Secondary queue=queue />
        </div>
    }
}

#[component]
fn RowChoices(row: Row, queue: StoredValue<SaveQueue>) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let translations = bs.translations;
    let (preferred, legend) = match row {
        Row::Main => (bs.preferred_main, "Hlavný preklad"),
        Row::Secondary => (bs.preferred_secondary, "Druhý preklad"),
    };
    let kind = row.kind();
    let groups = move || {
        translations.with(|installed| {
            translation_groups(installed, |code| match row {
                // NLT is never a main translation: no main chip for it either.
                Row::Main => offered_as_main(code, None),
                Row::Secondary => true,
            })
        })
    };
    let on_toggle = move |code: String, on: bool| {
        preferred.update(|list| *list = toggled(list, &code, on));
        let draft = both_lists(bs);
        if let Some(draft) = queue.try_update_value(|q| q.offer(draft)).flatten() {
            send(bs, ctx.clone(), queue, draft);
        }
    };

    view! {
        <fieldset class="bible-preferred__row" data-row=kind>
            <legend>{legend}</legend>
            <For
                each=groups
                key=|group: &TranslationGroup| (group.label, group.codes.clone())
                children=move |group| {
                    let on_toggle = on_toggle.clone();
                    view! {
                        <div class="bible-preferred__group">
                            <span class="bible-preferred__group-label">{group.label}</span>
                            {group
                                .codes
                                .into_iter()
                                .map(|code| {
                                    let name = translations
                                        .with_untracked(|list| translation_name(list, &code));
                                    let label = translation_short_code(&code);
                                    let mine = code.clone();
                                    let checked = Memo::new(move |_| {
                                        preferred.with(|list| list.iter().any(|c| *c == mine))
                                    });
                                    let on_toggle = on_toggle.clone();
                                    let toggle_code = code.clone();
                                    view! {
                                        <label class="bible-preferred__option">
                                            <input
                                                type="checkbox"
                                                data-role="preferred-translation"
                                                data-kind=kind
                                                data-translation=code.clone()
                                                prop:checked=move || checked.get()
                                                on:change=move |ev| {
                                                    on_toggle(toggle_code.clone(), event_target_checked(&ev))
                                                }
                                            />
                                            <span class="bible-preferred__code">{label}</span>
                                            <span class="bible-preferred__name">{name}</span>
                                        </label>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                }
            />
        </fieldset>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn a_tick_adds_at_the_end_and_an_untick_removes() {
        let list = codes(&["slk-seb", "slk-roh"]);
        assert_eq!(
            toggled(&list, "ces-csp", true),
            codes(&["slk-seb", "slk-roh", "ces-csp"])
        );
        assert_eq!(toggled(&list, "slk-seb", false), codes(&["slk-roh"]));
        // A repeated tick never duplicates.
        assert_eq!(
            toggled(&list, "slk-roh", true),
            codes(&["slk-seb", "slk-roh"])
        );
    }

    fn draft(main: &[&str]) -> BiblePreferencesDraft {
        BiblePreferencesDraft {
            preferred_main: Some(codes(main)),
            ..Default::default()
        }
    }

    #[test]
    fn saves_go_one_at_a_time_and_only_the_newest_waits() {
        let mut queue = SaveQueue::default();
        assert_eq!(queue.offer(draft(&["a"])), Some(draft(&["a"])));
        // Two more ticks while the first is on its way: only the newest waits.
        assert_eq!(queue.offer(draft(&["a", "b"])), None);
        assert_eq!(queue.offer(draft(&["a", "b", "c"])), None);
        assert_eq!(queue.answered(), Some(draft(&["a", "b", "c"])));
        // Nothing waits behind that one: the queue is free again.
        assert_eq!(queue.answered(), None);
        assert_eq!(queue.offer(draft(&["x"])), Some(draft(&["x"])));
    }
}
