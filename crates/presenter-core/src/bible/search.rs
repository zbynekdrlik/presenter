use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub(crate) fn normalise_book_key(input: &str) -> String {
    let mut result = String::new();
    for ch in input.nfd().filter(|c| !is_combining_mark(*c)) {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::normalise_book_key;

    #[test]
    fn strips_non_alphanumeric_characters() {
        assert_eq!(normalise_book_key("1 Korinťanom"), "1korintanom");
        assert_eq!(
            normalise_book_key("Acts (of the Apostles)"),
            "actsoftheapostles"
        );
    }
}
