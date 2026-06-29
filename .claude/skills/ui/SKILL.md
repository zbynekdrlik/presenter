---
name: presenter-ui
description: >
  Leptos/WASM frontend (presenter-ui) code-authoring gotchas: view! macro
  pitfalls, keyed <For> identity, reactive vs captured values. Use when editing
  crates/presenter-ui components. For BUILD/clippy/test commands see the deploy skill.
triggers:
  - leptos
  - view! macro
  - "<For>"
  - presenter-ui component
  - wasm frontend
  - sidebar / operator / stage component
---

# Presenter UI (Leptos/WASM) Skill

Build / wasm-clippy / host-test commands live in `.claude/skills/deploy`
(`presenter-ui` is OUTSIDE the workspace — own `Cargo.lock`, wasm32 target). This
skill is the CODE-authoring gotchas.

## `view!` macro: `each` (and attribute values) need a NAMED closure

The `view!` macro does NOT parse an inline `move || …` as an attribute/`each`
value — it errors with `expected identifier, found keyword 'move'` and a cascade
of bogus tag-mismatch errors. Bind the closure to a `let` first, pass it by name:

```rust
// WRONG — compile error inside view!
<For each=move || items().into_iter().enumerate().collect::<Vec<_>>() … />

// RIGHT
let indexed = move || items().into_iter().enumerate().collect::<Vec<_>>();
view! { <For each=indexed key=|(i, _)| *i children=move |(i, _)| { … } /> }
```

A plain `move || single_signal.get()` bound to a `let` is fine; the issue is only
inlining it *inside* the macro. Closures over only `Copy` signals (and the
`StageContext`/`AppContext`, which are `Copy`) can be copied into several `move`
closures, so re-using `items` in multiple derived closures is fine.

## Keyed `<For>`: key by a UNIQUE id, read changing values REACTIVELY (#496)

`key=|e| e.name.clone()` collides when two rows share a name (e.g. a worship set
that repeats a song — same name AND `presentation_id`). Leptos then reuses/mis-
reconciles row DOM and any value captured ONCE in `children` (e.g.
`let name = clean(&e.name)`) sticks at its first-render value.

- **Key by something unique.** When no per-item unique id reaches the client,
  enumerate and key by the **index** (`key=|(idx, _)| *idx`).
- **Read anything that can change REACTIVELY inside `children`** — the active
  class AND the display text — from the signal by index, not from the captured
  item. Under index keys a captured `display_name`/`is_active` would otherwise go
  stale when the list is edited live:

```rust
children=move |(idx, _entry)| {
    let snap = ctx.snapshot;
    let class = move || if snap.with(|o| /* active row == idx */) { "…--active" } else { "…" };
    let name  = move || snap.with(|o| /* entries[idx].name, cleaned */ );
    view! { <div class=class>{name}</div> }
}
```

- Disambiguating WHICH occurrence is active needs server help: the stage snapshot
  carries `active_entry_index` (per-occurrence), threaded from the trigger
  (`StageStateRequest.entry_index`). The sidebar resolves it via
  `worship_pp_helpers::active_sidebar_index(entries, snapshot_active_index)`
  (explicit index, fallback to first `is_active`). Don't re-derive by name/id.
