//! Stepping in the verse console (#850 B2b, spec §7.2): ◀ / ▶ move the
//! preview, ZOBRAZIŤ then shows it. Inside a typed passage of several
//! segments (`mt 5:3.5.7`, a cross-chapter range) a step walks the segments;
//! past them it is the verse before / after, across chapter bounds, never
//! into another book. Pure — host-tested.

use presenter_core::bible::resolver::{segment_label, BookRef, Segment, Step, Versification};

use super::bible_console::PreviewTarget;

/// The target one step from `current`: the neighbouring segment of the walked
/// passage `walk` if `current` is in it and has one, else the neighbouring
/// verse in the main translation, `None` at the end of the book.
pub fn step_target(
    current: &PreviewTarget,
    walk: &[PreviewTarget],
    structure: &Versification,
    step: Step,
) -> Option<PreviewTarget> {
    // By code and segment: a translation switch renames the book, not the walk.
    let walked = |target: &PreviewTarget| {
        target.book.code == current.book.code && target.segment == current.segment
    };
    if let Some(position) = walk.iter().position(walked) {
        let neighbour = match step {
            Step::Next => walk.get(position + 1),
            Step::Prev => position.checked_sub(1).and_then(|p| walk.get(p)),
        };
        // A verse this translation lacks is stepped over.
        if let Some(target) = neighbour.and_then(|target| rebase(target, structure)) {
            return Some(target);
        }
    }
    let segment = current.segment;
    let verse = match step {
        Step::Next => match segment.verse_end {
            Some(end) => end,
            None => structure
                .book(&current.book.code)?
                .verse_count(segment.chapter)?,
        },
        Step::Prev => segment.verse_start.unwrap_or(1),
    };
    let (code, chapter, verse) =
        structure.step(&current.book.code, segment.chapter, verse, step)?;
    (code == current.book.code).then(|| {
        let segment = Segment::verses(chapter, verse, verse);
        PreviewTarget {
            book: current.book.clone(),
            label: segment_label(&current.book.name, &segment),
            segment,
        }
    })
}

/// `target` in the translation of `structure`: the same passage under that
/// translation's book name, `None` when the translation does not have it.
pub fn rebase(target: &PreviewTarget, structure: &Versification) -> Option<PreviewTarget> {
    let book = structure.book(&target.book.code)?;
    let segment = target.segment;
    let verses = book.verse_count(segment.chapter)?;
    if segment.verse_end.or(segment.verse_start).unwrap_or(0) > verses {
        return None;
    }
    Some(PreviewTarget {
        book: BookRef {
            name: book.name.clone(),
            ..target.book.clone()
        },
        label: segment_label(&book.name, &segment),
        segment,
    })
}

/// The target without its book, for the ◀ / ▶ / ĎALŠÍ labels: `11:2`,
/// `5:3-12`, `13`.
pub fn short_label(target: &PreviewTarget) -> String {
    let full = segment_label("", &target.segment);
    full.trim_start().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::bible::BibleBookDto;
    use crate::state::bible_console::{console_view, versification, RowTarget};
    use presenter_core::bible::resolver::lookup;

    fn seb() -> Versification {
        let books: Vec<BibleBookDto> = serde_json::from_str(include_str!(
            "../../../presenter-core/src/bible/resolver/testdata/structure-slk-seb.json"
        ))
        .expect("the SEB structure fixture");
        versification(&books)
    }

    /// The targets of every row the console shows for `input`.
    fn walk(input: &str, structure: &Versification) -> Vec<PreviewTarget> {
        console_view(&lookup(input, structure), "slk-seb")
            .rows
            .into_iter()
            .filter_map(|row| match row.target {
                RowTarget::Preview(target) => Some(target),
                RowTarget::Complete(_) => None,
            })
            .collect()
    }

    fn labels(start: &str, steps: usize, step: Step) -> Vec<String> {
        let structure = seb();
        let walked = walk(start, &structure);
        let mut current = walked.first().cloned();
        let mut out = Vec::new();
        for _ in 0..steps {
            current = current.and_then(|t| step_target(&t, &walked, &structure, step));
            out.push(
                current
                    .as_ref()
                    .map_or("-".to_string(), |t| t.label.clone()),
            );
        }
        out
    }

    #[test]
    fn next_is_the_next_verse_across_a_chapter_end() {
        assert_eq!(
            labels("zid 11 1", 2, Step::Next),
            ["Hebrejom 11:2", "Hebrejom 11:3"]
        );
        assert_eq!(labels("zid 10 39", 1, Step::Next), ["Hebrejom 11:1"]);
        assert_eq!(labels("zid 11 1", 1, Step::Prev), ["Hebrejom 10:39"]);
    }

    #[test]
    fn a_verse_list_walks_its_verses_first() {
        assert_eq!(
            labels("mt 5:3.5.7", 3, Step::Next),
            ["Matúš 5:5", "Matúš 5:7", "Matúš 5:8"]
        );
    }

    #[test]
    fn a_range_steps_from_its_ends_and_a_chapter_from_its_last_verse() {
        assert_eq!(labels("mt 5,3-12", 1, Step::Next), ["Matúš 5:13"]);
        assert_eq!(labels("mt 5,3-12", 1, Step::Prev), ["Matúš 5:2"]);
        assert_eq!(labels("zid 12", 1, Step::Next), ["Hebrejom 13:1"]);
    }

    #[test]
    fn a_step_never_leaves_the_book() {
        assert_eq!(labels("zid 13 25", 1, Step::Next), ["-"]);
        assert_eq!(labels("zid 1 1", 1, Step::Prev), ["-"]);
    }

    fn renamed(target: &PreviewTarget, name: &str) -> PreviewTarget {
        let mut other = target.clone();
        other.book.name = name.to_string();
        other
    }

    #[test]
    fn rebase_renames_the_book_and_keeps_the_passage() {
        let structure = seb();
        let target = renamed(&walk("zid 11 2", &structure)[0], "Židom");
        let rebased = rebase(&target, &structure).expect("Hebrews 11:2 exists");
        assert_eq!(rebased.label, "Hebrejom 11:2");
        assert_eq!(rebased.book.name, "Hebrejom");
        assert_eq!(rebased.segment, target.segment);
    }

    #[test]
    fn rebase_refuses_what_the_translation_does_not_have() {
        let structure = seb();
        let mut target = walk("zid 13 25", &structure)[0].clone();
        target.segment = Segment::verses(13, 26, 26);
        assert_eq!(rebase(&target, &structure), None);
        target.segment = Segment::verses(14, 1, 1);
        assert_eq!(rebase(&target, &structure), None);
        target.book.code = "NOPE".to_string();
        assert_eq!(rebase(&target, &structure), None);
    }

    #[test]
    fn a_walk_goes_on_under_another_book_name() {
        let structure = seb();
        let walked: Vec<PreviewTarget> = walk("mt 5:3.5", &structure);
        let current = renamed(&walked[0], "Matthew");
        let next =
            step_target(&current, &walked, &structure, Step::Next).expect("the second verse");
        assert_eq!(next.label, "Matúš 5:5");
    }

    #[test]
    fn a_walked_verse_the_translation_lacks_is_stepped_over() {
        let structure = seb();
        let mut walked = walk("zid 13 24", &structure);
        let mut missing = walked[0].clone();
        missing.segment = Segment::verses(13, 26, 26);
        walked.push(missing);
        let next = step_target(&walked[0], &walked, &structure, Step::Next);
        assert_eq!(next.map(|t| t.label), Some("Hebrejom 13:25".to_string()));
    }

    #[test]
    fn short_labels_drop_the_book() {
        let structure = seb();
        let short: Vec<String> = ["zid 11 2", "mt 5,3-12", "zid 13"]
            .iter()
            .map(|input| short_label(&walk(input, &structure)[0]))
            .collect();
        assert_eq!(short, ["11:2", "5:3-12", "13"]);
    }
}
