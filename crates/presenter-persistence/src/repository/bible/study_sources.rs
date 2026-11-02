//! Strong's dictionaries and cross-references (#851 D3a, D3b, spec §5.6). A
//! source is replaced whole in one transaction: its row goes, the cascades take
//! its rows, the new ones go in. Readers see the old data until the commit.

use anyhow::Context;
use chrono::Utc;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, FromQueryResult, Statement,
    TransactionTrait, Value,
};
use tracing::{info, instrument};

use super::markup::insert_chunk;
use crate::repository::Repository;
use presenter_core::bible::{CrossReference, LexiconEntry, StudySource};

const SQLITE: DatabaseBackend = DatabaseBackend::Sqlite;
/// Rows per INSERT: well under SQLite's bound-variable limit at 11 columns.
const CHUNK: usize = 500;

async fn renew_source(txn: &DatabaseTransaction, source: &StudySource) -> anyhow::Result<()> {
    txn.execute(Statement::from_sql_and_values(
        SQLITE,
        r#"DELETE FROM "bible_study_sources" WHERE "code" = ?"#,
        [source.code.clone().into()],
    ))
    .await?;
    txn.execute(Statement::from_sql_and_values(
        SQLITE,
        r#"INSERT INTO "bible_study_sources"
           ("code", "name", "license", "source_digest", "imported_at") VALUES (?, ?, ?, ?, ?)"#,
        [
            source.code.clone().into(),
            source.name.clone().into(),
            source.license.clone().into(),
            source.source_digest.clone().into(),
            Utc::now().to_rfc3339().into(),
        ],
    ))
    .await?;
    Ok(())
}

#[derive(FromQueryResult)]
struct LexiconRow {
    strongs: String,
    lemma: String,
    transliteration: String,
    pronunciation: String,
    definition: String,
    derivation: String,
    kjv_renderings: String,
    related: String,
}

#[derive(FromQueryResult)]
struct CrossReferenceRow {
    from_book_number: i32,
    from_chapter: i32,
    from_verse: i32,
    to_book_number: i32,
    to_chapter: i32,
    to_verse: i32,
    to_book_number_end: i32,
    to_chapter_end: i32,
    to_verse_end: i32,
    votes: i32,
}

impl Repository {
    /// The digest a source was last imported with.
    pub async fn bible_study_source_digest(&self, code: &str) -> anyhow::Result<Option<String>> {
        #[derive(FromQueryResult)]
        struct Row {
            source_digest: String,
        }
        let row = Row::find_by_statement(Statement::from_sql_and_values(
            SQLITE,
            r#"SELECT "source_digest" FROM "bible_study_sources" WHERE "code" = ?"#,
            [code.into()],
        ))
        .one(&self.db)
        .await?;
        Ok(row.map(|row| row.source_digest))
    }

    #[instrument(skip_all, fields(code = %source.code, entries = entries.len()))]
    pub async fn replace_bible_lexicon(
        &self,
        source: &StudySource,
        entries: &[LexiconEntry],
    ) -> anyhow::Result<()> {
        let started = std::time::Instant::now();
        let txn = self.db.begin().await?;
        renew_source(&txn, source).await?;
        for chunk in entries.chunks(CHUNK) {
            let mut values: Vec<Value> = Vec::with_capacity(chunk.len() * 9);
            for entry in chunk {
                values.push(entry.strongs.clone().into());
                values.push(source.code.clone().into());
                values.push(entry.lemma.clone().into());
                values.push(entry.transliteration.clone().into());
                values.push(entry.pronunciation.clone().into());
                values.push(entry.definition.clone().into());
                values.push(entry.derivation.clone().into());
                values.push(entry.kjv_renderings.clone().into());
                values.push(
                    serde_json::to_string(&entry.related)
                        .context("serialize related ids")?
                        .into(),
                );
            }
            insert_chunk(
                &txn,
                r#"INSERT INTO "bible_lexicon_entries" ("strongs", "source_code", "lemma",
                   "transliteration", "pronunciation", "definition", "derivation",
                   "kjv_renderings", "related")"#,
                "",
                9,
                values,
            )
            .await?;
        }
        txn.commit().await?;
        info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "study source replaced"
        );
        Ok(())
    }

    #[instrument(skip_all, fields(code = %source.code, rows = rows.len()))]
    pub async fn replace_bible_cross_references(
        &self,
        source: &StudySource,
        rows: &[CrossReference],
    ) -> anyhow::Result<()> {
        let started = std::time::Instant::now();
        let txn = self.db.begin().await?;
        renew_source(&txn, source).await?;
        for chunk in rows.chunks(CHUNK) {
            let mut values: Vec<Value> = Vec::with_capacity(chunk.len() * 11);
            for row in chunk {
                values.push(source.code.clone().into());
                for number in [
                    row.from_book,
                    row.from_chapter,
                    row.from_verse,
                    row.to_book,
                    row.to_chapter,
                    row.to_verse,
                    row.to_book_end,
                    row.to_chapter_end,
                    row.to_verse_end,
                    row.votes,
                ] {
                    values.push(number.into());
                }
            }
            insert_chunk(
                &txn,
                r#"INSERT INTO "bible_cross_references" ("source_code", "from_book_number",
                   "from_chapter", "from_verse", "to_book_number", "to_chapter", "to_verse",
                   "to_book_number_end", "to_chapter_end", "to_verse_end", "votes")"#,
                "",
                11,
                values,
            )
            .await?;
        }
        txn.commit().await?;
        info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "study source replaced"
        );
        Ok(())
    }

    pub async fn bible_lexicon_entry(&self, strongs: &str) -> anyhow::Result<Option<LexiconEntry>> {
        let row = LexiconRow::find_by_statement(Statement::from_sql_and_values(
            SQLITE,
            r#"SELECT "strongs", "lemma", "transliteration", "pronunciation", "definition",
                      "derivation", "kjv_renderings", "related"
               FROM "bible_lexicon_entries" WHERE "strongs" = ?"#,
            [strongs.into()],
        ))
        .one(&self.db)
        .await?;
        row.map(|row| {
            Ok(LexiconEntry {
                related: serde_json::from_str(&row.related).context("parse related ids")?,
                strongs: row.strongs,
                lemma: row.lemma,
                transliteration: row.transliteration,
                pronunciation: row.pronunciation,
                definition: row.definition,
                derivation: row.derivation,
                kjv_renderings: row.kjv_renderings,
            })
        })
        .transpose()
    }

    /// The references of one verse, the most voted first.
    pub async fn bible_cross_references_from(
        &self,
        book: i32,
        chapter: i32,
        verse: i32,
    ) -> anyhow::Result<Vec<CrossReference>> {
        let rows = CrossReferenceRow::find_by_statement(Statement::from_sql_and_values(
            SQLITE,
            r#"SELECT "from_book_number", "from_chapter", "from_verse", "to_book_number",
                      "to_chapter", "to_verse", "to_book_number_end", "to_chapter_end",
                      "to_verse_end", "votes"
               FROM "bible_cross_references"
               WHERE "from_book_number" = ? AND "from_chapter" = ? AND "from_verse" = ?
               ORDER BY "votes" DESC, "to_book_number", "to_chapter", "to_verse""#,
            [book.into(), chapter.into(), verse.into()],
        ))
        .all(&self.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| CrossReference {
                from_book: row.from_book_number,
                from_chapter: row.from_chapter,
                from_verse: row.from_verse,
                to_book: row.to_book_number,
                to_chapter: row.to_chapter,
                to_verse: row.to_verse,
                to_book_end: row.to_book_number_end,
                to_chapter_end: row.to_chapter_end,
                to_verse_end: row.to_verse_end,
                votes: row.votes,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(code: &str, digest: &str) -> StudySource {
        StudySource {
            code: code.to_string(),
            name: "Test".to_string(),
            license: "Public Domain".to_string(),
            source_digest: digest.to_string(),
        }
    }

    fn entry(strongs: &str, definition: &str) -> LexiconEntry {
        LexiconEntry {
            strongs: strongs.to_string(),
            definition: definition.to_string(),
            related: vec!["H433".to_string()],
            ..LexiconEntry::default()
        }
    }

    fn reference(to_verse: i32, votes: i32) -> CrossReference {
        CrossReference {
            from_book: 1,
            from_chapter: 1,
            from_verse: 1,
            to_book: 24,
            to_chapter: 32,
            to_verse,
            to_book_end: 25,
            to_chapter_end: 33,
            to_verse_end: 2,
            votes,
        }
    }

    #[tokio::test]
    async fn a_new_import_replaces_the_source_and_leaves_the_others() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let hebrew = source("strongs-hebrew", "one");
        let greek = source("strongs-greek", "g");
        repo.replace_bible_lexicon(&hebrew, &[entry("H1", "old"), entry("H2", "gone")])
            .await
            .unwrap();
        repo.replace_bible_lexicon(&greek, &[entry("G1", "kept")])
            .await
            .unwrap();
        let again = source("strongs-hebrew", "two");
        repo.replace_bible_lexicon(&again, &[entry("H1", "new")])
            .await
            .unwrap();

        let h1 = repo.bible_lexicon_entry("H1").await.unwrap().unwrap();
        assert_eq!(h1.definition, "new");
        assert_eq!(h1.related, ["H433"]);
        assert!(repo.bible_lexicon_entry("H2").await.unwrap().is_none());
        assert!(repo.bible_lexicon_entry("G1").await.unwrap().is_some());
        let digest = repo.bible_study_source_digest("strongs-hebrew").await;
        assert_eq!(digest.unwrap().as_deref(), Some("two"));
        let missing = repo.bible_study_source_digest("nothing").await;
        assert_eq!(missing.unwrap(), None);
    }

    #[tokio::test]
    async fn references_come_back_whole_and_by_votes() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let rows = [reference(17, 3), reference(18, 90)];
        repo.replace_bible_cross_references(&source("openbible-xref", "a"), &rows)
            .await
            .unwrap();
        let read = repo.bible_cross_references_from(1, 1, 1).await.unwrap();
        assert_eq!(read, [rows[1], rows[0]]);
        assert!(repo
            .bible_cross_references_from(1, 1, 2)
            .await
            .unwrap()
            .is_empty());

        repo.replace_bible_cross_references(&source("openbible-xref", "b"), &rows[..1])
            .await
            .unwrap();
        let read = repo.bible_cross_references_from(1, 1, 1).await.unwrap();
        assert_eq!(read, [rows[0]]);
    }

    #[tokio::test]
    async fn a_failed_import_keeps_what_was_there() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let first = source("strongs-hebrew", "one");
        repo.replace_bible_lexicon(&first, &[entry("H1", "old")])
            .await
            .unwrap();
        let twice = [entry("H2", "x"), entry("H2", "x")];
        let second = source("strongs-hebrew", "two");
        assert!(repo.replace_bible_lexicon(&second, &twice).await.is_err());
        let held = repo.bible_lexicon_entry("H1").await.unwrap().unwrap();
        assert_eq!(held.definition, "old");
        let digest = repo.bible_study_source_digest("strongs-hebrew").await;
        assert_eq!(digest.unwrap().as_deref(), Some("one"));
    }
}
