//! The order of a verse search's hits (#983). The operator console wants the
//! best hit first (relevance, a few rows); Study lists every hit the way the
//! Bible runs (book, chapter, verse).

/// The most hits a relevance search answers.
const RELEVANCE_LIMIT: u32 = 100;
/// The most hits a search in the Bible's order answers: every hit of a
/// common word ("moc" = 653 in the SEB) fits.
const BIBLE_ORDER_LIMIT: u32 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchOrder {
    /// The best match first (FTS5 rank).
    #[default]
    Relevance,
    /// Book, chapter, verse.
    Bible,
}

impl SearchOrder {
    /// `None` for anything that is not an order. Empty means relevance.
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() || input.eq_ignore_ascii_case("relevance") {
            return Some(Self::Relevance);
        }
        input.eq_ignore_ascii_case("bible").then_some(Self::Bible)
    }

    /// The `order=` value of `/bible/search`.
    pub fn as_param(self) -> &'static str {
        match self {
            Self::Relevance => "relevance",
            Self::Bible => "bible",
        }
    }

    /// The most hits one search answers in this order.
    pub fn max_limit(self) -> u32 {
        match self {
            Self::Relevance => RELEVANCE_LIMIT,
            Self::Bible => BIBLE_ORDER_LIMIT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_order_is_one_of_two_words() {
        assert_eq!(SearchOrder::parse(""), Some(SearchOrder::Relevance));
        assert_eq!(
            SearchOrder::parse(" Relevance "),
            Some(SearchOrder::Relevance)
        );
        assert_eq!(SearchOrder::parse("bible"), Some(SearchOrder::Bible));
        assert_eq!(SearchOrder::parse("random"), None);
    }

    #[test]
    fn the_bible_order_answers_every_hit_up_to_1000() {
        assert_eq!(SearchOrder::Relevance.max_limit(), 100);
        assert_eq!(SearchOrder::Bible.max_limit(), 1_000);
        for order in [SearchOrder::Relevance, SearchOrder::Bible] {
            assert_eq!(SearchOrder::parse(order.as_param()), Some(order));
        }
    }
}
