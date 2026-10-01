//! Test-only sfnt byte surgery for the #778 browser-sanitiser fixtures.
//!
//! The broken fonts are DERIVED in memory from the committed OFL fixture
//! (`Gruppo-Regular.ttf`, OS/2 version 4, 96 bytes) — never committed as files:
//! a modified OFL font may not carry the original name, and the defect is a
//! few header bytes, so generating it in-test keeps the fixture set to the one
//! licence-clean font. Each helper patches exactly one field of the table
//! directory or the OS/2 table, so a test names the single defect it exercises.
//!
//! sfnt layout used here: a 12-byte header (`numTables` at byte 4), then one
//! 16-byte record per table: `tag[4] checksum[4] offset[4] length[4]` (all
//! big-endian). OTS ignores checksums, so a patched font needs no re-checksum.

/// The committed OFL fixture font (family "Gruppo"; `OS/2` version 4, 96 bytes).
pub(crate) const GRUPPO_TTF: &[u8] =
    include_bytes!("../../../../../tests/e2e/fixtures/fonts/Gruppo-Regular.ttf");

const SFNT_HEADER_LEN: usize = 12;
const TABLE_RECORD_LEN: usize = 16;

fn be_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Byte position of `tag`'s table record in the table directory.
fn record_pos(font: &[u8], tag: &[u8; 4]) -> usize {
    let num_tables = usize::from(u16::from_be_bytes([font[4], font[5]]));
    (0..num_tables)
        .map(|i| SFNT_HEADER_LEN + i * TABLE_RECORD_LEN)
        .find(|&pos| &font[pos..pos + 4] == tag)
        .unwrap_or_else(|| panic!("fixture has no {:?} table", String::from_utf8_lossy(tag)))
}

/// Byte offset of `tag`'s table data, as declared in its record.
pub(crate) fn table_offset(font: &[u8], tag: &[u8; 4]) -> u32 {
    be_u32(font, record_pos(font, tag) + 8)
}

/// Copy of `font` whose `OS/2` table carries `version` (length unchanged).
/// `with_os2_version(GRUPPO_TTF, 5)` is the exact font-id-71 defect seen on
/// SNV/PP: a version-5 OS/2 that is only 96 bytes long (v5 needs 100).
pub(crate) fn with_os2_version(font: &[u8], version: u16) -> Vec<u8> {
    let mut out = font.to_vec();
    let at = table_offset(font, b"OS/2") as usize;
    out[at..at + 2].copy_from_slice(&version.to_be_bytes());
    out
}

/// Copy of `font` whose directory declares `length` bytes for `tag`.
pub(crate) fn with_table_length(font: &[u8], tag: &[u8; 4], length: u32) -> Vec<u8> {
    let mut out = font.to_vec();
    let at = record_pos(font, tag) + 12;
    out[at..at + 4].copy_from_slice(&length.to_be_bytes());
    out
}

/// Copy of `font` whose directory points `tag` at `offset`.
pub(crate) fn with_table_offset(font: &[u8], tag: &[u8; 4], offset: u32) -> Vec<u8> {
    let mut out = font.to_vec();
    let at = record_pos(font, tag) + 8;
    out[at..at + 4].copy_from_slice(&offset.to_be_bytes());
    out
}

/// Copy of `font` whose `from` table record is re-tagged `to` — the table is
/// then MISSING under its real tag. Pick a `to` that keeps the directory
/// sorted (e.g. `post` → `pozt`) so only the one defect is introduced.
pub(crate) fn with_table_renamed(font: &[u8], from: &[u8; 4], to: &[u8; 4]) -> Vec<u8> {
    let mut out = font.to_vec();
    let at = record_pos(font, from);
    out[at..at + 4].copy_from_slice(to);
    out
}
