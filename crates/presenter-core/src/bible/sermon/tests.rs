//! #851 P1: the sermon core rules (spec §7.2–§7.3, §14 "presenter-core").

use super::*;
use chrono::NaiveDate;

/// SEB Heb 11:1, the mockup's example (12 tokens).
const HEB_11_1: &str = "Viera je zárukou toho, v čo dúfame, a zdôvodnením toho, čo nevidíme,";

fn source(verses: &[(u16, &str)]) -> Vec<SourceVerse> {
    verses
        .iter()
        .map(|(verse, text)| SourceVerse {
            verse: *verse,
            text: (*text).to_string(),
        })
        .collect()
}

fn edits(entries: &[(u16, &[u16], &[u16])]) -> ItemEdits {
    ItemEdits(
        entries
            .iter()
            .map(|(verse, deleted, emphasized)| {
                let verse_edits = VerseEdits {
                    deleted: deleted.to_vec(),
                    emphasized: emphasized.to_vec(),
                };
                (*verse, verse_edits)
            })
            .collect(),
    )
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
}

#[test]
fn tokens_keep_punctuation_and_diacritics() {
    let tokens = token_texts(HEB_11_1);
    assert_eq!(tokens.len(), 12);
    assert_eq!(&tokens[..4], ["Viera", "je", "zárukou", "toho,"]);
    assert_eq!(tokens[11], "nevidíme,");
    assert_eq!(tokenize(HEB_11_1)[2], Token { start: 9, end: 17 });
    assert_eq!(token_texts("  a\u{a0}b  c ").join(" "), "a b c");
}

#[test]
fn short_codes_and_labels() {
    assert_eq!(translation_short_code("slk-seb"), "SEB");
    assert_eq!(translation_short_code("eng-kjv"), "KJV");
    assert_eq!(translation_short_code("kjv"), "KJV");
    assert_eq!(
        item_label("Deuteronómium", 28, "20a", "slk-seb"),
        "Deuteronómium 28:20a (SEB)"
    );
}

#[test]
fn suffix_joins_runs_and_marks_only_their_edges() {
    let plain: Vec<VerseMark> = [5, 6, 7, 9].into_iter().map(VerseMark::plain).collect();
    assert_eq!(verse_suffix(&plain), "5-7, 9");
    let edges = [
        VerseMark {
            verse: 16,
            start_cut: true,
            end_cut: false,
        },
        VerseMark::plain(17),
        VerseMark {
            verse: 18,
            start_cut: false,
            end_cut: true,
        },
    ];
    assert_eq!(verse_suffix(&edges), "16b-18a");
    let single = [VerseMark {
        verse: 1,
        start_cut: false,
        end_cut: true,
    }];
    assert_eq!(verse_suffix(&single), "1a");
    let both = [VerseMark {
        verse: 1,
        start_cut: true,
        end_cut: true,
    }];
    assert_eq!(verse_suffix(&both), "1");
    assert_eq!(verse_suffix(&[]), "");
}

#[test]
fn default_date_is_the_next_sunday_or_today_on_a_sunday() {
    assert_eq!(default_service_date(date("2026-10-14")), date("2026-10-18"));
    assert_eq!(default_service_date(date("2026-10-18")), date("2026-10-18"));
    assert_eq!(default_service_date(date("2026-10-17")), date("2026-10-18"));
    assert_eq!(default_service_date(date("2026-10-19")), date("2026-10-25"));
    assert_eq!(
        default_sermon_title(date("2026-10-19")),
        "Kázeň 19. 10. 2026"
    );
}

#[test]
fn json_shapes_match_the_stored_columns() {
    let stored = edits(&[(1, &[7, 8], &[0])]);
    let json = serde_json::to_string(&stored).expect("serialize");
    assert_eq!(json, r#"{"1":{"deleted":[7,8],"emphasized":[0]}}"#);
    assert_eq!(
        serde_json::from_str::<ItemEdits>(&json).expect("parse"),
        stored
    );
    let partial: ItemEdits = serde_json::from_str(r#"{"3":{"deleted":[1]}}"#).expect("parse");
    assert_eq!(partial, edits(&[(3, &[1], &[])]));
    let verse = serde_json::to_string(&source(&[(1, "Viera")])).expect("serialize");
    assert_eq!(verse, r#"[{"verse":1,"text":"Viera"}]"#);
}
