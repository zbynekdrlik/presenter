//! Bookmark folders (#952, owner 2026-11-20: "viacej zalozkovych zloziek, ktore
//! si mozem samostatne vyberat"; folders, not tags). The Záložky sheet opens on
//! the default folder "Kde som skončil" (the list #951 always had) and shows
//! a row of folder chips over the list: the default one, the reader's own
//! folders, "＋ Nový priečinok". A chip shows that folder (`s=zalozky<id>`, a
//! `nav::replace`, so Back still closes the sheet in one step). An own folder
//! gets a menu row: "Premenovať" and "Zmazať priečinok", which asks first and
//! takes the folder's bookmarks with it. In the verse sheet, ▾ beside "Pridať
//! záložku" opens [`FolderPicker`]: one tap saves into the chosen folder.
//!
//! The folders are `Study::bookmark_folders`, read again whenever the sheet or
//! the picker opens; the server keeps them per reader, like the bookmarks. The
//! list on screen stays while a new read is on its way (no jumping chips on a
//! switch); only the NEWEST read may land (`Study::folder_reads`), and a sheet
//! judges its URL folder only on an answer asked after it opened ([`fresh`]).

use leptos::prelude::*;

use super::bookmarks::{self, BookmarkList};
use super::{nav, sermon_error, Study};
use crate::api::bookmarks::{self as api, Folder};
use crate::state::study::StudySheet;
use crate::state::study_bookmarks::{folder_delete_question, DEFAULT_FOLDER_NAME};

/// The folder reads: how many were asked, and the newest one that answered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct FolderReads {
    asked: u64,
    answered: u64,
}

/// Read the reader's folders again; answers this read's number. An answer
/// older than the newest one asked is dropped.
pub(super) fn load_folders(s: Study) -> u64 {
    let read = s
        .folder_reads
        .try_update(|reads| {
            reads.asked += 1;
            reads.asked
        })
        .unwrap_or(0);
    leptos::task::spawn_local(async move {
        let answer = api::folders().await;
        let newest = s.folder_reads.try_with_untracked(|reads| reads.asked) == Some(read);
        if !newest {
            return;
        }
        match answer {
            Ok(list) => {
                s.bookmark_folders.try_set(Some(list));
            }
            Err(err) => s.tell(sermon_error(&err)),
        }
        s.folder_reads.try_update(|reads| reads.answered = read);
    });
    read
}

/// The folder list on screen answers read `read` or a later one.
pub(super) fn fresh(s: Study, read: u64) -> bool {
    s.folder_reads.with(|reads| reads.answered >= read) && s.bookmark_folders.with(Option::is_some)
}

/// The ids of the reader's folders on screen.
pub(super) fn folder_ids(s: Study) -> Vec<i64> {
    s.bookmark_folders
        .with_untracked(|folders| folders.iter().flatten().map(|folder| folder.id).collect())
}

/// Show one folder in the sheet, in place (no history step).
pub(super) fn show(s: Study, folder: Option<i64>) {
    let route = s.route.get_untracked();
    let wanted = Some(StudySheet::Bookmarks(folder));
    if route.sheet != wanted {
        nav::replace(s, route.with_sheet(wanted));
    }
}

fn folder_by_id(s: Study, id: i64) -> Option<Folder> {
    s.bookmark_folders.with(|folders| {
        folders
            .as_ref()?
            .iter()
            .find(|folder| folder.id == id)
            .cloned()
    })
}

fn ask_name(question: &str, current: &str) -> Option<String> {
    crate::utils::window::window()
        .prompt_with_message_and_default(question, current)
        .ok()
        .flatten()
}

/// "＋ Nový priečinok": a name, then the new folder is shown (empty).
fn create(s: Study) {
    let Some(name) = ask_name("Názov nového priečinka:", "") else {
        return;
    };
    leptos::task::spawn_local(async move {
        match api::create_folder(&name).await {
            Ok(made) => {
                s.tell(format!("Priečinok „{}“ je vytvorený", made.name));
                show(s, Some(made.id));
            }
            // An empty name or one in use: the server's own sentence.
            Err(err) => s.tell(sermon_error(&err)),
        }
    });
}

fn rename(s: Study, folder: Folder) {
    let Some(name) = ask_name("Nový názov priečinka:", &folder.name) else {
        return;
    };
    if name.trim() == folder.name {
        return;
    }
    leptos::task::spawn_local(async move {
        match api::rename_folder(folder.id, &name).await {
            Ok(renamed) => {
                s.tell(format!("Priečinok sa volá „{}“", renamed.name));
                load_folders(s);
            }
            Err(err) => s.tell(sermon_error(&err)),
        }
    });
}

/// The bookmarks the folder holds now: the list on screen when it answered
/// (it is this folder's), else the count the folder list read.
fn held_count(s: Study, folder: &Folder) -> usize {
    s.bookmarks.with_untracked(|list| match list {
        BookmarkList::Ready(rows) => rows.len(),
        _ => folder.count,
    })
}

/// "Zmazať priečinok": asks (naming how many bookmarks go with it), then the
/// default folder shows.
fn delete(s: Study, folder: Folder) {
    let question = folder_delete_question(&folder.name, held_count(s, &folder));
    let confirmed = crate::utils::window::window()
        .confirm_with_message(&question)
        .unwrap_or(false);
    if !confirmed {
        return;
    }
    leptos::task::spawn_local(async move {
        match api::delete_folder(folder.id).await {
            Ok(gone) => {
                s.tell(format!("Priečinok „{}“ je zmazaný", gone.name));
                show(s, None);
            }
            Err(err) => {
                s.tell(sermon_error(&err));
                load_folders(s);
            }
        }
    });
}

/// The folder chips over the list. `read` = the folder read this sheet asked
/// for: `data-loaded` (the specs' settle marker) waits for it.
#[component]
pub(super) fn FolderRow(s: Study, folder: Option<i64>, read: u64) -> impl IntoView {
    let chip = move |id: Option<i64>, name: String| {
        view! {
            <button
                type="button"
                class="study-chip"
                data-role="bookmark-folder"
                data-folder=id.map(|id| id.to_string()).unwrap_or_default()
                aria-pressed=(folder == id).to_string()
                on:click=move |_| show(s, id)
            >
                {name}
            </button>
        }
    };
    view! {
        <div
            class="study-chips study-folders"
            data-role="bookmark-folders"
            data-loaded=move || fresh(s, read).then_some("true")
        >
            {chip(None, DEFAULT_FOLDER_NAME.to_string())}
            <For
                each=move || s.bookmark_folders.get().unwrap_or_default()
                key=|folder| (folder.id, folder.name.clone())
                children=move |folder| chip(Some(folder.id), folder.name)
            />
            <button
                type="button"
                class="study-chip study-folders__new"
                data-role="bookmark-folder-new"
                on:click=move |_| create(s)
            >
                "＋ Nový priečinok"
            </button>
        </div>
    }
}

/// An own folder's menu: rename, delete. The default folder has none.
#[component]
pub(super) fn FolderMenu(s: Study, folder: Option<i64>) -> impl IntoView {
    folder.map(|id| {
        let current = Memo::new(move |_| folder_by_id(s, id));
        view! {
            <div class="study-folder-menu" data-role="bookmark-folder-menu">
                <button
                    type="button"
                    class="study-button"
                    data-role="bookmark-folder-rename"
                    disabled=move || current.with(Option::is_none)
                    on:click=move |_| {
                        if let Some(folder) = current.get_untracked() {
                            rename(s, folder);
                        }
                    }
                >
                    "Premenovať"
                </button>
                <button
                    type="button"
                    class="study-button study-button--danger"
                    data-role="bookmark-folder-delete"
                    disabled=move || current.with(Option::is_none)
                    on:click=move |_| {
                        if let Some(folder) = current.get_untracked() {
                            delete(s, folder);
                        }
                    }
                >
                    "Zmazať priečinok"
                </button>
            </div>
        }
    })
}

/// ▾ beside "Pridať záložku": the folders, one tap saves the verse into one.
/// `closing` is the verse sheet's: a save closes it, so a second tap is no
/// second bookmark.
#[component]
pub(super) fn FolderPicker(s: Study, closing: StoredValue<bool>) -> impl IntoView {
    let read = load_folders(s);
    let pick = move |folder: Option<(i64, String)>| {
        if !closing.get_value() && bookmarks::add_to(s, folder) {
            closing.set_value(true);
        }
    };
    let none_of_own = move || {
        s.bookmark_folders
            .with(|folders| folders.as_ref().is_some_and(Vec::is_empty))
    };
    view! {
        <div
            class="study-folder-picker"
            data-role="verse-bookmark-folders"
            data-loaded=move || fresh(s, read).then_some("true")
        >
            <p class="study-sheet__hint">"Uložiť do priečinka:"</p>
            <div class="study-chips">
                <button
                    type="button"
                    class="study-chip"
                    data-role="verse-bookmark-folder"
                    data-folder=""
                    on:click=move |_| pick(None)
                >
                    {DEFAULT_FOLDER_NAME}
                </button>
                <For
                    each=move || s.bookmark_folders.get().unwrap_or_default()
                    key=|folder| (folder.id, folder.name.clone())
                    children=move |folder| {
                        let target = (folder.id, folder.name.clone());
                        view! {
                            <button
                                type="button"
                                class="study-chip"
                                data-role="verse-bookmark-folder"
                                data-folder=folder.id.to_string()
                                on:click=move |_| pick(Some(target.clone()))
                            >
                                {folder.name}
                            </button>
                        }
                    }
                />
            </div>
            <Show when=none_of_own>
                <p class="study-sheet__hint" data-role="verse-bookmark-folders-none">
                    "Vlastný priečinok vytvoríš v Záložkách („＋ Nový priečinok“)."
                </p>
            </Show>
        </div>
    }
}
