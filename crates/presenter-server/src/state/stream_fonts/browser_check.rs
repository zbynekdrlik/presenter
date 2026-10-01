//! Browser font-sanitiser pre-check (#778 reopen).
//!
//! Chrome and Firefox run OTS (the OpenType Sanitizer,
//! `github.com/khaledhosny/ots`) on EVERY downloaded web font and drop a font
//! it refuses: the face never renders and the page logs "Failed to decode
//! downloaded font" + "OTS parsing error: …". `read-fonts` is lazier (each
//! table only checks its fixed minimum size), so a font can pass our metadata
//! parse and still be refused by every browser — font id 71 on SNV/PP declared
//! OS/2 version 5 in a 96-byte table (v5 needs 100).
//!
//! This is NOT a full sanitiser (deep per-table validation of glyf, cmap,
//! GSUB… stays OTS's job). It mirrors exactly the OTS hard-fails below, each
//! read from the OTS source:
//!
//! - `ots.cc ProcessGeneric`: every table record is 4-byte aligned, starts
//!   after the table directory and inside the file, is non-empty, ends inside
//!   the file, and no two tables overlap;
//! - `ots.cc supported_tables`: the eight `required` tables are present
//!   (`maxp head OS/2 cmap hhea hmtx name post`), and each one with a fixed
//!   header is long enough for it (read-fonts' per-table minimum-size check);
//! - `os2.cc OpenTypeOS2::Parse`: the OS/2 version → minimum-length ladder
//!   (see [`check_os2`]).
//!
//! An upload failing it is refused with 422; an already-stored face failing it
//! is hidden from `/stream/fonts.css` and `/stream/api/fonts` (never deleted).

use read_fonts::types::Tag;
use read_fonts::{FontRef, TableProvider};

/// sfnt header bytes before the first table record.
const SFNT_HEADER_LEN: usize = 12;
/// Bytes per table record (`tag checksum offset length`).
const TABLE_RECORD_LEN: usize = 16;

/// The tables OTS marks `required` in `supported_tables` — a font missing
/// any of them fails with "missing required table".
const REQUIRED_TABLES: [Tag; 8] = [
    Tag::new(b"maxp"),
    Tag::new(b"head"),
    Tag::new(b"OS/2"),
    Tag::new(b"cmap"),
    Tag::new(b"hhea"),
    Tag::new(b"hmtx"),
    Tag::new(b"name"),
    Tag::new(b"post"),
];
const OS2: Tag = Tag::new(b"OS/2");

// OS/2 lengths, as OTS's `OpenTypeOS2::Parse` reads them.
/// Through `usWinDescent` — the version-0 fields every OS/2 table must carry.
const OS2_V0_LEN: usize = 78;
/// OTS compares `length < offsetof(OS2Data, code_page_range_2)`, which is 84
/// in its naturally-aligned C struct (2 padding bytes after `panose`), NOT an
/// on-disk offset. A v1+ table shorter than this is silently downgraded to v0
/// (a warning, accepted).
const OTS_V1_DOWNGRADE_BELOW: usize = 84;
/// Through `ulCodePageRange2` — the version-1 fields (a v1+ table of 84..86
/// bytes passes the downgrade test above and then fails reading them).
const OS2_V1_LEN: usize = 86;
/// OTS compares `length < offsetof(OS2Data, max_context)` (96 in the padded
/// struct): a v2+ table shorter than this is downgraded to v1 (accepted). As
/// the v2–v4 fields end at on-disk byte 96 too, reading them never fails.
const OTS_V2_DOWNGRADE_BELOW: usize = 96;
/// Through `usUpperOpticalPointSize` — the version-5 fields.
const OS2_V5_LEN: usize = 100;
/// Newest OS/2 version OTS accepts ("Unsupported table version" above it).
const OS2_MAX_VERSION: u16 = 5;

/// Why a browser's OpenType sanitiser would refuse a font. The `Display` text
/// is the user-facing reason in the upload `422`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum BrowserFontDefect {
    #[error("the font's table directory cannot be read")]
    UnreadableDirectory,
    #[error("table '{0}' is misaligned, empty or reaches outside the file")]
    BadTableRecord(Tag),
    #[error("tables '{0}' and '{1}' overlap")]
    OverlappingTables(Tag, Tag),
    #[error("required table '{0}' is missing")]
    MissingTable(Tag),
    #[error("table '{0}' is too short to hold its header")]
    TruncatedTable(Tag),
    #[error("OS/2 table version {0} is not supported (the newest is 5)")]
    Os2UnsupportedVersion(u16),
    #[error(
        "OS/2 table declares version {version} but is {length} bytes long; \
         that version needs at least {needed}"
    )]
    Os2TooShort {
        version: u16,
        length: usize,
        needed: usize,
    },
}

/// `Ok` when a browser will load the font — the OTS subset described in the
/// module docs. Pure: bytes in, verdict out.
pub(crate) fn check_browser_loadable(bytes: &[u8]) -> Result<(), BrowserFontDefect> {
    let font = FontRef::new(bytes).map_err(|_| BrowserFontDefect::UnreadableDirectory)?;
    check_table_records(&font, bytes.len())?;
    check_required_tables(&font)?;
    let os2 = font
        .table_data(OS2)
        .ok_or(BrowserFontDefect::MissingTable(OS2))?;
    check_os2(os2.as_bytes())
}

/// OTS `ProcessGeneric`: alignment, bounds, non-empty, no overlap.
fn check_table_records(font: &FontRef, file_len: usize) -> Result<(), BrowserFontDefect> {
    let directory = font.table_directory();
    let records = directory.table_records();
    // read-fonts yields an EMPTY slice when the declared records overrun the file.
    if records.len() != usize::from(directory.num_tables()) {
        return Err(BrowserFontDefect::UnreadableDirectory);
    }
    let directory_end = SFNT_HEADER_LEN + TABLE_RECORD_LEN * records.len();
    let mut spans = Vec::with_capacity(records.len());
    for record in records {
        let start = record.offset() as usize;
        let end = start.checked_add(record.length() as usize);
        let in_file = start >= directory_end && end.is_some_and(|end| end <= file_len);
        if !start.is_multiple_of(4) || record.length() == 0 || !in_file {
            return Err(BrowserFontDefect::BadTableRecord(record.tag()));
        }
        spans.push((start, start + record.length() as usize, record.tag()));
    }
    // Sorted by (start, end), any overlap shows up between neighbours (the
    // end breaks start ties so the reported pair is deterministic).
    spans.sort_unstable_by_key(|&(start, end, _)| (start, end));
    for pair in spans.windows(2) {
        let ((_, prev_end, prev_tag), (next_start, _, next_tag)) = (pair[0], pair[1]);
        if next_start < prev_end {
            return Err(BrowserFontDefect::OverlappingTables(prev_tag, next_tag));
        }
    }
    Ok(())
}

/// OTS `supported_tables`: the required tables exist and hold their headers.
fn check_required_tables(font: &FontRef) -> Result<(), BrowserFontDefect> {
    if let Some(&missing) = REQUIRED_TABLES
        .iter()
        .find(|&&tag| font.table_data(tag).is_none())
    {
        return Err(BrowserFontDefect::MissingTable(missing));
    }
    // Each read checks the table's fixed header size; OTS reads the same
    // fields and fails on a table too short for them. OS/2 has its own,
    // version-aware rule in `check_os2`; `hmtx` has no fixed header (its size
    // derives from hhea/maxp), so only its presence is checked above.
    let headers = [
        (Tag::new(b"head"), font.head().is_ok()),
        (Tag::new(b"hhea"), font.hhea().is_ok()),
        (Tag::new(b"maxp"), font.maxp().is_ok()),
        (Tag::new(b"cmap"), font.cmap().is_ok()),
        (Tag::new(b"name"), font.name().is_ok()),
        (Tag::new(b"post"), font.post().is_ok()),
    ];
    match headers.into_iter().find(|&(_, ok)| !ok) {
        Some((tag, _)) => Err(BrowserFontDefect::TruncatedTable(tag)),
        None => Ok(()),
    }
}

/// OTS `OpenTypeOS2::Parse`, the version → minimum-length ladder over the raw
/// `OS/2` table bytes (its declared length). The only hard-fails are a version
/// above 5 and a table too short for the fields its version promises; OTS
/// REPAIRS (warns, accepts) a v1+ table under 84 bytes (→ v0) and a v2+ table
/// under 96 bytes (→ v1), so those are accepted here too.
fn check_os2(table: &[u8]) -> Result<(), BrowserFontDefect> {
    let length = table.len();
    let Some(&[hi, lo]) = table.first_chunk::<2>() else {
        return Err(BrowserFontDefect::TruncatedTable(OS2));
    };
    let version = u16::from_be_bytes([hi, lo]);
    if version > OS2_MAX_VERSION {
        return Err(BrowserFontDefect::Os2UnsupportedVersion(version));
    }
    let too_short = |needed: usize| -> Result<(), BrowserFontDefect> {
        Err(BrowserFontDefect::Os2TooShort {
            version,
            length,
            needed,
        })
    };
    if length < OS2_V0_LEN {
        return too_short(OS2_V0_LEN);
    }
    if version == 0 || length < OTS_V1_DOWNGRADE_BELOW {
        return Ok(());
    }
    if length < OS2_V1_LEN {
        return too_short(OS2_V1_LEN);
    }
    if version < OS2_MAX_VERSION || length < OTS_V2_DOWNGRADE_BELOW {
        return Ok(());
    }
    if length < OS2_V5_LEN {
        return too_short(OS2_V5_LEN);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::stream_fonts::test_fonts::{
        table_offset, with_os2_version, with_table_length, with_table_offset, with_table_renamed,
        GRUPPO_TTF,
    };

    /// A bare OS/2 table of `length` bytes declaring `version`.
    fn os2_table(version: u16, length: usize) -> Vec<u8> {
        let mut table = vec![0u8; length];
        table[..2].copy_from_slice(&version.to_be_bytes());
        table
    }

    #[test]
    fn the_ofl_fixture_is_browser_loadable() {
        assert_eq!(check_browser_loadable(GRUPPO_TTF), Ok(()));
    }

    #[test]
    fn os2_v5_in_a_96_byte_table_is_refused_like_font_71() {
        // The exact SNV/PP font-id-71 defect (OTS: "Failed to read version
        // 5-specific fields").
        assert_eq!(
            check_browser_loadable(&with_os2_version(GRUPPO_TTF, 5)),
            Err(BrowserFontDefect::Os2TooShort {
                version: 5,
                length: 96,
                needed: 100
            })
        );
    }

    #[test]
    fn os2_version_ladder_matches_ots() {
        use super::BrowserFontDefect::{Os2TooShort, Os2UnsupportedVersion};
        let short = |version: u16, length: usize, needed: usize| -> Result<(), BrowserFontDefect> {
            Err(Os2TooShort {
                version,
                length,
                needed,
            })
        };
        let cases: &[(u16, usize, Result<(), BrowserFontDefect>)] = &[
            // Every version needs the 78-byte v0 fields.
            (0, 77, short(0, 77, 78)),
            (0, 78, Ok(())),
            (4, 70, short(4, 70, 78)),
            // v1+ under 84 bytes: OTS downgrades to v0 and accepts.
            (1, 80, Ok(())),
            (5, 83, Ok(())),
            // v1+ at 84..86 bytes: passes the downgrade test, then cannot
            // read ulCodePageRange1/2.
            (1, 84, short(1, 84, 86)),
            (3, 85, short(3, 85, 86)),
            (1, 86, Ok(())),
            // v2+ under 96 bytes: OTS downgrades to v1 and accepts.
            (2, 90, Ok(())),
            (5, 95, Ok(())),
            // v2..v4 fields end at 96.
            (4, 96, Ok(())),
            // v5 needs the two optical-size fields (100 bytes).
            (5, 96, short(5, 96, 100)),
            (5, 99, short(5, 99, 100)),
            (5, 100, Ok(())),
            (6, 100, Err(Os2UnsupportedVersion(6))),
        ];
        for (version, length, expected) in cases {
            assert_eq!(
                &check_os2(&os2_table(*version, *length)),
                expected,
                "OS/2 v{version}, {length} bytes"
            );
        }
        assert_eq!(check_os2(&[0]), Err(BrowserFontDefect::TruncatedTable(OS2)));
    }

    #[test]
    fn os2_truncated_below_v0_size_in_a_real_font_is_refused() {
        assert_eq!(
            check_browser_loadable(&with_table_length(GRUPPO_TTF, b"OS/2", 70)),
            Err(BrowserFontDefect::Os2TooShort {
                version: 4,
                length: 70,
                needed: 78
            })
        );
    }

    #[test]
    fn every_ots_required_table_is_needed() {
        for tag in REQUIRED_TABLES {
            let raw = tag.to_be_bytes();
            // Re-tag to a sorted-neighbour name so only that table goes missing.
            let mut renamed = raw;
            renamed[3] = renamed[3].wrapping_add(1);
            let font = with_table_renamed(GRUPPO_TTF, &raw, &renamed);
            assert_eq!(
                check_browser_loadable(&font),
                Err(BrowserFontDefect::MissingTable(tag)),
                "{tag} missing"
            );
        }
    }

    #[test]
    fn a_required_table_too_short_for_its_header_is_refused() {
        assert_eq!(
            check_browser_loadable(&with_table_length(GRUPPO_TTF, b"head", 40)),
            Err(BrowserFontDefect::TruncatedTable(Tag::new(b"head")))
        );
    }

    #[test]
    fn misaligned_table_is_refused() {
        let offset = table_offset(GRUPPO_TTF, b"gasp");
        assert_eq!(
            check_browser_loadable(&with_table_offset(GRUPPO_TTF, b"gasp", offset + 2)),
            Err(BrowserFontDefect::BadTableRecord(Tag::new(b"gasp")))
        );
    }

    #[test]
    fn table_reaching_past_the_end_of_the_file_is_refused() {
        let past_end = u32::try_from(GRUPPO_TTF.len()).unwrap();
        assert_eq!(
            check_browser_loadable(&with_table_length(GRUPPO_TTF, b"post", past_end)),
            Err(BrowserFontDefect::BadTableRecord(Tag::new(b"post")))
        );
    }

    #[test]
    fn empty_table_is_refused() {
        assert_eq!(
            check_browser_loadable(&with_table_length(GRUPPO_TTF, b"gasp", 0)),
            Err(BrowserFontDefect::BadTableRecord(Tag::new(b"gasp")))
        );
    }

    #[test]
    fn overlapping_tables_are_refused() {
        // Point `hhea` (36 bytes) at `head`'s bytes (54): both aligned + in
        // bounds, but shared. Sorted by (start, end) → hhea first.
        let head = table_offset(GRUPPO_TTF, b"head");
        assert_eq!(
            check_browser_loadable(&with_table_offset(GRUPPO_TTF, b"hhea", head)),
            Err(BrowserFontDefect::OverlappingTables(
                Tag::new(b"hhea"),
                Tag::new(b"head")
            ))
        );
    }

    #[test]
    fn non_font_bytes_are_refused() {
        assert_eq!(
            check_browser_loadable(b"definitely not a font"),
            Err(BrowserFontDefect::UnreadableDirectory)
        );
    }
}
