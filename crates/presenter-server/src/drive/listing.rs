//! Pure parser of Drive's keyless folder listing (#843):
//! `GET https://drive.google.com/embeddedfolderview?id=<FID>`.
//!
//! The page is flat, server-rendered HTML (live-checked on #843). Each entry is
//! one block:
//!
//! ```text
//! <div class="flip-entry" id="entry-<ID>" …><div class="flip-entry-info">
//!   <a href="https://drive.google.com/file/d/<ID>/view?usp=drive_web[&amp;resourcekey=<RK>]" …>
//!     … <div class="flip-entry-list-icon">
//!         <img src="https://drive-thirdparty.googleusercontent.com/16/type/<mime>" alt=""/>
//!       </div>
//!     <div class="flip-entry-title"><name, HTML-escaped></div>
//!   </a></div>…</div>
//! ```
//!
//! A subfolder links to `/drive/folders/<ID>` and carries an
//! `aria-label="Folder"` sprite instead of a MIME icon. The page has no size,
//! checksum or machine-readable date — change detection is the download HEAD.
//! No HTML-parser crate: the blocks are split on their fixed opening tag and
//! each field is read with a small regex (`regex` is already a dependency).

use regex::Regex;
use std::sync::LazyLock;

/// The literal that opens every entry block.
const ENTRY_OPEN: &str = r#"<div class="flip-entry" id="entry-"#;

// `Regex::new(...).ok()` is `None` only for a malformed literal — a bug the
// unit tests catch; production then parses nothing instead of panicking.
static HREF_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r#"<a href="([^"]*)""#).ok());
static MIME_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r#"googleusercontent\.com/\d+/type/([A-Za-z0-9.+-]+/[A-Za-z0-9.+-]+)""#).ok()
});
static TITLE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"<div class="flip-entry-title">([^<]*)</div>"#).ok());
static RESOURCE_KEY_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"resourcekey=([-\w]+)").ok());

/// One entry of a folder listing, in page order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DriveEntry {
    /// The Drive file (or subfolder) id.
    pub(crate) id: String,
    /// The display name, HTML entities decoded.
    pub(crate) name: String,
    /// The MIME type from the listing icon (`None` for a folder or an entry
    /// without an icon). The download's own `Content-Type` is NOT trusted for
    /// this (#843 research).
    pub(crate) mime: Option<String>,
    /// `true` for a subfolder (skipped by the playback sync).
    pub(crate) is_folder: bool,
    /// The file's own `resourcekey` (older shared files need it to download).
    pub(crate) resource_key: Option<String>,
}

/// `true` for a folder-listing page — an EMPTY folder still carries the
/// `flip-entries` container. Anything else (a sign-in page, a format change) is
/// not a listing and must never be read as "the folder is empty".
pub(crate) fn is_listing_page(html: &str) -> bool {
    html.contains(r#"class="flip-entries""#)
}

/// Every entry block of a listing page, in page order. Blocks whose id is not a
/// plausible Drive id are dropped (a page-format change then yields fewer
/// entries, never a path built from junk).
pub(crate) fn parse_listing(html: &str) -> Vec<DriveEntry> {
    let mut blocks = html.split(ENTRY_OPEN);
    // Everything before the first entry is the page head.
    let _head = blocks.next();
    blocks.filter_map(parse_block).collect()
}

/// One entry block (the text after `id="entry-`, up to the next block).
fn parse_block(block: &str) -> Option<DriveEntry> {
    let id = block.split('"').next()?;
    if !presenter_core::is_valid_drive_file_id(id) {
        return None;
    }
    let href = capture(&HREF_RE, block).unwrap_or_default();
    let is_folder = href.contains("/drive/folders/") || block.contains(r#"aria-label="Folder""#);
    let mime = capture(&MIME_RE, block).map(|m| m.to_ascii_lowercase());
    let name = capture(&TITLE_RE, block)
        .map(|raw| decode_entities(raw.trim()))
        .unwrap_or_default();
    let resource_key = capture(&RESOURCE_KEY_RE, href).map(str::to_string);
    Some(DriveEntry {
        id: id.to_string(),
        name,
        mime,
        is_folder,
        resource_key,
    })
}

/// The first capture group of `re` in `text` (`&HREF_RE` deref-coerces from
/// the `LazyLock`).
fn capture<'t>(re: &Option<Regex>, text: &'t str) -> Option<&'t str> {
    let re = re.as_ref()?;
    re.captures(text)?.get(1).map(|m| m.as_str())
}

/// Decode the entities Drive writes into titles (`&amp;`, `&#39;`, `&quot;`,
/// `&lt;`, `&gt;`, numeric `&#NN;` / `&#xHH;`). An unknown entity stays as is.
pub(crate) fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        match after.find(';').filter(|end| *end <= 10) {
            Some(end) => {
                let entity = &after[1..end];
                match decode_entity(entity) {
                    Some(ch) => out.push(ch),
                    None => out.push_str(&after[..=end]),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = entity.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        }
    }
}
