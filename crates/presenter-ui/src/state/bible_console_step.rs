//! Stepping in the verse console (#850 B2b, spec §7.2): ◀ / ▶ move the
//! preview, ZOBRAZIŤ then shows it. Inside a typed passage of several
//! segments (`mt 5:3.5.7`, a cross-chapter range) a step walks the segments;
//! past them it is the verse before / after, across chapter bounds, never
//! into another book. Pure — host-tested.

use presenter_core::bible::resolver::{segment_label, Segment, Step, Versification};

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
    if let Some(position) = walk.iter().position(|target| target == current) {
        let neighbour = match step {
            Step::Next => walk.get(position + 1),
            Step::Prev => position.checked_sub(1).and_then(|p| walk.get(p)),
        };
        if let Some(target) = neighbour {
            return Some(target.clone());
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
