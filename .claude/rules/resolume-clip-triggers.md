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

# Resolume clip triggers: one batch is CONCURRENT; the Bible clear triggers nothing (#807, #841, #848)

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
faduje opacitu". #841 read "v tom clipe" as the Bible text clip and dropped the `#bible-clear`
tag; #848 restored it as a COVER (next section). `resolume/bible_clear.rs`:

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

## `#bible-clear` is a COVER: a clear fades it UP, a verse fades it DOWN (#848)

Owner, 2026-10-12: "ked sa clearne v presentery vers alebo je clearnuty tak ma sa zapnut
opacita bible clear a ked sa zapne vers tak sa ma stiahnut opacita bible clear aby bol vers
vyditelny". On SNV the clip is the sermon title `NAZOV #bible-clear` (a TextBlock, layer 12
"L# Fit", transition 2.3 s, `Connected & previewing`, the tag after other words). PP has none.

- **Mapping:** `#bible-clear` (tokens `#bible`, `clear`) and `#bibleclear` are the lane-less
  `ClipKind::BibleClear` → `ClipMapping::bible_clear`. It is NOT in `destinations()`: the kind
  is optional, so it never shows in the missing-clip list, `destination_kinds()` or the
  provisional suspect logic, and no lane refetch waits for it.
- **Never triggered, never written.** Only its `video.opacity` is PUT.
- **The cover fade is NOT part of the push** (#848 review: the 2.3 s fade ran inside the clear
  and the first verse, so the serial worker froze the SNV `#timer` countdown and held the next
  verse; a cover 5xx failed the clear and opened a #484 backoff window). It is a deferred,
  best-effort, interruptible step machine in `resolume/bible_clear_cover.rs`:
  - `plan_clear_fade` holds the Bible clips only. When the clear's #841 fade succeeded (or had
    nothing to fade), `handle_bible` calls `schedule_cover_fade(Show)`; a connected verse calls
    `schedule_cover_fade(Hide)` after `trigger_clips` and the lane flips. A schedule REPLACES
    the pending fade (INFO line when a started fade is replaced). Show = up to the base, hide =
    down to 0.
  - **Length = the distance left** (#848 second review): each cover fades over
    `fade × |to − from| / base` of its OWN layer transition (capped by `MAX_CLEAR_FADE`;
    `cover_duration`). A whole show or hide is the whole transition; a hide that replaces a
    show cut short at 0.1 takes a tenth of it. A full fade from a replacement's start level
    stretched quick verses: the title stayed partly visible behind the verse ~2.3 s on SNV.
  - **Worker branch:** `run_host_worker`'s biased `select!` has
    `cover_fade_deadline(driver.cover_fade_due()) => driver.step_cover_fade()` right AFTER the
    commands branch (then the follow-up deadline, then the tick). The deadline is pending while
    nothing is scheduled. Every waiting command runs before the next 50 ms step; a command only
    waits for a step already in flight.
  - **First step reads Arena and plans:** `GET /parameter/by-id/{id}` per cover
    (Resolume REST `get_parameter_by_id`: an opacity is a `RangeParameter`, the level is
    `value`; verified in `resolume.com/docs/restapi/swagger.yaml`; SNV Arena 7.27.1 answers
    200 `ParamRange`, an unknown id 404), `ACTION_TIMEOUT`, concurrently. The read value
    becomes the tracked state with the fetch-sync base rules (`TrackedOpacity::observe`: a
    SHOWN cover dimmed in Arena keeps that as its base, 0 never becomes a base; an untracked
    cover at 0 is hidden from its mapped value, full when that is 0). A cover already at its
    target gets no PUT; a fade a replacement cut short goes on from the reached level (no jump).
    A failed read (non-404) keeps the tracked level (DEBUG). So every fade reads each cover
    once (a tiny GET) before its PUT steps. The first step PUTs nothing, except for a cover on
    a layer WITHOUT a transition: its duration is 0, so the first step sets its target.
  - Each step is one `put_opacities` batch at `fade_toward(from, to, elapsed, duration)`,
    landed PUTs recorded; it ends at the step where `elapsed >= longest duration` (exact `to`).
    Targets are tracked before the first PUT. The first step plans at elapsed 0, and every step
    skips a cover whose level would not move (`cover_levels`, within `SAME_OPACITY`), so no
    fade re-sends the level it starts from; the first PUT is one step into the fade (a layer
    without a transition: the first step sets the target).
  - **A level within `SAME_OPACITY` of `to` IS `to`** (`fade_toward`, shared with the #841
    Bible fade): both fades stop counting a clip once it is that close, so a step landing a
    rounding short of the end must send `to` itself. CI run 38288218234: a replacement hide
    lasts exactly as long as the show had run, and its steps repeat the show's timing, so a
    step came 1.4 µs before the end, sent 9.3e-7, and the cover never got its 0.
  - **Best-effort:** a cover error never fails a push, never calls `record_error`, never
    touches the host status or backoff.
    - A failed step (5xx, timeout, refused) does not leave the cover halfway: the next step,
      `FADE_STEP` later, is ONE final step that PUTs each target (`CoverFade::failed_step`).
      If that fails too: one WARN, the fade stops. At most 2 failed PUTs per cover (plus the
      first-step read), never a retry loop that holds the worker. A recovered fade logs the
      first error in its done line (`error_before_final_step`).
    - **A cover answering 404 (read or PUT) is left out of THIS fade; the others go on**
      (`drop_cover`; #848 third review: one stale cover stopped them all, and a remembered
      title still playing stayed over the verse). A remembered one is forgotten (below).
      The first step leaves EVERY gone cover out before it plans and only then logs each
      (#848 fourth review: dropped one at a time before the plan, it listed other 404'd
      covers and covers already at their target). A no-transition cover's first-step PUT
      and every later step's PUTs go through `drop_gone_covers` on a 404: it takes all of
      the step's `puts.missing` out of the covers AND targets, then `drop_cover` logs each
      (#848 fifth review: one at a time, X's stale WARN listed Y). A 404 for a parameter
      that is not a cover of the fade is one DEBUG line listing only those ids. So the
      stale WARN's `target_clip_ids` is the fade's targets (mid-fade they include covers
      already at their `to`), never another cover gone in the same step.
    - A MAPPED cover's 404 also `invalidate_mapping(StaleId)` — only while the mapping it was
      scheduled with is still cached, at most once per fetched mapping, never again for an id
      that already refetched (the refetched mapping still lists it), and not while stale
      refetches are paused (`cover_404_refetch` → `CoverRefetch`). Every id that refetched is
      kept until a config change (`CoverState::refetched_ids`), not only the last one: now that
      every mapped 404 is reached, two broken ids would otherwise take turns refetching on
      every fade. A broken cover id therefore costs ONE 16 MB refetch, never one per
      clear/verse. A mapped 404 that comes while the refetch for the fade's mapping is
      still pending (that mapping was dropped for a cover's 404 and no mapping is cached
      now, so a fetch is pending and the next mapping comes after this 404:
      `cover_refetch_pending`) is added too, since that refetch lists it again (#848
      fourth review: the second broken cover of a fade got `MappingChanged` and cost the
      next fade a second refetch). Two broken covers in one fade cost ONE refetch. A 404
      that gets `MappingChanged` with a NEWER mapping cached is not added: nothing is
      pending, and the id's 404 on that mapping still refetches once (#848 fifth review).
      Pinned by the unit test `a_cached_newer_mapping_ends_a_pending_cover_refetch`
      (`bible_clear_cover_refetch_tests.rs`, #848 sixth review): after the cover refetch
      for a mapping, a newer mapping cached ends it, and only the
      `last_mapping_refresh.is_none()` check sees that (the refetched mapping is still
      the fade's). The worker test
      `a_mapped_cover_404_after_a_newer_mapping_was_fetched_refetches_nothing` is the
      end-to-end check of the case with no cover refetch for the fade's mapping (a newer
      mapping cached → the id stays free); it passes without that check.
      An unlisted id cannot 404 as mapped; the set grows one `i64` per distinct broken id.
      The next push or tick does the refetch. A refetch invalidates the mapping,
      so `MappingChanged` answers before `AlreadyForMapping` (a guard, reachable only if a
      refetch ever kept `last_mapping_refresh`).
    - Logs: the refetch is one WARN. Every later 404 WARNs with its reason (a newer mapping is
      cached / the mapping is dropped / refetched for this id already / refetched for another
      cover already / stale refetches paused) through the #563h limiter, key `cover-404:<id>`
      (`CoverState::warned_404`, `MISSING_CLIP_WARN_INTERVAL`, DEBUG in between), so a cover
      that stays broken is not a WARN per clear/verse. The 404 WARN carries any other PUT error
      of the same step (`other_error`) and the failed step's error before the final step
      (`first_error`, also on the replaced line). The done line lists faded covers
      (`clip_ids`, `from`, `to`) apart from forgotten ones (`forgotten_clip_ids`) and the
      mapped ones left out on a 404 (`stale_clip_ids`).
  - **Remembered covers, a small set per layer** (`bible_clear_cover_memory.rs`,
    `RememberedCovers` = layer (1-based, as in `/composition/layers/{n}`) → covers, oldest
    first; reset with the host config, like `TrackedOpacities`). A deck switch keeps the old
    deck's cover playing on its layer, so later shows and hides include remembered covers the
    mapping no longer lists (`mapped: false`). Which cover plays is known only at fetch time:
    the operator can trigger another one in Arena after it. On SNV every title sits on layer
    12, so the second-review rule "one per layer, replaced only by one seen playing" became
    "remember only playing covers" (the rejected variant): a title triggered after its deck's
    fetch was never remembered and stayed over the verse after a switch back (#848 third
    review). So, each time a fade is scheduled, every cover of the mapping is remembered
    (`remember_mapping`):
    - a fetch collapses each layer at most ONCE, to the covers it saw PLAYING
      (`connected`) there: Resolume plays one clip per layer, so the others are no longer
      visible. The first playing cover on a layer collapses it (`collapse_layer`), every
      further playing one of that fetch is added beside it (`add_cover`), so ALL of them
      are kept (#848 fifth review: two playing titles on one layer, the second one's
      collapse dropped the first, and the later column won);
    - a fetch's playing covers are remembered FIRST (`mapped_covers` lists them first, a
      stable sort, composition order otherwise), so the set never depends on the column
      order (#848 fourth review: T1 in column 1 and T2 playing in column 2 left only T2;
      T1, triggered in Arena before a deck switch, stayed over the verse);
    - one not playing is ADDED (each param id once, the last seen last); at the cap
      (`REMEMBERED_PER_LAYER` = 4) the oldest not playing goes;
    - remembering a cover removes its param id from every other layer (a layer inserted or
      deleted in Arena shifts every index; it was kept on both, holding a slot of each;
      `cover_candidates` dedups by param id, so it was never faded twice).
    - A remembered cover costs what a mapped one does (#848 fourth review): every clear
      shows it and every verse hides it, one GET at the first step, then one PUT per 50 ms
      step over its layer's transition (~46 PUTs over SNV's 2.3 s, each on a fresh
      connection, #808). Four titles plus a mapped one on another layer are 5 PUTs per
      step, below the #841 Bible fade's 8. Fading a cover that does not play is invisible.
    - Residual: a title that drops out of the memory (a collapse or the cap) keeps the
      level the last fade left it at in Arena (0 after a verse) until the next Bible push
      on its deck. That is visible when the cap drops a title while it is at its base
      after a clear: triggered in Arena before a deck switch, it stays over the next verse
      until a Bible push on its own deck (#848 fifth review).
    - A remembered cover answering 404 is forgotten (`forget_cover`, INFO) in this fade and
      later ones, no refetch; the other covers keep fading.
    - Tests: `bible_clear_cover_memory_tests.rs` (A→B→A with deck B's title set to 1.0 in
      Arena after B's fetch: the verse on deck A hides it; the mirror at 0: a clear on deck A
      shows it; collapse on playing; cap eviction; layer shift; a title left of a playing
      one hidden after a deck switch; pure `remember_mapping` tests for both column
      orders, with one playing title and with two playing titles plus one not playing;
      pure `remember_mapping` tests on small compositions for the cap with a playing title
      kept (one fetch), and the cross-layer forget on the map itself (three fetches), which
      the worker tests cannot see: `cover_candidates` dedups by parameter).
    - Residual: the memory is per process (a restart between a show and a deck switch loses
      it), and it is updated only when a fade is scheduled, from the cached mapping. Switches
      no push saw (the operator goes B→A, triggers deck A's title, and back to B between two
      pushes) leave the cached deck-B mapping saying deck B's title plays: the next fade
      collapses the layer to it, and deck A's title can stay over the verse until a fetch
      sees it play.
  - **Residual: a restart or host config change mid-fade.** It drops the fade and the tracked
    state (`update_config` resets `CoverState`). The next fade reads the cover where the
    dropped one left it and adopts that partial value as its base, so a later clear shows the
    title only that far. It needs a restart (a deploy counts) inside the ~2.3 s fade window;
    the operator fixes it by setting the opacity in Arena. Preferring the mapped base would
    not help: the composition fetched after the restart reads the same partial value.
- **`sync_with_fetch` skips the covers:** the deferred fade can write between a fetch and its
  sync, so a fetch may cache a mid-fade value; the first-step read is fresher.
- A clip tagged both cover and Bible lane is left to the lane (`cover_candidates` dedups by
  param id).
- Tests: `bible_clear_cover_tests.rs` (mapping, `fade_toward`, the clear plan without covers,
  the hide plan + lane dedup through `cover_candidates` + `plan_cover_fade`, the length share,
  a clear/verse sends the cover nothing inside its push), `bible_clear_cover_worker_tests.rs`
  (the real worker, see "Testing a fade"), `bible_clear_cover_step_tests.rs` (length, first
  and final step, every 404 branch incl. one cover left out, two left out in one mid-fade
  step, one refetch per broken id and one for two broken covers in one fade, a 404 with a
  newer mapping cached that leaves the id free to refetch once, a failed read, a config
  change),
  `bible_clear_cover_memory_tests.rs` (the per-layer set) and
  `bible_clear_cover_refetch_tests.rs` (`cover_404_refetch` branches, `cover_refetch_pending`
  and the #563h limiter, direct unit tests on a paused clock).

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
2026-10-11: `#bible-a` Connected on layer 30, the old blank `#bible-clear` Connected at opacity
0 on 29; since 2026-10-12 the SNV `#bible-clear` is the sermon title on layer 12).

## Testing a fade

`resolume/bible_clear_tests.rs` (core contract), `bible_clear_deck_tests.rs` (deck switch,
fetch sync, per-layer duration, lane seed through `handle_bible`),
`bible_clear_cover_tests.rs` (#848 cover planners, push side),
`bible_clear_cover_worker_tests.rs` (#848 deferred fade; its `Worker` and waits are shared),
`bible_clear_cover_step_tests.rs` (#848 second review: the fade's steps),
`bible_clear_cover_memory_tests.rs` and `bible_clear_cover_refetch_tests.rs` (#848 third
review); the siblings import
the shared `pub(super)` helpers (`assert_faded` lives in the deck file). A sibling `*_tests.rs` counts
toward the 800/1000 file-size gate (`bible_clear_tests.rs` sits at 800), so add new tests to
the smaller file or a new sibling:

- The shared mock Arena (`mount_arena` / `start_arena`) keeps parameter values: seeded from the
  composition's opacities, updated by every PUT the catch-all route accepts, served by
  `GET /parameter/by-id/{id}` (404 for an unknown id). `ArenaParams::set` models an operator
  change made in Arena; after `server.reset()` the new mount starts a new store.
- **The cover fade runs only in the worker.** A direct `handle_bible` call schedules it and
  returns; drive it through `run_host_worker` (`Worker::spawn` in the worker tests: commands
  over the channel, the shared status snapshot for "healthy", `RefreshMapping` for a deck
  switch). Wait with `wait_for` (poll `received_requests`, 10 s limit), never a sleep.
- **Prove a cover step ran by its opacity READ (a GET), then a timer-frame barrier.** A command
  sent right after a push runs BEFORE that push's first cover step (commands win the biased
  `select!`), so a barrier alone proves nothing; after the GET it waits for the step in flight.
- **A test pins one conjunct of a predicate only where every other conjunct is true.** Trace
  it before writing "pinned by" (#848 sixth review: the worker test cited for
  `cover_refetch_pending`'s `is_none()` ran with `refetched_mapping == None`, so the equality
  already made it false; the conjuncts are now pinned by
  `a_cached_newer_mapping_ends_a_pending_cover_refetch`).
- **Bound a fade's length by its step COUNT, not the clock.** A step is due `FADE_STEP` after
  the previous one ended, so a fade of length `d` sends at most `floor(d / FADE_STEP) + 1`
  PUTs (`a_hide_right_after_a_short_show_lasts_only_for_the_distance_left`). Load only lowers
  the count.
- **Push-level tests prove "not inside the push" by the requests, never by `took <`**: seed the
  cover where a synchronous fade WOULD send a PUT (shown at 1.0 for a verse's hide), then
  assert no cover PUT after the direct `handle_bible` call.
- Proving an ABSENCE (no step after a config change) needs a bounded wait past the time the
  dropped fade would have run; it is the one wait that is not a poll for a request.
  "The fade STOPPED" is the same: the barrier frame runs before a further step could be due,
  so a retry storm passes a read at the barrier; read again after `settled` (5 × `FADE_STEP`)
  and assert the exact count (#848 third review).
- **To fail the N-th PUT of a param, mount the answers by priority before the push:**
  `answer_param(.., 200, N-1, 1)` + `answer_param(.., 500, 1, 2)`; the rest falls through to
  the stateful route. Mounting a failure mid-fade cannot tell which PUT it hit. The N−1 200s
  are answered by the priority route, so they never reach `ArenaParams`: the store keeps the
  value from before them, and a later read (`GET /parameter/by-id`) does not see them.

- Read `server.received_requests()`, which keeps arrival order.
- Assert values only go down and end at 0, and that there are at least 3 steps from near 1.0
  (a cover a clear shows: only up, from near 0, ending within `1e-9` of its base).
- Assert the restore PUT index is below the first `/connect`.
- Assert timing only as a LOWER bound (`took >= FADE`): the loop cannot end before the
  duration, and load only lengthens it.
- Fail one param with a priority-1 wiremock (`answer_param`: `with_priority(1)` +
  `up_to_n_times`) to prove the half-failed-fade restore.
- Model a deck switch with `server.reset()` + `mount_arena(&server, deck_layers(DECK_TWO, ..))`
  + `driver.refresh_mapping()`: the composition lists other ids, and writes to the old deck's
  clips still succeed (the clips still exist).
- **Never compare a read-back fade level with `==`.** The values come back through
  `serde_json::from_slice`, and the workspace has no `float_roundtrip` feature, so a long
  fraction can be one ULP off the f64 that was sent (CI run 38178528722: `…3333` vs `…3332`).
  Compare with a tolerance (`< 1e-9`, far below the ~0.08 between two steps). Short decimals
  (`0.0`, `0.5`, `0.6`, `1.0`) parse exactly, so `vec![0.0, 0.5]` checks are fine.

## Live verification on the rig (no Arena GUI needed)

- Read only the Bible layers: `GET {arena}/api/v1/composition/layers/{n}` and print each
  `#bible*` clip's `connected.value` + `video.opacity.value`. SNV main wall `10.77.9.201:8090`,
  layers 29 (reference) / 30 (verse), transition 0.5 s. PP Arena-Bridge `10.77.8.201:8090`,
  layers 20-23, 0.7 s. PP's Arena is reachable only from the PP box: pipe the probe through
  `ssh newlevel@companion-pp.lan "python3 -I -" < probe.py`.
- Drive the cycle through the presenter API: `POST /bible/trigger-slide` (camelCase
  `{mainText, mainReference, secondaryText, secondaryReference}`), then `POST /bible/clear`.
  Sample the layers about 0.3 s into the clear (mid-fade value, same on both sides) and after
  1.5 s (all 0, the connected clip unchanged = nothing triggered). The journal line
  `resolume bible clear: faded … faded_clip_ids=[…] steps=N ok=true` and, after a start,
  `resolume lanes seeded … seeded=[...]` confirm the server side.
- #848 on SNV: also read layer 12 (`NAZOV #bible-clear`). After a clear its opacity rises to
  its base over ~2.3 s while the push has already returned; after the next verse it falls to 0
  over ~2.3 s (`resolume bible: faded the #bible-clear cover … direction="show"|"hide" … ok=true`),
  and its `connected.value` never changes. A transient cover error ends in that line with
  `error_before_final_step=…`; a second failure is one WARN `a #bible-clear cover fade step
  failed and so did its final step`, never a host error.
- **Re-read the cover's opacity param id before sampling.** Arena regenerates parameter ids
  when it reloads a composition while the clip ids stay (SNV 2026-10-13: clip 1791557304764
  kept its id, its opacity param went 1791557304838 → 1791836131729), so a param id from an
  earlier session answers 404. Sample `GET /parameter/by-id/{id}` every ~250 ms from the
  presenter box (same LAN as Arena). A cover that is not playing (`Disconnected`) still fades
  its parameter, invisibly on the wall.
- `POST /bible/trigger` `{translation, book, chapter, verseStart}` takes the translation's own
  book name (`Ján` for `slk-roh`, from `GET /bible/books?translation=…`); `John` answers 404.
- End with a clear, so the wall is left empty.

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
