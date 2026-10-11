---
paths:
  - "crates/presenter-server/src/resolume/handlers.rs"
  - "crates/presenter-server/src/resolume/bible_clear*.rs"
  - "crates/presenter-server/src/resolume/driver.rs"
  - "crates/presenter-server/src/resolume/clip_map.rs"
  - "crates/presenter-server/src/resolume/types.rs"
---

# Resolume clip triggers: one batch is CONCURRENT; the Bible clear triggers nothing (#807, #841)

## `trigger_clips` POSTs every `/connect` of a batch at once

`HostDriver::trigger_clips` (driver.rs) sends all `/connect` calls of one batch concurrently
through a `FuturesUnordered`. The batch has NO ordering. If two clips in the batch sit in the
SAME Resolume layer, their connects race inside that layer, and whichever Resolume handles
LAST stays live (#807: SNV layer 29 held `#bible-reference-a/b` and the old `#bible-clear`;
the clear clip showed ~50% of the time).

- **Never put two clips that must land in a fixed order into one `trigger_clips` batch.** Call
  `trigger_clips` twice. It returns only after every connect of its batch completed.
- Keep the A/B lane flip tied to "lane text was filled", never to "clip was triggered".

## The Bible clear is a FADE, never a trigger (#841, owner 2026-10-11)

Owner: "bible clear chcem aby fungoval tak ze netrigeruje ziaden clip ale iba v tom clipe
faduje opacitu". `#bible-clear` is no longer a tag (dropped from `clip_map.rs`; such a clip
in a composition is ignored). `resolume/bible_clear.rs`:

- **A clear PUTs `video.opacity` of every Bible lane clip (A AND B sides) down to 0.**
  - Nothing is connected and no text is written.
  - Both sides fade, because the live side is unknown after a restart. Fading a disconnected
    clip is invisible.
- **The fade length is the clip LAYER's `transition.duration`** (seconds, parsed per layer in
  `ClipMapping::from_composition` into `ClipTarget::fade`, capped at `MAX_CLEAR_FADE` = 3 s:
  the host worker is serial).
  - SNV Bible layers are 0.5 s, PP 0.7 s.
  - No transition means one step straight to 0.
- **The fade is time-based** (`fade_level`), stepped every `FADE_STEP` (30 ms), all clips
  concurrently each step.
- **A clear fills no lane, so it flips nothing.** The next verse lands on the other side, and
  the layer crossfades it in from an invisible clip.
- **Restore:** `handle_bible` calls `restore_opacity(&to_trigger)` AFTER the text PUTs and
  BEFORE `trigger_clips`. The clip being connected is the non-live side, so the restore is
  invisible.
- **`DimmedClips` (on `HostDriver`, keyed by opacity param id) = "may be off its base
  opacity".**
  - A clip enters BEFORE its first fade PUT and leaves only after a successful restore PUT,
    so a fade that fails halfway is still restored.
  - Do not flip this to "insert after success", the dedup-payload order: that order would
    leave a half-faded clip stuck.
  - A clip the mapping shows at opacity 0 is adopted (base 1.0) once per fetch
    (`adopt_invisible`, keyed on `last_mapping_refresh`). That covers a previous process that
    faded it, and stops a stale cached mapping from re-restoring on every verse.
  - A second clear sends nothing.
- **404 on an opacity param = `StaleIdError::opacity_parameter`** (stale mapping → refetch +
  retry, `.claude/rules/resolume-mapping-refresh.md`).
- The opacity param and its mapped value come from `clip.video.opacity {id, value}`. Read
  them live with `GET /api/v1/composition/layers/{n}` (one layer, ~200 KB), never the 16 MB
  `/composition`.

## Testing a fade

`resolume/bible_clear_tests.rs`:

- Read `server.received_requests()`, which keeps arrival order.
- Assert values only go down and end at 0, and that there are at least 3 steps from near 1.0.
- Assert the restore PUT index is below the first `/connect`.
- Assert timing only as a LOWER bound (`took >= FADE`): the loop cannot end before the
  duration, and load only lengthens it.
- Fail one param with a priority-1 wiremock (`fail_param`: `with_priority(1)` +
  `up_to_n_times`) to prove the half-failed-fade restore.

## `clip_map.rs`: keep `parse_clip_destinations` small

`parse_clip_destinations` builds ONE `ClipTarget` and moves it into the destination variant.
The tag-kind parsing lives in `parse_clip_kind`. Before #807 the function was 173 lines,
because each of the 10 variants repeated its own struct literal. The fn-length gate checks
every CHANGED file, so any edit of `clip_map.rs` would hard-fail it. When you add a
`ClipTarget` field, set it once on the shared target. Do not re-expand the per-arm literals.

## Renaming clips to presenter tags on a LIVE Arena (#813, PP 2026-10-07)

No file surgery is needed while Arena runs. `PUT /api/v1/composition/clips/by-id/{id}`
with `{"name":{"value":"#bible-translate-a"}}` returns 204. Read it back with a GET, then
use "Refresh mapping" on the presenter host. The rename lives only in the running Arena
until the operator saves the composition (Ctrl+S). Verify the saved `.avc` with a
read-only search for `value="#tag"`. Editing the `.avc` directly works only with that
Arena closed, and needs a backup first.
