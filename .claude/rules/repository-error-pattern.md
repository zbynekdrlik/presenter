---
paths:
  - "crates/presenter-persistence/src/repository/**"
  - "crates/presenter-server/src/router/**"
  - "crates/presenter-server/src/state/**"
---

# Typed repository-refusal → HTTP-status pattern (#584, extended by #586/#587)

When a repository/state method refuses because the URL's resource doesn't exist, it must return
the router a **typed** error, never a bare `anyhow!("... not found")` string — a bare anyhow falls
through the router's default `impl From<anyhow::Error> for AppError` to a 500, even though the
correct response is 404 (or 422 for a body-referenced missing target).

## The pattern

1. **Producer** (repository or state method): `RepositoryError::NotFound(&'static str)` (URL
   resource missing → 404) or `RepositoryError::TargetNotFound(&'static str)` (a resource named in
   the request BODY is missing → 422) — both already defined in
   `crates/presenter-persistence/src/repository/util.rs`. Use `.ok_or(RepositoryError::NotFound("..."))?`
   or `return Err(RepositoryError::NotFound("...").into());` — see "clippy gotcha" below for why
   `ok_or`, not `ok_or_else`.
2. **Consumer** (router handler): add a small **private, per-file** helper —
   ```rust
   fn map_repository_not_found(err: anyhow::Error) -> AppError {
       match err.downcast_ref::<presenter_persistence::RepositoryError>() {
           Some(presenter_persistence::RepositoryError::NotFound(msg)) => AppError::not_found(*msg),
           _ => err.into(),
       }
   }
   ```
   and wire it with `.map_err(map_repository_not_found)?` on the specific call that can fail this
   way. **Never match on `err.to_string()`** — that silently stops matching the moment a
   `.context(...)` gets added anywhere upstream (the #578→#584 regression).
3. This is a **per-file private helper**, duplicated across `router/libraries.rs`,
   `router/presentations.rs`, `router/playlists.rs`, `router/sync.rs`,
   `router/bible/presentations.rs`, `router/integrations/{resolume,android_stage,video_source}.rs` —
   NOT a shared/exported helper. Each PR that touches this pattern should keep it that way: a
   shared helper is a bigger refactor than a scoped bug-fix batch needs, and increases review
   surface for no behavior change.

## Clippy gotcha: `ok_or`, not `ok_or_else`

`.ok_or_else(|| RepositoryError::NotFound("..."))?` fails `cargo clippy` with
`unnecessary_lazy_evaluations` — constructing a `NotFound(&'static str)` tuple variant from a
string literal is cheap enough that clippy wants the eager form:
`.ok_or(RepositoryError::NotFound("..."))?`. This bit #586/#587 (8 sites, caught by CI's Clippy job,
fixed in a follow-up commit) — get it right the first time.

## Trace every CALLER before assuming the repository layer is the only 500 source

A "500 instead of 404" ticket's issue body often only lists `repository/*.rs` sites. But
`state/*.rs` methods sometimes do their OWN existence check independent of (or in ADDITION to) the
repository layer — e.g. `state/bible.rs`'s `delete_bible_slide`/`reorder_bible_slides` fetch the
presentation themselves and raise their own `anyhow!("bible presentation not found")`, never
touching the repository-layer refusal the issue body pointed at. Before fixing, grep every HTTP
route's full call chain (router handler → state method → repository method) for `anyhow!(` — don't
assume the issue body's site list is complete, especially after a file-split refactor (#590)
changed line numbers.

## Only convert sites with a real HTTP consumer

Not every `anyhow!("... not found")` in these layers is reachable from HTTP traffic — some exist
only for an internal background task (e.g. `resolume.rs`'s `update_resolume_host_active_port`,
called only by the port-drift probe, which already `tracing::warn!`s and continues on error) or a
CLI-only path (e.g. `bible/import.rs`'s `set_bible_source_digest`, reachable only via
`ingest_bibles` / the `import-data.yml` workflow). Converting those adds scope with zero
user-visible bug fixed — check every caller before including a site in the batch.
