//! Miscellaneous tools that don't belong to a specific domain.

pub(super) async fn get_style_guide() -> anyhow::Result<(String, String)> {
    // Path is relative to this file (src/ai/tools/), so step up one level
    // to reach src/ai/style_guide.md.
    let guide = include_str!("../style_guide.md");
    Ok((guide.to_string(), "Style guide loaded".to_string()))
}
