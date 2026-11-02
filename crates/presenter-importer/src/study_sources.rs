//! Study sources, one pass each after the translations (#851 D3a, D3b, spec
//! §5.6): skipped while the digest matches, else replaced in one transaction.

use anyhow::{Context, Result};
use presenter_bible::{
    parse_cross_references, parse_greek_lexicon, parse_hebrew_lexicon, read_zip_member,
};
use presenter_core::bible::{LexiconEntry, StudySource};
use presenter_persistence::Repository;

use crate::compute_source_digest;

/// Bump on any parse change: it re-imports even when the bytes are unchanged.
pub const STUDY_PARSER_VERSION: u32 = 1;

const PUBLIC_DOMAIN: &str = "Public Domain";
const OPENBIBLE_LICENSE: &str = "CC BY (OpenBible.info)";

/// The compiled-in files.
pub struct StudyFiles<'a> {
    pub strongs_zip: &'a [u8],
    pub cross_references_zip: &'a [u8],
}

fn study_source(code: &str, name: &str, license: &str, digest: String) -> StudySource {
    StudySource {
        code: code.to_string(),
        name: name.to_string(),
        license: license.to_string(),
        source_digest: digest,
    }
}

fn skipped(code: &str, digest: &str) -> String {
    format!("Skipping {code} (up-to-date, digest {})", &digest[..12])
}

async fn import_lexicon(
    repository: &Repository,
    strongs_zip: &[u8],
    member: &str,
    (code, name): (&str, &str),
    parse: fn(&[u8]) -> Result<Vec<LexiconEntry>>,
) -> Result<String> {
    let xml = read_zip_member(strongs_zip, member)?;
    let digest = compute_source_digest(&xml, STUDY_PARSER_VERSION);
    if repository.bible_study_source_digest(code).await?.as_ref() == Some(&digest) {
        return Ok(skipped(code, &digest));
    }
    let entries = parse(&xml).with_context(|| format!("failed to parse {member}"))?;
    let source = study_source(code, name, PUBLIC_DOMAIN, digest);
    repository.replace_bible_lexicon(&source, &entries).await?;
    Ok(format!(
        "Imported {} Strong's entries ({code}, digest {})",
        entries.len(),
        &source.source_digest[..12]
    ))
}

async fn import_cross_references(repository: &Repository, zip: &[u8]) -> Result<String> {
    const CODE: &str = "openbible-xref";
    let digest = compute_source_digest(zip, STUDY_PARSER_VERSION);
    if repository.bible_study_source_digest(CODE).await?.as_ref() == Some(&digest) {
        return Ok(skipped(CODE, &digest));
    }
    let rows = parse_cross_references(zip)?;
    let source = study_source(CODE, "OpenBible.info", OPENBIBLE_LICENSE, digest);
    repository
        .replace_bible_cross_references(&source, &rows)
        .await?;
    Ok(format!(
        "Imported {} cross-references ({CODE}, digest {})",
        rows.len(),
        &source.source_digest[..12]
    ))
}

/// One line per source, for the import log.
pub async fn import_study_sources(
    repository: &Repository,
    files: &StudyFiles<'_>,
) -> Result<Vec<String>> {
    let hebrew = import_lexicon(
        repository,
        files.strongs_zip,
        "StrongHebrewG.xml",
        ("strongs-hebrew", "Strong's Hebrew Dictionary"),
        parse_hebrew_lexicon,
    )
    .await
    .context("strongs-hebrew")?;
    let greek = import_lexicon(
        repository,
        files.strongs_zip,
        "strongsgreek.xml",
        ("strongs-greek", "Strong's Greek Dictionary"),
        parse_greek_lexicon,
    )
    .await
    .context("strongs-greek")?;
    let references = import_cross_references(repository, files.cross_references_zip)
        .await
        .context("openbible-xref")?;
    Ok(vec![hebrew, greek, references])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    const STRONGS: &[u8] = include_bytes!("../../../data/study/strongs.zip");
    const REFERENCES: &[u8] = include_bytes!("../../../data/study/openbible-cross-references.zip");

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in files {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(text.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn strongs(definition: &str) -> Vec<u8> {
        let hebrew = format!(
            r#"<osis><div type="entry"><w ID="H1"/><note type="explanation">{definition}</note></div></osis>"#
        );
        zip_of(&[
            ("StrongHebrewG.xml", &hebrew),
            ("strongsgreek.xml", r#"<d><entry strongs="00001"/></d>"#),
        ])
    }

    fn references() -> Vec<u8> {
        zip_of(&[("cross_references.txt", "Gen.1.1\tJer.32.17\t9\n")])
    }

    #[tokio::test]
    async fn the_second_run_skips_and_changed_bytes_replace() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let (first, cross) = (strongs("old"), references());
        let files = StudyFiles {
            strongs_zip: &first,
            cross_references_zip: &cross,
        };
        let lines = import_study_sources(&repo, &files).await.unwrap();
        assert!(
            lines[0].starts_with("Imported 1 Strong's entries (strongs-hebrew"),
            "{lines:?}"
        );
        assert!(
            lines[2].starts_with("Imported 1 cross-references"),
            "{lines:?}"
        );

        let lines = import_study_sources(&repo, &files).await.unwrap();
        assert!(
            lines.iter().all(|line| line.starts_with("Skipping")),
            "{lines:?}"
        );

        let changed = strongs("new");
        let files = StudyFiles {
            strongs_zip: &changed,
            cross_references_zip: &cross,
        };
        let lines = import_study_sources(&repo, &files).await.unwrap();
        assert!(lines[0].starts_with("Imported"), "{lines:?}");
        assert!(
            lines[1].starts_with("Skipping"),
            "the Greek file did not change: {lines:?}"
        );
        let entry = repo.bible_lexicon_entry("H1").await.unwrap().unwrap();
        assert_eq!(entry.definition, "new");
    }

    #[tokio::test]
    async fn the_committed_files_import_whole() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let files = StudyFiles {
            strongs_zip: STRONGS,
            cross_references_zip: REFERENCES,
        };
        let lines = import_study_sources(&repo, &files).await.unwrap();
        assert!(lines[0].contains("8674 Strong's entries"), "{lines:?}");
        assert!(lines[1].contains("5624 Strong's entries"), "{lines:?}");
        assert!(lines[2].contains("328139 cross-references"), "{lines:?}");
        let faith = repo.bible_lexicon_entry("G4102").await.unwrap().unwrap();
        assert_eq!(faith.lemma, "πίστις");
        let john = repo.bible_cross_references_from(43, 3, 16).await.unwrap();
        assert!(john.len() > 5, "{}", john.len());
        assert!(john[0].votes >= john[1].votes);
    }
}
