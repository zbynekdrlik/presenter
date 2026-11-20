use super::*;
use serde_json::json;

/// The `context` object as the Soniox config writes it (serde_json, compact).
fn wire_json(context: &ContextTerms) -> String {
    let general: Vec<_> = context
        .general
        .iter()
        .map(|(key, value)| json!({"key": key, "value": value}))
        .collect();
    json!({"general": general, "terms": context.terms}).to_string()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

#[test]
fn all_66_books_are_terms() {
    let context = context_terms(&ContextInputs::default());
    assert_eq!(&context.terms[..66], BOOK_LONG_EN.map(str::to_string));
    for book in [
        "Mark",
        "John",
        "James",
        "Job",
        "Ruth",
        "Song of Songs",
        "1 Thessalonians",
    ] {
        assert!(context.terms.iter().any(|t| t == book), "{book}");
    }
    assert_eq!(
        context.general,
        vec![
            ("domain".to_string(), "Christian church sermon".to_string()),
            (
                "topic".to_string(),
                "preacher reads and cites Bible books, chapters and verses".to_string()
            ),
        ]
    );
    assert!(
        CHRISTIAN_TERMS.len() <= 60,
        "the curated list stays reviewed and short"
    );
    assert!(context.terms.iter().any(|t| t == "Holy Spirit"));
}

#[test]
fn prepared_references_follow_the_books() {
    let inputs = ContextInputs {
        prepared_refs: strings(&["Romans 8:37-39", "John 3:16"]),
        tip_terms: strings(&["Kerygma"]),
    };
    let context = context_terms(&inputs);
    assert_eq!(context.terms[66], "Romans 8:37-39");
    assert_eq!(context.terms[67], "John 3:16");
    assert_eq!(
        context.terms[68], "Kerygma",
        "tip terms after the references"
    );
    assert_eq!(context.terms[69], CHRISTIAN_TERMS[0], "curated last");
}

#[test]
fn duplicates_are_dropped_case_insensitively() {
    let inputs = ContextInputs {
        prepared_refs: strings(&["romans", "  John 3:16 ", "john  3:16", ""]),
        tip_terms: strings(&["holy \t spirit", "   ", "Agape"]),
    };
    let context = context_terms(&inputs);
    let count = |needle: &str| {
        context
            .terms
            .iter()
            .filter(|t| t.to_lowercase() == needle)
            .count()
    };
    assert_eq!(count("romans"), 1, "a book named again stays once");
    assert_eq!(count("john 3:16"), 1);
    assert_eq!(count("holy spirit"), 1);
    assert!(
        context.terms.contains(&"John 3:16".to_string()),
        "trimmed, first spelling kept"
    );
    assert!(
        context.terms.contains(&"holy spirit".to_string()),
        "the earlier group's spelling wins, inner whitespace collapsed"
    );
    assert!(!context.terms.contains(&"Holy Spirit".to_string()));
    assert!(context
        .terms
        .iter()
        .all(|t| !t.trim().is_empty() && t.trim() == t));
    assert_eq!(context.terms.len(), 66 + 1 + 2 + CHRISTIAN_TERMS.len() - 1);
}

#[test]
fn the_cap_drops_curated_terms_first_then_tip_terms() {
    let small = context_terms(&ContextInputs::default());
    assert_eq!(
        context_json_chars(&small.general, &small.terms),
        wire_json(&small).chars().count(),
        "the size counts exactly what the wire carries"
    );
    let tips: Vec<String> = (0..400).map(|n| format!("tip \"term\" {n:03}")).collect();
    let inputs = ContextInputs {
        prepared_refs: strings(&["Romans 8:37"]),
        tip_terms: tips.clone(),
    };
    let context = context_terms(&inputs);
    let size = wire_json(&context).chars().count();
    assert!(size <= CONTEXT_MAX_CHARS, "{size}");
    assert_eq!(context_json_chars(&context.general, &context.terms), size);
    assert!(
        CHRISTIAN_TERMS
            .iter()
            .all(|c| !context.terms.contains(&c.to_string())),
        "every curated term went before any tip term"
    );
    let kept_tips = context.terms.len() - 67;
    assert!(kept_tips > 0 && kept_tips < tips.len());
    assert_eq!(
        &context.terms[67..],
        &tips[..kept_tips],
        "tips dropped from the end"
    );
    assert_eq!(&context.terms[..66], BOOK_LONG_EN.map(str::to_string));
    assert_eq!(context.terms[66], "Romans 8:37");
    let one_more = [context.terms.clone(), vec![tips[kept_tips].clone()]].concat();
    assert!(
        context_json_chars(&context.general, &one_more) > CONTEXT_MAX_CHARS,
        "only as many terms as needed were dropped"
    );
}

#[test]
fn reference_term_formats_ranges_with_english_names() {
    assert_eq!(reference_term(44, 8, Some(37), Some(39)), "Romans 8:37-39");
    assert_eq!(reference_term(44, 8, Some(37), None), "Romans 8:37");
    assert_eq!(reference_term(44, 8, Some(37), Some(37)), "Romans 8:37");
    assert_eq!(reference_term(44, 8, Some(37), Some(30)), "Romans 8:37");
    assert_eq!(reference_term(44, 8, None, Some(39)), "Romans 8");
    assert_eq!(reference_term(0, 1, Some(1), None), "Genesis 1:1");
    assert_eq!(reference_term(18, 23, Some(1), None), "Psalm 23:1");
    assert_eq!(reference_term(45, 13, None, None), "1 Corinthians 13");
    assert_eq!(
        reference_term(65, 22, Some(20), Some(21)),
        "Revelation 22:20-21"
    );
    assert_eq!(
        reference_term(66, 1, None, None),
        "",
        "an unknown book gives nothing"
    );
}
