---
paths:
  - "crates/presenter-server/src/resolume/handlers.rs"
  - "crates/presenter-server/src/resolume/bible_clear*.rs"
  - "crates/presenter-server/src/resolume/driver.rs"
  - "crates/presenter-server/src/resolume/clip_map.rs"
  - "crates/presenter-server/src/resolume/types.rs"
---

# Resolume clip triggers — one batch is CONCURRENT; same-layer clips race (#807)

## `trigger_clips` POSTs every `/connect` of a batch at once

`HostDriver::trigger_clips` (driver.rs) sends all `/connect` calls of one batch concurrently
through a `FuturesUnordered`. The batch has NO ordering. If two clips in the batch sit in the
SAME Resolume layer, their connects race inside that layer, and whichever Resolume handles
LAST stays live. Real incident #807: SNV's layer 29 holds `#bible-reference-a/b` and
`#bible-clear`. The clear path put the blanked reference clip and `#bible-clear` in one batch,
so the clear clip showed only ~50% of the time.

Rules:

- **Never put two clips that must land in a fixed order into one `trigger_clips` batch.** Call
  `trigger_clips` twice instead. It returns only after every connect of its batch completed,
  so the second call can never race the first.
- **A "clear"-style clip replaces its layer's content.** Do NOT also trigger a lane clip in the
  same layer: it would race the clear clip, or cut to blank first. Blank that lane's TEXT anyway,
  so the next verse starts clean. `bible_clear.rs` does this. `plan_bible_clear_triggers`
  partitions the blanked lane clips by `ClipTarget::layer_index`. Phase 1 triggers the lane
  clips, then phase 2 triggers `#bible-clear`.
- **`ClipTarget::layer_index`** is the clip's position in the composition's `layers[]` array,
  recorded by `ClipMapping::from_composition`. Use it whenever trigger logic depends on the
  layer. Never infer a layer from the clip name.
- Keep the A/B lane flip tied to "lane text was blanked/filled", never to "clip was triggered".
  The clear path skips a same-layer clip but still flips (`clear_flips_the_lanes_so_the_next_verse_lands_on_lane_b`).

## Testing trigger ORDER — arrival gap ≥ response delay, never wall-clock

To prove "B is sent only after A completed", mount a wiremock `Respond` impl that records
`(clip_id, Instant::now())` at receipt and answers after a fixed `set_delay(D)`. Then assert
`arrival(B) - max(arrival(A…)) >= D`. A correct sequential implementation cannot arrive
earlier: the response delay sits between the two. Machine load only widens the gap. A
concurrent batch arrives within milliseconds, so the test is deterministically RED on the
racy code. Copy `ConnectRecorder` / `assert_clear_after_lanes` from
`resolume/bible_clear_tests.rs`. This is the sequencing counterpart of the #529
`ArrivalRecorder` parallel-dispatch test in `tests.rs`.

Note: `mount_full_composition` in `tests.rs` puts EVERY clip in ONE layer, together with
`#bible-clear`. So on a clear in that fixture, all Bible lane clips are skipped and only the
clear clip connects. Build per-layer compositions (as `bible_clear_tests.rs` does) for any
test whose behaviour depends on layers.

## `clip_map.rs`: keep `parse_clip_destinations` small

`parse_clip_destinations` builds ONE `ClipTarget` and moves it into the destination variant.
The tag-kind parsing lives in `parse_clip_kind`. Before #807 the function was 173 lines,
because each of the 10 variants repeated its own struct literal. The fn-length gate checks
every CHANGED file, so any edit of `clip_map.rs` would hard-fail it. When you add a
`ClipTarget` field, set it once on the shared target. Do not re-expand the per-arm literals.
