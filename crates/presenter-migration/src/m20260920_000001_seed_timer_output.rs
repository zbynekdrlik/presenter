//! #785: seed the `timer` stream-graphics OUTPUT so the OBS countdown overlay
//! becomes designable in the same stream editor as everything else. Replaces the
//! old hand-coded SSR `/overlays/timer` page (deleted in this PR; `/overlays/timer`
//! now 302-redirects to `/stream/timer`).
//!
//! Seeds, additively + idempotently on top of `m20260820_000001`'s stream tables:
//! - output `slug='timer'` ("Timer overlay"),
//! - one BASE scene "Timer" for it, set as the output's active scene,
//! - one `countdown` element styled like the retired overlay (Inter 700, ~21.3vh
//!   ≈ the old `12vw`, `letter-spacing 0.08em`, the `0 12px 40px` shadow, centred,
//!   bound to `timer_id=1` = `countdown_to_start`).
//!
//! Idempotent: the output insert is `INSERT OR IGNORE` on the UNIQUE slug; the
//! scene + element inserts are guarded by `NOT EXISTS`, and the active-scene set
//! is guarded by `active_scene_id IS NULL`. So a re-run (or running on a DB where
//! an operator has already edited the timer output) is a no-op that preserves
//! their edits. The element `props` JSON is built from the REAL
//! `presenter_core::StreamElementProps` via serde, so its wire shape is
//! guaranteed to match what `load_output_def` parses (no hand-written JSON drift).

use presenter_core::{ContentTransition, Frame, Shadow, StreamElementProps, TextAlign, TextStyle};
use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, Statement, Value};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// The seed timestamp used for every seeded row (RFC3339, entity-readable — the
/// same explicit-timestamp idiom as `m20260820_000001`'s output seed).
const SEED_TS: &str = "2026-09-20T00:00:00+00:00";

/// Build the seeded countdown element's props, matching the retired SSR overlay's
/// look. Kept as a function so the migration test can assert against the same
/// value the migration inserts.
pub(crate) fn seed_countdown_props() -> StreamElementProps {
    StreamElementProps::Countdown {
        timer_id: 1,
        style: TextStyle {
            font_family: "Inter".to_string(),
            // ~21.3vh ≈ the retired overlay's `font-size: 12vw` on the 16:9 canvas
            // (12% of 1920 = 230.4px = 21.33% of 1080).
            size_pct: 21.3,
            color: "#f8fafc".to_string(),
            weight: 700,
            align: TextAlign::Center,
            line_height: 1.0,
            shadow: Some(Shadow {
                x_px: 0.0,
                y_px: 12.0,
                blur_px: 40.0,
                // rgba(15, 23, 42, 0.55) == #0f172a at alpha 0x8c (0.55*255).
                color: "#0f172a8c".to_string(),
            }),
            letter_spacing_em: Some(0.08),
        },
        frame: Frame {
            x_pct: 5.0,
            y_pct: 30.0,
            w_pct: 90.0,
            h_pct: 40.0,
        },
        // A per-tick fade flickers a countdown (#776), so the seeded element cuts.
        content_transition: ContentTransition::Cut,
        r#box: None,
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // 1) The output row (idempotent on the UNIQUE slug).
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "INSERT OR IGNORE INTO stream_outputs (slug, name, created_at, updated_at) \
                 VALUES ('timer', 'Timer overlay', '{SEED_TS}', '{SEED_TS}')"
            ),
        ))
        .await?;

        // 2) The base scene "Timer" — only if the timer output has no scenes yet.
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "INSERT INTO stream_scenes \
                 (output_id, name, kind, position, is_active, created_at, updated_at) \
                 SELECT o.id, 'Timer', 'base', 0, 0, '{SEED_TS}', '{SEED_TS}' \
                 FROM stream_outputs o \
                 WHERE o.slug = 'timer' \
                 AND NOT EXISTS (SELECT 1 FROM stream_scenes s WHERE s.output_id = o.id)"
            ),
        ))
        .await?;

        // 3) The countdown element — only if the timer output has no elements yet.
        //    The props JSON is serialised from the typed core enum (typo-proof).
        let props_json = serde_json::to_string(&seed_countdown_props())
            .map_err(|e| DbErr::Custom(format!("serialise seed countdown props: {e}")))?;
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "INSERT INTO stream_elements (scene_id, kind, z_order, props, created_at, updated_at) \
                 SELECT s.id, 'countdown', 0, ?1, '{SEED_TS}', '{SEED_TS}' \
                 FROM stream_scenes s \
                 JOIN stream_outputs o ON o.id = s.output_id \
                 WHERE o.slug = 'timer' AND s.kind = 'base' \
                 AND NOT EXISTS ( \
                   SELECT 1 FROM stream_elements e \
                   JOIN stream_scenes s2 ON s2.id = e.scene_id \
                   WHERE s2.output_id = o.id \
                 )"
            ),
            [Value::from(props_json)],
        ))
        .await?;

        // 4) Activate the base scene (only if the output has no active scene yet,
        //    so an operator who later cleared it is not overridden on re-run).
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "UPDATE stream_outputs \
             SET active_scene_id = ( \
               SELECT s.id FROM stream_scenes s \
               WHERE s.output_id = stream_outputs.id AND s.kind = 'base' \
               ORDER BY s.id LIMIT 1 \
             ) \
             WHERE slug = 'timer' AND active_scene_id IS NULL"
                .to_string(),
        ))
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove the seeded timer output; the FK CASCADE on stream_scenes /
        // stream_elements drops its scene + element with it. Guarded on the slug
        // so nothing else is touched.
        manager
            .get_connection()
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "DELETE FROM stream_outputs WHERE slug = 'timer'".to_string(),
            ))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::StreamElementProps;
    use sea_orm::{Database, DbBackend, Statement};

    /// Fresh DB (stream tables migrated first): seeds exactly one `timer` output,
    /// one active base scene "Timer", and one countdown element whose stored
    /// props parse back to the seeded value.
    #[tokio::test]
    async fn up_seeds_timer_output_scene_and_active_countdown() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let manager = SchemaManager::new(&db);
        crate::m20260820_000001_create_stream_tables::Migration
            .up(&manager)
            .await
            .expect("stream tables up");

        Migration.up(&manager).await.expect("seed up");

        // One timer output, name 'Timer overlay', with an active scene set.
        let row = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n, MIN(name) AS name, \
                 (SELECT active_scene_id FROM stream_outputs WHERE slug='timer') AS active \
                 FROM stream_outputs WHERE slug='timer'"
                    .to_string(),
            ))
            .await
            .expect("query")
            .expect("row");
        let n: i64 = row.try_get_by("n").expect("count");
        let name: String = row.try_get_by("name").expect("name");
        let active: Option<i64> = row.try_get_by("active").expect("active");
        assert_eq!(n, 1, "exactly one timer output");
        assert_eq!(name, "Timer overlay");
        assert!(active.is_some(), "the base scene must be set active");

        // Exactly one base scene "Timer", and it IS the active one.
        let scene = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT s.id AS id, s.name AS name, s.kind AS kind FROM stream_scenes s \
                 JOIN stream_outputs o ON o.id = s.output_id WHERE o.slug='timer'"
                    .to_string(),
            ))
            .await
            .expect("query")
            .expect("scene row");
        let scene_id: i64 = scene.try_get_by("id").expect("id");
        let scene_name: String = scene.try_get_by("name").expect("name");
        let scene_kind: String = scene.try_get_by("kind").expect("kind");
        assert_eq!(scene_name, "Timer");
        assert_eq!(scene_kind, "base");
        assert_eq!(
            active,
            Some(scene_id),
            "active scene must be the base scene"
        );

        // Exactly one countdown element whose props parse to the seeded value.
        let el = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n, MIN(kind) AS kind, MIN(props) AS props \
                 FROM stream_elements e JOIN stream_scenes s ON s.id = e.scene_id \
                 JOIN stream_outputs o ON o.id = s.output_id WHERE o.slug='timer'"
                    .to_string(),
            ))
            .await
            .expect("query")
            .expect("element row");
        let el_n: i64 = el.try_get_by("n").expect("count");
        let el_kind: String = el.try_get_by("kind").expect("kind");
        let props_str: String = el.try_get_by("props").expect("props");
        assert_eq!(el_n, 1, "exactly one seeded countdown element");
        assert_eq!(el_kind, "countdown");
        let parsed: StreamElementProps =
            serde_json::from_str(&props_str).expect("stored props must parse into the core enum");
        assert_eq!(
            parsed,
            seed_countdown_props(),
            "stored props must equal the seeded value"
        );
        // Core validation must accept the seeded props (letter spacing in range).
        assert!(presenter_core::validate_props(&parsed, &[]).is_ok());
    }

    /// Re-running is a no-op: no duplicate output / scene / element rows.
    #[tokio::test]
    async fn up_is_idempotent() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let manager = SchemaManager::new(&db);
        crate::m20260820_000001_create_stream_tables::Migration
            .up(&manager)
            .await
            .expect("stream tables up");

        Migration.up(&manager).await.expect("first up");
        Migration.up(&manager).await.expect("second up (no-op)");

        let counts = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT \
                 (SELECT COUNT(*) FROM stream_outputs WHERE slug='timer') AS outs, \
                 (SELECT COUNT(*) FROM stream_scenes s JOIN stream_outputs o ON o.id=s.output_id \
                  WHERE o.slug='timer') AS scenes, \
                 (SELECT COUNT(*) FROM stream_elements e JOIN stream_scenes s ON s.id=e.scene_id \
                  JOIN stream_outputs o ON o.id=s.output_id WHERE o.slug='timer') AS elements"
                    .to_string(),
            ))
            .await
            .expect("query")
            .expect("row");
        let outs: i64 = counts.try_get_by("outs").expect("outs");
        let scenes: i64 = counts.try_get_by("scenes").expect("scenes");
        let elements: i64 = counts.try_get_by("elements").expect("elements");
        assert_eq!(outs, 1, "output not duplicated");
        assert_eq!(scenes, 1, "scene not duplicated");
        assert_eq!(elements, 1, "element not duplicated");
    }
}
