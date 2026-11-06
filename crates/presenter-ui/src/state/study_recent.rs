//! The study field's earlier searches (owner, 2026-11-06: "šípku dole aby som
//! si mohol predplniť niečo z histórie čo som už hľadal"). What was typed and
//! opened or searched is kept per browser, newest first; the ▾ list fills the
//! field with one of them and Enter opens it, as if typed again.

/// How many earlier searches the list keeps.
pub const RECENT_LIMIT: usize = 12;

/// The browser's storage key (`session::get_persistent`).
pub const RECENT_KEY: &str = "studyRecent";

/// The list after `text` was used: trimmed, an empty text changes nothing,
/// the same text in another case or spacing moves to the top instead of
/// appearing twice, and the oldest go past [`RECENT_LIMIT`].
pub fn remember(list: &[String], text: &str) -> Vec<String> {
    let entry = collapse(text);
    if entry.is_empty() {
        return list.to_vec();
    }
    let key = entry.to_lowercase();
    std::iter::once(entry)
        .chain(
            list.iter()
                .filter(|kept| collapse(kept).to_lowercase() != key)
                .cloned(),
        )
        .take(RECENT_LIMIT)
        .collect()
}

/// The stored list: a JSON array of strings; anything else is an empty list.
pub fn parse(stored: Option<&str>) -> Vec<String> {
    stored
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .map(|list| {
            list.into_iter()
                .map(|text| collapse(&text))
                .filter(|text| !text.is_empty())
                .take(RECENT_LIMIT)
                .collect()
        })
        .unwrap_or_default()
}

/// The list as stored.
pub fn serialize(list: &[String]) -> String {
    serde_json::to_string(list).unwrap_or_else(|_| "[]".to_string())
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn a_stored_list_with_a_repeat_keeps_the_first() {
        let parsed = parse(Some(r#"["ján 3","2 kor 5","JÁN  3"]"#));
        assert_eq!(parsed, list(&["ján 3", "2 kor 5"]));
    }

    #[test]
    fn the_newest_search_comes_first() {
        let after = remember(&list(&["ján 3"]), "2 kor 5");
        assert_eq!(after, list(&["2 kor 5", "ján 3"]));
    }

    #[test]
    fn the_same_search_moves_to_the_top_instead_of_repeating() {
        let after = remember(&list(&["ján 3", "2 kor 5"]), "  2  KOR 5 ");
        assert_eq!(after, list(&["2 KOR 5", "ján 3"]));
    }

    #[test]
    fn an_empty_text_changes_nothing() {
        assert_eq!(remember(&list(&["ján 3"]), "   "), list(&["ján 3"]));
    }

    #[test]
    fn the_oldest_go_past_the_limit() {
        let mut kept = Vec::new();
        for number in 0..20 {
            kept = remember(&kept, &format!("žalm {number}"));
        }
        assert_eq!(kept.len(), RECENT_LIMIT);
        assert_eq!(kept[0], "žalm 19");
        assert_eq!(kept[RECENT_LIMIT - 1], "žalm 8");
    }

    #[test]
    fn a_stored_list_round_trips_and_garbage_is_empty() {
        let stored = serialize(&list(&["ján 3", "viera"]));
        assert_eq!(parse(Some(&stored)), list(&["ján 3", "viera"]));
        assert_eq!(parse(Some("not json")), Vec::<String>::new());
        assert_eq!(parse(Some("[1, 2]")), Vec::<String>::new());
        assert_eq!(parse(None), Vec::<String>::new());
        assert_eq!(parse(Some("[\"  \", \"a  b\"]")), list(&["a b"]));
    }
}
