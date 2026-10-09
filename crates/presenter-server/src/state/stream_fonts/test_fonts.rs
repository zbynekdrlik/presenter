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
//!
//! The #830 helpers derive the FACES of a test family the same way: rename the
//! style in the `name` table ([`with_names`]), set the `OS/2` weight
//! ([`with_os2_weight`]) or flag the face italic ([`with_os2_italic`]).

use std::collections::BTreeMap;

use read_fonts::{FontRef, TableProvider};

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

/// Copy of `font` whose `OS/2` `usWeightClass` (bytes 4..6) is `weight`.
pub(crate) fn with_os2_weight(font: &[u8], weight: u16) -> Vec<u8> {
    let mut out = font.to_vec();
    let at = table_offset(font, b"OS/2") as usize + 4;
    out[at..at + 2].copy_from_slice(&weight.to_be_bytes());
    out
}

/// Copy of `font` flagged italic in `OS/2` `fsSelection` (bytes 62..64): the
/// ITALIC bit set and the REGULAR bit cleared, as a real italic face has it.
pub(crate) fn with_os2_italic(font: &[u8]) -> Vec<u8> {
    let mut out = font.to_vec();
    let at = table_offset(font, b"OS/2") as usize + 62;
    let flags = u16::from_be_bytes([out[at], out[at + 1]]);
    let flags = (flags | 0x0001) & !0x0040;
    out[at..at + 2].copy_from_slice(&flags.to_be_bytes());
    out
}

/// Copy of `font` whose `name` table carries `names` (name id, text). Each pair
/// replaces that id's string; every other Unicode/Windows name is kept.
///
/// The rebuilt table (format 0, Windows Unicode-BMP en-US records sorted by id)
/// is APPENDED at a 4-byte aligned offset and the `name` record is repointed at
/// it. The old table stays behind as an unused gap, which OTS accepts (it only
/// checks alignment, bounds and overlap), so no other table moves.
pub(crate) fn with_names(font: &[u8], names: &[(u16, &str)]) -> Vec<u8> {
    let mut strings = existing_names(font);
    for &(id, text) in names {
        strings.insert(id, text.to_string());
    }
    let table = build_name_table(&strings);
    let mut out = font.to_vec();
    while out.len() % 4 != 0 {
        out.push(0);
    }
    let offset = u32::try_from(out.len()).expect("fixture size fits u32");
    let length = u32::try_from(table.len()).expect("name table size fits u32");
    out.extend_from_slice(&table);
    let at = record_pos(font, b"name");
    out[at + 8..at + 12].copy_from_slice(&offset.to_be_bytes());
    out[at + 12..at + 16].copy_from_slice(&length.to_be_bytes());
    out
}

/// The font's Unicode (0) / Windows (3) names by id, first record per id.
fn existing_names(font: &[u8]) -> BTreeMap<u16, String> {
    let font = FontRef::new(font).expect("fixture parses");
    let name = font.name().expect("fixture has a name table");
    let data = name.string_data();
    let mut out = BTreeMap::new();
    for record in name.name_record() {
        if !matches!(record.platform_id(), 0 | 3) {
            continue;
        }
        if let Ok(text) = record.string(data) {
            out.entry(record.name_id().to_u16())
                .or_insert_with(|| text.to_string());
        }
    }
    out
}

/// A format-0 `name` table: one Windows (3) / Unicode BMP (1) / en-US (0x0409)
/// record per id, UTF-16BE strings.
fn build_name_table(strings: &BTreeMap<u16, String>) -> Vec<u8> {
    let count = u16::try_from(strings.len()).expect("name count fits u16");
    let mut records = Vec::new();
    let mut storage: Vec<u8> = Vec::new();
    for (&id, text) in strings {
        let encoded: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let length = u16::try_from(encoded.len()).expect("name length fits u16");
        let offset = u16::try_from(storage.len()).expect("name storage fits u16");
        for field in [3u16, 1, 0x0409, id, length, offset] {
            records.extend_from_slice(&field.to_be_bytes());
        }
        storage.extend_from_slice(&encoded);
    }
    let mut table = Vec::new();
    table.extend_from_slice(&0u16.to_be_bytes());
    table.extend_from_slice(&count.to_be_bytes());
    table.extend_from_slice(&(6 + 12 * count).to_be_bytes());
    table.extend_from_slice(&records);
    table.extend_from_slice(&storage);
    table
}
