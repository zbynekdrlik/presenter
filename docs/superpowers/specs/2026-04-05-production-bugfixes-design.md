# Production Bugfixes — Follow Scroll, RTF "0" Character, NDI Startup Restore

## Context

Three bugs discovered during production use at church (2026-04-05, v0.4.2):

1. **Follow mode doesn't scroll slide list** — operator UI auto-navigates to the correct presentation when follow is ON, but the slide list never scrolls to show the active slide. The operator must manually scroll to find the highlighted slide.

2. **"0" character prepended to slide text** — many slides show a spurious `0` at the beginning of their text in the "next slide" box on the worship-snv stage layout. Affects ~148 slides across multiple libraries.

3. **NDI source activation lost on restart** — after deploy/restart, the previously activated NDI source is not restored. The database still has `is_active = true`, so the UI shows "ACTIVE", but the NDI stream isn't running. The operator must click the button (which deactivates), then click again (which re-activates).

---

## Fix 1: Follow Mode — Scroll Active Slide Into View

### Root Cause

In `crates/presenter-ui/src/components/slide_list.rs`, the `is-active` CSS class is applied to the active slide card (line 448), but no `scrollIntoView()` call is made. Follow mode (in `operator.rs:298-354`) auto-navigates to the correct presentation and loads slides, but never scrolls the slide list container.

The legacy JS operator (removed in commit `ce3b11f`) likely had this behavior, but it was never ported to the WASM operator.

### Fix

Add a reactive `Effect` in the slide list component that watches `stage_snapshot`. When the active slide ID changes, call `scrollIntoView({ block: "nearest", behavior: "smooth" })` on the `.is-active` element within the slide list container.

Constraints:
- Only scroll when the slide list is the operator's main slide panel (not stage preview)
- Use `block: "nearest"` so it only scrolls when the active slide is off-screen (doesn't jump when already visible)
- Use `behavior: "smooth"` for non-jarring visual feedback
- Must work when follow auto-navigates to a new presentation (slides load async, then scroll)

### Files

- **Modify:** `crates/presenter-ui/src/components/slide_list.rs` — add scroll effect after slide list renders

---

## Fix 2: RTF Parser — `\up0` Misinterpreted as Unicode Escape

### Root Cause

In `crates/presenter-importer/src/rtf.rs`, the `fallback_rtf_to_text` parser (line 62-68) has a greedy `\u` match:

```rust
Some('u') => {
    chars.next();  // consumes 'u'
    if let Some(decoded) = decode_rtf_unicode(&mut chars) {
        result.push(decoded);
    }
    skip_unicode_fallback(&mut chars, unicode_skip);
}
```

RTF has two kinds of `\u` sequences:
- `\u1234?` — Unicode escape (number + fallback char)
- `\up0`, `\ul`, `\ulnone` — regular control words starting with `u`

The parser doesn't distinguish between them. For `\up0`:
1. `\u` is matched, `u` consumed
2. `decode_rtf_unicode` sees `p` (not digit/minus), returns `None`
3. `skip_unicode_fallback` consumes `p` (1 fallback char)
4. `0` remains as literal text in the output

**History:** Before commit `d81b83b` (Feb 21), the `rtf-parser` crate was used as the primary decoder and handled `\up0` correctly. The custom parser was only a fallback. When `rtf-parser` was removed (due to diacritics bugs with `\'cd\'8e`), all slides started going through the custom parser, exposing this bug.

The `clean_text()` function (line 229-235) has a filter for spurious `0` before non-ASCII letters, but the dominant pattern is `0` followed by a space (e.g., `0 MILUJEM`), which the filter doesn't catch.

### Fix

Before entering the Unicode decode path, peek at the character after `u`. A valid RTF Unicode escape always has a digit or `-` immediately after `\u` (e.g., `\u353?`, `\u-234?`). If the next character is alphabetic (like `p` in `\up`), treat the entire sequence as a regular control word.

Change the `Some('u')` branch (lines 62-68) to:

```rust
Some('u') => {
    // Distinguish \uNNNN (Unicode escape) from \up, \ul, etc. (control words)
    // Unicode escapes always have a digit or '-' after \u
    if matches!(chars.peek(), Some('-') | Some('0'..='9')) {
        // This is not a peek past 'u' — 'u' was NOT consumed yet at this point
        // Wait — 'u' IS consumed on line 63. So we peek at what follows 'u'.
        chars.next(); // consume 'u'
        if let Some(decoded) = decode_rtf_unicode(&mut chars) {
            result.push(decoded);
        }
        skip_unicode_fallback(&mut chars, unicode_skip);
    } else {
        // Regular control word starting with 'u' (e.g., \up0, \ul, \ulnone)
        // Don't consume 'u' — let read_control_word handle the full word
        let word = read_control_word(&mut chars);
        // These are formatting control words — no text output needed
        if let Some(' ') = chars.peek() {
            chars.next();
        }
    }
}
```

Wait — looking at the code more carefully, `u` is consumed on line 63 (`chars.next()`). The fix must either:
- (A) Peek BEFORE consuming `u`, or
- (B) Consume `u`, peek, and if alphabetic, prepend `u` to a `read_control_word` call

Option (A) is cleaner:

```rust
Some('u') if matches!(chars.peek().and_then(|c| { /* need double peek */ }), ...) => {
```

But we can't double-peek with `Peekable`. So option (B):

```rust
Some('u') => {
    chars.next(); // consume 'u'
    match chars.peek() {
        Some('-') | Some('0'..='9') => {
            // Unicode escape: \u1234?
            if let Some(decoded) = decode_rtf_unicode(&mut chars) {
                result.push(decoded);
            }
            skip_unicode_fallback(&mut chars, unicode_skip);
        }
        _ => {
            // Control word starting with 'u': \up0, \ul, \ulnone, etc.
            // read_control_word reads remaining alpha chars + optional number
            let _word = read_control_word(&mut chars);
            if let Some(' ') = chars.peek() {
                chars.next();
            }
        }
    }
}
```

After fixing the parser, re-import data to clean existing `0` prefixes. The `clean_text` zero-filter (lines 229-235) can be left as a safety net.

### Files

- **Modify:** `crates/presenter-importer/src/rtf.rs` — fix `\u` branch (lines 62-68), add test for `\up0`
- **Action:** Run Import Data workflow after deploy to re-import slides

---

## Fix 3: NDI Source — Restore Active Source on Startup

### Root Cause

In `crates/presenter-server/src/state/mod.rs`, the `from_config()` method (lines 245-317) initializes the NDI manager and syncs various integrations, but **never queries the database for the previously active video source** and never starts the NDI stream.

The database correctly persists `is_active = true` on the video source row. But after restart:
1. NDI manager is created (line 151) — persistent finder starts discovering sources
2. No code calls `get_active_video_source()` or `activate_video_source()`
3. UI reads `is_active = true` from DB → shows "ACTIVE" button
4. User clicks "ACTIVE" → calls deactivate endpoint → sets `is_active = false`
5. User must click "Activate" again → this time it actually starts the stream

### Fix

After existing integration syncs (line 306), add NDI source restoration:

```rust
// Restore active NDI video source
if state.ndi_manager().is_some() {
    if let Ok(Some(source)) = state.repository.get_active_video_source().await {
        if let Err(err) = state.activate_video_source(source.id.to_string()).await {
            tracing::warn!(?err, ndi_name = %source.ndi_name, "failed to restore active NDI source on startup — will retry when source appears on network");
        }
    }
}
```

The persistent finder + receiver retry loop (added in the NDI hardening work) should handle the case where the NDI source isn't immediately available on the network at startup — it will keep trying.

### Files

- **Modify:** `crates/presenter-server/src/state/mod.rs` — add restoration after line 306

---

## Testing

### Fix 1 — Follow scroll
- **E2E (Playwright):** Trigger a slide deep in the list remotely, verify the slide card with `is-active` class is within the visible viewport of the slide list container
- Console: zero errors/warnings

### Fix 2 — RTF parser
- **Unit test:** `\up0 text` should produce `text` (no leading `0`)
- **Unit test:** `\u353?` should still produce `ť` (Unicode escape still works)
- **Unit test:** `\ulnone text` should produce `text` (other `\u*` control words)
- **Re-import verification:** After deploy + import, query API and verify zero slides have `0`-prefixed text

### Fix 3 — NDI startup restore
- **E2E:** Activate a video source, restart server, verify the source is still active and streaming
- Graceful handling when source isn't on network at startup (warn log, no crash)
