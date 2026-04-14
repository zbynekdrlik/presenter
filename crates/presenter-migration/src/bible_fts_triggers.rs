//! FTS5 trigger SQL for `bible_passage_fts`, shared by the initial migration
//! and the fast-import path in `presenter-persistence`. Keeping both sites on
//! the same source prevents silent drift if a column is ever added to the FTS
//! table.

pub const TRIGGER_NAMES: [&str; 3] = [
    "bible_passage_fts_insert",
    "bible_passage_fts_delete",
    "bible_passage_fts_update",
];

pub const INSERT_TRIGGER_SQL: &str = "CREATE TRIGGER bible_passage_fts_insert \
     AFTER INSERT ON bible_passages BEGIN \
        INSERT INTO bible_passage_fts(passage_id, translation_code, book, content) \
        VALUES (new.id, new.translation_code, new.book, new.content); \
     END";

pub const DELETE_TRIGGER_SQL: &str = "CREATE TRIGGER bible_passage_fts_delete \
     AFTER DELETE ON bible_passages BEGIN \
        DELETE FROM bible_passage_fts WHERE passage_id = old.id; \
     END";

pub const UPDATE_TRIGGER_SQL: &str = "CREATE TRIGGER bible_passage_fts_update \
     AFTER UPDATE ON bible_passages BEGIN \
        DELETE FROM bible_passage_fts WHERE passage_id = old.id; \
        INSERT INTO bible_passage_fts(passage_id, translation_code, book, content) \
        VALUES (new.id, new.translation_code, new.book, new.content); \
     END";

/// The three CREATE TRIGGER statements in insert, delete, update order.
pub const CREATE_TRIGGER_STATEMENTS: [&str; 3] =
    [INSERT_TRIGGER_SQL, DELETE_TRIGGER_SQL, UPDATE_TRIGGER_SQL];
