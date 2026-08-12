//! #647 library resolution for a peer PRESENTATION apply: which local
//! library row a peer song's `library_sync_id`/`library_name` attaches to.
//! Split out of `sync_apply.rs` (mirroring the existing `library_sync.rs`
//! split) — `sync_apply.rs` was already at the file-size gate's 989-prod-line
//! cap before this ticket's join-by-identity change needed room to grow.
//!
//! `library_sync_id` (added by #647) is the STABLE identity carried on the
//! wire alongside `library_name` — never affected by a rename or a #636
//! disambiguated collision name. Every resolver below tries it FIRST via
//! `find_library_by_sync_id`, live OR tombstoned: library reconciliation
//! (`state/sync.rs`'s `reconcile_libraries`, via `apply_sync_library` in
//! `library_sync.rs`) runs BEFORE presentations in every sync cycle, so a
//! local library row matching the peer's `library_sync_id` has normally
//! already converged (created/adopted/updated) by the time a presentation
//! referencing it is processed here. Only when the identity has NOT
//! converged locally yet — an OLD peer that never sends `library_sync_id`
//! (`None`), or (rare) a transient library-manifest fetch failure this one
//! cycle — do these fall back to the PRE-#647 name-based resolution,
//! byte-for-byte unchanged. That is the mixed-version compatibility window:
//! a NEW peer talking to an OLD (name-only) peer degrades to exactly
//! today's behavior, #636/#646 collision handling included, since the name
//! path below is never touched by this change.
use super::Repository;
use crate::entities::library;
use chrono::{DateTime, Utc};
use presenter_core::search::fold_query;
use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};

impl Repository {
    /// The local library row currently carrying `library_sync_id` — live OR
    /// tombstoned; its CURRENT state is decided by the separate
    /// `apply_sync_library` LWW channel, not here. `None` when
    /// `library_sync_id` is `None` (an old, name-only peer) or no local
    /// library has converged to this identity yet.
    async fn find_library_by_sync_id<C: sea_orm::ConnectionTrait>(
        conn: &C,
        library_sync_id: Option<&str>,
    ) -> anyhow::Result<Option<library::Model>> {
        let Some(sync_id) = library_sync_id else {
            return Ok(None);
        };
        Ok(library::Entity::find()
            .filter(library::Column::SyncId.eq(sync_id.to_string()))
            .one(conn)
            .await?)
    }

    /// Given a library row ALREADY resolved (by identity or by name),
    /// decide whether to revive it (write) or leave it tombstoned — the
    /// LWW-revive decision `ensure_library`'s name-based path has always
    /// made (#580), extracted so the #647 sync_id fast path and the legacy
    /// name-based fallback share ONE implementation instead of drifting
    /// apart. A live row returns immediately, no write.
    ///
    /// #646: `library_updated` alone (never maxed against
    /// `presentation_updated_at`) keeps a forced tombstone LWW-neutral — see
    /// `ensure_library`'s own doc comment for why maxing was a data-loss bug.
    async fn revive_or_keep_tombstoned(
        txn: &sea_orm::DatabaseTransaction,
        lib: library::Model,
        presentation_updated_at: DateTime<Utc>,
    ) -> anyhow::Result<(String, Option<DateTime<Utc>>)> {
        if lib.deleted_at.is_none() {
            return Ok((lib.id, None));
        }
        let library_updated: DateTime<Utc> = lib.updated_at.into();
        if presentation_updated_at > library_updated {
            library::Entity::update_many()
                .col_expr(
                    library::Column::DeletedAt,
                    Expr::value(Option::<String>::None),
                )
                .col_expr(
                    library::Column::UpdatedAt,
                    Expr::value(presentation_updated_at.to_rfc3339()),
                )
                .filter(library::Column::Id.eq(lib.id.clone()))
                .filter(library::Column::DeletedAt.is_not_null())
                .exec(txn)
                .await?;
            return Ok((lib.id, None));
        }
        Ok((lib.id, Some(library_updated)))
    }

    /// Look up an existing LIVE library — by identity first (#647), else by
    /// name — NEVER creates one (#558 round-4 U4). Used wherever "no
    /// library" can be treated as "nothing to do" without paying for a
    /// write. Generic over the connection (#558 W5/W8/W9) so the SAME
    /// implementation serves both the real apply (inside its transaction)
    /// and `resolve_sync_apply_target` (a plain pre-transaction probe, no
    /// transaction involved).
    ///
    /// #647: a `library_sync_id` that resolves to a local library is
    /// AUTHORITATIVE — a tombstoned match means "no live candidate" and
    /// returns `None` directly, NEVER falling through to a name lookup
    /// (that fallback is exactly what would reintroduce the mis-filing this
    /// ticket fixes — a stale/renamed name could then match a completely
    /// unrelated live library). The name lookup only ever runs when the
    /// identity itself did not resolve locally at all.
    ///
    /// #580: a same-named TOMBSTONED library is deliberately invisible in
    /// the name-based path — `ensure_library` (the only caller that may
    /// WRITE) decides that case itself via LWW against the incoming
    /// presentation's `updated_at`, rather than treating "no live match" as
    /// "nothing exists".
    pub(super) async fn find_library_id<C: sea_orm::ConnectionTrait>(
        conn: &C,
        library_sync_id: Option<&str>,
        library_name: &str,
    ) -> anyhow::Result<Option<String>> {
        if let Some(lib) = Self::find_library_by_sync_id(conn, library_sync_id).await? {
            return Ok(lib.deleted_at.is_none().then_some(lib.id));
        }
        Ok(library::Entity::find()
            .filter(library::Column::Name.eq(library_name.to_string()))
            .filter(library::Column::DeletedAt.is_null())
            .one(conn)
            .await?
            .map(|l| l.id))
    }

    /// Look up an existing LIVE library, or REVIVE-or-attach-to a
    /// same-identity/same-named TOMBSTONED one, or create a brand-new live
    /// library if none exists (#558 round-4 U4; #580; #647). Call this ONLY
    /// once the caller is committed to writing a presentation row into it —
    /// never speculatively, or a skipped/never-write apply leaves behind a
    /// phantom, permanently-empty library row.
    ///
    /// #647: `library_sync_id`, when it resolves to a local library, is
    /// authoritative and the name is never consulted for that presentation —
    /// a rename in flight, or a #636 disambiguated collision name, must
    /// never redirect this presentation onto the WRONG library (mis-filing)
    /// or manufacture a phantom one, just because the name it happens to
    /// carry right now matches/mismatches something else locally. Falls
    /// back to the pre-#647 name-based resolution below only when the
    /// identity has not converged locally yet (see the module doc comment).
    ///
    /// #580: previously, the moment no LIVE same-named library existed, this
    /// ALWAYS minted a fresh one — even when a TOMBSTONED library of that
    /// name already existed. That let a peer's concurrent
    /// library-delete-vs-presentation-create race diverge the two instances
    /// forever. Fix (decision on #580, option b, preserved verbatim by
    /// #647's identity-first refactor): decide live-vs-tombstoned from the
    /// resolved library's own `updated_at` against the incoming
    /// presentation's `updated_at` via `revive_or_keep_tombstoned` — the
    /// SAME LWW mechanism `apply_sync_library` already uses for library
    /// manifest sync.
    ///
    /// Returns `(library_id, forced_tombstone_at)` — see
    /// `revive_or_keep_tombstoned`'s doc comment for what the second element
    /// means and how the caller must use it (#634/#646).
    pub(super) async fn ensure_library(
        txn: &sea_orm::DatabaseTransaction,
        library_sync_id: Option<&str>,
        library_name: &str,
        presentation_updated_at: DateTime<Utc>,
    ) -> anyhow::Result<(String, Option<DateTime<Utc>>)> {
        if let Some(lib) = Self::find_library_by_sync_id(txn, library_sync_id).await? {
            return Self::revive_or_keep_tombstoned(txn, lib, presentation_updated_at).await;
        }
        if let Some(id) = Self::find_library_id(txn, None, library_name).await? {
            return Ok((id, None));
        }
        // #626: resolved via the SHARED `find_most_recent_tombstoned_library`
        // helper — see its doc comment for why this must be the ONE query
        // both this function and `ensure_library_for_tombstone` use.
        if let Some(tombstoned) =
            Self::find_most_recent_tombstoned_library(txn, library_name).await?
        {
            return Self::revive_or_keep_tombstoned(txn, tombstoned, presentation_updated_at).await;
        }
        let id = uuid::Uuid::new_v4().to_string();
        library::Entity::insert(library::ActiveModel {
            id: Set(id.clone()),
            name: Set(library_name.to_string()),
            search_name: Set(fold_query(library_name)),
            created_at: Set(Utc::now().into()),
            // #578: a library implicitly created to hold a synced presentation
            // is live with a fresh sync identity; the library manifest sync
            // converges that identity with the peer via the name-match adopt.
            updated_at: Set(Utc::now().into()),
            sync_id: Set(uuid::Uuid::new_v4().to_string()),
            deleted_at: Set(None),
        })
        .exec(txn)
        .await?;
        Ok((id, None))
    }

    /// The most recently updated TOMBSTONED library row named `library_name`,
    /// or `None` if none exists. Ties break by `SyncId` DESC (peer-stable —
    /// never the local-only `Id`, or two peers could break a tie in
    /// DIFFERENT directions and diverge). Purely name-based — used only by
    /// the pre-#647 fallback path once the sync_id lookup has already come
    /// up empty.
    ///
    /// Shared by `ensure_library` and `ensure_library_for_tombstone` so the
    /// two can never independently order the same tombstones differently
    /// (#626: the latter used to run its OWN `DeletedAt ASC`-primary query,
    /// which only agreed with this one when every same-named tombstone
    /// shared the exact same `deleted_at`).
    async fn find_most_recent_tombstoned_library(
        txn: &sea_orm::DatabaseTransaction,
        library_name: &str,
    ) -> anyhow::Result<Option<library::Model>> {
        Ok(library::Entity::find()
            .filter(library::Column::Name.eq(library_name.to_string()))
            .filter(library::Column::DeletedAt.is_not_null())
            .order_by_desc(library::Column::UpdatedAt)
            .order_by_desc(library::Column::SyncId)
            .one(txn)
            .await?)
    }

    /// #578: resolve a library id for a brand-new TOMBSTONED presentation
    /// WITHOUT creating an empty LIVE library shell. #647: identity first —
    /// live OR tombstoned, since the presentation being written here is
    /// tombstoned regardless of the resolved library's own state, so no
    /// revive decision is needed (unlike `ensure_library`). Falls back to
    /// the pre-#647 name-based resolution when the identity has not
    /// converged locally yet: attach to any existing same-named library (a
    /// LIVE one always wins via `find_library_id`; otherwise the most
    /// recently updated TOMBSTONED one — #626); if none exists, create a
    /// TOMBSTONED library (its identity converges separately via the peer's
    /// library manifest). Used only by `apply_unknown_sync_id` for a
    /// tombstone — a live entry keeps using `ensure_library`.
    pub(super) async fn ensure_library_for_tombstone(
        txn: &sea_orm::DatabaseTransaction,
        library_sync_id: Option<&str>,
        library_name: &str,
    ) -> anyhow::Result<String> {
        if let Some(lib) = Self::find_library_by_sync_id(txn, library_sync_id).await? {
            return Ok(lib.id);
        }
        if let Some(id) = Self::find_library_id(txn, None, library_name).await? {
            return Ok(id);
        }
        if let Some(row) = Self::find_most_recent_tombstoned_library(txn, library_name).await? {
            return Ok(row.id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        library::Entity::insert(library::ActiveModel {
            id: Set(id.clone()),
            name: Set(library_name.to_string()),
            search_name: Set(fold_query(library_name)),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            sync_id: Set(uuid::Uuid::new_v4().to_string()),
            deleted_at: Set(Some(now.into())),
        })
        .exec(txn)
        .await?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use crate::entities::library;
    use chrono::{Duration, Utc};
    use sea_orm::{EntityTrait, Set, TransactionTrait};

    #[tokio::test]
    async fn tombstone_helpers_agree_on_same_named_tombstone() {
        // #595 regression: ensure_library and ensure_library_for_tombstone
        // must resolve to the SAME library row when multiple same-named
        // tombstones exist. Before the fix, ensure_library_for_tombstone
        // used only DeletedAt ASC — with equal deleted_at values, SQLite
        // falls back to rowid (insertion) order and picks the OLDER row.
        // ensure_library uses UpdatedAt DESC + SyncId DESC, picking the
        // NEWER row → disagreement.
        let repo = super::Repository::connect_in_memory()
            .await
            .expect("in-memory db");
        let base = Utc::now();
        let delete_time = base - Duration::hours(1);

        // Two tombstones: same name, same deleted_at, different updated_at/sync_id.
        // Updated_at differs: older row = base-120min, newer row = base-0min.
        // Both are older than the presentation time we'll pass to ensure_library,
        // so no tombstone revival happens (pure resolution test).
        for (id, updated_offset_min, sync_val) in [
            ("lib-older", 120_i64, "sync-aaa"),
            ("lib-newer", 0_i64, "sync-bbb"),
        ] {
            library::Entity::insert(library::ActiveModel {
                id: Set(id.to_string()),
                name: Set("Songs".to_string()),
                search_name: Set("songs".to_string()),
                created_at: Set((base - Duration::hours(2)).into()),
                updated_at: Set((base - Duration::minutes(updated_offset_min)).into()),
                sync_id: Set(sync_val.to_string()),
                deleted_at: Set(Some(delete_time.into())),
            })
            .exec(&repo.db)
            .await
            .expect("insert tombstone");
        }

        // ensure_library_for_tombstone: among equal deleted_at, the secondary
        // sort (after fix) must match ensure_library's UpdatedAt DESC.
        let txn1 = repo.db.begin().await.expect("begin txn");
        let id_from_tombstone_helper =
            super::Repository::ensure_library_for_tombstone(&txn1, None, "Songs")
                .await
                .expect("resolve via tombstone helper");
        txn1.rollback().await.expect("rollback");

        // ensure_library: picks most recent tombstone by UpdatedAt DESC.
        // presentation_updated_at is OLDER than both → no revival, pure read.
        let txn2 = repo.db.begin().await.expect("begin txn");
        let (id_from_live_helper, forced_delete) =
            super::Repository::ensure_library(&txn2, None, "Songs", base - Duration::hours(3))
                .await
                .expect("resolve via live helper");
        txn2.rollback().await.expect("rollback");
        assert!(
            forced_delete.is_some(),
            "presentation_updated_at is older than the tombstone -- it stays \
             tombstoned, so the caller must be told to force-tombstone too (#634)"
        );

        assert_eq!(
            id_from_tombstone_helper, id_from_live_helper,
            "both tombstone helpers must resolve to the same library row; \
             tombstone helper picked {id_from_tombstone_helper}, \
             live helper picked {id_from_live_helper}"
        );
    }

    #[tokio::test]
    async fn tombstone_helpers_agree_on_same_named_tombstone_with_different_deleted_at() {
        // #626 regression: the #595 test above only proves agreement when
        // BOTH same-named tombstones share the EXACT same `deleted_at` --
        // in that case DeletedAt ASC can't discriminate between them and
        // the query falls through to UpdatedAt DESC, which happens to
        // agree with `ensure_library`. An ordinary delete/recreate/delete
        // cycle leaves DIFFERENT `deleted_at` values per attempt: here the
        // most-recently-UPDATED row is deliberately NOT the earliest-
        // DELETED row, so the OLD `ensure_library_for_tombstone` (DeletedAt
        // ASC primary sort) and `ensure_library` (UpdatedAt DESC) pick
        // DIFFERENT rows.
        let repo = super::Repository::connect_in_memory()
            .await
            .expect("in-memory db");
        let base = Utc::now();

        // "lib-recent-update": most recently UPDATED, but NOT the earliest
        // deleted (deleted only 10 min ago) -- ensure_library (UpdatedAt
        // DESC) must pick this one.
        // "lib-earliest-delete": updated 2h ago, but deleted earliest (5h
        // ago) -- the OLD ensure_library_for_tombstone (DeletedAt ASC
        // primary) wrongly picks this one instead.
        for (id, updated_offset_min, deleted_offset_min, sync_val) in [
            ("lib-recent-update", 0_i64, 10_i64, "sync-aaa"),
            ("lib-earliest-delete", 120_i64, 300_i64, "sync-bbb"),
        ] {
            library::Entity::insert(library::ActiveModel {
                id: Set(id.to_string()),
                name: Set("Songs".to_string()),
                search_name: Set("songs".to_string()),
                created_at: Set((base - Duration::hours(6)).into()),
                updated_at: Set((base - Duration::minutes(updated_offset_min)).into()),
                sync_id: Set(sync_val.to_string()),
                deleted_at: Set(Some((base - Duration::minutes(deleted_offset_min)).into())),
            })
            .exec(&repo.db)
            .await
            .expect("insert tombstone");
        }

        let txn1 = repo.db.begin().await.expect("begin txn");
        let id_from_tombstone_helper =
            super::Repository::ensure_library_for_tombstone(&txn1, None, "Songs")
                .await
                .expect("resolve via tombstone helper");
        txn1.rollback().await.expect("rollback");

        // presentation_updated_at older than every tombstone -> no revival.
        let txn2 = repo.db.begin().await.expect("begin txn");
        let (id_from_live_helper, forced_delete) =
            super::Repository::ensure_library(&txn2, None, "Songs", base - Duration::hours(7))
                .await
                .expect("resolve via live helper");
        txn2.rollback().await.expect("rollback");
        assert!(
            forced_delete.is_some(),
            "presentation_updated_at is older than every tombstone -- stays tombstoned"
        );

        assert_eq!(
            id_from_tombstone_helper, id_from_live_helper,
            "both tombstone helpers must resolve to the same library row even when \
             deleted_at values differ; tombstone helper picked {id_from_tombstone_helper}, \
             live helper picked {id_from_live_helper}"
        );
    }

    #[tokio::test]
    async fn find_library_id_prefers_sync_id_over_a_stale_current_name() {
        // #647 RED→GREEN: library A holds identity "sid-a" but was renamed
        // locally to "New Name". A DIFFERENT, unrelated library B now
        // happens to own the name "Old Name" (A's former name). A caller
        // still quoting the stale name alongside the correct identity must
        // resolve to A, never B (mis-filing) — and never to `None` either
        // (which would risk manufacturing a phantom library upstream).
        let repo = super::Repository::connect_in_memory()
            .await
            .expect("in-memory db");
        let now = Utc::now();
        library::Entity::insert(library::ActiveModel {
            id: Set("lib-a".to_string()),
            name: Set("New Name".to_string()),
            search_name: Set("new name".to_string()),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            sync_id: Set("sid-a".to_string()),
            deleted_at: Set(None),
        })
        .exec(&repo.db)
        .await
        .expect("insert library A");
        library::Entity::insert(library::ActiveModel {
            id: Set("lib-b".to_string()),
            name: Set("Old Name".to_string()),
            search_name: Set("old name".to_string()),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            sync_id: Set("sid-b".to_string()),
            deleted_at: Set(None),
        })
        .exec(&repo.db)
        .await
        .expect("insert library B");

        let resolved = super::Repository::find_library_id(&repo.db, Some("sid-a"), "Old Name")
            .await
            .expect("resolve by identity");
        assert_eq!(
            resolved,
            Some("lib-a".to_string()),
            "must resolve library A by its stable identity, never library B by the stale name"
        );
    }

    #[tokio::test]
    async fn find_library_id_falls_back_to_name_when_identity_is_unknown_locally() {
        // #647 compat window: an old peer (or an identity this cycle's
        // library reconciliation hasn't converged yet) sends no usable
        // sync_id match — the name-based resolution must still work,
        // unchanged, exactly like before #647.
        let repo = super::Repository::connect_in_memory()
            .await
            .expect("in-memory db");
        let now = Utc::now();
        library::Entity::insert(library::ActiveModel {
            id: Set("lib-only".to_string()),
            name: Set("Songs".to_string()),
            search_name: Set("songs".to_string()),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            sync_id: Set("sid-only".to_string()),
            deleted_at: Set(None),
        })
        .exec(&repo.db)
        .await
        .expect("insert library");

        // `library_sync_id: None` (old peer).
        let resolved_old_peer = super::Repository::find_library_id(&repo.db, None, "Songs")
            .await
            .expect("resolve by name (old peer)");
        assert_eq!(resolved_old_peer, Some("lib-only".to_string()));

        // `library_sync_id: Some(_)` but nothing local carries it yet.
        let resolved_unconverged =
            super::Repository::find_library_id(&repo.db, Some("sid-never-seen"), "Songs")
                .await
                .expect("resolve by name (identity not converged)");
        assert_eq!(resolved_unconverged, Some("lib-only".to_string()));
    }
}
