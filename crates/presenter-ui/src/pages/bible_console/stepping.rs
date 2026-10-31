//! ◀ / ▶ and "after a show" (#850 B2b, spec §7.2). A step first walks the
//! slides of a split passage, then the segments of the typed passage, then
//! the verses around it (`state::bible_console_step`). ◀ / ▶ only move the
//! preview; after a show the field clears and the preview is already on the
//! next part, so the primary button reads "ĎALŠÍ: …" and one tap shows it.

use leptos::prelude::*;
use presenter_core::bible::resolver::Step;

use super::{Console, Picked, Preview};
use crate::state::bible_console::{PreviewTarget, RowTarget};
use crate::state::bible_console_step::step_target;

/// Where one step goes: another slide of the split passage, or another passage.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Move {
    Slide(usize),
    Target(PreviewTarget),
}

/// The move one `step` away from what the preview shows; `None` = the button
/// is disabled (nothing loaded, or the end of the book).
pub(super) fn neighbour(c: Console, step: Step) -> Option<Move> {
    let target = c.target.get()?;
    let key = c.target_key.get();
    let slide = c.preview.with(|p| match p {
        Preview::Ready {
            key: loaded,
            slides,
            index,
            ..
        } if key.as_deref() == Some(loaded.as_str()) => Some((*index, slides.len())),
        _ => None,
    });
    let within = match (step, slide) {
        (Step::Next, Some((index, count))) if index + 1 < count => Some(Move::Slide(index + 1)),
        (Step::Prev, Some((index, _))) if index > 0 => Some(Move::Slide(index - 1)),
        _ => None,
    };
    within.or_else(|| {
        c.walk
            .with(|walk| c.structure.with(|s| step_target(&target, walk, s, step)))
            .map(Move::Target)
    })
}

/// ◀ / ▶: move the preview; nothing goes to the wall. Not while a show is on
/// its way: it moves on from the preview when it lands, a part would be skipped.
pub(super) fn apply_move(c: Console, step: Step) {
    if c.showing.get_untracked() {
        return;
    }
    let Some(next) = untrack(|| neighbour(c, step)) else {
        return;
    };
    c.at_end.set(false);
    let (input, current) = (c.input.get_untracked(), c.target.get_untracked());
    land(c, input, current, next);
}

/// After a successful show: clear the field and preview the next part.
pub(super) fn advance_after_show(c: Console) {
    let next = untrack(|| neighbour(c, Step::Next));
    // The empty field resolves to nothing: read the target before it clears.
    let current = c.target.get_untracked();
    c.after_show.set(true);
    c.input.set(String::new());
    match next {
        Some(next) => land(c, String::new(), current, next),
        // The end of the book: the preview stays on the verse on the wall.
        None => {
            c.at_end.set(true);
            pin(c, String::new(), current);
        }
    }
}

fn land(c: Console, input: String, current: Option<PreviewTarget>, next: Move) {
    match next {
        Move::Target(target) => pin(c, input, Some(target)),
        Move::Slide(index) => {
            pin(c, input, current);
            c.preview.update(|p| {
                if let Preview::Ready { index: current, .. } = p {
                    *current = index;
                }
            });
        }
    }
}

/// Make `target` the preview for `input`, whatever the field resolves to.
fn pin(c: Console, input: String, target: Option<PreviewTarget>) {
    if let Some(target) = target {
        c.picked.set(Some(Picked {
            input,
            main: c.main.get_untracked(),
            target,
        }));
    }
}

/// Keep the typed passage's segments while the field changes; an emptied
/// field (after a show) keeps the last ones, so the walk goes on.
pub(super) fn setup_walk(c: Console) {
    Effect::new(move |_| {
        let targets: Vec<PreviewTarget> = c.view.with(|v| {
            v.rows
                .iter()
                .filter_map(|row| match &row.target {
                    RowTarget::Preview(target) => Some(target.clone()),
                    RowTarget::Complete(_) => None,
                })
                .collect()
        });
        if !targets.is_empty() {
            c.walk.set(targets);
        }
    });
}
