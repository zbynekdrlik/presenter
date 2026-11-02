//! Verse markup and Strong's occurrences (#851 D2b, spec §5.3). Written only
//! inside the import transaction; `bible_passage` stays untouched, so search
//! and passage reads load no markup.

use anyhow::Context;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, FromQueryResult, Statement, Value,
};
use tracing::instrument;

use crate::repository::Repository;
use presenter_core::bible::VerseMarkup;

const SQLITE: DatabaseBackend = DatabaseBackend::Sqlite;
/// Rows per INSERT: well under SQLite's bound-variable limit at 7 columns.
const MARKUP_CHUNK: usize = 500;

/// A freshly inserted passage that has markup.
pub(super) struct MarkupRow {
    pub passage_id: String,
    pub book_code: String,
    pub book_number: i32,
    pub chapter: i32,
    pub verse: i32,
    pub markup: VerseMarkup,
}

pub(super) async fn insert_chunk(
    txn: &DatabaseTransaction,
    head: &str,
    tail: &str,
    columns: usize,
    values: Vec<Value>,
) -> anyhow::Result<()> {
    let row = format!("({})", vec!["?"; columns].join(","));
    let rows = vec![row; values.len() / columns].join(",");
    txn.execute(Statement::from_sql_and_values(
        SQLITE,
        format!("{head} VALUES {rows} {tail}"),
        values,
    ))
    .await?;
    Ok(())
}

/// Both tables for one translation. Returns (markup rows, occurrence rows).
pub(super) async fn insert_markup(
    txn: &DatabaseTransaction,
    translation_code: &str,
    rows: &[MarkupRow],
) -> anyhow::Result<(usize, usize)> {
    for chunk in rows.chunks(MARKUP_CHUNK) {
        let mut values = Vec::with_capacity(chunk.len() * 2);
        for row in chunk {
            let json = serde_json::to_string(&row.markup).context("serialize verse markup")?;
            values.push(row.passage_id.clone().into());
            values.push(json.into());
        }
        insert_chunk(
            txn,
            r#"INSERT INTO "bible_passage_markup" ("passage_id", "markup_json")"#,
            "",
            2,
            values,
        )
        .await?;
    }

    let occurrences: Vec<(&MarkupRow, String, u16)> = rows
        .iter()
        .flat_map(|row| {
            let counts = row.markup.strongs_counts();
            counts.into_iter().map(move |(id, count)| (row, id, count))
        })
        .collect();
    for chunk in occurrences.chunks(MARKUP_CHUNK) {
        let mut values = Vec::with_capacity(chunk.len() * 7);
        for (row, strongs, count) in chunk {
            values.push(translation_code.into());
            values.push(strongs.clone().into());
            values.push(row.book_code.clone().into());
            values.push(row.book_number.into());
            values.push(row.chapter.into());
            values.push(row.verse.into());
            values.push(i32::from(*count).into());
        }
        insert_chunk(
            txn,
            r#"INSERT INTO "bible_strongs_occurrences" ("translation_code", "strongs",
               "book_code", "book_number", "chapter", "verse", "word_count")"#,
            // `\v 1-2` beside `\v 1` is one place: its words add up.
            r#"ON CONFLICT DO UPDATE SET "word_count" = "word_count" + excluded."word_count""#,
            7,
            values,
        )
        .await?;
    }
    Ok((rows.len(), occurrences.len()))
}

#[derive(FromQueryResult)]
struct ChapterMarkupRow {
    verse_start: i32,
    markup_json: String,
}

impl Repository {
    /// The verses of a chapter that have markup, in order.
    #[instrument(skip(self))]
    pub async fn bible_chapter_markup(
        &self,
        translation_code: &str,
        book_code: &str,
        chapter: u16,
    ) -> anyhow::Result<Vec<(u16, VerseMarkup)>> {
        let rows = ChapterMarkupRow::find_by_statement(Statement::from_sql_and_values(
            SQLITE,
            r#"SELECT p."verse_start", m."markup_json" FROM "bible_passage_markup" m
               JOIN "bible_passages" p ON p."id" = m."passage_id"
               WHERE p."translation_code" = ? AND p."book_code" = ? AND p."chapter" = ?
               ORDER BY p."verse_start""#,
            [
                translation_code.into(),
                book_code.into(),
                i32::from(chapter).into(),
            ],
        ))
        .all(self.connection())
        .await?;
        rows.into_iter()
            .map(|row| {
                let verse = u16::try_from(row.verse_start).context("verse number")?;
                let markup = serde_json::from_str(&row.markup_json).context("stored markup")?;
                Ok((verse, markup))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::repository::Repository;
    use presenter_core::bible::{
        BibleIngestionBatch, BiblePassage, BibleReference, MarkupSegment, VerseMarkup,
    };
    use presenter_core::BibleTranslation;
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

    fn translation() -> BibleTranslation {
        BibleTranslation::new("eng-kjv", "KJV", "en")
    }

    fn word(text: &str, strongs: &str) -> MarkupSegment {
        MarkupSegment {
            text: text.to_string(),
            strongs: vec![strongs.to_string()],
            ..MarkupSegment::default()
        }
    }

    fn passage(verse: u16, text: &str) -> BiblePassage {
        let reference =
            BibleReference::new_with_code("Hebrews", "HEB", 58, 11, verse, verse).expect("ref");
        BiblePassage::new(reference, translation(), text.to_string())
    }

    /// Verse 1 tagged (one number twice), verse 2 bare.
    fn batch(first: &str) -> BibleIngestionBatch {
        let tagged = VerseMarkup(vec![
            word(first, "G4102"),
            MarkupSegment::text(" "),
            word("is", "G4102"),
            MarkupSegment::text(" "),
            word("now", "G1161"),
        ]);
        let text = tagged.plain_text();
        BibleIngestionBatch::new(translation(), vec![passage(1, &text), passage(2, "bare")])
            .expect("batch")
            .with_markup(vec![Some(tagged), None])
            .expect("markup")
    }

    async fn count(repo: &Repository, table: &str) -> i64 {
        let row = repo
            .connection()
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!(r#"SELECT count(*) AS n FROM "{table}""#),
            ))
            .await
            .expect("count")
            .expect("row");
        row.try_get("", "n").expect("n")
    }

    async fn occurrence(repo: &Repository, strongs: &str) -> Option<(i32, i32, i32)> {
        let row = repo
            .connection()
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                r#"SELECT "book_number", "verse", "word_count" FROM "bible_strongs_occurrences"
                   WHERE "strongs" = ?"#,
                [strongs.into()],
            ))
            .await
            .expect("query")?;
        Some((
            row.try_get("", "book_number").expect("book"),
            row.try_get("", "verse").expect("verse"),
            row.try_get("", "word_count").expect("count"),
        ))
    }

    #[tokio::test]
    async fn an_import_stores_markup_and_occurrences_for_tagged_verses_only() {
        let repo = Repository::connect_in_memory().await.expect("repo");
        repo.replace_bible_translation_passages(&batch("faith"))
            .await
            .expect("import");

        let chapter = repo
            .bible_chapter_markup("eng-kjv", "HEB", 11)
            .await
            .expect("read");
        assert_eq!(chapter.len(), 1);
        assert_eq!(chapter[0].0, 1);
        assert_eq!(chapter[0].1.plain_text(), "faith is now");
        assert_eq!(occurrence(&repo, "G4102").await, Some((58, 1, 2)));
        assert_eq!(occurrence(&repo, "G1161").await, Some((58, 1, 1)));
        assert_eq!(count(&repo, "bible_strongs_occurrences").await, 2);
    }

    #[tokio::test]
    async fn a_reimport_leaves_no_row_of_the_old_import() {
        let repo = Repository::connect_in_memory().await.expect("repo");
        repo.replace_bible_translation_passages(&batch("faith"))
            .await
            .expect("first");
        repo.replace_bible_translation_passages(&batch("hope"))
            .await
            .expect("second");

        let chapter = repo
            .bible_chapter_markup("eng-kjv", "HEB", 11)
            .await
            .expect("read");
        assert_eq!(chapter[0].1.plain_text(), "hope is now");
        assert_eq!(count(&repo, "bible_passage_markup").await, 1);
        assert_eq!(count(&repo, "bible_strongs_occurrences").await, 2);
    }

    #[tokio::test]
    async fn a_batch_without_markup_clears_the_old_rows() {
        let repo = Repository::connect_in_memory().await.expect("repo");
        repo.replace_bible_translation_passages(&batch("faith"))
            .await
            .expect("first");
        let plain = BibleIngestionBatch::new(translation(), vec![passage(1, "plain")])
            .expect("plain batch");
        repo.replace_bible_translation_passages(&plain)
            .await
            .expect("second");
        assert_eq!(count(&repo, "bible_passage_markup").await, 0);
        assert_eq!(count(&repo, "bible_strongs_occurrences").await, 0);
    }

    #[tokio::test]
    async fn rows_past_one_insert_chunk_all_arrive() {
        let repo = Repository::connect_in_memory().await.expect("repo");
        let verses = 1..=1_210_u16;
        let passages = verses
            .clone()
            .map(|verse| passage(verse, "x"))
            .collect::<Vec<_>>();
        let tagged = VerseMarkup(vec![word("x", "G1")]);
        let batch = BibleIngestionBatch::new(translation(), passages)
            .expect("batch")
            .with_markup(verses.map(|_| Some(tagged.clone())).collect())
            .expect("markup");
        repo.replace_bible_translation_passages(&batch)
            .await
            .expect("import");
        assert_eq!(count(&repo, "bible_passage_markup").await, 1_210);
        assert_eq!(count(&repo, "bible_strongs_occurrences").await, 1_210);
    }

    #[tokio::test]
    async fn overlapping_verses_add_their_words_up() {
        let repo = Repository::connect_in_memory().await.expect("repo");
        let span = |end: u16| {
            let reference =
                BibleReference::new_with_code("Hebrews", "HEB", 58, 11, 1, end).expect("ref");
            BiblePassage::new(reference, translation(), "x".to_string())
        };
        let tagged = VerseMarkup(vec![word("x", "G1")]);
        let batch = BibleIngestionBatch::new(translation(), vec![span(1), span(2)])
            .expect("batch")
            .with_markup(vec![Some(tagged.clone()), Some(tagged)])
            .expect("markup");
        repo.replace_bible_translation_passages(&batch)
            .await
            .expect("import");
        assert_eq!(occurrence(&repo, "G1").await, Some((58, 1, 2)));
    }

    #[tokio::test]
    async fn deleting_a_translation_takes_its_markup_with_it() {
        let repo = Repository::connect_in_memory().await.expect("repo");
        repo.replace_bible_translation_passages(&batch("faith"))
            .await
            .expect("import");
        assert!(repo
            .delete_bible_translation("eng-kjv")
            .await
            .expect("delete"));
        assert_eq!(count(&repo, "bible_passage_markup").await, 0);
        assert_eq!(count(&repo, "bible_strongs_occurrences").await, 0);
    }

    #[tokio::test]
    async fn another_translation_keeps_its_markup_through_a_reimport() {
        let repo = Repository::connect_in_memory().await.expect("repo");
        repo.replace_bible_translation_passages(&batch("faith"))
            .await
            .expect("kjv");
        let other = BibleTranslation::new("eng-two", "Two", "en");
        let reference = BibleReference::new_with_code("Hebrews", "HEB", 58, 11, 1, 1).expect("ref");
        let verse = VerseMarkup(vec![MarkupSegment::note("n"), MarkupSegment::text("x")]);
        let second = BibleIngestionBatch::new(
            other.clone(),
            vec![BiblePassage::new(reference, other, "x".to_string())],
        )
        .expect("batch")
        .with_markup(vec![Some(verse)])
        .expect("markup");
        repo.replace_bible_translation_passages(&second)
            .await
            .expect("two");
        repo.replace_bible_translation_passages(&batch("hope"))
            .await
            .expect("kjv again");
        assert_eq!(
            repo.bible_chapter_markup("eng-two", "HEB", 11)
                .await
                .expect("read")
                .len(),
            1
        );
    }
}
