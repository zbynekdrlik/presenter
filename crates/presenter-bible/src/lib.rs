// Modularized interface for presenter-bible (#46)

mod parse;
mod provider;
mod spec;

pub use parse::{
    build_ingestion_batch, parse_mysword_sqlite_zip, parse_obohu_sqlite_zip, parse_usfm_zip,
};
pub use provider::{BibleContentProvider, BibleScraper, HttpBibleContentProvider};
pub use spec::{
    default_translation_specs, BibleImportSummary, BibleSource, BibleSourceFormat,
    BibleTranslationSpec, KING_JAMES_SOURCE, SLOVAK_ECUMENICKY_SOURCE, SLOVAK_EVANGELICKY_SOURCE,
    SLOVAK_ROHACEK_SOURCE,
};

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;
    use zip::write::FileOptions;

    use std::collections::HashSet;
    use std::io::{Cursor, Write};

    struct StubProvider {
        payload: Vec<u8>,
    }

    #[async_trait]
    impl BibleContentProvider for StubProvider {
        async fn fetch_bytes(&self, _url: &str) -> anyhow::Result<Vec<u8>> {
            Ok(self.payload.clone())
        }
    }

    fn sample_translation() -> presenter_core::BibleTranslation {
        presenter_core::BibleTranslation::new("en-test", "Test Translation", "en")
    }

    fn make_sample_archive() -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let cursor = Cursor::new(&mut buffer);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            let content =
                "\\id JHN John\n\\c 3\n\\v 16 For God so loved the world\n\\v 17 that He gave\n";
            writer.start_file("JHN.usfm", options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        buffer
    }

    fn make_mysword_archive() -> Vec<u8> {
        let mut temp = NamedTempFile::new().unwrap();
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute(
            "CREATE TABLE Bible (Book INTEGER, Chapter INTEGER, Verse INTEGER, Scripture TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO Bible (Book, Chapter, Verse, Scripture) VALUES (1, 1, 1, 'Na počiatku stvoril Boh nebo a zem.')",
            [],
        )
        .unwrap();
        let filename = "sample.bbl.mybible";
        let bytes = std::fs::read(temp.path()).unwrap();

        let mut buffer = Vec::new();
        {
            let cursor = Cursor::new(&mut buffer);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            writer.start_file(filename, options).unwrap();
            writer.write_all(&bytes).unwrap();
            writer.finish().unwrap();
        }
        buffer
    }

    fn make_obohu_archive(primary_name: &str) -> Vec<u8> {
        let mut temp = NamedTempFile::new().unwrap();
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute(
            "CREATE TABLE books (book_number INTEGER PRIMARY KEY, long_name TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE verses (book_number INTEGER, chapter INTEGER, verse INTEGER, text TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books (book_number, long_name) VALUES (?1, ?2)",
            rusqlite::params![1, "Genezis"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO verses (book_number, chapter, verse, text) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![1, 1, 1, "V počiatku stvoril Boh nebo a zem."],
        )
        .unwrap();
        let bytes = std::fs::read(temp.path()).unwrap();

        let mut buffer = Vec::new();
        {
            let cursor = Cursor::new(&mut buffer);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            writer.start_file(primary_name, options).unwrap();
            writer.write_all(&bytes).unwrap();
            writer.finish().unwrap();
        }
        buffer
    }

    #[tokio::test]
    async fn parses_usfm_archive_into_passages() {
        let archive = make_sample_archive();
        let provider = StubProvider { payload: archive };
        let scraper = BibleScraper::new(provider);
        let translation = sample_translation();
        let spec = BibleTranslationSpec {
            translation: translation.clone(),
            source: BibleSource::Url {
                url: "memory".to_string(),
            },
            format: BibleSourceFormat::UsfmZip {
                book_name_overrides: std::collections::HashMap::new(),
            },
        };

        let (batch, summary) = scraper.scrape(&spec).await.unwrap();
        assert_eq!(summary.translation_code, "en-test");
        assert_eq!(summary.passage_count, 2);
        assert_eq!(batch.translation(), &translation);
        let passages = batch.passages();
        assert_eq!(passages.len(), 2);
        assert_eq!(passages[0].reference.book, "John");
        assert_eq!(passages[0].reference.chapter, 3);
        assert_eq!(passages[0].reference.verse_start, 16);
        assert_eq!(passages[0].text, "For God so loved the world");
        assert_eq!(passages[1].text, "that He gave");
    }

    #[test]
    fn default_specs_include_expected_translations() {
        let codes: HashSet<_> = default_translation_specs()
            .into_iter()
            .map(|spec| spec.translation.code.clone())
            .collect();
        assert!(codes.contains("eng-kjv"));
        assert!(codes.contains("slk-seb"));
        assert!(codes.contains("slk-roh"));
        assert!(codes.contains("slk-sevp"));
    }

    #[tokio::test]
    async fn default_spec_overrides_psalms_label() {
        let mut spec = default_translation_specs()
            .into_iter()
            .find(|spec| spec.translation.code == "eng-kjv")
            .expect("eng-kjv spec");
        spec.source = BibleSource::Url {
            url: "memory".to_string(),
        };

        let mut archive = Vec::new();
        {
            use zip::write::FileOptions;
            let cursor = Cursor::new(&mut archive);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            let content = "\\id PSA Psalms\n\\c 23\n\\v 1 The Lord is my shepherd\n";
            writer.start_file("PSA.usfm", options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
            writer.finish().unwrap();
        }

        let provider = StubProvider { payload: archive };
        let scraper = BibleScraper::new(provider);
        let (batch, _) = scraper.scrape(&spec).await.unwrap();
        let passages = batch.passages();
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].reference.book, "Psalms");
        assert_eq!(passages[0].reference.chapter, 23);
        assert_eq!(passages[0].reference.verse_start, 1);
    }

    #[tokio::test]
    async fn parses_mysword_archive_into_passages() {
        let archive = make_mysword_archive();
        let provider = StubProvider { payload: archive };
        let scraper = BibleScraper::new(provider);
        let mut spec = crate::spec::slovak_ekumenicky_spec();
        spec.source = BibleSource::Url {
            url: "memory".to_string(),
        };

        let (batch, summary) = scraper.scrape(&spec).await.unwrap();
        assert_eq!(summary.translation_code, "slk-seb");
        let passages = batch.passages();
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].reference.book, "Genezis");
        assert_eq!(passages[0].reference.chapter, 1);
        assert_eq!(passages[0].reference.verse_start, 1);
        assert_eq!(passages[0].text, "Na počiatku stvoril Boh nebo a zem.");
    }

    #[tokio::test]
    async fn parses_obohu_archive_into_passages() {
        let archive = make_obohu_archive("ROH-AV.SQLite3");
        let provider = StubProvider { payload: archive };
        let scraper = BibleScraper::new(provider);
        let mut spec = crate::spec::slovak_rohacek_spec();
        spec.source = BibleSource::Url {
            url: "memory".to_string(),
        };

        let (batch, summary) = scraper.scrape(&spec).await.unwrap();
        assert_eq!(summary.translation_code, "slk-roh");
        let passages = batch.passages();
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].reference.book, "Genezis");
        assert_eq!(passages[0].reference.chapter, 1);
        assert_eq!(passages[0].reference.verse_start, 1);
        assert_eq!(passages[0].text, "V počiatku stvoril Boh nebo a zem.");
    }
}
