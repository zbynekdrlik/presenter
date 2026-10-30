//! Result rows, the hint and the preview status (spec §7.1 item 3, §5.7). A
//! tap on a row previews it; a book-only row completes the field. Nothing here
//! sends to the wall.

use leptos::prelude::*;

use super::{Console, Picked, Preview};
use crate::state::bible_console::{ConsoleRow, RowTarget};

#[component]
pub(super) fn ConsoleResults(console: Console) -> impl IntoView {
    let c = console;
    let rows = move || c.view.with(|v| v.rows.clone());
    let hint = move || c.view.with(|v| v.hint.clone());
    let status = move || {
        c.preview.with(|p| match p {
            Preview::Loading { .. } => Some(("console-status", "Načítavam\u{2026}".to_string())),
            Preview::Failed { message, .. } => Some(("console-preview-error", message.clone())),
            Preview::Ready {
                warning: Some(text),
                ..
            } => Some(("console-preview-warning", text.clone())),
            _ => None,
        })
    };

    view! {
        <div class="console-results" data-role="console-results">
            <For
                each=rows
                key=|row| row.key.clone()
                children=move |row| view! { <ResultButton console=c row=row role="console-row" /> }
            />
            {move || {
                hint()
                    .map(|text| {
                        view! {
                            <p class="console-hint" data-role="console-hint" role="status">
                                {text}
                            </p>
                        }
                    })
            }}
            {move || {
                status()
                    .map(|(role, text)| {
                        view! {
                            <p class="console-status" data-role=role role="status">
                                {text}
                            </p>
                        }
                    })
            }}
        </div>
    }
}

#[component]
fn ResultButton(console: Console, row: ConsoleRow, role: &'static str) -> impl IntoView {
    let c = console;
    let (book_code, chapter, verse_start, verse_end) = match &row.target {
        RowTarget::Preview(target) => (
            target.book.code.clone(),
            target.segment.chapter.to_string(),
            target
                .segment
                .verse_start
                .map(|v| v.to_string())
                .unwrap_or_default(),
            target
                .segment
                .verse_end
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ),
        RowTarget::Complete(_) => Default::default(),
    };
    let preview_of = match &row.target {
        RowTarget::Preview(target) => Some(target.clone()),
        RowTarget::Complete(_) => None,
    };
    let selected = Memo::new(move |_| {
        preview_of
            .as_ref()
            .is_some_and(|mine| c.target.with(|current| current.as_ref() == Some(mine)))
    });
    let target = row.target.clone();
    let on_tap = move |_| match &target {
        RowTarget::Preview(preview) => c.picked.set(Some(Picked {
            input: c.input.get_untracked(),
            main: c.main.get_untracked(),
            target: preview.clone(),
        })),
        RowTarget::Complete(text) => {
            c.input.set(text.clone());
            if let Some(field) = c.field.get_untracked() {
                let _ = field.focus();
            }
        }
    };
    let class = if role == "console-row" {
        "console-row"
    } else {
        "console-chip"
    };

    view! {
        <button
            type="button"
            class=class
            data-role=role
            data-kind=row.kind
            data-key=row.key.clone()
            data-book-code=book_code
            data-chapter=chapter
            data-verse-start=verse_start
            data-verse-end=verse_end
            data-default=if row.default { "true" } else { "false" }
            aria-pressed=move || if selected.get() { "true" } else { "false" }
            on:click=on_tap
        >
            <span class="console-row__label">{row.label.clone()}</span>
            {(!row.detail.is_empty())
                .then(|| view! { <small class="console-row__detail">{row.detail.clone()}</small> })}
        </button>
    }
}
