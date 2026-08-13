//! Replays a captured `create_bible_presentation` tool call through the REAL
//! production parser/packer/validator, and the verse-text fidelity checks
//! that ride on that replay's parsed items.

use crate::corpus::Case;
use presenter_server::ai::bible_validator::validate_bible_slide;
use presenter_server::ai::tools::bible_presentation::parse_bible_items;
use presenter_server::ai::ChatMessage;
use presenter_server::state::slides::{compose_bible_items_into_slides, BibleItem};
use serde_json::Value;

/// Default self-correction budget when a case's `expected.selfCorrectWithinRetries`
/// is absent — mirrors `corpus/SCHEMA.md`'s own worked example.
const DEFAULT_SELF_CORRECT_RETRIES: u32 = 3;

/// One `create_bible_presentation` tool call, replayed through the real
/// parser/packer/validator.
pub struct BiblePresentationAttempt {
    /// `None` = the replay found no parse/validation error (a clean
    /// attempt); `Some(rule)` = the exact error/rule key the REAL code
    /// produced (`missing_items`, `invalid_verse_item`, `invalid_item_kind`,
    /// `invalid_emphasis_item`, or one of `ValidationRule::as_str()`'s five
    /// values).
    pub rule: Option<String>,
    /// The parsed verse/emphasis items — present whenever item-PARSING
    /// itself succeeded, even if a later packer/validator check on `rule`
    /// failed. Verse-fidelity checks only look at items from attempts
    /// where `rule.is_none()` (an attempt that succeeded end-to-end).
    pub items: Vec<BibleItem>,
}

/// Find every `create_bible_presentation` call in `turn` (in order) and
/// replay each one's raw `arguments` JSON through the real
/// `parse_bible_items` → `compose_bible_items_into_slides` →
/// `validate_bible_slide` chain — exactly what
/// `ai::tools::bible_presentation::create_bible_presentation` itself does,
/// minus persistence (which needs `AppState` and is irrelevant to
/// structural scoring).
pub fn collect_bible_presentation_attempts(
    turn: &[ChatMessage],
    char_limit: u32,
) -> Vec<BiblePresentationAttempt> {
    turn.iter()
        .filter(|m| m.role == "assistant")
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .filter(|tc| tc.function.name == "create_bible_presentation")
        .map(|tc| replay_create_bible_presentation(&tc.function.arguments, char_limit))
        .collect()
}

fn replay_create_bible_presentation(
    arguments_json: &str,
    char_limit: u32,
) -> BiblePresentationAttempt {
    let args: Value = match serde_json::from_str(arguments_json) {
        Ok(v) => v,
        Err(e) => {
            return BiblePresentationAttempt {
                rule: Some(format!("unparseable_arguments: {e}")),
                items: Vec::new(),
            }
        }
    };

    // Mirrors `bible_presentation.rs::create_bible_presentation`'s own
    // items-array-presence check — trivial `Value` field extraction, not a
    // validation RULE, so this one line is not "parallel re-implementation"
    // in the sense the ticket bans; every rule/error KEY beyond this point
    // comes from the real `parse_bible_items`/`validate_bible_slide`.
    let Some(items_arr) = args.get("items").and_then(Value::as_array) else {
        return BiblePresentationAttempt {
            rule: Some("missing_items".to_string()),
            items: Vec::new(),
        };
    };

    let items = match parse_bible_items(items_arr) {
        Ok(Ok(items)) => items,
        Ok(Err((json_body, _preview))) => {
            return BiblePresentationAttempt {
                rule: Some(extract_error_key(&json_body)),
                items: Vec::new(),
            }
        }
        Err(e) => {
            return BiblePresentationAttempt {
                rule: Some(format!("parse_bible_items_error: {e}")),
                items: Vec::new(),
            }
        }
    };

    // Same order production checks in: compose, then validate each
    // composed slide, returning on the FIRST failure — matching
    // `create_bible_presentation`'s own loop exactly.
    let composed = compose_bible_items_into_slides(&items, char_limit);
    for slide in &composed {
        if let Err(err) = validate_bible_slide(&slide.main, &slide.main_reference, char_limit) {
            return BiblePresentationAttempt {
                rule: Some(err.rule.as_str().to_string()),
                items,
            };
        }
    }

    BiblePresentationAttempt { rule: None, items }
}

fn extract_error_key(json_body: &str) -> String {
    serde_json::from_str::<Value>(json_body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| format!("unrecognized_error_body: {json_body}"))
}

/// `expected.validationErrors` + `expected.selfCorrectWithinRetries`
/// (SCHEMA.md's Layer-1 mapping table, report §6.5 bar #2).
pub fn check_validation_errors(
    case: &Case,
    attempts: &[BiblePresentationAttempt],
    failures: &mut Vec<String>,
) {
    if case.expected.validation_errors.is_empty() {
        for (idx, a) in attempts.iter().enumerate() {
            if let Some(rule) = &a.rule {
                failures.push(format!(
                    "create_bible_presentation attempt {idx}: unexpected validation error \
                     '{rule}' (case declares no expected.validationErrors)"
                ));
            }
        }
        return;
    }

    let retries = case
        .expected
        .self_correct_within_retries
        .unwrap_or(DEFAULT_SELF_CORRECT_RETRIES) as usize;

    for expected_rule in &case.expected.validation_errors {
        let Some(first_fail_idx) = attempts
            .iter()
            .position(|a| a.rule.as_deref() == Some(expected_rule.as_str()))
        else {
            failures.push(format!(
                "expected.validationErrors: rule '{expected_rule}' never fired in any \
                 create_bible_presentation attempt"
            ));
            continue;
        };

        let recovered = attempts
            .iter()
            .skip(first_fail_idx + 1)
            .take(retries)
            .any(|a| a.rule.is_none());
        if !recovered {
            failures.push(format!(
                "expected.validationErrors: rule '{expected_rule}' fired at attempt \
                 {first_fail_idx} but no self-correction succeeded within {retries} retries"
            ));
        }
    }
}

/// Every `BibleItem` from attempts that succeeded end-to-end (`rule.is_none()`)
/// — i.e. the items that actually ended up in a persisted presentation, not
/// an intermediate mistake the model self-corrected away from.
fn successful_items(attempts: &[BiblePresentationAttempt]) -> Vec<&BibleItem> {
    attempts
        .iter()
        .filter(|a| a.rule.is_none())
        .flat_map(|a| a.items.iter())
        .collect()
}

fn find_verse<'a>(
    items: &[&'a BibleItem],
    book: &str,
    chapter: u32,
    number: u32,
    translation: &str,
) -> Option<&'a str> {
    items.iter().find_map(|item| match item {
        BibleItem::Verse {
            number: n,
            text,
            book: b,
            chapter: c,
            translation: t,
        } if *n == number
            && *c == chapter
            && b.eq_ignore_ascii_case(book)
            && t.eq_ignore_ascii_case(translation) =>
        {
            Some(text.as_str())
        }
        _ => None,
    })
}

/// Parse a `"Book Ch:V"` reference (SCHEMA.md's `verbatimVerses`/
/// `overriddenVerses` `ref` field — always a single verse in this corpus,
/// never a range) into `(book, chapter, verse_number)`. The book name may
/// itself contain spaces/periods ("1. Samuelova"), so this splits on the
/// LAST space rather than the first.
fn parse_verse_ref(reference: &str) -> Option<(String, u32, u32)> {
    let (book, chapter_verse) = reference.trim().rsplit_once(' ')?;
    let (chapter, verse) = chapter_verse.split_once(':')?;
    let chapter: u32 = chapter.trim().parse().ok()?;
    let number: u32 = verse.trim().parse().ok()?;
    Some((book.trim().to_string(), chapter, number))
}

/// `expected.verbatimVerses` — exact-string fidelity for a verse the sermon
/// quoted verbatim.
pub fn check_verbatim_verses(
    case: &Case,
    attempts: &[BiblePresentationAttempt],
    failures: &mut Vec<String>,
) {
    if case.expected.verbatim_verses.is_empty() {
        return;
    }
    let items = successful_items(attempts);
    for v in &case.expected.verbatim_verses {
        let Some((book, chapter, number)) = parse_verse_ref(&v.reference) else {
            failures.push(format!(
                "verbatimVerses: could not parse ref '{}'",
                v.reference
            ));
            continue;
        };
        match find_verse(&items, &book, chapter, number, &v.translation) {
            Some(actual) if actual == v.text => {}
            Some(actual) => failures.push(format!(
                "verbatimVerses: {} ({}) text mismatch — expected {:?}, got {:?}",
                v.reference, v.translation, v.text, actual
            )),
            None => failures.push(format!(
                "verbatimVerses: {} ({}) not found among submitted verse items",
                v.reference, v.translation
            )),
        }
    }
}

/// `expected.overriddenVerses` — the sermon's wording must win over the
/// unedited DB text (agent.rs system-prompt step 3: "the sermon is
/// authoritative for text content").
pub fn check_overridden_verses(
    case: &Case,
    attempts: &[BiblePresentationAttempt],
    failures: &mut Vec<String>,
) {
    if case.expected.overridden_verses.is_empty() {
        return;
    }
    let items = successful_items(attempts);
    for v in &case.expected.overridden_verses {
        let Some((book, chapter, number)) = parse_verse_ref(&v.reference) else {
            failures.push(format!(
                "overriddenVerses: could not parse ref '{}'",
                v.reference
            ));
            continue;
        };
        match find_verse(&items, &book, chapter, number, &v.translation) {
            Some(actual) if actual == v.expected_text => {}
            Some(actual) if actual == v.db_text => failures.push(format!(
                "overriddenVerses: {} ({}) silently reverted to the unedited DB text instead \
                 of the sermon's wording",
                v.reference, v.translation
            )),
            Some(actual) => failures.push(format!(
                "overriddenVerses: {} ({}) text mismatch — expected sermon wording {:?}, got {:?}",
                v.reference, v.translation, v.expected_text, actual
            )),
            None => failures.push(format!(
                "overriddenVerses: {} ({}) not found among submitted verse items",
                v.reference, v.translation
            )),
        }
    }
}
