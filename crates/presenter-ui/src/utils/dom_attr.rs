//! Small DOM attribute helpers shared across pages.

/// Render a bool as the `"true"`/`"false"` string used by every `data-active`
/// / `data-selected` / `data-dirty` attribute (the stream editor's rows and the
/// shared nameplate list, #871), instead of the same inline ternary everywhere.
pub fn bool_attr(b: bool) -> &'static str {
    if b {
        "true"
    } else {
        "false"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bool_renders_as_the_attribute_text() {
        assert_eq!(bool_attr(true), "true");
        assert_eq!(bool_attr(false), "false");
    }
}
