//! Where a Strong's number stands (#851 D4, spec §11): the verse count of a
//! number, its counts per book and one page of its verses with their markup.

use anyhow::Context;
use presenter_core::bible::{BookScope, VerseMarkup};
use sea_orm::{DatabaseBackend, FromQueryResult, Statement};
use tracing::instrument;

use crate::repository::Repository;

const SQLITE: DatabaseBackend = DatabaseBackend::Sqlite;

/// One verse holding a number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrongsVerse {
    pub book_code: String,
    pub book_number: u16,
    pub book: String,
    pub chapter: u16,
    pub verse: u16,
    pub text: String,
    pub markup: Option<VerseMarkup>,
}

#[derive(FromQueryResult)]
struct CountRow {
    count: i64,
}

#[derive(FromQueryResult)]
struct BookCountRow {
    book_code: String,
    count: i64,
}

#[derive(FromQueryResult)]
struct VerseRow {
    book_code: String,
    book_number: i32,
    book: String,
    chapter: i32,
    verse: i32,
    content: String,
    markup_json: Option<String>,
}

impl VerseRow {
    fn into_verse(self) -> anyhow::Result<StrongsVerse> {
        let markup = self
            .markup_json
            .map(|json| serde_json::from_str(&json).context("stored markup"))
            .transpose()?;
        Ok(StrongsVerse {
            book_number: u16::try_from(self.book_number).context("book number")?,
            chapter: u16::try_from(self.chapter).context("chapter number")?,
            verse: u16::try_from(self.verse).context("verse number")?,
            book_code: self.book_code,
            book: self.book,
            text: self.content,
            markup,
        })
    }
}

impl Repository {
    /// How many verses of the scope hold the number.
    #[instrument(skip(self))]
    pub async fn bible_strongs_verse_count(
        &self,
        translation_code: &str,
        strongs: &str,
        scope: BookScope,
    ) -> anyhow::Result<u64> {
        let (first_book, last_book) = scope.book_numbers();
        let row = CountRow::find_by_statement(Statement::from_sql_and_values(
            SQLITE,
            r#"SELECT COUNT(*) AS "count" FROM "bible_strongs_occurrences"
               WHERE "translation_code" = ? AND "strongs" = ?
                 AND "book_number" BETWEEN ? AND ?"#,
            [
                translation_code.into(),
                strongs.into(),
                i32::from(first_book).into(),
                i32::from(last_book).into(),
            ],
        ))
        .one(self.connection())
        .await?;
        let count = row.map_or(0, |row| row.count);
        u64::try_from(count).context("verse count")
    }

    /// The verse count per book, in canonical order; books without the number
    /// are left out.
    #[instrument(skip(self))]
    pub async fn bible_strongs_book_counts(
        &self,
        translation_code: &str,
        strongs: &str,
    ) -> anyhow::Result<Vec<(String, u64)>> {
        let rows = BookCountRow::find_by_statement(Statement::from_sql_and_values(
            SQLITE,
            r#"SELECT "book_code", COUNT(*) AS "count" FROM "bible_strongs_occurrences"
               WHERE "translation_code" = ? AND "strongs" = ?
               GROUP BY "book_number", "book_code" ORDER BY "book_number""#,
            [translation_code.into(), strongs.into()],
        ))
        .all(self.connection())
        .await?;
        rows.into_iter()
            .map(|row| Ok((row.book_code, u64::try_from(row.count).context("count")?)))
            .collect()
    }

    /// One page of the scope's verses holding the number, in canonical order.
    #[instrument(skip(self))]
    pub async fn bible_strongs_verses(
        &self,
        translation_code: &str,
        strongs: &str,
        scope: BookScope,
        offset: u32,
        limit: u32,
    ) -> anyhow::Result<Vec<StrongsVerse>> {
        let (first_book, last_book) = scope.book_numbers();
        let rows = VerseRow::find_by_statement(Statement::from_sql_and_values(
            SQLITE,
            r#"SELECT o."book_code", o."book_number", p."book", o."chapter", o."verse",
                      p."content", m."markup_json"
               FROM "bible_strongs_occurrences" o
               JOIN "bible_passages" p
                 ON p."translation_code" = o."translation_code"
                AND p."book_code" = o."book_code"
                AND p."chapter" = o."chapter"
                AND p."verse_start" = o."verse"
               LEFT JOIN "bible_passage_markup" m ON m."passage_id" = p."id"
               WHERE o."translation_code" = ? AND o."strongs" = ?
                 AND o."book_number" BETWEEN ? AND ?
               ORDER BY o."book_number", o."chapter", o."verse"
               LIMIT ? OFFSET ?"#,
            [
                translation_code.into(),
                strongs.into(),
                i32::from(first_book).into(),
                i32::from(last_book).into(),
                i64::from(limit).into(),
                i64::from(offset).into(),
            ],
        ))
        .all(self.connection())
        .await?;
        rows.into_iter().map(VerseRow::into_verse).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::bible::{BibleIngestionBatch, BiblePassage, BibleReference, MarkupSegment};
    use presenter_core::BibleTranslation;

    fn word(text: &str, strongs: &str) -> MarkupSegment {
        MarkupSegment {
            text: text.to_string(),
            strongs: vec![strongs.to_string()],
            ..MarkupSegment::default()
        }
    }

    fn place(book: &str, code: &str, number: u16, verse: u16, strongs: &str) -> BiblePassage {
        let reference =
            BibleReference::new_with_code(book, code, number, 1, verse, verse).expect("reference");
        let translation = BibleTranslation::new("eng-kjv", "KJV", "en");
        BiblePassage::new(reference, translation, format!("word {strongs}"))
    }

    /// G26 in Genesis 1:2, Matthew 1:1, Matthew 1:3; G27 in Matthew 1:2.
    async fn seeded() -> Repository {
        let repo = Repository::connect_in_memory().await.expect("repository");
        let passages = vec![
            place("Genesis", "GEN", 1, 2, "G26"),
            place("Matthew", "MAT", 40, 1, "G26"),
            place("Matthew", "MAT", 40, 2, "G27"),
            place("Matthew", "MAT", 40, 3, "G26"),
        ];
        let translation = BibleTranslation::new("eng-kjv", "KJV", "en");
        let markup = passages
            .iter()
            .map(|passage| {
                let strongs = passage.text.trim_start_matches("word ");
                Some(VerseMarkup(vec![
                    word("word", strongs),
                    MarkupSegment::text(format!(" {strongs}")),
                ]))
            })
            .collect();
        let batch = BibleIngestionBatch::new(translation, passages)
            .expect("batch")
            .with_markup(markup)
            .expect("markup");
        repo.replace_bible_translation_passages(&batch)
            .await
            .expect("seed");
        repo
    }

    #[tokio::test]
    async fn the_counts_follow_the_scope_and_the_books_do_not() {
        let repo = seeded().await;
        let count = |scope| repo.bible_strongs_verse_count("eng-kjv", "G26", scope);
        assert_eq!(count(BookScope::All).await.unwrap(), 3);
        assert_eq!(count(BookScope::OldTestament).await.unwrap(), 1);
        assert_eq!(count(BookScope::NewTestament).await.unwrap(), 2);
        assert_eq!(count(BookScope::Book(58)).await.unwrap(), 0);
        assert_eq!(
            repo.bible_strongs_book_counts("eng-kjv", "G26")
                .await
                .unwrap(),
            vec![("GEN".to_string(), 1), ("MAT".to_string(), 2)]
        );
    }

    #[tokio::test]
    async fn a_page_is_canonical_and_carries_the_markup() {
        let repo = seeded().await;
        let page = repo
            .bible_strongs_verses("eng-kjv", "G26", BookScope::All, 1, 1)
            .await
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!((page[0].book_code.as_str(), page[0].verse), ("MAT", 1));
        assert_eq!(page[0].book, "Matthew");
        assert_eq!(page[0].text, "word G26");
        let markup = page[0].markup.as_ref().expect("markup");
        assert_eq!(markup.0[0].strongs, vec!["G26".to_string()]);

        let scoped = repo
            .bible_strongs_verses("eng-kjv", "G26", BookScope::NewTestament, 0, 10)
            .await
            .unwrap();
        let verses: Vec<u16> = scoped.iter().map(|verse| verse.verse).collect();
        assert_eq!(verses, vec![1, 3]);
    }
}
