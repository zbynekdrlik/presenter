use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;
use unicode_normalization::{is_nfc, UnicodeNormalization};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// All UPDATE statements run in a single transaction so the migration is
    /// atomic: either every NFD row is rewritten to NFC or none are.
    /// sea-orm-migration wraps `up()` in a SQLite transaction by default,
    /// satisfying the spec's atomicity requirement (#305 risk table).
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        for (table, columns) in TARGETS {
            normalize_table(db, table, columns).await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // NFC is canonical; reverting to NFD has no value. No-op.
        Ok(())
    }
}

const TARGETS: &[(&str, &[&str])] = &[
    ("libraries", &["name"]),
    ("presentations", &["name"]),
    (
        "slides",
        &["worship_main", "worship_translate", "worship_stage"],
    ),
];

async fn normalize_table<C: ConnectionTrait>(
    db: &C,
    table: &str,
    columns: &[&str],
) -> Result<(), DbErr> {
    let projection = std::iter::once("id".to_string())
        .chain(columns.iter().map(|c| (*c).to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    let select_sql = format!("SELECT {projection} FROM {table}");
    let rows = db
        .query_all(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            select_sql,
        ))
        .await?;

    for row in rows {
        let id: String = row.try_get("", "id")?;
        let mut updates: Vec<(String, String)> = Vec::new();
        for column in columns {
            let value: String = row.try_get("", column)?;
            if !is_nfc(&value) {
                let nfc: String = value.nfc().collect();
                updates.push(((*column).to_string(), nfc));
            }
        }
        if updates.is_empty() {
            continue;
        }
        let set_clause = updates
            .iter()
            .map(|(col, _)| format!("{col} = ?"))
            .collect::<Vec<_>>()
            .join(", ");
        let update_sql = format!("UPDATE {table} SET {set_clause} WHERE id = ?");
        let mut values: Vec<sea_orm::Value> = updates.into_iter().map(|(_, v)| v.into()).collect();
        values.push(id.into());
        db.execute(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            update_sql,
            values,
        ))
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DbBackend, Statement};

    async fn setup_db() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        for ddl in &[
            "CREATE TABLE libraries (id TEXT PRIMARY KEY, name TEXT NOT NULL, search_name TEXT NOT NULL DEFAULT '')",
            "CREATE TABLE presentations (id TEXT PRIMARY KEY, library_id TEXT NOT NULL, name TEXT NOT NULL, search_name TEXT NOT NULL DEFAULT '')",
            "CREATE TABLE slides (id TEXT PRIMARY KEY, presentation_id TEXT NOT NULL, worship_main TEXT NOT NULL DEFAULT '', worship_translate TEXT NOT NULL DEFAULT '', worship_stage TEXT NOT NULL DEFAULT '')",
        ] {
            db.execute(Statement::from_string(
                DbBackend::Sqlite,
                (*ddl).to_string(),
            ))
            .await
            .expect("ddl");
        }
        db
    }

    async fn insert_row(db: &sea_orm::DatabaseConnection, sql: &str) {
        db.execute(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
            .expect("insert");
    }

    async fn fetch_one_string(db: &sea_orm::DatabaseConnection, sql: &str) -> String {
        let row = db
            .query_one(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
            .expect("query")
            .expect("row");
        row.try_get_by_index::<String>(0).expect("string col")
    }

    #[tokio::test]
    async fn migration_normalizes_nfd_rows_and_is_idempotent() {
        let db = setup_db().await;

        // Seed: NFD library, NFC library, NFD presentation, NFD slide fields.
        insert_row(
            &db,
            "INSERT INTO libraries (id, name) VALUES ('lib-nfd', 'TYz\u{30c}MY')",
        )
        .await;
        insert_row(
            &db,
            "INSERT INTO libraries (id, name) VALUES ('lib-nfc', 'CLEAN')",
        )
        .await;
        insert_row(
            &db,
            "INSERT INTO presentations (id, library_id, name) VALUES \
             ('p-nfd', 'lib-nfd', 'Po Tebe Pane z\u{30c}i\u{301}znim')",
        )
        .await;
        insert_row(
            &db,
            "INSERT INTO slides (id, presentation_id, worship_main, worship_translate, worship_stage) VALUES \
             ('s-nfd', 'p-nfd', 'z\u{30c}', 'CLEAN', 'i\u{301}')",
        )
        .await;

        for (table, columns) in TARGETS {
            super::normalize_table(&db, table, columns)
                .await
                .expect("up");
        }

        // After: every row's text fields are NFC.
        assert_eq!(
            fetch_one_string(&db, "SELECT name FROM libraries WHERE id='lib-nfd'").await,
            "TY\u{17e}MY"
        );
        assert_eq!(
            fetch_one_string(&db, "SELECT name FROM libraries WHERE id='lib-nfc'").await,
            "CLEAN"
        );
        assert_eq!(
            fetch_one_string(&db, "SELECT name FROM presentations WHERE id='p-nfd'").await,
            "Po Tebe Pane \u{17e}\u{ed}znim"
        );
        assert_eq!(
            fetch_one_string(&db, "SELECT worship_main FROM slides WHERE id='s-nfd'").await,
            "\u{17e}"
        );
        assert_eq!(
            fetch_one_string(&db, "SELECT worship_translate FROM slides WHERE id='s-nfd'").await,
            "CLEAN"
        );
        assert_eq!(
            fetch_one_string(&db, "SELECT worship_stage FROM slides WHERE id='s-nfd'").await,
            "\u{ed}"
        );

        // Idempotency: rerun is a no-op.
        for (table, columns) in TARGETS {
            super::normalize_table(&db, table, columns)
                .await
                .expect("rerun");
        }
        assert_eq!(
            fetch_one_string(&db, "SELECT name FROM libraries WHERE id='lib-nfd'").await,
            "TY\u{17e}MY"
        );
        assert_eq!(
            fetch_one_string(&db, "SELECT worship_main FROM slides WHERE id='s-nfd'").await,
            "\u{17e}"
        );
    }
}
