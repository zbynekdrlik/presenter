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

#[test]
fn normalize_drops_untouched_verses_and_refuses_bad_indices() {
    let heb = source(&[(1, HEB_11_1)]);
    assert_eq!(
        normalize_edits(&heb, &edits(&[(1, &[], &[])])).expect("valid"),
        ItemEdits::default()
    );
    assert_eq!(
        normalize_edits(&heb, &edits(&[(1, &[12], &[])])),
        Err(EditError::IndexOutOfRange {
            verse: 1,
            index: 12
        })
    );
    assert_eq!(
        normalize_edits(&heb, &edits(&[(2, &[0], &[])])),
        Err(EditError::UnknownVerse { verse: 2 })
    );
    assert_eq!(
        EditError::UnknownVerse { verse: 2 }.to_string(),
        "Verš 2 nie je v tejto položke"
    );
}

#[test]
fn refresh_keeps_only_the_edits_of_unchanged_verses() {
    let old = source(&[(1, "a b"), (2, "c d"), (3, "e f")]);
    let new = source(&[(1, "a b"), (2, "c x")]);
    let kept = retain_unchanged_edits(
        &old,
        &new,
        &edits(&[(1, &[0], &[]), (2, &[1], &[]), (3, &[0], &[])]),
    );
    assert_eq!(kept, edits(&[(1, &[0], &[])]));
}

fn texts(item: &RenderedItem) -> Vec<(u16, &str)> {
    item.verses
        .iter()
        .map(|verse| (verse.verse, verse.text.as_str()))
        .collect()
}

#[test]
fn the_mockup_example_cuts_to_1a_with_emphasis() {
    let item = render_item(
        &source(&[(1, HEB_11_1)]),
        &edits(&[(1, &[7, 8, 9, 10, 11], &[0])]),
    );
    assert_eq!(texts(&item), [(1, "VIERA je zárukou toho, v čo dúfame")]);
    assert_eq!(item.suffix, "1a");
    assert_eq!(
        item_label("Hebrejom", 11, &item.suffix, "slk-seb"),
        "Hebrejom 11:1a (SEB)"
    );
    assert_eq!(
        item.numbered_text(),
        "1. VIERA je zárukou toho, v čo dúfame"
    );
}

#[test]
fn a_start_cut_is_b_in_the_original_case() {
    let item = render_item(&source(&[(1, HEB_11_1)]), &edits(&[(1, &[0, 1], &[])]));
    assert_eq!(
        texts(&item),
        [(
            1,
            "zárukou toho, v čo dúfame, a zdôvodnením toho, čo nevidíme,"
        )]
    );
    assert_eq!(item.suffix, "1b");
}

#[test]
fn a_range_cut_at_both_edges_is_b_to_a() {
    let item = render_item(
        &source(&[
            (16, "Prvý verš má päť slov."),
            (17, "Druhý verš ostáva celý."),
            (18, "Tretí verš končí čiarkou, a pokračuje"),
        ]),
        &edits(&[(16, &[0], &[]), (18, &[4, 5], &[])]),
    );
    assert_eq!(
        texts(&item),
        [
            (16, "verš má päť slov."),
            (17, "Druhý verš ostáva celý."),
            (18, "Tretí verš končí čiarkou"),
        ]
    );
    assert_eq!(item.suffix, "16b-18a");
}

#[test]
fn a_gap_inside_a_verse_reads_as_an_ellipsis() {
    let item = render_item(&source(&[(1, HEB_11_1)]), &edits(&[(1, &[3, 4, 5], &[])]));
    assert_eq!(
        texts(&item),
        [(
            1,
            "Viera je zárukou … dúfame, a zdôvodnením toho, čo nevidíme,"
        )]
    );
    assert_eq!(item.suffix, "1");
}

#[test]
fn one_verse_cut_at_both_ends_has_ellipses_and_no_letter() {
    let item = render_item(&source(&[(1, HEB_11_1)]), &edits(&[(1, &[0, 1, 11], &[])]));
    assert_eq!(
        texts(&item),
        [(1, "… zárukou toho, v čo dúfame, a zdôvodnením toho, čo …")]
    );
    assert_eq!(item.suffix, "1");
}

#[test]
fn inner_edges_get_ellipses_but_never_after_a_full_stop() {
    let item = render_item(
        &source(&[
            (16, "Prvý verš končí bodkou. A pokračuje"),
            (17, "Stredný verš má na začiatku aj na konci niečo navyše,"),
            (18, "Posledný verš."),
        ]),
        &edits(&[(16, &[4, 5], &[]), (17, &[0, 9], &[])]),
    );
    assert_eq!(
        texts(&item),
        [
            (16, "Prvý verš končí bodkou."),
            (17, "… verš má na začiatku aj na konci niečo …"),
            (18, "Posledný verš."),
        ]
    );
    assert_eq!(item.suffix, "16-18");
}

#[test]
fn a_struck_verse_splits_the_label() {
    let item = render_item(
        &source(&[
            (5, "a b,"),
            (6, "c d"),
            (7, "e f, g"),
            (8, "h i"),
            (9, "j k"),
        ]),
        &edits(&[(7, &[2], &[]), (8, &[0, 1], &[])]),
    );
    assert_eq!(
        texts(&item),
        [(5, "a b,"), (6, "c d"), (7, "e f"), (9, "j k")]
    );
    assert_eq!(item.suffix, "5-7a, 9");
}

#[test]
fn an_end_cut_drops_a_trailing_dash_or_comma_but_adds_no_period() {
    let item = render_item(
        &source(&[(
            20,
            "Zošle kliatbu na všetko, čo budeš podnikať — až kým nezahynieš",
        )]),
        &edits(&[(20, &[8, 9, 10], &[])]),
    );
    assert_eq!(
        texts(&item),
        [(20, "Zošle kliatbu na všetko, čo budeš podnikať")]
    );
    assert_eq!(item.suffix, "20a");
}

#[test]
fn an_item_with_every_token_struck_renders_nothing() {
    let item = render_item(
        &source(&[(1, "a b"), (2, "c d")]),
        &edits(&[(1, &[0, 1], &[]), (2, &[0, 1], &[])]),
    );
    assert!(item.is_empty());
    assert_eq!(item.suffix, "1-2");
}

#[test]
fn emphasis_is_unicode_upper_case_and_struck_beats_emphasized() {
    let item = render_item(
        &source(&[(14, "aby sme skrze vieru dostali prisľúbeného Ducha.")]),
        &edits(&[(14, &[], &[3, 4, 5])]),
    );
    assert_eq!(
        texts(&item),
        [(14, "aby sme skrze VIERU DOSTALI PRISĽÚBENÉHO Ducha.")]
    );
    let normalized = normalize_edits(
        &source(&[(1, HEB_11_1)]),
        &edits(&[(1, &[2, 0, 2], &[0, 4, 4])]),
    )
    .expect("valid edits");
    assert_eq!(normalized, edits(&[(1, &[0, 2], &[4])]));
}

/// ROH and SEVP write the dash as a spaced hyphen (a made-up sentence in the
/// same shape; the module text is private).
const SPACED_HYPHEN: &str = "Prišli sme domov - dvere boli otvorené";

#[test]
fn an_end_cut_drops_a_spaced_hyphen_but_never_a_hyphen_inside_a_word() {
    let run_end = render_item(
        &source(&[(1, SPACED_HYPHEN)]),
        &edits(&[(1, &[4, 5, 6], &[])]),
    );
    assert_eq!(texts(&run_end), [(1, "Prišli sme domov")]);
    assert_eq!(run_end.suffix, "1a");
    let inner = render_item(
        &source(&[(1, SPACED_HYPHEN), (2, "Vošli sme dnu.")]),
        &edits(&[(1, &[4, 5, 6], &[])]),
    );
    assert_eq!(
        texts(&inner),
        [(1, "Prišli sme domov …"), (2, "Vošli sme dnu.")]
    );
    assert_eq!(inner.suffix, "1-2");
    let suspended = render_item(
        &source(&[(1, "Stavby nad- a podzemné")]),
        &edits(&[(1, &[2, 3], &[])]),
    );
    assert_eq!(texts(&suspended), [(1, "Stavby nad-")]);
    let joined = render_item(
        &source(&[(1, "Videli Bar-Jonu pri mori")]),
        &edits(&[(1, &[2, 3], &[])]),
    );
    assert_eq!(texts(&joined), [(1, "Videli Bar-Jonu")]);
}

#[test]
fn an_end_cut_that_keeps_only_a_dash_keeps_the_dash() {
    let item = render_item(&source(&[(1, "— a b")]), &edits(&[(1, &[1, 2], &[])]));
    assert_eq!(texts(&item), [(1, "—")]);
    assert_eq!(item.suffix, "1a");
}

#[test]
fn a_sentence_end_before_a_closing_quote_or_bracket_gets_no_ellipsis() {
    let item = render_item(
        &source(&[
            (1, "Povedal: (Toto je koniec.) A potom odišiel"),
            (2, "Zvolal: „Poď sem!“ a čakal"),
            (3, "Spýtal sa: «Kto si?» a mlčal"),
            (4, "Odpísal: ‚Áno, prídem.’ a odišiel"),
            (5, "Spomenul (iné slovo) a ďalej"),
            (6, "Koniec."),
        ]),
        &edits(&[
            (1, &[4, 5, 6], &[]),
            (2, &[3, 4], &[]),
            (3, &[4, 5], &[]),
            (4, &[3, 4], &[]),
            (5, &[3, 4], &[]),
        ]),
    );
    assert_eq!(
        texts(&item),
        [
            (1, "Povedal: (Toto je koniec.)"),
            (2, "Zvolal: „Poď sem!“"),
            (3, "Spýtal sa: «Kto si?»"),
            (4, "Odpísal: ‚Áno, prídem.’"),
            (5, "Spomenul (iné slovo) …"),
            (6, "Koniec."),
        ]
    );
    assert_eq!(item.suffix, "1-6");
    let more = render_item(
        &source(&[
            (1, "Riekol: “Choď.” a išiel"),
            (2, "Dodal: 'Hneď.' a stál"),
            (3, "Zapísal [toto.] a viac"),
            (4, "Amen."),
        ]),
        &edits(&[(1, &[2, 3], &[]), (2, &[2, 3], &[]), (3, &[2, 3], &[])]),
    );
    assert_eq!(
        texts(&more),
        [
            (1, "Riekol: “Choď.”"),
            (2, "Dodal: 'Hneď.'"),
            (3, "Zapísal [toto.]"),
            (4, "Amen."),
        ]
    );
    assert_eq!(more.suffix, "1-4");
}

#[test]
fn a_verse_with_only_one_kind_of_edit_is_kept() {
    let heb = source(&[(1, HEB_11_1), (2, "Pre ňu dostali starší svedectvo.")]);

    let normalized = normalize_edits(&heb, &edits(&[(1, &[0], &[]), (2, &[], &[1])]));

    assert_eq!(normalized, Ok(edits(&[(1, &[0], &[]), (2, &[], &[1])])));
}

#[test]
fn an_item_with_a_kept_token_is_not_empty() {
    let heb = source(&[(1, HEB_11_1)]);

    let item = render_item(&heb, &ItemEdits::default());

    assert!(!item.is_empty());
}
