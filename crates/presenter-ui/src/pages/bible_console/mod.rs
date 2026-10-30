//! The verse console (#850 B2a, spec §7): one field that understands any way
//! of writing a reference (the core resolver runs on every keystroke, in the
//! browser) and the passage's slides loaded while typing. Phone and desktop
//! share it; on a phone it replaces the old Live-tab inputs, on a desktop it
//! sits above them until B2c removes them. The slides land in the existing
//! grid (`bs.slides`), whose cards show on a tap in Live mode; the preview
//! card and ZOBRAZIŤ follow in the next unit (B2a-4).

mod field;
mod results;

use std::cell::RefCell;
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::bible::resolver::{lookup, Versification};

use crate::api::bible::{self as api, BibleSlideDto};
use crate::state::bible::BibleState;
use crate::state::bible_console::{
    console_view, preview_key, resolve_request, versification, ConsoleView, PreviewTarget,
};

use field::ConsoleField;
use results::ConsoleResults;

/// The wait after the last keystroke before the preview request (spec §7.2).
const PREVIEW_DELAY_MS: u32 = 150;

/// The loaded passage. Every state carries the preview key it was started
/// for; an answer for an older key is dropped, never shown.
#[derive(Debug, Clone)]
enum Preview {
    Empty,
    Loading {
        key: String,
    },
    Ready {
        key: String,
        slides: Vec<BibleSlideDto>,
        index: usize,
        warning: Option<String>,
    },
    Failed {
        key: String,
        message: String,
    },
}

impl Preview {
    fn key(&self) -> Option<&str> {
        match self {
            Self::Empty => None,
            Self::Loading { key } | Self::Ready { key, .. } | Self::Failed { key, .. } => Some(key),
        }
    }

    /// The slide ZOBRAZIŤ would send, while the preview is ready.
    fn slide(&self) -> Option<&BibleSlideDto> {
        match self {
            Self::Ready { slides, index, .. } => slides.get(*index),
            _ => None,
        }
    }

    fn state(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Loading { .. } => "loading",
            Self::Ready { .. } => "ready",
            Self::Failed { .. } => "error",
        }
    }
}

/// A row or chip the operator tapped: valid for the input and the main
/// translation it was tapped in, never after either changed.
#[derive(Debug, Clone, PartialEq)]
struct Picked {
    input: String,
    main: Option<String>,
    target: PreviewTarget,
}

/// The console's signals; `Copy`, so every child takes it by value.
#[derive(Clone, Copy)]
struct Console {
    input: RwSignal<String>,
    field: NodeRef<leptos::html::Input>,
    picked: RwSignal<Option<Picked>>,
    view: Memo<ConsoleView>,
    target: Memo<Option<PreviewTarget>>,
    /// `preview_key` of `target` in the current translations; `None` = nothing
    /// to preview.
    target_key: Memo<Option<String>>,
    preview: RwSignal<Preview>,
    /// The preview shows the current target (the E2E settle signal).
    ready: Memo<bool>,
    /// Bumped by Enter: a failed preview is requested again.
    retry: RwSignal<u32>,
    /// The main translation the books (and so the rows) come from.
    main: Memo<Option<String>>,
}

impl Console {
    fn new(bs: &BibleState) -> Self {
        let books = bs.books;
        let books_translation = bs.books_translation;
        let secondary = bs.secondary_translation;
        let limit = bs.character_limit;
        let input = RwSignal::new(String::new());
        let picked = RwSignal::new(None::<Picked>);
        let structure: Memo<Versification> = Memo::new(move |_| books.with(|b| versification(b)));
        let main = Memo::new(move |_| books_translation.get());
        let view = Memo::new(move |_| {
            let translation = main.get().unwrap_or_default();
            input.with(|text| structure.with(|s| console_view(&lookup(text, s), &translation)))
        });
        let target = Memo::new(move |_| match picked.get() {
            Some(p) if input.with(|text| *text == p.input) && main.get() == p.main => {
                Some(p.target)
            }
            _ => view.with(|v| v.default_target().cloned()),
        });
        let target_key = Memo::new(move |_| {
            let target = target.get()?;
            let main = main.get()?;
            Some(preview_key(
                &target,
                &main,
                secondary.get().as_deref(),
                limit.get(),
            ))
        });
        let preview = RwSignal::new(Preview::Empty);
        let ready = Memo::new(move |_| {
            let key = target_key.get();
            preview.with(|p| matches!(p, Preview::Ready { .. }) && p.key() == key.as_deref())
        });
        Self {
            input,
            field: NodeRef::new(),
            picked,
            view,
            target,
            target_key,
            preview,
            ready,
            retry: RwSignal::new(0),
            main,
        }
    }
}

#[component]
pub fn BibleConsole() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let console = Console::new(&bs);
    setup_auto_preview(console, &bs);
    let c = console;

    view! {
        <section
            class="console"
            data-role="bible-console"
            data-preview-state=move || c.preview.with(Preview::state)
            data-preview-ready=move || if c.ready.get() { "true" } else { "false" }
            data-preview-reference=move || {
                c.preview.with(|p| p.slide().map(|s| s.bible_main_reference.clone()))
            }
        >
            <ConsoleField console=console />
            <ConsoleResults console=console />
        </section>
    }
}

/// Request the current target 150 ms after it last changed; a newer target
/// cancels the timer and an answer for an older key is dropped (a slow church
/// Wi-Fi must never put a stale verse in front of the operator). The slides go
/// to the grid too, so the desktop column and the phone list stay in step.
fn setup_auto_preview(c: Console, bs: &BibleState) {
    let secondary = bs.secondary_translation;
    let limit = bs.character_limit;
    let grid = bs.slides;
    let grid_selection = bs.selected_slide_ids;
    let pending: Rc<RefCell<Option<Timeout>>> = Rc::new(RefCell::new(None));
    Effect::new(move |_| {
        let key = c.target_key.get();
        c.retry.track();
        let Some(key) = key else {
            pending.borrow_mut().take();
            c.preview.set(Preview::Empty);
            return;
        };
        let current = c.preview.with_untracked(|p| {
            !matches!(p, Preview::Failed { .. }) && p.key() == Some(key.as_str())
        });
        if current {
            return;
        }
        let (Some(target), Some(main)) = (c.target.get_untracked(), c.main.get_untracked()) else {
            return;
        };
        let request = resolve_request(
            &target,
            &main,
            secondary.get_untracked().as_deref(),
            limit.get_untracked(),
        );
        c.preview.set(Preview::Loading { key: key.clone() });
        let timer = Timeout::new(PREVIEW_DELAY_MS, move || {
            leptos::task::spawn_local(async move {
                let answer = api::resolve_slides(&request).await;
                if c.target_key.get_untracked().as_deref() != Some(key.as_str()) {
                    return;
                }
                c.preview.set(match answer {
                    Ok(resp) if !resp.slides.is_empty() => {
                        grid.set(resp.slides.clone());
                        grid_selection.set(Default::default());
                        Preview::Ready {
                            key,
                            slides: resp.slides,
                            index: 0,
                            warning: resp.warning,
                        }
                    }
                    Ok(_) => Preview::Failed {
                        key,
                        message: "Pre tento odkaz preklad nemá text".to_string(),
                    },
                    Err(e) => Preview::Failed {
                        key,
                        message: format!("Náhľad sa nenačítal: {e}"),
                    },
                });
            });
        });
        *pending.borrow_mut() = Some(timer);
    });
}
