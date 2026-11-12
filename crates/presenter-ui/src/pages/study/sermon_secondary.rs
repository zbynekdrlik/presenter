//! "Druhý preklad" (#937, owner 2026-11-12: "vyrobilo to nejaké dvojjazyčné
//! slidy … neviem prečo a kto o tom rozhodol"): the second language under a
//! sermon's slides is the preacher's visible choice, "žiadny" unless he picks
//! one. A pick is one sermon write through the editor's queue; the server
//! generates the slides again.

use leptos::prelude::*;

use super::sermon_editor::{run_write, Editor};
use crate::api::sermons;

/// The `<select>` value of "žiadny".
const NONE: &str = "";

fn pick(e: Editor, value: String) {
    let held = e.detail.with_untracked(|detail| {
        detail
            .as_ref()
            .map(|held| held.secondary_translation.clone())
    });
    let wanted = (value != NONE).then_some(value);
    let Some(current) = held else {
        return;
    };
    if current == wanted {
        return;
    }
    leptos::task::spawn_local(async move {
        run_write(
            e,
            "Kázeň sa medzitým zmenila — skús znova",
            move |id, revision| async move {
                sermons::set_secondary(&id, revision, wanted.as_deref()).await
            },
        )
        .await;
    });
}

#[component]
pub(super) fn SecondaryChoice(e: Editor) -> impl IntoView {
    let select_ref = NodeRef::<leptos::html::Select>::new();
    // Memos: an edit save or a socket event changes `detail` without touching
    // these, and must not rebuild the options under an open picker.
    let current = Memo::new(move |_| {
        e.detail.with(|detail| {
            detail
                .as_ref()
                .and_then(|held| held.secondary_translation.clone())
                .unwrap_or_default()
        })
    });
    let main = Memo::new(move |_| {
        e.detail.with(|detail| {
            detail
                .as_ref()
                .map(|held| held.default_translation.clone())
                .unwrap_or_default()
        })
    });
    // The sermon's own translation is no second language; a stored code no
    // longer installed still shows, or the select would claim "žiadny".
    let options = Memo::new(move |_| {
        let main = main.get();
        let held = current.get();
        let mut list: Vec<(String, String)> = e.s.translations.with(|all| {
            all.iter()
                .filter(|translation| translation.code != main)
                .map(|translation| (translation.code.clone(), translation.name.clone()))
                .collect()
        });
        if !held.is_empty() && !list.iter().any(|(code, _)| *code == held) {
            list.push((held.clone(), held));
        }
        list
    });
    // A failed write leaves `current` as it was, so the Memo does not notify:
    // `detail`'s own change (`show_held` / the reload) puts the shown option
    // back to what the server holds.
    Effect::new(move |_| {
        e.detail.track();
        let held = current.get_untracked();
        if let Some(select) = select_ref.get_untracked() {
            if select.value() != held {
                select.set_value(&held);
            }
        }
    });
    view! {
        <label class="sermon-secondary">
            <span class="sermon-secondary__label">"Druhý preklad"</span>
            <select
                class="study-input sermon-secondary__select"
                data-role="sermon-secondary"
                node_ref=select_ref
                on:change=move |ev| pick(e, event_target_value(&ev))
            >
                <option value=NONE prop:selected=move || current.with(String::is_empty)>
                    "žiadny"
                </option>
                {move || {
                    options
                        .get()
                        .into_iter()
                        .map(|(code, name)| {
                            let selected = code.clone();
                            view! {
                                <option
                                    value=code.clone()
                                    prop:selected=move || current.with(|held| *held == selected)
                                >
                                    {name}
                                </option>
                            }
                        })
                        .collect_view()
                }}
            </select>
        </label>
    }
}
