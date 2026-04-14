use anyhow::Result;
use presenter_bible::{
    default_translation_specs, BibleContentProvider, BibleImportSummary, BibleScraper,
    BibleTranslationSpec, HttpBibleContentProvider,
};
use presenter_persistence::Repository;

pub struct BibleIngestionService<'a, P: BibleContentProvider> {
    repository: &'a Repository,
    scraper: BibleScraper<P>,
}

impl<'a> BibleIngestionService<'a, HttpBibleContentProvider> {
    pub fn with_http(repository: &'a Repository) -> Result<Self> {
        let provider = HttpBibleContentProvider::new()?;
        Ok(Self::new(repository, provider))
    }
}

impl<'a, P> BibleIngestionService<'a, P>
where
    P: BibleContentProvider,
{
    pub fn new(repository: &'a Repository, provider: P) -> Self {
        Self {
            repository,
            scraper: BibleScraper::new(provider),
        }
    }

    pub async fn ingest(&self, spec: &BibleTranslationSpec) -> Result<BibleImportSummary> {
        let (batch, summary) = self.scraper.scrape(spec).await?;
        self.repository
            .replace_bible_translation_passages(&batch)
            .await?;
        Ok(summary)
    }

    pub async fn ingest_with_bytes(
        &self,
        spec: &BibleTranslationSpec,
        bytes: &[u8],
    ) -> Result<BibleImportSummary> {
        let (batch, summary) = self.scraper.scrape_with_bytes(spec, bytes)?;
        self.repository
            .replace_bible_translation_passages(&batch)
            .await?;
        Ok(summary)
    }

    pub async fn ingest_specs<I>(&self, specs: I) -> Result<Vec<BibleImportSummary>>
    where
        I: IntoIterator<Item = BibleTranslationSpec>,
    {
        let mut summaries = Vec::new();
        for spec in specs {
            let summary = self.ingest(&spec).await?;
            summaries.push(summary);
        }
        Ok(summaries)
    }

    pub async fn ingest_default_translations(&self) -> Result<Vec<BibleImportSummary>> {
        self.ingest_specs(default_translation_specs()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use presenter_bible::{BibleSource, BibleSourceFormat};
    use presenter_core::{BibleReference, BibleTranslation};
    use presenter_persistence::Repository;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;
    use zip::write::FileOptions;

    use std::collections::{HashMap, HashSet};
    use std::io::{Cursor, Write};

    struct SinglePayloadProvider {
        payload: Vec<u8>,
        url: String,
    }

    #[async_trait]
    impl BibleContentProvider for SinglePayloadProvider {
        async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
            assert_eq!(url, self.url);
            Ok(self.payload.clone())
        }
    }

    struct MapProvider {
        payloads: HashMap<String, Vec<u8>>,
    }

    #[async_trait]
    impl BibleContentProvider for MapProvider {
        async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
            self.payloads
                .get(url)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no payload registered for {url}"))
        }
    }

    fn make_usfm_archive() -> Vec<u8> {
        const CONTENT: &str = r"\id JHN John
\c 3
\v 16 For God so loved the world
\v 17 that He gave
";
        let mut buffer = Vec::new();
        {
            let cursor = Cursor::new(&mut buffer);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            writer.start_file("JHN.usfm", options).unwrap();
            writer.write_all(CONTENT.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        buffer
    }

    fn make_mysword_archive() -> Vec<u8> {
        let temp = NamedTempFile::new().unwrap();
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute(
            "CREATE TABLE Bible (Book INTEGER, Chapter INTEGER, Verse INTEGER, Scripture TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO Bible (Book, Chapter, Verse, Scripture) VALUES (?1, ?2, ?3, ?4)",
            (1, 1, 1, "Na počiatku stvoril Boh nebo a zem. <f>[++]</f>"),
        )
        .unwrap();
        let bytes = std::fs::read(temp.path()).unwrap();

        let mut buffer = Vec::new();
        {
            let cursor = Cursor::new(&mut buffer);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = FileOptions::default();
            writer.start_file("SEB.bbl.mybible", options).unwrap();
            writer.write_all(&bytes).unwrap();
            writer.finish().unwrap();
        }
        buffer
    }

    fn make_obohu_archive(filename: &str) -> Vec<u8> {
        let temp = NamedTempFile::new().unwrap();
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
            (10, "Genezis"),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO verses (book_number, chapter, verse, text) VALUES (?1, ?2, ?3, ?4)",
            (10, 1, 1, "V počiatku stvoril Boh nebo a zem."),
        )
        .unwrap();
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

    fn sample_spec() -> BibleTranslationSpec {
        BibleTranslationSpec {
            translation: BibleTranslation::new("en-test", "Test", "en"),
            source: BibleSource::Url {
                url: "memory".to_string(),
            },
            format: BibleSourceFormat::UsfmZip {
                book_name_overrides: Default::default(),
            },
        }
    }

    #[tokio::test]
    async fn stores_passages_via_repository() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let provider = SinglePayloadProvider {
            payload: make_usfm_archive(),
            url: "memory".to_string(),
        };
        let service = BibleIngestionService::new(&repo, provider);
        let summary = service.ingest(&sample_spec()).await.unwrap();
        assert_eq!(summary.translation_code, "en-test");
        assert_eq!(summary.passage_count, 2);

        let translations = repo.list_bible_translations().await.unwrap();
        assert_eq!(translations.len(), 1);
        let translation = &translations[0];
        assert_eq!(translation.code, "en-test");

        let reference = BibleReference::new_with_code("John", "JHN", 43, 3, 16, 16).unwrap();
        let passage = repo
            .find_bible_passage(&translation.code, &reference)
            .await
            .unwrap()
            .expect("passage");
        assert!(passage.text.contains("For God so loved"));
    }

    #[tokio::test]
    async fn ingest_default_translations_uses_specs() {
        let repo = Repository::connect_in_memory().await.unwrap();

        // Write mock bible archives to temp files and set env vars
        let tmp = tempfile::tempdir().unwrap();

        let kjv_path = tmp.path().join("kjv.usfm.zip");
        std::fs::write(&kjv_path, make_usfm_archive()).unwrap();
        std::env::set_var("PRESENTER_BIBLE_KJV", &kjv_path);

        let seb_path = tmp.path().join("seb.bbl.mybible.zip");
        std::fs::write(&seb_path, make_mysword_archive()).unwrap();
        std::env::set_var("PRESENTER_BIBLE_SEB", &seb_path);

        let roh_path = tmp.path().join("roh.bbl.mybible.zip");
        std::fs::write(&roh_path, make_mysword_archive()).unwrap();
        std::env::set_var("PRESENTER_BIBLE_ROHACEK", &roh_path);

        let sevp_path = tmp.path().join("sevp.obohu.mybible.zip");
        std::fs::write(&sevp_path, make_obohu_archive("SEVP.SQLite3")).unwrap();
        std::env::set_var("PRESENTER_BIBLE_SEVP", &sevp_path);

        let mil_path = tmp.path().join("mil.bbl.mybible.zip");
        std::fs::write(&mil_path, make_mysword_archive()).unwrap();
        std::env::set_var("PRESENTER_BIBLE_MILOST", &mil_path);

        // Provider is unused since all specs are LocalFile, but required by type
        let provider = MapProvider {
            payloads: HashMap::new(),
        };
        let service = BibleIngestionService::new(&repo, provider);

        let available_specs: Vec<_> = default_translation_specs()
            .into_iter()
            .filter(|spec| spec.source.is_available())
            .collect();
        assert_eq!(
            available_specs.len(),
            5,
            "all 5 translations should be available"
        );
        let summaries = service.ingest_specs(available_specs).await.unwrap();
        let codes: HashSet<_> = summaries
            .into_iter()
            .map(|summary| summary.translation_code)
            .collect();
        assert!(codes.contains("eng-kjv"));
        assert!(codes.contains("slk-seb"));
        assert!(codes.contains("slk-roh"));
        assert!(codes.contains("slk-sevp"));
        assert!(codes.contains("slk-mil"));

        // Clean up env vars
        std::env::remove_var("PRESENTER_BIBLE_KJV");
        std::env::remove_var("PRESENTER_BIBLE_SEB");
        std::env::remove_var("PRESENTER_BIBLE_ROHACEK");
        std::env::remove_var("PRESENTER_BIBLE_SEVP");
        std::env::remove_var("PRESENTER_BIBLE_MILOST");
    }
}
