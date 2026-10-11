---
paths:
  - "crates/presenter-server/src/resolume/batch.rs"
  - "crates/presenter-server/src/resolume/lane_seed.rs"
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
  `trigger_clips` twice. It returns only after every connect of its batch completed, also when
  one failed: the connect and text-PUT batches go through `batch::drain`, which awaits the
  whole batch before the first error wins (`put_opacities` collects its batch the same way but
  keeps every outcome, because a 404 there is not an error). Returning early
  dropped requests mid-flight, and one landed after its push had returned (CI run
  38176143346).
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
- **The fade is time-based** (`fade_level`), stepped every `FADE_STEP` (50 ms = 20 Hz), all
  clips concurrently each step.
  - Every step is one PUT per still-visible clip (the 8 mapped Bible clips plus any visible
    unmapped connected clips) on fresh connections (#808, no pool), so do not raise the rate. A
    clip at 0 gets no further steps.
  - The loop ends at the step where `elapsed >= longest duration`, a hard bound.
  - `put_opacities` awaits EVERY request of a step, also after one failed, and reports
    `OpacityPuts {landed, missing, error}`. Returning early dropped the others mid-flight, and
    one landed after the clear had returned (CI run 38176143346; test
    `a_failed_step_waits_for_its_other_puts_before_failing`). The PUTs that landed are recorded
    even in a failed step, so the tracked levels match Arena and `sync_with_fetch` never reads
    a half fade as a dim made in Arena (`a_failed_step_still_records_the_puts_that_landed`).
- **A clear fills no lane, so it flips nothing.** The next verse lands on the other side, and
  the layer crossfades it in from an invisible clip.
- **Restore:** `handle_bible` calls `restore_before_connect(&to_trigger)` AFTER the text PUTs and
  BEFORE `trigger_clips`. The clip being connected is the non-live side, so the restore is
  invisible.
- **`TrackedOpacities` (`HostDriver::opacities`, keyed by opacity param id) holds `{base,
  level}` for every clip this process faded or connected.**
  - A clip is tracked BEFORE its first fade PUT, so a fade that fails halfway is still
    restored. Do not flip this to "insert after success", the dedup-payload order: that order
    would leave a half-faded clip stuck.
  - The clip STAYS tracked after its restore (`level = base`). A composition fetched while it
    was faded caches 0 (or a mid-fade value), and that cached number must never override a
    write made after that fetch.
  - The bug this prevents (review finding): with entries removed on restore, a refetch at 0
    made the next clear skip the clip, and the verse stayed on the wall. The test is
    `a_clip_restored_after_a_fetch_at_zero_is_faded_by_the_next_clear`.
  - A restore is sent only while `level < base`, so the verse after next sends no extra PUT.
  - Once per fetch (`sync_opacities` → `sync_with_fetch`, keyed on `last_mapping_refresh`, run
    at the top of `handle_bible` before any write): an UNtracked clip the mapping shows at
    opacity 0 is adopted as `{1.0, 0.0}` (a previous process faded it, or an operator hid it:
    the two look the same, and the verse comes back at full opacity), and a tracked clip takes
    the fetched value as its level. Nothing writes Bible opacities between a fetch and its
    sync, so a difference was made in Arena (or by a step that failed halfway). A VISIBLE clip
    (level >= base) also takes a non-zero fetched value as its base, so a dim made in Arena
    survives the next verse; a faded one keeps restoring to at least its old base. Tests:
    `a_clip_shown_again_in_arena_after_a_clear_is_faded_by_the_next_clear`,
    `a_clip_dimmed_in_arena_keeps_that_opacity_after_the_next_verse`
    (`bible_clear_deck_tests.rs`).
  - A second clear sends nothing.
- **A deck switch keeps the old deck's clip PLAYING** (final review finding): `/composition`
  then lists only the new deck, so fading only mapped clips left the verse on the wall (on main
  the clear connected the new deck's clear clip on that layer). `restore_before_connect`
  therefore remembers every clip a verse connects (`ConnectedClip {clip_id, fade}`), and
  `plan_clear_fade` adds every still-visible connected clip the mapping no longer lists
  (`FadeTarget::mapped = false`). Never drop these entries on a refetch of the SAME deck: the
  provisional follow-ups (2/5/15/30/60 s) refetch constantly, and a deck switch after that
  would lose the clip again. Test:
  `a_clear_after_a_deck_switch_fades_the_clips_the_verse_left_playing`.
  - **Residual risk:** this memory lives in the process. A Presenter restart (a deploy counts)
    or a host config change BETWEEN a verse and its clear, with a deck switch in between too,
    loses it, and that verse stays on the wall until the next verse or a manual clear in
    Arena. Main handled it because its clear connected a blank clip.
- **404 on an opacity param:** a MAPPED one = `StaleIdError::opacity_parameter` (stale mapping
  → refetch + retry, `.claude/rules/resolume-mapping-refresh.md`). An UNMAPPED one (a connected
  clip from another deck that Resolume no longer knows) is forgotten and the fade goes on
  (`drop_missing`); it says nothing about the mapping, so it must not cause a refetch or the
  60 s pause. `put_opacities` returns the 404 params and lets the caller decide.
- The opacity param and its mapped value come from `clip.video.opacity {id, value}`. Read
  them live with `GET /api/v1/composition/layers/{n}` (one layer, ~200 KB), never the 16 MB
  `/composition`.

## The A/B lanes are seeded from the connected clips after a start (`lane_seed.rs`)

The lane state resets to side A on every start and host config change. When side A is still
the CONNECTED clip, the first push wrote into the live clip and connected it again: no
crossfade, the text popped in. Since #841 the clear also leaves the reference clip connected at
opacity 0, so its restore popped the reference in too. After the FIRST fetch that lists any lane
clip (`ClipMapping` `has_lane_clips`, so a mid-load composition does not count),
`HostDriver::seed_lanes` starts each slot on the side that is NOT connected when exactly one
side is (`next_lane_from_connected`; a Bible slot's reference clips count for its side).
Unknown (neither or both) keeps A. It runs once (`SlotState::seeded`, reset with the state).
`ClipTarget::connected` = Resolume's `connected.value` starts with `Connected` (the choices are
`Empty`, `Disconnected`, `Previewing`, `Connected`, `Connected & previewing`; SNV verified
2026-10-11: `#bible-a` Connected on layer 30, `#bible-clear` Connected at opacity 0 on 29).

## Testing a fade

`resolume/bible_clear_tests.rs` (core contract) and `bible_clear_deck_tests.rs` (deck switch,
fetch sync, per-layer duration, lane seed through `handle_bible`; it imports the shared
`pub(super)` helpers). A sibling `*_tests.rs` counts toward the 800/1000 file-size gate, so add
new tests to the smaller file or a new sibling:

- Read `server.received_requests()`, which keeps arrival order.
- Assert values only go down and end at 0, and that there are at least 3 steps from near 1.0.
- Assert the restore PUT index is below the first `/connect`.
- Assert timing only as a LOWER bound (`took >= FADE`): the loop cannot end before the
  duration, and load only lengthens it.
- Fail one param with a priority-1 wiremock (`answer_param`: `with_priority(1)` +
  `up_to_n_times`) to prove the half-failed-fade restore.
- Model a deck switch with `server.reset()` + `mount_arena(&server, deck_layers(DECK_TWO, ..))`
  + `driver.refresh_mapping()`: the composition lists other ids, and writes to the old deck's
  clips still succeed (the clips still exist).

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
