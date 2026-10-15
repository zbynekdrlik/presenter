# Listening C1b — Settings "Počúvanie" card + recorder (#852) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship unit **C1b** of #852 in seven lanes, each its own PR unit: the operator manages the listening microphones in Settings → **"Počúvanie"** (add / edit / remove through the existing whole-list `GET|PUT /listen/mics`, the server's Slovak 422 rule shown inline, a live per-mic health row from `GET /listen/status`, the socket line), and a **recorder** — off by default, switched on per DAY from the same card — writes each receiving mic's 16 kHz mono s16le frames (exactly what the ASR will get) to `<recordings root>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav`, one file per continuous stretch of audio, deletes day folders older than 30 days once a day, and reports per mic in `/listen/status` (state, file, bytes, dropped frames, error).

**Architecture:** C1a left the 16 kHz frames inside each mic's chain ("no frame channel until its first consumer"). C1b adds the channel: one `tokio::sync::broadcast` per mic id in `ListenService`, kept for the life of the process, into which every chain of the mic sends its `Arc<[i16]>` frames only while its generation is current — `broadcast::send` never waits; a consumer 12.8 s (128 frames) behind loses the oldest frames and is told how many. The recorder is that channel's first consumer: one supervisor task (started with the socket task, never in validate mode) prunes expired day folders once per local date and keeps one writer per configured mic while the toggle (`app_settings` `listen.recorder` = `{"enabledFor": "<today>"}`) names today; each writer batches frames and writes them in `spawn_blocking` (header sizes patched after every batch, so a crash leaves a readable file), opens its file lazily with the first frame, starts a new file whenever the audio drifts more than 1 s from the wall clock, drops the frames heard after midnight and ends, and backs off 1, 2, 4 … 60 min after a failed write. The Settings card is a thin card over the shared `list_card::ListCard`, which lane 1 first generalises from "host + port" connections to any editor form (`host_editor::EditorForm`, the connection form split out into `connection_form.rs`); the card polls `/listen/status` every 2 s while it is on screen.

**Tech Stack:** Rust 2021 (clippy msrv 1.85, toolchain floor 1.87 since C1a-3), tokio 1.52 (`sync::broadcast`, `sync::oneshot`, `task::spawn_blocking`, `time`), axum 0.8, chrono 0.4.44 (`NaiveDateTime`, `TimeDelta`), serde / thiserror / tracing; Leptos 0.7 CSR (presenter-ui, its own lockfile); Playwright (TypeScript) + Python 3 stdlib (the C1a VBAN sender `scripts/dev/listen/vban_send.py`) for the E2E.

**Spec:** `docs/superpowers/specs/2026-10-14-sermon-listening-interpreter-assist-design.md` (APPROVED by the owner 2026-10-14, #852) — §1 C-S1 / C-S8 / C-S10, §4, §5.4 (per-mic chain: 16 kHz mono s16le 100 ms frames, level, health), §5.5 (recorder), §5.6 (configuration, Settings → "Počúvanie", the ids `guest` / `interp`), §11 (VBAN rows), §12 (privacy, retention), §13 (bake-off inputs: the recordings + show-log times), §14 (observability), §15 (testing), §16 (C1b row, ≤ 600 LoC per PR, version bump first), §18 defaults 5 and 9, §19 (owner consent before recording).

**Builds on:** C1a, merged on `dev`. Base of this plan: `dev` @ 52271e2b (v0.4.322, bumped after the last release). Read `.claude/rules/listen-vban.md` and `.claude/rules/settings-ui.md` before any task; they auto-load on the touched paths.

**Review:** round 1 (2026-10-15) — lanes re-cut and MEASURED (R1); the connection form split out of `host_editor.rs` (Y1); rotation on drift instead of a 2 s gap (Y2); the status poll goes stale after 2 failures (Y3); the E2E waits for audio before switching off (Y4); header-write cleanup + restart back-off (B1); ids from the role, shown in the row (B2); the switch locked without a mic (B3); the checkbox bound to a `Memo<bool>` (B4); the midnight split (B5); a temp default folder under `cfg(test)` (B6); no 4 GiB guard, the whole-list race rule (B7). The table "Round-1 findings → where they landed" below maps each one. Lane 4 as suggested (writers + drift rotation, with the B1 / B5 fixes) measured 681 production lines, so the rotation, the midnight split, the back-off and the header cleanup became their own lane (C1b-5): seven lanes instead of six.

**How the code in this plan was checked (Tier-0):** every Rust and TypeScript block below was written into a scratch copy of the touched files at 52271e2b, formatted with `rustfmt --edition 2021` (syntax only; nothing compiled), run through `scripts/dev/fn_length_check.py` (no violations, no warnings) and `scripts/dev/count_prod_lines.sh`, and the E2E spec through `tsc --noEmit` with the repo's `tsconfig.json`. The **patches** below are `git diff` output against 52271e2b plus the previous lanes: apply them with `git apply` (or `git apply -3` if `dev` moved), never by retyping. The whole plan was then replayed: every Create / Apply step, in order, on a `git archive` of 52271e2b reproduces each lane head exactly, and on the last head `cargo fmt --check` (root and presenter-ui), `fn_length_check.py`, `no_panic_check.py`, `placeholder_check.sh`, `feature_router_check.sh` and `tsc --noEmit` pass on the full tree. CI is the first compiler.

---

## Decisions (the design points the supervisor asked for, with reasons)

1. **The card = `ListCard`, generalised first (lane 1).** `ListCard` / `ListEditor` are hard-wired to `label / host / port / enabled / extra`, and their buttons, confirm text and tooltips are English. A microphone has a name, a stream name, an optional sender IP and three selects. Lane 1 makes the editor generic over a small `EditorForm` trait (values, validation, field rows) with all copy in `CardText` / `EditorSpec`; the connection form moves to a new `settings/connection_form.rs` (275 production lines), so `host_editor.rs` keeps only the shared state machine (722 → 575). Resolume and Android keep their markup and copy byte-for-byte, pinned by the existing settings E2E specs (unchanged).
2. **Whole-list API behind per-mic save / delete.** `CardItem::save` / `delete` read `GET /listen/mics`, change one mic and `PUT` the whole list. Two tabs saving within that one round trip can lose an edit: accepted for one Settings card and at most 4 mics, and written into the rule (B7). A save of a mic deleted meanwhile is refused ("Tento mikrofón medzitým niekto odstránil."), never re-added. No new endpoints.
3. **A new mic's id comes from its ROLE** (core `mic_id_for_role`: guest → `guest`, interpreter → `interp`, preacher → `preacher`, then `-2`, `-3` …): the spec §5.6 ids, so the files are `guest_…wav` / `interp_…wav`. The id never changes on edit; the row shows it ("… · id guest").
4. **Health = a 2 s poll of the quiet `GET /listen/status`, only while the card is on screen** (`offsetParent`: the operator's Settings panel stays mounted when hidden), numbered by `ResponseOrder`. After 2 failed polls in a row the card shows nothing rather than stale numbers (`video_sources::is_stale`, badges "—"). No `LiveEvent`. `GET /listen/mics` becomes a quiet GET too (the list card's 5 s poll).
5. **The toggle is a per-DAY switch.** `listen.recorder` = `{"enabledFor": "YYYY-MM-DD"}` (the server's local date when switched on) or `{"enabledFor": null}`; the recorder records while that date is today. (a) Off by default, never on unless switched on (spec §12/§19); (b) **a restart the same day resumes** — auto-deploys can land mid-sermon; (c) **a forgotten switch costs at most one day**: VB-Matrix sends continuously (a muted mic sends silence, §5.7), so an always-on recorder would write ~5.5 GB/day and ~166 GB before the 30-day retention caught up. The card says "Nahrávať dnes" and "vypne sa samo o polnoci"; without a mic the switch is locked with the hint "Najprv pridaj mikrofón" (an ON switch always stays switchable off).
6. **Recordings root** = env `PRESENTER_LISTEN_RECORDINGS_DIR`, else `recordings` next to the sqlite file of `PRESENTER_DB_URL` (`state::stream_assets::dir_next_to_db`): prod/PP `/opt/presenter/recordings` (the spec's path), dev `/opt/presenter-dev/recordings` — inside the units' `ReadWritePaths`, never touched by deploys; no unit change. Unit tests default to a temp dir (`cfg(test)`, B6), never `./recordings` inside the crate; the E2E spec uses its own temp dir.
7. **One file per continuous stretch, named by its start:** `<root>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav` (`-2` … `-9` within one second). Opened lazily with the first frame (dev, whose senders send to prod, writes nothing). Before each batch the writer computes when the batch began (now − its frames − the frames still queued) and compares it with the end of the audio already in the file (its start + its length); more than 1 s apart — a pause of the sender (0.1–2 s pauses add up), a resync, `Lagged(n)`, a slow disk — closes the file and starts a new one named at the batch's start (Y2); the "recording started" INFO carries the millisecond start. So every file is continuous audio from the time in its name: the bake-off places the `presenter::bible::show` times (§13) without parsing the journal, and the writer never reopens, appends to or truncates a file. Spec deviation (§5.5 says `<mic>.wav`), recorded on #852.
8. **WAV correctness:** canonical 44-byte PCM header (byte-checked against Python's `wave`), sizes patched after EVERY batch (a crash or SIGKILL leaves a file whose header covers the last completed batch), `fsync` on close; `create_new` only (a symlink at the name counts as taken), a linked day folder refused, a new file removed again when not even its header can be written (B1). No 4 GiB guard (B7): a mic's day is at most 2.76 GB and ends at midnight; `wav_riff_size` saturates.
9. **File I/O off the runtime, never back-pressure:** one `spawn_blocking` per batch of ≤ 50 frames; the broadcast ring (128 frames) is the bounded queue. A lagging writer drops the oldest frames, counts them (`droppedFrames`) and WARNs on power-of-two lag events; a failed write sets `error` (Slovak), WARNs on power-of-two failures in a row and ends the writer; the supervisor restarts it after 1, 2, 4 … 60 min (B1; wall clock, so tests drive it with the settable clock). Switching off clears every error and back-off.
10. **Midnight (B5):** the frames of a batch heard after midnight (each frame is 100 ms after the previous) are dropped, the ones before are written, and the writer ends; no writer starts until the switch names the new day.
11. **Retention:** once per local date (startup, then the first tick after midnight) the supervisor deletes day folders older than 30 days (a folder dated D goes when today − D > 30): only canonical `YYYY-MM-DD` folders, only `<mic id>_<HHMMSS>[-n].wav` regular files, the folder only when emptied; `DirEntry::file_type` never follows a symlink; one INFO per deleted file. Ships in lane 2, before anything can record. **Stated deviation from C-S8 (B3):** the supervisor's 60 s tick also runs on SNV and PP without a mic (it has nothing to do there but the daily retention pass); no socket, no writer, no audio.
12. **No delete-all button (YAGNI).** "Delete right after the bake-off report" is a one-off on prod data, done with the owner's go (`rm` of the listed day folders) or left to the retention.
13. **Logging** (`presenter::listen::recorder`): INFO switched on / off (actor, day), recording started (mic, file, millisecond start), recording stopped (mic, file, bytes, why: switched off or the mic was removed / a gap in the audio / the day ended / a write failed), recording deleted (retention), folder kept; WARN through the #484 power-of-two gate: fell behind, write failed (with the restart pause).
14. **Privacy:** the toggle ships off everywhere and the plan never switches it on at SNV or PP. The owner's consent (spec §19) is asked by the supervisor; the per-day switch needs no separate owner decision (see "After C1b").

### Round-1 findings → where they landed

| Finding | Change | Pinned by |
|---|---|---|
| 🔴 R1 lane sizes | 7 lanes measured with the script below: 561 / 515 / 140 / 594 / 118 / 500 / 341; every close task re-measures | each close task's "Measure the lane" step |
| 🟡 Y1 `host_editor.rs` size | `connection_form.rs` (ConnectionForm, ConnectionSpec, DraftValues, Submission, ExtraField, `validate_connection`, `validate_draft`, `render_extra_field`, `connection_new_values` + their tests); `host_editor.rs` 575 | Task 3 |
| 🟡 Y2 rotation | drift rule (> 1 s between the file's audio end and the batch start), file named at the batch start, ms `started` in the INFO; no `ROTATE_AFTER_GAP` | Task 18: `a_pause_of_the_sender_starts_a_new_file_named_by_its_start`, `a_lag_in_the_middle_of_a_file_starts_a_new_one_at_the_right_time`, `a_batch_more_than_a_second_after_the_files_audio_drifted` |
| 🟡 Y3 stale status | `after_poll`: 2 failures in a row → `None`, badges "—", one console warn | Task 26: `one_failed_poll_keeps_the_last_status_and_two_show_nothing` |
| 🟡 Y4 E2E race | `expect.poll(… recorder.mics[0].bytes).toBeGreaterThan(16_000)` before `uncheck()` | Task 27 |
| 🔵 B1 | `RecordingFile::begin` deletes the file when the header write fails; restart back-off 1, 2, 4 … 60 min | Task 14 `a_failing_write_after_a_good_create_changes_no_size_and_says_why`; Task 18 `a_header_that_cannot_be_written_leaves_no_file_behind`, `a_failed_writer_waits_1_2_4_up_to_60_minutes`, `a_failed_writer_is_not_restarted_before_its_pause` |
| 🔵 B2 | id from the role, "id …" in the row aside | Task 2 `a_new_mics_id_comes_from_its_role_and_is_never_taken_twice`; Task 22 `a_save_changes_one_mic_in_place_and_a_new_one_gets_its_roles_id`, `the_row_shows_the_id_and_its_key_every_field_it_shows_or_edits` + Task 23 E2E (`li[data-id="guest"]`) |
| 🔵 B3 | switch disabled with no mic + Slovak hint; the 60 s tick without mics stated as a deviation | Task 27 (disabled + hint); Decision 11 |
| 🔵 B4 | `prop:checked` bound to `Memo<bool>` of `recorder.enabled` only | Task 26 |
| 🔵 B5 | `split_at_midnight` keeps the frames before midnight | Task 18 `only_the_frames_heard_before_midnight_stay_in_a_batch`, `a_batch_across_midnight_keeps_only_the_frames_heard_before_it` |
| 🔵 B6 | `cfg(test)` `default_root()` = `<temp>/presenter-unit-test-recordings` | Task 7 `the_recordings_folder_is_the_env_value_or_next_to_the_database` |
| 🔵 B7 | no `WAV_MAX` / `Full`; the whole-list read-modify-write race line in `settings-ui.md` | Task 6 `the_riff_size_is_the_audio_plus_36_and_never_wraps`; Task 24 rule |
| consent wording | "After C1b" names the per-day switch | "After C1b" |

## Global Constraints

- **Seven lanes, seven PR units**, each from `origin/dev` with the previous lane merged, MEASURED with the script below on the plan's own code (production lines added or changed):

| Lane | Content | Measured |
|---|---|---|
| C1b-1 | generic list editor (`EditorForm`), `connection_form.rs`, core `mic_id_for_role` | **561** |
| C1b-2 | recorder contract + WAV header (core), recordings folder + `dir_next_to_db`, 30-day retention, frame channel | **515** |
| C1b-3 | `PUT /listen/recorder`, startup read, audit row, `ListenStatus.recorder` (every mic `off`) | **140** |
| C1b-4 | `RecordingFile`, `RecorderHealth`, the writers | **594** |
| C1b-5 | drift rotation, midnight split, restart back-off, header-write cleanup | **118** |
| C1b-6 | card list + editor (`MicForm`), client, quiet `GET /listen/mics`, E2E add / 422 / edit / delete / 360 px | **500** |
| C1b-7 | health / socket / recorder lines, 2 s poll with staleness, the per-day switch, E2E with a real sender + WAV check | **341** |

  Every lane ≤ 600 (spec §16). If the supervisor prefers six PRs, C1b-5 folds into C1b-7 (118 + 341 = 459) — both touch disjoint files, so only the commit order changes. Each lane compiles and is tested at its own head; no `pub(crate)` / private item without a production caller (test-only accessors are `#[cfg(test)]`); `pub` core API may precede its first caller (as C1a-1 did).
- **Measure honestly** — save this as `$SCRATCH/lane_measure.sh` once (Write tool) and run it from the repo root in every close task (`bash "$SCRATCH/lane_measure.sh"`). It applies `scripts/dev/count_prod_lines.sh` to both sides of every touched `.rs` file (a `#[cfg(test)] mod tests;` DECLARATION does not stop the count; an inline test module does), counts CSS whole, skips `tests.rs` / `tests/` / specs / docs, and counts the `+` lines of moved code too:

```bash
#!/usr/bin/env bash
# Production lines a lane adds or changes (scripts/dev/count_prod_lines.sh rules: a file's
# lines before its INLINE #[cfg(test)] module; tests.rs, tests/, *.spec.ts, docs excluded).
# Usage: bash "$SCRATCH/lane_measure.sh" [base, default origin/dev]
set -euo pipefail
base=$(git merge-base "${1:-origin/dev}" HEAD)
cd "$(git rev-parse --show-toplevel)"
tmp=$(mktemp -d)
total=0
for f in $(git diff --name-only "$base" HEAD); do
  case "$f" in
    *tests.rs | */tests/*.rs | tests/* | *.md | scripts/*) continue ;;
    *.rs | *.css) ;;
    *) continue ;;
  esac
  git cat-file -e "HEAD:$f" 2>/dev/null || continue
  git show "HEAD:$f" > "$tmp/new"
  if git cat-file -e "$base:$f" 2>/dev/null; then git show "$base:$f" > "$tmp/old"; else : > "$tmp/old"; fi
  if [[ "$f" == *.rs ]]; then
    head -n "$(bash scripts/dev/count_prod_lines.sh "$tmp/new")" "$tmp/new" > "$tmp/after"
    if [ -s "$tmp/old" ]; then
      head -n "$(bash scripts/dev/count_prod_lines.sh "$tmp/old")" "$tmp/old" > "$tmp/before"
    else
      : > "$tmp/before"
    fi
  else
    cp "$tmp/new" "$tmp/after"
    cp "$tmp/old" "$tmp/before"
  fi
  added=$( (diff "$tmp/before" "$tmp/after" || true) | grep -c '^>' || true)
  printf '%-72s +%s\n' "$f" "$added"
  total=$((total + added))
done
echo "lane total: $total production lines added or changed"
rm -rf "$tmp"
```

- **Version bump first** (`version-bumping`): each lane's FIRST commit runs the version check (Task 1) and bumps `[workspace.package].version` when dev ≤ main. On 52271e2b dev is 0.4.322 (bumped after the release); a lane bumps only if main caught up again.
- **Tier-0:** NO local compiling cargo (`build` / `check` / `clippy` / `test` / `run`); CI compiles and runs every Rust test. Allowed locally: `cargo fmt --all` (+ `-- --check`), `(cd crates/presenter-ui && cargo fmt --check)`, `cargo metadata --offline`, the Python / bash gates, `python3 -m unittest`, `node node_modules/typescript/bin/tsc --noEmit -p tsconfig.json`. Never read secret files or credential stores; never put personal data in a network request, header or User-Agent.
- **Size gates on EVERY touched `.rs` file:** `bash scripts/dev/count_prod_lines.sh <file>` (warn > 800, fail > 1000) and `QC_TARGETS="<newline-separated files>" python3 scripts/dev/fn_length_check.py .` (warn > 80, fail > 120). Measured after the last lane: `host_editor.rs` 575, `connection_form.rs` 275, `list_card.rs` 416, `settings/listen.rs` 767, `listen/recorder.rs` 549, `listen/recording_file.rs` 219, `listen/health.rs` 655, `listen/mod.rs` 231, `state/listen.rs` 178, core `listen/recorder.rs` 153 — all under 800; `listen/recorder/tests.rs` (815) is a `tests.rs` file and exempt. `state/mod.rs` and `listen/vban_rx.rs` are not touched.
- **No `unwrap()` / `expect()` / `panic!` in server / core production code** (`python3 scripts/dev/no_panic_check.py .`); presenter-ui is exempt but kept panic-free. A `std::sync::Mutex` is locked with `.unwrap_or_else(PoisonError::into_inner)` and never held across an `.await`. chrono arithmetic uses `checked_add_signed` / `checked_sub_signed` with a fallback, never `+` / `-` in production.
- **Clippy** = `cargo clippy --workspace --all-targets --all-features -- -D warnings -W clippy::all`, msrv 1.85: no std API newer than 1.85 (`io::ErrorKind::DirectoryNotEmpty` 1.83, `Option::is_none_or` 1.82, `is_some_and` 1.70 are fine); wrapped `///` list items indented (`.claude/rules/quality-gates.md`). presenter-ui is clippy-excluded in CI but must compile, pass its unit tests and `cargo fmt --check` in its own directory.
- **Operator-facing text is Slovak** (the card, its editor, the toasts, the status `error` strings); log messages English. The connection cards keep their English copy byte-for-byte.
- **Public repo:** no e-mails, keys or real IPs in code or tests beyond the spec's; tests use RFC 5737 addresses (`192.0.2.10`, `198.51.100.7`, `203.0.113.9`) and loopback. `$DEV` / `$DEV_HOST` come from CLAUDE.md "Deployed Instances".
- **Tests:** features, not bug fixes (no RED/GREEN commits), but every task ships its tests in its lane, verified by CI. Existing test files are read-only except where a task's patch shows the exact edit and its step says why (a wire shape that grows, an API that changed shape, a GET that became polled, tests that move with their code). Every Playwright test ends with the zero-console-errors assertion (a real 422 is count-asserted exactly once, `.claude/rules/settings-ui.md`).
- **Commits:** `type(scope): … (#852 C1b-N)`; `[no-test: <reason>]` on a commit without tests; every message ends with a blank line + `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`. Stage explicit paths only; never rewrite history.
- **Flow:** two-branch (`dev` → PR → `main`, merge commit); the supervisor integrates lanes into `dev` and opens release PRs when dev is quiet. Before a push load `ci-push-discipline`; after it monitor the run to a terminal state (`ci-monitoring`, bounded foreground polls, never `gh run watch`); after a deploy `post-deploy-verification`; after each lane `playbook-review`.

Shell variables used below:

```bash
REPO=/home/newlevel/devel/presenter/presenter-dev2   # or your worktree root
SCRATCH=<your session scratchpad directory>           # the system prompt lists it
DEV=<dev instance URL, CLAUDE.md "Deployed Instances">  # http://<host>:8080
DEV_HOST=<the dev instance's host from the same table>
SNV=http://10.77.9.205
PP=http://companion-pp.lan
```

**Patch recipe** (every "Apply" step): save the block verbatim to `$SCRATCH/<task>.patch` with the Write tool, then `git -C "$REPO" apply --recount --check "$SCRATCH/<task>.patch" && git -C "$REPO" apply --recount "$SCRATCH/<task>.patch"` (`--recount` tolerates a blank context line whose single space got lost in copying; if `dev` moved: add `-3`; a hunk that still fails is applied by hand from the block, which is plain unified-diff text). A "Create" step writes the file verbatim with the Write tool.

**Commit recipe** (every "Commit" step): write the message to `$SCRATCH/msg.txt` with the Write tool (subject, blank line, body, blank line, the `Co-Authored-By:` line), run `cargo fmt --all` (and `(cd crates/presenter-ui && cargo fmt)` when the UI changed; both must leave the plan's code unchanged — a diff means a block was mangled), then `git -C "$REPO" add <the listed paths> && git -C "$REPO" commit -F "$SCRATCH/msg.txt"`.

**Local gates** (every close task, from `$REPO`):

```bash
cargo fmt --all -- --check && (cd crates/presenter-ui && cargo fmt --check)
bash scripts/dev/placeholder_check.sh . && bash scripts/dev/feature_router_check.sh .
QC_TARGETS="$(git diff --name-only origin/dev...HEAD -- '*.rs')" python3 scripts/dev/fn_length_check.py .
for f in $(git diff --name-only origin/dev...HEAD -- '*.rs'); do printf '%s ' "$f"; bash scripts/dev/count_prod_lines.sh "$f"; done
python3 scripts/dev/no_panic_check.py .
git status --short    # nothing unexpected
```

**Push + CI** (every close task): load `ci-push-discipline`; `git fetch origin && git merge origin/dev`; push as the supervisor directs; monitor `pipeline.yml` to a terminal state — every job; a red job: `gh run view <id> --log-failed`, root cause, ONE fix commit.

## Review Focus

The spec says what the recorder must do; these five conditions it implies but does not spell out are the most likely to bite during a real service. Each line names the tests that pin it in their owning task.

1. **A deploy or crash mid-sermon while recording.** The audio recorded so far must stay readable (its header covers every completed batch), the same day's recording must continue into a NEW file — never truncating or rewriting the old one — and a restart the next day must not resume. Tests: `every_batch_leaves_a_header_that_covers_it` (Task 14), `a_restart_the_same_day_records_into_a_new_file_and_keeps_the_old_one` (Task 15), `the_startup_resumes_a_recorder_switched_on_today_but_not_yesterdays` (Task 11).
2. **A slow or full disk.** The chain and the receive loop never wait for the disk: a writer more than 12.8 s behind loses the oldest frames and counts them; a failed write shows `error`, WARNs on power-of-two failures, leaves no empty file and is retried after 1, 2, 4 … 60 min, not every minute. Tests: `a_writer_that_falls_behind_loses_the_oldest_frames_and_counts_them`, `a_failed_write_is_shown_warned_on_power_of_two_failures_and_restarted` (Task 15), `a_failing_write_after_a_good_create_changes_no_size_and_says_why` (Task 14), `a_failed_writer_is_not_restarted_before_its_pause`, `a_header_that_cannot_be_written_leaves_no_file_behind` (Task 18).
3. **The switch is forgotten.** Recording ends at midnight (frames heard after it are not written) and no writer starts the next day, although VB-Matrix keeps sending. Tests: `the_recording_ends_at_midnight_and_no_writer_starts_the_next_day` (Task 15), `a_batch_across_midnight_keeps_only_the_frames_heard_before_it` (Task 18), the startup test (Task 11).
4. **Retention deletes only what the recorder wrote.** A symlinked folder, a foreign file in a day folder, a folder not named like a date, a file not named like a recording and the 30th day all survive. Tests: `a_linked_folder_is_never_followed`, `a_folder_with_other_files_or_another_name_is_kept`, `old_day_folders_go_file_by_file_and_the_30th_day_stays` (Task 7).
5. **The FOH sender stops for a while mid-sermon, or the writer stalls.** The audio before and after must both be kept, each in a file named by its own start, within 1 s of the wall clock. Tests: `a_pause_of_the_sender_starts_a_new_file_named_by_its_start` (a 0.6 s pause keeps the file, a 1.5 s pause starts one), `a_lag_in_the_middle_of_a_file_starts_a_new_one_at_the_right_time` (Task 18).

---

## File map

| Lane | File | Responsibility |
|---|---|---|
| 1 | `crates/presenter-core/src/listen/mics.rs` | `mic_id_for_role` |
| 1 | `crates/presenter-ui/src/pages/settings/host_editor.rs` | the generic editor: `EditorForm`, `EditorStatus`, `ListEditor<F>`, `render_editor`, copy in `EditorSpec` |
| 1 | `crates/presenter-ui/src/pages/settings/connection_form.rs` (new) | `ConnectionForm` / `ConnectionSpec` / `DraftValues` / `Submission` / `ExtraField` / validation / the extra field row |
| 1 | `crates/presenter-ui/src/pages/settings/{list_card,resolume,android,mod}.rs` | `CardItem::Form` / `address` / `values` / `timestamps`, `CardText<F>`, `failure_text`; the two connection cards on `ConnectionForm` |
| 2 | `crates/presenter-core/src/audio/{mod,wav}.rs`, `crates/presenter-core/src/listen/{mod,recorder,mics}.rs` | WAV header; setting, toggle, status DTOs, file layout, retention rule |
| 2 | `crates/presenter-server/src/listen/{recorder,recording_file}.rs`, `listen/recorder/tests.rs`, `state/stream_assets.rs` | folder + clock (`RecorderEnv`), the supervisor's retention pass, `prune_recordings`, `dir_next_to_db` |
| 2 | `crates/presenter-server/src/listen/{mod,mic_chain,health}.rs` | the frame channel; `record_chain → bool` |
| 3 | `crates/presenter-core/src/listen/{recorder,status}.rs`, `crates/presenter-server/src/listen/{mod,health}.rs`, `listen/recorder/tests.rs`, `state/listen.rs`, `router/listen.rs` | `MicRecording::off`, `ListenStatus.recorder`, the toggle watch, `PUT /listen/recorder`, startup read, audit |
| 4 | `crates/presenter-server/src/listen/recording_file.rs` | `RecordingFile` |
| 4 | `crates/presenter-server/src/listen/{recorder,health,mod}.rs`, `listen/recorder/tests.rs` | the writers, `RecorderHealth` |
| 5 | `crates/presenter-server/src/listen/{recorder,health,recording_file}.rs`, `listen/recorder/tests.rs` | drift rotation, midnight split, back-off, header-write cleanup |
| 6 | `crates/presenter-ui/src/api/{mod,listen}.rs`, `crates/presenter-server/src/router/request_log.rs` | `put_json_refusal`, the client, quiet `GET /listen/mics` |
| 6 | `crates/presenter-ui/src/pages/settings/{listen,mod}.rs`, `crates/presenter-ui/styles/settings.css`, `tests/e2e/settings-listen.spec.ts` | the card's list + editor; `select` in the shared form row; E2E |
| 7 | `crates/presenter-ui/src/api/listen.rs`, `crates/presenter-ui/src/pages/settings/listen.rs`, `tests/e2e/settings-listen.spec.ts` | health / socket / recorder lines, the poll, the switch; E2E |
| 1–7 | `.claude/rules/{settings-ui,listen-vban}.md`, `docs/configuration.md`, `CLAUDE.md` | playbook, docs (each lane its own part) |

---

## PR unit C1b-1 — the list card's editor over any form (561 lines)

Starts from `origin/dev`. No behaviour change on screen: the Resolume and Android cards keep their markup, `data-role`s, copy and guards; the existing specs `tests/e2e/settings-inline-edit.spec.ts`, `settings-aria.spec.ts`, `settings.spec.ts`, `operator-settings-native.spec.ts` are the regression net and are NOT edited. Core gains `mic_id_for_role` (its first caller is lane 6's card).

### Task 1: Version check (bump only when dev ≤ main)

**Files:**
- Modify (only if needed): `Cargo.toml` (`[workspace.package] version`), `Cargo.lock` (the workspace-member entries)

**Interfaces:** none.

- [ ] **Step 1: Compare the versions** (Tasks 5, 10, 13, 17, 20 and 25 reuse this recipe)

```bash
cd "$REPO" && git fetch origin -q && git merge --ff-only origin/dev
MAIN=$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)
DEV=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "main=$MAIN dev=$DEV"
python3 -c 'import sys; a=tuple(map(int,sys.argv[1].split("."))); b=tuple(map(int,sys.argv[2].split("."))); print("BUMP" if a<=b else "OK")' "$DEV" "$MAIN"
```

Expected on 52271e2b: `main=0.4.321 dev=0.4.322`, `OK` → done. `BUMP` (a release merged since) → Steps 2–3.

- [ ] **Step 2: Bump to the next patch without compiling**

```bash
NEXT=$(python3 -c 'import sys; a, b, c = map(int, sys.argv[1].split(".")); print(f"{a}.{b}.{c + 1}")' "$MAIN")
sed -i "0,/^version = \"$DEV\"\$/s//version = \"$NEXT\"/" Cargo.toml
cargo metadata --format-version 1 --offline > /dev/null
grep -m1 '^version' Cargo.toml && git diff --stat    # Cargo.toml 1 line, Cargo.lock the member entries
```

- [ ] **Step 3: Commit** — paths `Cargo.toml Cargo.lock`, message `chore: bump version to <NEXT> [no-test: version bump only]`.

### Task 2: Core — a new microphone's id from its role

**Files:**
- Modify: `crates/presenter-core/src/listen/mics.rs` (new fn after `validate_mics`; a test appended to its `mod tests`)

**Interfaces:**
- Consumes: `MicRole`, `LISTEN_MAX_MICS` (same file).
- Produces: `pub fn mic_id_for_role(role: MicRole, taken: &[&str]) -> String` (`presenter_core::listen::mics::mic_id_for_role`, no root re-export) — lane 6's card calls it for every new mic. Ids are `guest` / `interp` / `preacher`, then `-2` … (spec §5.6); a validated list holds at most 4 mics, so one of the 5 candidates is always free (the `unwrap_or_else` only keeps the function total).

- [ ] **Step 1: Apply** (patch recipe):

```diff
diff --git a/crates/presenter-core/src/listen/mics.rs b/crates/presenter-core/src/listen/mics.rs
index 7bfb0b7..ae5197e 100644
--- a/crates/presenter-core/src/listen/mics.rs
+++ b/crates/presenter-core/src/listen/mics.rs
@@ -113,6 +113,25 @@ pub fn validate_mics(mics: Vec<MicConfig>) -> Result<Vec<MicConfig>, MicsValidat
     Ok(valid)
 }
 
+/// The id of a new mic (#852 C1b: the Settings card never asks for one; the id
+/// names the mic's recording files and never changes): its role's short name —
+/// `guest`, `interp`, `preacher`, the spec §5.6 ids — then `-2`, `-3` … while
+/// that is taken. Always a valid id.
+pub fn mic_id_for_role(role: MicRole, taken: &[&str]) -> String {
+    let base = match role {
+        MicRole::Guest => "guest",
+        MicRole::Interpreter => "interp",
+        MicRole::Preacher => "preacher",
+    };
+    (1..=LISTEN_MAX_MICS + 1)
+        .map(|n| match n {
+            1 => base.to_string(),
+            n => format!("{base}-{n}"),
+        })
+        .find(|id| !taken.contains(&id.as_str()))
+        .unwrap_or_else(|| format!("{base}-{}", taken.len() + 1))
+}
+
 fn normalize_mic(mut mic: MicConfig) -> Result<MicConfig, MicsValidationError> {
     if !is_valid_mic_id(&mic.id) {
         return Err(MicsValidationError::BadId(mic.id));
@@ -363,4 +382,29 @@ mod tests {
         let json = SPEC_EXAMPLE.replacen(r#""channel": "left""#, r#""channel": "center""#, 1);
         assert!(serde_json::from_str::<Vec<MicConfig>>(&json).is_err());
     }
+
+    #[test]
+    fn a_new_mics_id_comes_from_its_role_and_is_never_taken_twice() {
+        assert_eq!(mic_id_for_role(MicRole::Guest, &[]), "guest");
+        assert_eq!(mic_id_for_role(MicRole::Interpreter, &[]), "interp");
+        assert_eq!(mic_id_for_role(MicRole::Preacher, &[]), "preacher");
+        assert_eq!(mic_id_for_role(MicRole::Guest, &["guest"]), "guest-2");
+        assert_eq!(
+            mic_id_for_role(MicRole::Guest, &["guest", "guest-2", "guest-3", "guest-4"]),
+            "guest-5",
+            "a full list of 4 still gets a free id"
+        );
+        for id in ["guest", "interp", "preacher", "guest-5"] {
+            assert!(is_valid_mic_id(id), "{id}");
+        }
+        let spec = validate_mics(serde_json::from_str(SPEC_EXAMPLE).unwrap()).unwrap();
+        assert_eq!(
+            (spec[0].id.as_str(), spec[1].id.as_str()),
+            (
+                mic_id_for_role(MicRole::Guest, &[]).as_str(),
+                mic_id_for_role(MicRole::Interpreter, &[]).as_str()
+            ),
+            "the spec §5.6 ids"
+        );
+    }
 }
```

The test also checks that the function's ids equal the spec §5.6 example list's ids (`SPEC_EXAMPLE`, already in the test module).

- [ ] **Step 2: Gates** — `cargo fmt --all -- --check`; `QC_TARGETS="crates/presenter-core/src/listen/mics.rs" python3 scripts/dev/fn_length_check.py .`; `python3 scripts/dev/no_panic_check.py .`.

- [ ] **Step 3: Commit** — path `crates/presenter-core/src/listen/mics.rs`, message `feat(listen): a new microphone's id from its role (#852 C1b-1)`.

### Task 3: The settings list card's editor over any form

**Files:**
- Create: `crates/presenter-ui/src/pages/settings/connection_form.rs`
- Modify: `crates/presenter-ui/src/pages/settings/host_editor.rs`, `list_card.rs`, `resolume.rs`, `android.rs`, `mod.rs` (`mod connection_form;`)

**Interfaces:**
- `host_editor` produces (all `pub(super)`): `trait EditorForm: Copy + Send + Sync + 'static { type Values: Clone + Default + PartialEq + Send + Sync; type Submission; type Config: Copy + Send + Sync; fn new(Config) -> Self; fn new_values(self) -> Values; fn load(self, &Values); fn values(self) -> Values /* tracked */; fn values_untracked(self) -> Values; fn validate(self, &Values) -> Result<Submission, &'static str>; fn render_fields(self, EditorSpec, NodeRef<Input>, Signal<bool>) -> AnyView; fn render_action_lead(self, EditorSpec) -> AnyView }`; `ListEditor<F: EditorForm>` (`new(F::Config)`, `open_new`, `open_item(id, F::Values)`, `begin_save() -> Option<(SaveTicket, F::Submission)>`, unchanged guards); `render_editor<F, S>`; `EditorSpec` gains `save_text`, `cancel_text`, `unsaved_title`, `saving_title`; `TriggerLock::title(&EditorSpec)`; `UNSAVED_TITLE` / `SAVING_TITLE` (the connection cards' English tooltips).
- `connection_form` produces (`pub(super)`): `ConnectionForm` (implements `EditorForm` with `Values = DraftValues`, `Submission = Submission`, `Config = ConnectionSpec`), `ConnectionSpec { label_placeholder, host_placeholder, default_port, extra: Option<ExtraField> }`, `DraftValues`, `Submission`, `ConnectionFields`, `ExtraField`, `validate_connection`, `validate_draft`, `connection_new_values`. The validation tests move here WITH their code (same bodies, imports adjusted) — that is the one edit to existing tests in this task, besides `a_lock_says_why_on_the_button` which now passes the spec to `title`.
- `list_card` produces: `CardItem { type Form: EditorForm; …; fn address(&self) -> (String, String); fn values(&self) -> <Form as EditorForm>::Values; fn is_enabled(&self) -> bool { true }; fn timestamps(&self) -> Option<String>; async fn save(Option<String>, <Form as EditorForm>::Submission) }`, `CardText<F: EditorForm>` (adds `form: F::Config`, `edit_text`, `delete_text`, `delete_question: (&str, &str)`), `pub(super) fn failure_text(prefix, &ApiError) -> String` (an `ApiError::Refused` or `NotFound` shows the server's / card's own message — the 422 path lane 6 needs; unit-tested here).
- Consumed by: lanes 6–7 (`MicForm`, the listening card).

- [ ] **Step 1: Create `crates/presenter-ui/src/pages/settings/connection_form.rs`** (Write tool):

```rust
//! #852 C1b: the connection form of the Resolume and Android list cards — label,
//! host, port, the card's optional extra field (Android's launch package) and
//! "Enabled" — as an `host_editor::EditorForm`. Split out of `host_editor` (#819),
//! which keeps only the editor state machine every list card shares; the
//! markup, the `data-role`s, the copy and the checks are the #819 ones.

use leptos::prelude::*;

use super::host_editor::{EditorForm, EditorSpec};
use super::parse_port_in_range;

/// The validated label / host / port of an editor submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ConnectionFields {
    pub(super) label: String,
    pub(super) host: String,
    pub(super) port: u16,
}

/// Validate the fields both connection cards share, in the order the operator
/// sees them.
///
/// The port input has no native `min`/`max`, so an out-of-range value (99999)
/// reaches this check and gets the styled message instead of the browser silently
/// blocking the submit — this is the single authority for the 1..=65535 range (#455).
pub(super) fn validate_connection(
    label: &str,
    host: &str,
    port: &str,
) -> Result<ConnectionFields, &'static str> {
    let label = label.trim();
    if label.is_empty() {
        return Err("Label cannot be empty.");
    }
    let host = host.trim();
    if host.is_empty() {
        return Err("Host cannot be empty.");
    }
    let port = parse_port_in_range(port).ok_or("Port must be between 1 and 65535.")?;
    Ok(ConnectionFields {
        label: label.to_string(),
        host: host.to_string(),
        port,
    })
}

/// A card's own extra text field in the editor (Android's launch package). Its value
/// is part of the draft like every other field.
#[derive(Clone, Copy, Debug)]
pub(super) struct ExtraField {
    pub(super) caption: &'static str,
    pub(super) role: &'static str,
    pub(super) placeholder: &'static str,
    /// The value a new item starts with.
    pub(super) default: &'static str,
    /// The message when it is left empty.
    pub(super) empty_message: &'static str,
}

/// What a connection editor holds: the values it was opened with, and what is on
/// screen. The two differ exactly when the editor has unsaved changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DraftValues {
    pub(super) label: String,
    pub(super) host: String,
    /// The port as typed (the input's text), validated only on save.
    pub(super) port: String,
    pub(super) enabled: bool,
    /// The card's extra field (Android's launch package); empty when it has none.
    pub(super) extra: String,
}

/// A validated connection submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Submission {
    pub(super) fields: ConnectionFields,
    pub(super) enabled: bool,
    /// The trimmed extra field; empty when the card has none.
    pub(super) extra: String,
}

/// Validate an editor's values: the shared fields first, then the card's extra field
/// (required when the card has one), in the order the operator sees them.
pub(super) fn validate_draft(
    values: &DraftValues,
    extra: Option<&ExtraField>,
) -> Result<Submission, &'static str> {
    let fields = validate_connection(&values.label, &values.host, &values.port)?;
    let extra_value = values.extra.trim();
    if let Some(field) = extra {
        if extra_value.is_empty() {
            return Err(field.empty_message);
        }
    }
    Ok(Submission {
        fields,
        enabled: values.enabled,
        extra: extra_value.to_string(),
    })
}

/// What differs between the Resolume and the Android connection form.
#[derive(Clone, Copy, Debug)]
pub(super) struct ConnectionSpec {
    pub(super) label_placeholder: &'static str,
    pub(super) host_placeholder: &'static str,
    /// The port a new item starts with.
    pub(super) default_port: u16,
    pub(super) extra: Option<ExtraField>,
}

/// The values a new connection starts with: blank label and host, the card's
/// port, enabled, the extra field's default.
pub(super) fn connection_new_values(spec: &ConnectionSpec) -> DraftValues {
    DraftValues {
        port: spec.default_port.to_string(),
        enabled: true,
        extra: spec
            .extra
            .map(|e| e.default)
            .unwrap_or_default()
            .to_string(),
        ..DraftValues::default()
    }
}

/// The connection form's signals plus its card's spec. `Copy`.
#[derive(Clone, Copy)]
pub(super) struct ConnectionForm {
    label: RwSignal<String>,
    host: RwSignal<String>,
    port: RwSignal<String>,
    enabled: RwSignal<bool>,
    extra: RwSignal<String>,
    spec: ConnectionSpec,
}

impl EditorForm for ConnectionForm {
    type Values = DraftValues;
    type Submission = Submission;
    type Config = ConnectionSpec;

    fn new(spec: ConnectionSpec) -> Self {
        Self {
            label: RwSignal::new(String::new()),
            host: RwSignal::new(String::new()),
            port: RwSignal::new(String::new()),
            enabled: RwSignal::new(true),
            extra: RwSignal::new(String::new()),
            spec,
        }
    }

    fn new_values(self) -> DraftValues {
        connection_new_values(&self.spec)
    }

    fn load(self, values: &DraftValues) {
        self.label.set(values.label.clone());
        self.host.set(values.host.clone());
        self.port.set(values.port.clone());
        self.enabled.set(values.enabled);
        self.extra.set(values.extra.clone());
    }

    fn values(self) -> DraftValues {
        DraftValues {
            label: self.label.get(),
            host: self.host.get(),
            port: self.port.get(),
            enabled: self.enabled.get(),
            extra: self.extra.get(),
        }
    }

    fn values_untracked(self) -> DraftValues {
        DraftValues {
            label: self.label.get_untracked(),
            host: self.host.get_untracked(),
            port: self.port.get_untracked(),
            enabled: self.enabled.get_untracked(),
            extra: self.extra.get_untracked(),
        }
    }

    fn validate(self, values: &DraftValues) -> Result<Submission, &'static str> {
        validate_draft(values, self.spec.extra.as_ref())
    }

    fn render_fields(
        self,
        spec: EditorSpec,
        first: NodeRef<leptos::html::Input>,
        invalid: Signal<bool>,
    ) -> AnyView {
        let role = move |field: &str| format!("{}-{field}", spec.role);
        let aria_invalid = move || invalid.get().to_string();
        let extra = self
            .spec
            .extra
            .map(|field| render_extra_field(field, spec.message_id, self.extra, invalid));
        view! {
            <>
                <div class="settings__form-row settings__form-row--connection">
                    <label>
                        <span>"Label"</span>
                        <input type="text" node_ref=first data-role=role("label")
                            placeholder=self.spec.label_placeholder required
                            aria-required="true"
                            aria-describedby=spec.message_id
                            aria-invalid=aria_invalid
                            prop:value=move || self.label.get()
                            on:input=move |ev| self.label.set(event_target_value(&ev)) />
                    </label>
                    <label>
                        <span>"Hostname or DNS"</span>
                        <input type="text" data-role=role("host") placeholder=self.spec.host_placeholder required
                            aria-required="true"
                            aria-describedby=spec.message_id
                            aria-invalid=aria_invalid
                            prop:value=move || self.host.get()
                            on:input=move |ev| self.host.set(event_target_value(&ev)) />
                    </label>
                    <label class="settings__form-control--small">
                        <span>"Port"</span>
                        // No native min/max/required — see `validate_connection` (#455).
                        <input type="number" data-role=role("port")
                            aria-required="true"
                            aria-describedby=spec.message_id
                            aria-invalid=aria_invalid
                            prop:value=move || self.port.get()
                            on:input=move |ev| self.port.set(event_target_value(&ev)) />
                    </label>
                </div>
                {extra}
            </>
        }
        .into_any()
    }

    fn render_action_lead(self, spec: EditorSpec) -> AnyView {
        view! {
            <label class="settings__form-checkbox">
                <input type="checkbox" data-role=format!("{}-enabled", spec.role)
                    prop:checked=move || self.enabled.get()
                    on:change=move |ev| self.enabled.set(event_target_checked(&ev)) />
                <span>"Enabled"</span>
            </label>
        }
        .into_any()
    }
}

/// The card's extra text field (Android's launch package), on its own row.
fn render_extra_field(
    field: ExtraField,
    message_id: &'static str,
    value: RwSignal<String>,
    invalid: Signal<bool>,
) -> impl IntoView {
    view! {
        <div class="settings__form-row">
            <label>
                <span>{field.caption}</span>
                <input type="text" data-role=field.role placeholder=field.placeholder required
                    aria-required="true"
                    aria-describedby=message_id
                    aria-invalid=move || invalid.get().to_string()
                    prop:value=move || value.get()
                    on:input=move |ev| value.set(event_target_value(&ev)) />
            </label>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        connection_new_values, validate_connection, validate_draft, ConnectionFields,
        ConnectionSpec, DraftValues, ExtraField, Submission,
    };

    #[test]
    fn valid_fields_are_trimmed() {
        assert_eq!(
            validate_connection("  Main Arena ", " resolume.lan ", " 8090 "),
            Ok(ConnectionFields {
                label: "Main Arena".into(),
                host: "resolume.lan".into(),
                port: 8090,
            })
        );
    }

    #[test]
    fn a_blank_label_or_host_is_rejected_in_field_order() {
        assert_eq!(
            validate_connection("   ", "", "x"),
            Err("Label cannot be empty.")
        );
        assert_eq!(
            validate_connection("Main", "  ", "x"),
            Err("Host cannot be empty.")
        );
    }

    #[test]
    fn an_out_of_range_port_is_rejected_not_truncated() {
        // #455: 99999 must show the range message, never save a wrapped / default port.
        for port in ["99999", "0", "", "80x"] {
            assert_eq!(
                validate_connection("Main", "resolume.lan", port),
                Err("Port must be between 1 and 65535."),
                "port {port:?}"
            );
        }
    }

    const PACKAGE: ExtraField = ExtraField {
        caption: "Launch Package",
        role: "android-component",
        placeholder: "com.tcl.browser",
        default: "com.tcl.browser",
        empty_message: "Launch component cannot be empty.",
    };

    fn values(label: &str, port: &str, extra: &str) -> DraftValues {
        DraftValues {
            label: label.into(),
            host: "sd1l.lan".into(),
            port: port.into(),
            enabled: false,
            extra: extra.into(),
        }
    }

    #[test]
    fn the_extra_field_is_part_of_the_submission_and_required_when_the_card_has_one() {
        // Round 2, item 7: Android's launch package is validated like every field.
        assert_eq!(
            validate_draft(
                &values("Stage", "5555", "  com.example/.Main "),
                Some(&PACKAGE)
            ),
            Ok(Submission {
                fields: ConnectionFields {
                    label: "Stage".into(),
                    host: "sd1l.lan".into(),
                    port: 5555,
                },
                enabled: false,
                extra: "com.example/.Main".into(),
            })
        );
        assert_eq!(
            validate_draft(&values("Stage", "5555", "   "), Some(&PACKAGE)),
            Err("Launch component cannot be empty.")
        );
        // The shared fields come first, in screen order.
        assert_eq!(
            validate_draft(&values("Stage", "99999", ""), Some(&PACKAGE)),
            Err("Port must be between 1 and 65535.")
        );
        // A card without an extra field never rejects (or sends) one.
        assert_eq!(
            validate_draft(&values("Stage", "5555", ""), None).map(|s| s.extra),
            Ok(String::new())
        );
    }

    #[test]
    fn a_new_connection_starts_with_the_cards_port_and_extra_default() {
        let android = ConnectionSpec {
            label_placeholder: "Stage Left",
            host_placeholder: "sd1l.lan",
            default_port: 5555,
            extra: Some(PACKAGE),
        };
        assert_eq!(
            connection_new_values(&android),
            DraftValues {
                label: String::new(),
                host: String::new(),
                port: "5555".into(),
                enabled: true,
                extra: "com.tcl.browser".into(),
            }
        );
        let resolume = ConnectionSpec {
            default_port: 8090,
            extra: None,
            ..android
        };
        assert_eq!(connection_new_values(&resolume).port, "8090");
        assert_eq!(connection_new_values(&resolume).extra, "");
    }
}
```

- [ ] **Step 2: Apply the rest** (patch recipe) — `host_editor.rs` loses the connection form and becomes generic; `list_card.rs`, `resolume.rs`, `android.rs` move to the generic types with their copy in `CardText`; `mod.rs` declares the new module:

```diff
diff --git a/crates/presenter-ui/src/pages/settings/android.rs b/crates/presenter-ui/src/pages/settings/android.rs
index 2f17756..bea89b4 100644
--- a/crates/presenter-ui/src/pages/settings/android.rs
+++ b/crates/presenter-ui/src/pages/settings/android.rs
@@ -10,9 +10,10 @@
 
 use leptos::prelude::*;
 
-use super::host_editor::{EditorSpec, ExtraField, Submission};
+use super::connection_form::{ConnectionForm, ConnectionSpec, DraftValues, ExtraField, Submission};
+use super::host_editor::{EditorSpec, SAVING_TITLE, UNSAVED_TITLE};
 use super::list_card::{CardItem, CardText, ListCard, RowParts};
-use super::row_status::{android_attempts, android_state};
+use super::row_status::{android_attempts, android_state, updated_created};
 use super::ToastHandle;
 use crate::api::settings::{self, AndroidDisplayDraft, AndroidDisplayDto, AndroidStatusDto};
 use crate::api::ApiError;
@@ -20,16 +21,24 @@ use crate::api::ApiError;
 /// The launch package a new display starts with.
 const DEFAULT_COMPONENT: &str = "com.tcl.browser";
 
-static TEXT: CardText = CardText {
+static TEXT: CardText<ConnectionForm> = CardText {
     editor: EditorSpec {
         role: "android",
         message_id: "android-form-status",
         message_role: "android-form-status",
-        label_placeholder: "Stage Left",
-        host_placeholder: "sd1l.lan",
         new_title: "New Android stage display",
         new_submit: "Add display",
         edit_title: "Edit Android stage display",
+        save_text: "Save",
+        cancel_text: "Cancel",
+        unsaved_title: UNSAVED_TITLE,
+        saving_title: SAVING_TITLE,
+    },
+    form: ConnectionSpec {
+        label_placeholder: "Stage Left",
+        host_placeholder: "sd1l.lan",
+        // The adb port.
+        default_port: 5555,
         extra: Some(ExtraField {
             caption: "Launch Package",
             role: "android-component",
@@ -38,12 +47,13 @@ static TEXT: CardText = CardText {
             empty_message: "Launch component cannot be empty.",
         }),
     },
-    // The adb port.
-    default_port: 5555,
     list_role: "android-display-list",
     add_text: "+ Add display",
     empty_text: "No Android stage displays configured yet.",
     fallback_name: "this display",
+    edit_text: "Edit",
+    delete_text: "Delete",
+    delete_question: ("Remove ", "? Presenter will stop reconnecting."),
     saving: "Updating display…",
     creating: "Creating display…",
     updated: "Saved Android stage display.",
@@ -68,6 +78,7 @@ fn row_key(d: &AndroidDisplayDto) -> (String, String, String, u16, String, bool)
 }
 
 impl CardItem for AndroidDisplayDto {
+    type Form = ConnectionForm;
     type Status = AndroidStatusDto;
     type Key = (String, String, String, u16, String, bool);
 
@@ -83,32 +94,30 @@ impl CardItem for AndroidDisplayDto {
         &self.label
     }
 
-    fn host(&self) -> &str {
-        &self.host
+    fn address(&self) -> (String, String) {
+        (self.host.clone(), format!(":{}", self.port))
     }
 
-    fn port(&self) -> u16 {
-        self.port
+    fn values(&self) -> DraftValues {
+        DraftValues {
+            label: self.label.clone(),
+            host: self.host.clone(),
+            port: self.port.to_string(),
+            enabled: self.is_enabled,
+            extra: self.launch_component.clone(),
+        }
     }
 
     fn is_enabled(&self) -> bool {
         self.is_enabled
     }
 
-    fn extra(&self) -> &str {
-        &self.launch_component
-    }
-
     fn status(&self) -> Option<AndroidStatusDto> {
         self.status.clone()
     }
 
-    fn created_at(&self) -> &str {
-        &self.created_at
-    }
-
-    fn updated_at(&self) -> &str {
-        &self.updated_at
+    fn timestamps(&self) -> Option<String> {
+        Some(updated_created(&self.updated_at, &self.created_at))
     }
 
     async fn list() -> Result<Vec<Self>, ApiError> {
diff --git a/crates/presenter-ui/src/pages/settings/host_editor.rs b/crates/presenter-ui/src/pages/settings/host_editor.rs
index 0285717..77107b1 100644
--- a/crates/presenter-ui/src/pages/settings/host_editor.rs
+++ b/crates/presenter-ui/src/pages/settings/host_editor.rs
@@ -1,4 +1,6 @@
-//! #819: the inline add / edit editor shared by the Resolume and Android list cards.
+//! #819: the inline add / edit editor shared by the settings list cards — the
+//! Resolume connections, the Android stage displays and (#852 C1b) the listening
+//! microphones.
 //!
 //! Before #819 each card had ONE permanent add/edit form at its top, and a row's
 //! **Edit** only loaded that form — measured 1170 px above the row the operator had
@@ -7,27 +9,30 @@
 //! list. One editor per card is open at a time ([`EditTarget`]).
 //!
 //! [`ListEditor`] is the card's editor state machine; the card plumbing around it
-//! (fetch / save / delete / rows) is `list_card`. The draft lives at CARD level,
-//! outside the keyed `<For>` rows, so the 5 s status poll can never reset what the
+//! (fetch / save / delete / rows) is `list_card`. What the editor edits is the
+//! card's [`EditorForm`] (#852 C1b): `connection_form::ConnectionForm` — label,
+//! host, port, an optional extra field, "Enabled" — for Resolume and Android,
+//! `listen::MicForm` for the microphones. The draft lives at CARD level, outside
+//! the keyed `<For>` rows, so the 5 s status poll can never reset what the
 //! operator is typing. Its guards ([`trigger_lock`], round 2):
 //! - one save at a time; while it is in flight no editor opens or switches and the
 //!   operator cannot close it (every Edit and "+ Add" is disabled, Escape and Cancel
 //!   are ignored) — only a row deleted elsewhere still closes its editor;
 //! - unsaved changes lock the other triggers until they are saved or cancelled;
 //! - a save that finishes late never closes a newer editor ([`save_is_current`]);
-//! - only a real open focuses the Label field, never a remount ([`takes_focus`]);
+//! - only a real open focuses the first field, never a remount ([`takes_focus`]);
 //! - focus returns to the button that opened the editor, or to "+ Add" when the
 //!   row was deleted elsewhere — once that button is unlocked ([`focus_on_close`]).
 
 use leptos::prelude::*;
 
-use super::parse_port_in_range;
 use crate::utils::keyboard::KEY_ESCAPE;
 
-/// The tooltip of an Edit / "+ Add" button locked by unsaved changes.
-const UNSAVED_TITLE: &str = "Save or cancel the open editor first";
-/// The tooltip of an Edit / "+ Add" button locked by a save in flight.
-const SAVING_TITLE: &str = "Wait until the save has finished";
+/// The connection cards' tooltip of an Edit / "+ Add" button locked by unsaved
+/// changes (every card's [`EditorSpec`] names its own).
+pub(super) const UNSAVED_TITLE: &str = "Save or cancel the open editor first";
+/// The connection cards' tooltip of a button locked by a save in flight.
+pub(super) const SAVING_TITLE: &str = "Wait until the save has finished";
 
 /// Which item a card's single inline editor is open on.
 #[derive(Clone, Debug, Default, PartialEq, Eq)]
@@ -67,104 +72,46 @@ impl EditTarget {
     }
 }
 
-/// The validated label / host / port of an editor submission.
-#[derive(Clone, Debug, PartialEq, Eq)]
-pub(super) struct ConnectionFields {
-    pub(super) label: String,
-    pub(super) host: String,
-    pub(super) port: u16,
-}
-
-/// Validate the fields both editors share, in the order the operator sees them.
-///
-/// The port input has no native `min`/`max`, so an out-of-range value (99999)
-/// reaches this check and gets the styled message instead of the browser silently
-/// blocking the submit — this is the single authority for the 1..=65535 range (#455).
-pub(super) fn validate_connection(
-    label: &str,
-    host: &str,
-    port: &str,
-) -> Result<ConnectionFields, &'static str> {
-    let label = label.trim();
-    if label.is_empty() {
-        return Err("Label cannot be empty.");
-    }
-    let host = host.trim();
-    if host.is_empty() {
-        return Err("Host cannot be empty.");
-    }
-    let port = parse_port_in_range(port).ok_or("Port must be between 1 and 65535.")?;
-    Ok(ConnectionFields {
-        label: label.to_string(),
-        host: host.to_string(),
-        port,
-    })
-}
-
-/// A card's own extra text field in the editor (Android's launch package). Its value
-/// is part of the draft like every other field.
-#[derive(Clone, Copy, Debug)]
-pub(super) struct ExtraField {
-    pub(super) caption: &'static str,
-    pub(super) role: &'static str,
-    pub(super) placeholder: &'static str,
-    /// The value a new item starts with.
-    pub(super) default: &'static str,
-    /// The message when it is left empty.
-    pub(super) empty_message: &'static str,
-}
-
-/// What an editor holds: the values it was opened with, and what is on screen. The
-/// two differ exactly when the editor has unsaved changes.
-#[derive(Clone, Debug, Default, PartialEq, Eq)]
-pub(super) struct DraftValues {
-    pub(super) label: String,
-    pub(super) host: String,
-    /// The port as typed (the input's text), validated only on save.
-    pub(super) port: String,
-    pub(super) enabled: bool,
-    /// The card's extra field (Android's launch package); empty when it has none.
-    pub(super) extra: String,
-}
-
-/// A validated editor submission.
-#[derive(Clone, Debug, PartialEq, Eq)]
-pub(super) struct Submission {
-    pub(super) fields: ConnectionFields,
-    pub(super) enabled: bool,
-    /// The trimmed extra field; empty when the card has none.
-    pub(super) extra: String,
-}
-
-/// Validate an editor's values: the shared fields first, then the card's extra field
-/// (required when the card has one), in the order the operator sees them.
-pub(super) fn validate_draft(
-    values: &DraftValues,
-    extra: Option<&ExtraField>,
-) -> Result<Submission, &'static str> {
-    let fields = validate_connection(&values.label, &values.host, &values.port)?;
-    let extra_value = values.extra.trim();
-    if let Some(field) = extra {
-        if extra_value.is_empty() {
-            return Err(field.empty_message);
-        }
-    }
-    Ok(Submission {
-        fields,
-        enabled: values.enabled,
-        extra: extra_value.to_string(),
-    })
+/// What one card's inline editor edits (#852 C1b). The target, the locks, the save
+/// ticket, the focus and the message line are the same for every card
+/// ([`ListEditor`]); the fields, their values and their checks are the form's.
+pub(super) trait EditorForm: Copy + Send + Sync + 'static {
+    /// What the fields hold: the editor has unsaved changes exactly while they
+    /// differ from the values it was opened with.
+    type Values: Clone + Default + PartialEq + Send + Sync + 'static;
+    /// A validated submission.
+    type Submission: 'static;
+    /// What differs between two cards using the same form (placeholders, the
+    /// extra field, the new item's port).
+    type Config: Copy + Send + Sync + 'static;
+
+    /// The form's signals (created once per card).
+    fn new(config: Self::Config) -> Self;
+    /// The values a new item ("+ Add …") starts with.
+    fn new_values(self) -> Self::Values;
+    fn load(self, values: &Self::Values);
+    /// Tracked: the values on screen.
+    fn values(self) -> Self::Values;
+    fn values_untracked(self) -> Self::Values;
+    /// Check the values in the order the operator sees the fields.
+    fn validate(self, values: &Self::Values) -> Result<Self::Submission, &'static str>;
+    /// The field rows. `first` is the input that takes the caret once per open;
+    /// every field's `aria-describedby` is `spec.message_id`, and it is
+    /// `aria-invalid` while `invalid` holds (the message line shows an error).
+    fn render_fields(
+        self,
+        spec: EditorSpec,
+        first: NodeRef<leptos::html::Input>,
+        invalid: Signal<bool>,
+    ) -> AnyView;
+    /// Controls in front of Save / Cancel on the actions row (the connection
+    /// form's "Enabled"); an empty view for none.
+    fn render_action_lead(self, spec: EditorSpec) -> AnyView;
 }
 
-/// The open editor's values plus its message line. `Copy` (signals only), so every
-/// handler and row closure can hold it.
+/// The editor's message line and its save-in-flight flag. `Copy` (signals only).
 #[derive(Clone, Copy)]
-struct ConnectionDraft {
-    label: RwSignal<String>,
-    host: RwSignal<String>,
-    port: RwSignal<String>,
-    enabled: RwSignal<bool>,
-    extra: RwSignal<String>,
+struct EditorStatus {
     /// The line under the editor: "Saving changes…" or a validation / save error.
     message: RwSignal<String>,
     /// `idle` / `info` / `error` — the line's `data-state`; `error` also sets
@@ -174,14 +121,9 @@ struct ConnectionDraft {
     busy: RwSignal<bool>,
 }
 
-impl Default for ConnectionDraft {
+impl Default for EditorStatus {
     fn default() -> Self {
         Self {
-            label: RwSignal::new(String::new()),
-            host: RwSignal::new(String::new()),
-            port: RwSignal::new(String::new()),
-            enabled: RwSignal::new(true),
-            extra: RwSignal::new(String::new()),
             message: RwSignal::new(String::new()),
             state: RwSignal::new(String::from("idle")),
             busy: RwSignal::new(false),
@@ -189,44 +131,13 @@ impl Default for ConnectionDraft {
     }
 }
 
-impl ConnectionDraft {
-    /// Fill the editor.
-    fn load(self, values: &DraftValues) {
-        self.label.set(values.label.clone());
-        self.host.set(values.host.clone());
-        self.port.set(values.port.clone());
-        self.enabled.set(values.enabled);
-        self.extra.set(values.extra.clone());
-        self.clear_message();
-    }
-
-    /// Tracked: the values on screen.
-    fn values(self) -> DraftValues {
-        DraftValues {
-            label: self.label.get(),
-            host: self.host.get(),
-            port: self.port.get(),
-            enabled: self.enabled.get(),
-            extra: self.extra.get(),
-        }
-    }
-
-    fn values_untracked(self) -> DraftValues {
-        DraftValues {
-            label: self.label.get_untracked(),
-            host: self.host.get_untracked(),
-            port: self.port.get_untracked(),
-            enabled: self.enabled.get_untracked(),
-            extra: self.extra.get_untracked(),
-        }
-    }
-
+impl EditorStatus {
     fn show(self, state: &str, message: &str) {
         self.state.set(state.to_string());
         self.message.set(message.to_string());
     }
 
-    fn clear_message(self) {
+    fn clear(self) {
         self.show("idle", "");
     }
 }
@@ -255,11 +166,11 @@ impl TriggerLock {
         }
     }
 
-    /// The button's tooltip.
-    pub(super) fn title(self) -> Option<&'static str> {
+    /// The button's tooltip, in the card's words.
+    pub(super) fn title(self, spec: &EditorSpec) -> Option<&'static str> {
         match self {
-            Self::Saving => Some(SAVING_TITLE),
-            Self::Unsaved => Some(UNSAVED_TITLE),
+            Self::Saving => Some(spec.saving_title),
+            Self::Unsaved => Some(spec.unsaved_title),
             Self::Open => None,
         }
     }
@@ -286,27 +197,25 @@ fn trigger_lock(
 }
 
 /// The open editor shows values other than the ones it was opened with.
-fn is_dirty(open: &EditTarget, current: &DraftValues, loaded: &DraftValues) -> bool {
+fn is_dirty<V: PartialEq>(open: &EditTarget, current: &V, loaded: &V) -> bool {
     *open != EditTarget::Closed && current != loaded
 }
 
 /// An editor mount takes focus only for an open that has not been focused yet. A
 /// remount of the same editor (its row re-keyed by an edit elsewhere) must not pull
-/// the caret back to Label while the operator types in another field.
+/// the caret back to the first field while the operator types in another one.
 fn takes_focus(focused_generation: u64, generation: u64) -> bool {
     focused_generation != generation
 }
 
-/// One card's inline-editor state machine. `Copy` (signals only).
-#[derive(Clone, Copy)]
-pub(super) struct ListEditor {
+/// One card's inline-editor state machine over its form `F`. `Copy` (signals only).
+pub(super) struct ListEditor<F: EditorForm> {
     editing: RwSignal<EditTarget>,
-    draft: ConnectionDraft,
+    form: F,
+    status: EditorStatus,
     /// What the open editor was filled with; [`is_dirty`] compares against it.
-    loaded: RwSignal<DraftValues>,
+    loaded: RwSignal<F::Values>,
     dirty: Memo<bool>,
-    /// The card's extra field, validated by [`Self::begin_save`].
-    extra: Option<ExtraField>,
     /// Bumped on every open: a save that finishes after the operator re-opened an
     /// editor (even on the same row) must not close or overwrite that newer editor.
     generation: RwSignal<u64>,
@@ -317,6 +226,14 @@ pub(super) struct ListEditor {
     focus_return: RwSignal<Option<EditTarget>>,
 }
 
+impl<F: EditorForm> Clone for ListEditor<F> {
+    fn clone(&self) -> Self {
+        *self
+    }
+}
+
+impl<F: EditorForm> Copy for ListEditor<F> {}
+
 /// What a save in flight was started from.
 pub(super) struct SaveTicket {
     target: EditTarget,
@@ -330,55 +247,46 @@ impl SaveTicket {
     }
 }
 
-impl ListEditor {
-    /// A card's editor; `extra` is the card's own extra field, if it has one.
-    pub(super) fn new(extra: Option<ExtraField>) -> Self {
+impl<F: EditorForm> ListEditor<F> {
+    /// A card's editor over a form built from the card's `config`.
+    pub(super) fn new(config: F::Config) -> Self {
         let editing = RwSignal::new(EditTarget::Closed);
-        let draft = ConnectionDraft::default();
-        let loaded = RwSignal::new(DraftValues::default());
+        let form = F::new(config);
+        let loaded = RwSignal::new(F::Values::default());
         let dirty = Memo::new(move |_| {
-            let current = draft.values();
+            let current = form.values();
             editing.with(|open| loaded.with(|opened_with| is_dirty(open, &current, opened_with)))
         });
         Self {
             editing,
-            draft,
+            form,
+            status: EditorStatus::default(),
             loaded,
             dirty,
-            extra,
             generation: RwSignal::new(0),
             focused: StoredValue::new(0),
             focus_return: RwSignal::new(None),
         }
     }
 
-    /// Open the "+ Add …" editor: blank label and host, `default_port`, enabled, the
-    /// extra field's default.
-    pub(super) fn open_new(self, default_port: u16) {
-        let values = DraftValues {
-            port: default_port.to_string(),
-            enabled: true,
-            extra: self
-                .extra
-                .map(|e| e.default)
-                .unwrap_or_default()
-                .to_string(),
-            ..DraftValues::default()
-        };
+    /// Open the "+ Add …" editor with the form's new-item values.
+    pub(super) fn open_new(self) {
+        let values = self.form.new_values();
         self.open(EditTarget::New, values);
     }
 
     /// Open the editor on an existing row, filled with its `values`.
-    pub(super) fn open_item(self, id: String, values: DraftValues) {
+    pub(super) fn open_item(self, id: String, values: F::Values) {
         self.open(EditTarget::Item(id), values);
     }
 
     /// The same guard the disabled buttons show: a locked trigger never opens.
-    fn open(self, target: EditTarget, values: DraftValues) {
+    fn open(self, target: EditTarget, values: F::Values) {
         if self.trigger_lock_untracked(&target).is_some() {
             return;
         }
-        self.draft.load(&values);
+        self.form.load(&values);
+        self.status.clear();
         self.loaded.set(values);
         self.generation.update(|g| *g += 1);
         self.editing.set(target);
@@ -387,7 +295,7 @@ impl ListEditor {
     /// Cancel / Escape: close without saving. Ignored while a save is in flight —
     /// the save settles the editor itself.
     pub(super) fn cancel(self) {
-        if !self.draft.busy.get_untracked() {
+        if !self.status.busy.get_untracked() {
             self.close();
         }
     }
@@ -404,7 +312,7 @@ impl ListEditor {
     /// Close without moving focus.
     fn discard(self) {
         self.editing.set(EditTarget::Closed);
-        self.draft.clear_message();
+        self.status.clear();
     }
 
     /// The row being edited was deleted from this card.
@@ -438,14 +346,14 @@ impl ListEditor {
 
     /// Tracked: why `trigger` (a row's Edit, or "+ Add" for `New`) is locked now.
     pub(super) fn trigger_lock(self, trigger: &EditTarget) -> Option<TriggerLock> {
-        let busy = self.draft.busy.get();
+        let busy = self.status.busy.get();
         let dirty = self.dirty.get();
         self.editing
             .with(|open| trigger_lock(open, busy, dirty, trigger))
     }
 
     fn trigger_lock_untracked(self, trigger: &EditTarget) -> Option<TriggerLock> {
-        let busy = self.draft.busy.get_untracked();
+        let busy = self.status.busy.get_untracked();
         let dirty = self.dirty.get_untracked();
         self.editing
             .with_untracked(|open| trigger_lock(open, busy, dirty, trigger))
@@ -454,18 +362,18 @@ impl ListEditor {
     /// Validate and start a save. `None` when no editor is open, a save is already in
     /// flight (held Enter, a double click), or a field is invalid (its message is
     /// shown).
-    pub(super) fn begin_save(self) -> Option<(SaveTicket, Submission)> {
+    pub(super) fn begin_save(self) -> Option<(SaveTicket, F::Submission)> {
         let target = self.editing.get_untracked();
-        if target == EditTarget::Closed || self.draft.busy.get_untracked() {
+        if target == EditTarget::Closed || self.status.busy.get_untracked() {
             return None;
         }
-        match validate_draft(&self.draft.values_untracked(), self.extra.as_ref()) {
+        match self.form.validate(&self.form.values_untracked()) {
             Ok(submission) => {
                 let generation = self.generation.get_untracked();
                 Some((SaveTicket { target, generation }, submission))
             }
             Err(message) => {
-                self.draft.show("error", message);
+                self.status.show("error", message);
                 None
             }
         }
@@ -474,8 +382,8 @@ impl ListEditor {
     /// The request is going out: Save, Cancel and every trigger stay locked until
     /// [`Self::finish_save`].
     pub(super) fn mark_saving(self, message: &str) {
-        self.draft.busy.set(true);
-        self.draft.show("info", message);
+        self.status.busy.set(true);
+        self.status.show("info", message);
     }
 
     /// A save finished (after a success, call it AFTER the list reload; a failure
@@ -492,10 +400,10 @@ impl ListEditor {
         if current {
             match error {
                 None => self.close(),
-                Some(message) => self.draft.show("error", message),
+                Some(message) => self.status.show("error", message),
             }
         }
-        self.draft.busy.set(false);
+        self.status.busy.set(false);
         current
     }
 
@@ -536,8 +444,8 @@ fn save_is_current(ticket: &SaveTicket, generation: u64, editing: &EditTarget) -
 /// the save settles. The focus itself runs one task later, after the render effects
 /// already queued by the same change (the button's own `disabled`, the editor's
 /// removal) — a newly woken Effect is not ordered against them otherwise.
-pub(super) fn focus_on_close(
-    editor: ListEditor,
+pub(super) fn focus_on_close<F: EditorForm>(
+    editor: ListEditor<F>,
     button: NodeRef<leptos::html::Button>,
     target: EditTarget,
 ) {
@@ -565,7 +473,7 @@ fn focus_is_free() -> bool {
         .is_none_or(|el| el.tag_name() == "BODY")
 }
 
-/// What differs between the Resolume and the Android editor.
+/// What differs between two cards' editors besides their fields.
 #[derive(Clone, Copy)]
 pub(super) struct EditorSpec {
     /// The `data-role` prefix: `host` → `host-label`, `host-submit`, `host-cancel`, …
@@ -574,43 +482,47 @@ pub(super) struct EditorSpec {
     pub(super) message_id: &'static str,
     /// `data-role` of that message line.
     pub(super) message_role: &'static str,
-    pub(super) label_placeholder: &'static str,
-    pub(super) host_placeholder: &'static str,
     /// The editor heading and the Save button text for a NEW item.
     pub(super) new_title: &'static str,
     pub(super) new_submit: &'static str,
     /// The editor heading when editing an existing item in place.
     pub(super) edit_title: &'static str,
-    pub(super) extra: Option<ExtraField>,
+    /// The Save button of an existing item, and Cancel.
+    pub(super) save_text: &'static str,
+    pub(super) cancel_text: &'static str,
+    /// The tooltips of a locked Edit / "+ Add" ([`TriggerLock::title`]).
+    pub(super) unsaved_title: &'static str,
+    pub(super) saving_title: &'static str,
 }
 
 /// The inline editor. `creating` = the "+ Add …" item (Save creates); otherwise an
 /// existing row (Save updates). `on_save` validates and saves; Cancel and Escape
 /// close the editor without saving. Only one editor exists in a card at a time, so
 /// the field `data-role`s and the `id`s stay unique on the page.
-pub(super) fn render_connection_editor<S>(
+pub(super) fn render_editor<F, S>(
     spec: EditorSpec,
-    editor: ListEditor,
+    editor: ListEditor<F>,
     creating: bool,
     on_save: S,
 ) -> AnyView
 where
+    F: EditorForm,
     S: Fn() + Copy + Send + 'static,
 {
-    let draft = editor.draft;
+    let status = editor.status;
     let role = move |field: &str| format!("{}-{field}", spec.role);
-    let invalid = move || (draft.state.get() == "error").to_string();
+    let invalid = Signal::derive(move || status.state.get() == "error");
     let (mode, title, submit_text) = if creating {
         ("create", spec.new_title, spec.new_submit)
     } else {
-        ("edit", spec.edit_title, "Save")
+        ("edit", spec.edit_title, spec.save_text)
     };
 
     // The operator clicked Edit / "+ Add" to type: put the caret in the first field —
     // once per open, never again when this editor is remounted.
-    let label_ref = NodeRef::<leptos::html::Input>::new();
+    let first = NodeRef::<leptos::html::Input>::new();
     Effect::new(move || {
-        if let Some(el) = label_ref.get() {
+        if let Some(el) = first.get() {
             if editor.take_first_focus() {
                 let _ = el.focus();
             }
@@ -630,10 +542,8 @@ where
             editor.cancel();
         }
     };
-
-    let extra = spec
-        .extra
-        .map(|field| render_extra_field(field, spec.message_id, draft));
+    let fields = editor.form.render_fields(spec, first, invalid);
+    let lead = editor.form.render_action_lead(spec);
 
     view! {
         <form class="settings__form settings__form--inline-editor" data-role=role("editor")
@@ -641,91 +551,34 @@ where
             autocomplete="off" on:submit=on_submit on:keydown=on_keydown>
             <h3 class="settings__editor-title" id=role("editor-title")
                 data-role=role("editor-title")>{title}</h3>
-            <div class="settings__form-row settings__form-row--connection">
-                <label>
-                    <span>"Label"</span>
-                    <input type="text" node_ref=label_ref data-role=role("label")
-                        placeholder=spec.label_placeholder required
-                        aria-required="true"
-                        aria-describedby=spec.message_id
-                        aria-invalid=invalid
-                        prop:value=move || draft.label.get()
-                        on:input=move |ev| draft.label.set(event_target_value(&ev)) />
-                </label>
-                <label>
-                    <span>"Hostname or DNS"</span>
-                    <input type="text" data-role=role("host") placeholder=spec.host_placeholder required
-                        aria-required="true"
-                        aria-describedby=spec.message_id
-                        aria-invalid=invalid
-                        prop:value=move || draft.host.get()
-                        on:input=move |ev| draft.host.set(event_target_value(&ev)) />
-                </label>
-                <label class="settings__form-control--small">
-                    <span>"Port"</span>
-                    // No native min/max/required — see `validate_connection` (#455).
-                    <input type="number" data-role=role("port")
-                        aria-required="true"
-                        aria-describedby=spec.message_id
-                        aria-invalid=invalid
-                        prop:value=move || draft.port.get()
-                        on:input=move |ev| draft.port.set(event_target_value(&ev)) />
-                </label>
-            </div>
-            {extra}
+            {fields}
             <div class="settings__form-row settings__form-row--inline">
-                <label class="settings__form-checkbox">
-                    <input type="checkbox" data-role=role("enabled")
-                        prop:checked=move || draft.enabled.get()
-                        on:change=move |ev| draft.enabled.set(event_target_checked(&ev)) />
-                    <span>"Enabled"</span>
-                </label>
+                {lead}
                 <div class="settings__form-actions">
                     <button type="submit" class="settings__button settings__button--primary"
-                        data-role=role("submit") prop:disabled=move || draft.busy.get()>
+                        data-role=role("submit") prop:disabled=move || status.busy.get()>
                         {submit_text}
                     </button>
                     <button type="button" class="settings__button settings__button--ghost"
-                        data-role=role("cancel") prop:disabled=move || draft.busy.get()
-                        on:click=move |_| editor.cancel()>"Cancel"</button>
+                        data-role=role("cancel") prop:disabled=move || status.busy.get()
+                        on:click=move |_| editor.cancel()>{spec.cancel_text}</button>
                 </div>
             </div>
             <p id=spec.message_id class="settings__form-status" data-role=spec.message_role
-                data-state=move || draft.state.get()>
-                {move || draft.message.get()}
+                data-state=move || status.state.get()>
+                {move || status.message.get()}
             </p>
         </form>
     }
     .into_any()
 }
 
-/// The card's extra text field (Android's launch package), on its own row.
-fn render_extra_field(
-    field: ExtraField,
-    message_id: &'static str,
-    draft: ConnectionDraft,
-) -> impl IntoView {
-    view! {
-        <div class="settings__form-row">
-            <label>
-                <span>{field.caption}</span>
-                <input type="text" data-role=field.role placeholder=field.placeholder required
-                    aria-required="true"
-                    aria-describedby=message_id
-                    aria-invalid=move || (draft.state.get() == "error").to_string()
-                    prop:value=move || draft.extra.get()
-                    on:input=move |ev| draft.extra.set(event_target_value(&ev)) />
-            </label>
-        </div>
-    }
-}
-
 #[cfg(test)]
 mod tests {
+    use super::super::connection_form::DraftValues;
     use super::{
-        is_dirty, save_is_current, takes_focus, trigger_lock, validate_connection, validate_draft,
-        ConnectionFields, DraftValues, EditTarget, ExtraField, SaveTicket, Submission, TriggerLock,
-        UNSAVED_TITLE,
+        is_dirty, save_is_current, takes_focus, trigger_lock, EditTarget, EditorSpec, SaveTicket,
+        TriggerLock, SAVING_TITLE, UNSAVED_TITLE,
     };
 
     #[test]
@@ -795,94 +648,6 @@ mod tests {
         assert!(!EditTarget::Closed.is_item_missing(std::iter::empty()));
     }
 
-    #[test]
-    fn valid_fields_are_trimmed() {
-        assert_eq!(
-            validate_connection("  Main Arena ", " resolume.lan ", " 8090 "),
-            Ok(ConnectionFields {
-                label: "Main Arena".into(),
-                host: "resolume.lan".into(),
-                port: 8090,
-            })
-        );
-    }
-
-    #[test]
-    fn a_blank_label_or_host_is_rejected_in_field_order() {
-        assert_eq!(
-            validate_connection("   ", "", "x"),
-            Err("Label cannot be empty.")
-        );
-        assert_eq!(
-            validate_connection("Main", "  ", "x"),
-            Err("Host cannot be empty.")
-        );
-    }
-
-    #[test]
-    fn an_out_of_range_port_is_rejected_not_truncated() {
-        // #455: 99999 must show the range message, never save a wrapped / default port.
-        for port in ["99999", "0", "", "80x"] {
-            assert_eq!(
-                validate_connection("Main", "resolume.lan", port),
-                Err("Port must be between 1 and 65535."),
-                "port {port:?}"
-            );
-        }
-    }
-
-    const PACKAGE: ExtraField = ExtraField {
-        caption: "Launch Package",
-        role: "android-component",
-        placeholder: "com.tcl.browser",
-        default: "com.tcl.browser",
-        empty_message: "Launch component cannot be empty.",
-    };
-
-    fn values(label: &str, port: &str, extra: &str) -> DraftValues {
-        DraftValues {
-            label: label.into(),
-            host: "sd1l.lan".into(),
-            port: port.into(),
-            enabled: false,
-            extra: extra.into(),
-        }
-    }
-
-    #[test]
-    fn the_extra_field_is_part_of_the_submission_and_required_when_the_card_has_one() {
-        // Round 2, item 7: Android's launch package is validated like every field.
-        assert_eq!(
-            validate_draft(
-                &values("Stage", "5555", "  com.example/.Main "),
-                Some(&PACKAGE)
-            ),
-            Ok(Submission {
-                fields: ConnectionFields {
-                    label: "Stage".into(),
-                    host: "sd1l.lan".into(),
-                    port: 5555,
-                },
-                enabled: false,
-                extra: "com.example/.Main".into(),
-            })
-        );
-        assert_eq!(
-            validate_draft(&values("Stage", "5555", "   "), Some(&PACKAGE)),
-            Err("Launch component cannot be empty.")
-        );
-        // The shared fields come first, in screen order.
-        assert_eq!(
-            validate_draft(&values("Stage", "99999", ""), Some(&PACKAGE)),
-            Err("Port must be between 1 and 65535.")
-        );
-        // A card without an extra field never rejects (or sends) one.
-        assert_eq!(
-            validate_draft(&values("Stage", "5555", ""), None).map(|s| s.extra),
-            Ok(String::new())
-        );
-    }
-
     #[test]
     fn a_save_in_flight_locks_every_trigger() {
         // Round 2, item 4: no editor opens or switches until the save settles.
@@ -935,17 +700,40 @@ mod tests {
         }
     }
 
+    const SPEC: EditorSpec = EditorSpec {
+        role: "host",
+        message_id: "resolume-form-status",
+        message_role: "form-status",
+        new_title: "New Resolume connection",
+        new_submit: "Add connection",
+        edit_title: "Edit Resolume connection",
+        save_text: "Save",
+        cancel_text: "Cancel",
+        unsaved_title: UNSAVED_TITLE,
+        saving_title: SAVING_TITLE,
+    };
+
     #[test]
     fn a_lock_says_why_on_the_button() {
-        assert_eq!(TriggerLock::Unsaved.title(), Some(UNSAVED_TITLE));
+        assert_eq!(TriggerLock::Unsaved.title(&SPEC), Some(UNSAVED_TITLE));
         assert_eq!(UNSAVED_TITLE, "Save or cancel the open editor first");
-        assert!(TriggerLock::Saving.title().is_some());
-        assert_eq!(TriggerLock::Open.title(), None);
+        assert_eq!(TriggerLock::Saving.title(&SPEC), Some(SAVING_TITLE));
+        assert_eq!(TriggerLock::Open.title(&SPEC), None);
         assert_eq!(TriggerLock::Saving.data_lock(), "saving");
         assert_eq!(TriggerLock::Unsaved.data_lock(), "unsaved");
         assert_eq!(TriggerLock::Open.data_lock(), "open");
     }
 
+    fn values(label: &str, port: &str, extra: &str) -> DraftValues {
+        DraftValues {
+            label: label.into(),
+            host: "sd1l.lan".into(),
+            port: port.into(),
+            enabled: false,
+            extra: extra.into(),
+        }
+    }
+
     #[test]
     fn the_draft_is_dirty_only_when_it_differs_from_what_was_loaded() {
         let loaded = values("Stage", "5555", "com.tcl.browser");
diff --git a/crates/presenter-ui/src/pages/settings/list_card.rs b/crates/presenter-ui/src/pages/settings/list_card.rs
index 76d24e8..69c5896 100644
--- a/crates/presenter-ui/src/pages/settings/list_card.rs
+++ b/crates/presenter-ui/src/pages/settings/list_card.rs
@@ -1,7 +1,8 @@
-//! #819: the list card shared by the Resolume connections and the Android stage
-//! displays.
+//! #819: the list card shared by the settings list cards — the Resolume
+//! connections, the Android stage displays and (#852 C1b) the listening
+//! microphones.
 //!
-//! [`ListCard`] owns everything the two cards used to copy between them, so its
+//! [`ListCard`] owns everything the cards used to copy between them, so its
 //! guards live in one place:
 //! - the list fetch, numbered by `list_sync::ResponseOrder` (a poll sent before a
 //!   save and answered after it is dropped) and followed by `forget_missing` (an
@@ -12,19 +13,18 @@
 //! - every keyed row: its live status / timestamp `Memo`s, the inline editor or the
 //!   summary, the Edit / Delete buttons with their trigger locks and focus return.
 //!
-//! A card supplies its item type and API ([`CardItem`]), its copy ([`CardText`]) and
-//! the card-specific parts of a row ([`RowParts`]).
+//! A card supplies its item type and API ([`CardItem`] — its `Form` is what the
+//! inline editor edits, `host_editor::EditorForm`), its copy ([`CardText`]) and the
+//! card-specific parts of a row ([`RowParts`]).
 
 use std::hash::Hash;
 
 use leptos::prelude::*;
 
 use super::host_editor::{
-    focus_on_close, render_connection_editor, DraftValues, EditTarget, EditorSpec, ListEditor,
-    Submission, TriggerLock,
+    focus_on_close, render_editor, EditTarget, EditorForm, EditorSpec, ListEditor, TriggerLock,
 };
 use super::list_sync::ResponseOrder;
-use super::row_status::updated_created;
 use super::{ToastHandle, STATUS_REFRESH_MS};
 use crate::api::ApiError;
 use crate::components::modal::confirm;
@@ -34,6 +34,8 @@ use crate::components::modal::confirm;
 // need to be `Send`.
 #[allow(async_fn_in_trait)]
 pub(super) trait CardItem: Clone + Send + Sync + 'static {
+    /// What the inline editor edits.
+    type Form: EditorForm;
     /// The live status the row reads through a `Memo`.
     type Status: Clone + PartialEq + Send + Sync + 'static;
     /// The row's `<For>` key: the id plus every field the row shows or edits. Never
@@ -42,36 +44,48 @@ pub(super) trait CardItem: Clone + Send + Sync + 'static {
 
     fn id(&self) -> &str;
     fn key(&self) -> Self::Key;
+    /// The row's title; it also names the item in the delete confirmation.
     fn label(&self) -> &str;
-    fn host(&self) -> &str;
-    fn port(&self) -> u16;
-    fn is_enabled(&self) -> bool;
-    /// The card's extra editor field (Android's launch package); empty without one.
-    fn extra(&self) -> &str {
-        ""
+    /// The address line: the `<code>` part and the text right after it
+    /// (`host` + `:port` for a connection).
+    fn address(&self) -> (String, String);
+    /// The values the editor opens with for this item.
+    fn values(&self) -> <Self::Form as EditorForm>::Values;
+    /// The row's `data-enabled`; an item without such a switch is always enabled.
+    fn is_enabled(&self) -> bool {
+        true
     }
     fn status(&self) -> Option<Self::Status>;
-    fn created_at(&self) -> &str;
-    fn updated_at(&self) -> &str;
+    /// The muted "Updated … · Created …" line; `None` leaves the line out.
+    fn timestamps(&self) -> Option<String>;
 
     async fn list() -> Result<Vec<Self>, ApiError>;
     /// Create (`id` is `None`) or update an item from a validated editor submission.
-    async fn save(id: Option<String>, submission: Submission) -> Result<(), ApiError>;
+    async fn save(
+        id: Option<String>,
+        submission: <Self::Form as EditorForm>::Submission,
+    ) -> Result<(), ApiError>;
     async fn delete(id: String) -> Result<(), ApiError>;
 }
 
-/// A list card's copy and roles. `editor.role` prefixes the shared `data-role`s:
-/// `<role>-add`, `<role>-new-item`, `<role>-empty`, `<role>-edit`, `<role>-delete`.
-pub(super) struct CardText {
+/// A list card's copy, roles and form config. `editor.role` prefixes the shared
+/// `data-role`s: `<role>-add`, `<role>-new-item`, `<role>-empty`, `<role>-edit`,
+/// `<role>-delete`.
+pub(super) struct CardText<F: EditorForm> {
     pub(super) editor: EditorSpec,
-    /// The port a new item starts with.
-    pub(super) default_port: u16,
+    /// The card's form config (placeholders, the extra field, the new item's port).
+    pub(super) form: F::Config,
     /// `data-role` of the `<ul>`.
     pub(super) list_role: &'static str,
     pub(super) add_text: &'static str,
     pub(super) empty_text: &'static str,
     /// Names the item in the delete confirmation when its label is unknown.
     pub(super) fallback_name: &'static str,
+    /// The row's buttons.
+    pub(super) edit_text: &'static str,
+    pub(super) delete_text: &'static str,
+    /// The delete confirmation: these two around the item's label.
+    pub(super) delete_question: (&'static str, &'static str),
     /// The editor's message line while an update / a create is in flight.
     pub(super) saving: &'static str,
     pub(super) creating: &'static str,
@@ -79,7 +93,7 @@ pub(super) struct CardText {
     pub(super) updated: &'static str,
     pub(super) added: &'static str,
     pub(super) deleted: &'static str,
-    /// Prefixes of the failure messages (the API error follows).
+    /// Prefixes of the failure messages (the error follows).
     pub(super) save_failed: &'static str,
     pub(super) delete_failed: &'static str,
     /// The toast when the row being edited was deleted elsewhere.
@@ -88,7 +102,7 @@ pub(super) struct CardText {
 
 /// The card-specific parts of a row's summary.
 pub(super) struct RowParts {
-    /// After `host:port` on the address line: the status badge, then an aside.
+    /// After the address on the address line: the status badge, then an aside.
     pub(super) line: AnyView,
     /// Muted lines above "Updated … · Created …".
     pub(super) meta: Option<AnyView>,
@@ -98,14 +112,15 @@ pub(super) struct RowParts {
     pub(super) actions: AnyView,
 }
 
-/// The editor values of an item.
-fn draft_values<T: CardItem>(item: &T) -> DraftValues {
-    DraftValues {
-        label: item.label().to_string(),
-        host: item.host().to_string(),
-        port: item.port().to_string(),
-        enabled: item.is_enabled(),
-        extra: item.extra().to_string(),
+/// A failed request's line: the card's `prefix`, then the server's own (Slovak)
+/// message when it sent one ([`ApiError::Refused`]) or the card's own refusal
+/// ([`ApiError::NotFound`]), else the error as it is.
+pub(super) fn failure_text(prefix: &str, err: &ApiError) -> String {
+    match err {
+        ApiError::Refused(_, message) | ApiError::NotFound(message) => {
+            format!("{prefix} {message}")
+        }
+        other => format!("{prefix} {other}"),
     }
 }
 
@@ -114,10 +129,10 @@ pub(super) struct ListCard<T: CardItem> {
     /// The last applied list. Written only by [`Self::fetch`]; cards read it through
     /// [`Self::items`].
     items: RwSignal<Vec<T>>,
-    editor: ListEditor,
+    editor: ListEditor<T::Form>,
     order: ResponseOrder,
     toast: ToastHandle,
-    text: &'static CardText,
+    text: &'static CardText<T::Form>,
 }
 
 impl<T: CardItem> Clone for ListCard<T> {
@@ -130,10 +145,10 @@ impl<T: CardItem> Copy for ListCard<T> {}
 
 impl<T: CardItem> ListCard<T> {
     /// The card's state; loads the list and starts the 5 s status poll.
-    pub(super) fn new(toast: ToastHandle, text: &'static CardText) -> Self {
+    pub(super) fn new(toast: ToastHandle, text: &'static CardText<T::Form>) -> Self {
         let card = Self {
             items: RwSignal::new(Vec::new()),
-            editor: ListEditor::new(text.editor.extra),
+            editor: ListEditor::new(text.form),
             order: ResponseOrder::default(),
             toast,
             text,
@@ -171,13 +186,13 @@ impl<T: CardItem> ListCard<T> {
     }
 
     fn open_new(self) {
-        self.editor.open_new(self.text.default_port);
+        self.editor.open_new();
     }
 
     fn open_edit(self, id: &str) {
         let values = self
             .items
-            .with_untracked(|list| list.iter().find(|item| item.id() == id).map(draft_values));
+            .with_untracked(|list| list.iter().find(|item| item.id() == id).map(T::values));
         if let Some(values) = values {
             self.editor.open_item(id.to_string(), values);
         }
@@ -204,7 +219,7 @@ impl<T: CardItem> ListCard<T> {
                     self.editor.finish_save(&ticket, None);
                 }
                 Err(err) => {
-                    let message = format!("{} {err}", text.save_failed);
+                    let message = failure_text(text.save_failed, &err);
                     if !self.editor.finish_save(&ticket, Some(&message)) {
                         self.toast.show(&message, "error");
                     }
@@ -222,7 +237,8 @@ impl<T: CardItem> ListCard<T> {
                     .map(|item| item.label().to_string())
             })
             .unwrap_or_else(|| self.text.fallback_name.to_string());
-        if !confirm(&format!("Remove {name}? Presenter will stop reconnecting.")) {
+        let (before, after) = self.text.delete_question;
+        if !confirm(&format!("{before}{name}{after}")) {
             return;
         }
         leptos::task::spawn_local(async move {
@@ -234,7 +250,7 @@ impl<T: CardItem> ListCard<T> {
                 }
                 Err(err) => self
                     .toast
-                    .show(&format!("{} {err}", self.text.delete_failed), "error"),
+                    .show(&failure_text(self.text.delete_failed, &err), "error"),
             }
         });
     }
@@ -267,7 +283,7 @@ impl<T: CardItem> ListCard<T> {
                     <button type="button" class="settings__button settings__button--primary"
                         node_ref=add_ref data-role=format!("{role}-add")
                         data-lock=move || add_lock.get().map(TriggerLock::data_lock)
-                        title=move || add_lock.get().and_then(TriggerLock::title)
+                        title=move || add_lock.get().and_then(|lock| lock.title(&spec))
                         prop:disabled=move || add_lock.get().is_some()
                         on:click=move |_| self.open_new()>{add_text}</button>
                 </div>
@@ -275,7 +291,7 @@ impl<T: CardItem> ListCard<T> {
                     <Show when=adding>
                         <li class="settings__list-item" data-role=format!("{role}-new-item")
                             data-editing="true">
-                            {render_connection_editor(spec, editor, true, save)}
+                            {render_editor(spec, editor, true, save)}
                         </li>
                     </Show>
                     <Show when=move || items.with(Vec::is_empty) && !adding()>
@@ -313,12 +329,7 @@ impl<T: CardItem> ListCard<T> {
         let timestamps = {
             let id = id.clone();
             Memo::new(move |_| {
-                items.with(|list| {
-                    list.iter()
-                        .find(|x| x.id() == id)
-                        .map(|x| updated_created(x.updated_at(), x.created_at()))
-                        .unwrap_or_default()
-                })
+                items.with(|list| list.iter().find(|x| x.id() == id).and_then(T::timestamps))
             })
         };
         let enabled = item.is_enabled().to_string();
@@ -329,7 +340,7 @@ impl<T: CardItem> ListCard<T> {
             <li class="settings__list-item" data-id=id data-enabled=enabled
                 data-editing=move || editing_this.get().to_string()>
                 {move || if editing_this.get() {
-                    render_connection_editor(spec, editor, false, save)
+                    render_editor(spec, editor, false, save)
                 } else {
                     summary()
                 }}
@@ -337,20 +348,23 @@ impl<T: CardItem> ListCard<T> {
         }
     }
 
-    /// A row's summary: label, `host:port` + the card's line parts, the card's meta,
-    /// "Updated … · Created …", the warning, then the card's buttons + Edit / Delete.
+    /// A row's summary: label, the address + the card's line parts, the card's meta,
+    /// "Updated … · Created …" (when the item has it), the warning, then the card's
+    /// buttons + Edit / Delete.
     fn render_summary<R>(
         self,
         item: &T,
         status: Memo<Option<T::Status>>,
-        timestamps: Memo<String>,
+        timestamps: Memo<Option<String>>,
         row: R,
     ) -> AnyView
     where
         R: Fn(&T, Memo<Option<T::Status>>) -> RowParts,
     {
         let editor = self.editor;
-        let role = self.text.editor.role;
+        let text = self.text;
+        let spec = text.editor;
+        let role = spec.role;
         let id = item.id().to_string();
         let edit_ref = NodeRef::<leptos::html::Button>::new();
         focus_on_close(editor, edit_ref, EditTarget::Item(id.clone()));
@@ -360,8 +374,7 @@ impl<T: CardItem> ListCard<T> {
         };
         let parts = row(item, status);
         let label = item.label().to_string();
-        let host = item.host().to_string();
-        let port = format!(":{}", item.port());
+        let (code, after) = item.address();
         let (id_edit, id_delete) = (id.clone(), id);
 
         view! {
@@ -372,15 +385,15 @@ impl<T: CardItem> ListCard<T> {
                     </div>
                     <p class="settings__list-line">
                         <span class="settings__host-addr">
-                            <code>{host}</code>
-                            {port}
+                            <code>{code}</code>
+                            {after}
                         </span>
                         {parts.line}
                     </p>
                     {parts.meta}
-                    <p class="settings__list-meta settings__list-meta--muted">
-                        {move || timestamps.get()}
-                    </p>
+                    {move || timestamps.get().map(|line| view! {
+                        <p class="settings__list-meta settings__list-meta--muted">{line}</p>
+                    })}
                     {parts.warning}
                 </div>
                 <div class="settings__list-actions">
@@ -388,15 +401,41 @@ impl<T: CardItem> ListCard<T> {
                     <button type="button" class="settings__button settings__button--ghost settings__button--small"
                         node_ref=edit_ref data-role=format!("{role}-edit") data-id=id_edit.clone()
                         data-lock=move || lock.get().map(TriggerLock::data_lock)
-                        title=move || lock.get().and_then(TriggerLock::title)
+                        title=move || lock.get().and_then(|lock| lock.title(&spec))
                         prop:disabled=move || lock.get().is_some()
-                        on:click=move |_| self.open_edit(&id_edit)>"Edit"</button>
+                        on:click=move |_| self.open_edit(&id_edit)>{text.edit_text}</button>
                     <button type="button" class="settings__button settings__button--danger settings__button--small"
                         data-role=format!("{role}-delete") data-id=id_delete.clone()
-                        on:click=move |_| self.delete(id_delete.clone())>"Delete"</button>
+                        on:click=move |_| self.delete(id_delete.clone())>{text.delete_text}</button>
                 </div>
             </div>
         }
         .into_any()
     }
 }
+
+#[cfg(test)]
+mod tests {
+    use super::failure_text;
+    use crate::api::ApiError;
+
+    #[test]
+    fn a_refusal_shows_the_servers_own_words_and_anything_else_the_error() {
+        let refused = ApiError::Refused(422, "Mikrofón „guest“: neplatná IP adresa.".into());
+        assert_eq!(
+            failure_text("Mikrofón sa nepodarilo uložiť.", &refused),
+            "Mikrofón sa nepodarilo uložiť. Mikrofón „guest“: neplatná IP adresa."
+        );
+        let gone = ApiError::NotFound("Tento mikrofón medzitým niekto odstránil.".into());
+        assert_eq!(
+            failure_text("Mikrofón sa nepodarilo uložiť.", &gone),
+            "Mikrofón sa nepodarilo uložiť. Tento mikrofón medzitým niekto odstránil."
+        );
+        let status = ApiError::Status(500, "Internal Server Error".into());
+        assert_eq!(
+            failure_text("Unable to save connection.", &status),
+            "Unable to save connection. HTTP 500: Internal Server Error",
+            "the connection cards' lines are unchanged"
+        );
+    }
+}
diff --git a/crates/presenter-ui/src/pages/settings/mod.rs b/crates/presenter-ui/src/pages/settings/mod.rs
index 4188ea8..c246d38 100644
--- a/crates/presenter-ui/src/pages/settings/mod.rs
+++ b/crates/presenter-ui/src/pages/settings/mod.rs
@@ -17,6 +17,7 @@ use crate::components::version_label::VersionLabel;
 mod ableton;
 mod android;
 mod companion;
+mod connection_form;
 mod host_editor;
 mod list_card;
 mod list_sync;
diff --git a/crates/presenter-ui/src/pages/settings/resolume.rs b/crates/presenter-ui/src/pages/settings/resolume.rs
index e1b419a..76e5dbb 100644
--- a/crates/presenter-ui/src/pages/settings/resolume.rs
+++ b/crates/presenter-ui/src/pages/settings/resolume.rs
@@ -10,31 +10,43 @@
 
 use leptos::prelude::*;
 
-use super::host_editor::{EditorSpec, Submission};
+use super::connection_form::{ConnectionForm, ConnectionSpec, DraftValues, Submission};
+use super::host_editor::{EditorSpec, SAVING_TITLE, UNSAVED_TITLE};
 use super::list_card::{CardItem, CardText, ListCard, RowParts};
-use super::row_status::{latency_text, resolume_state, resolume_warning, HostWarning};
+use super::row_status::{
+    latency_text, resolume_state, resolume_warning, updated_created, HostWarning,
+};
 use super::{capitalize, ToastHandle};
 use crate::api::settings::{self, ResolumeHostDraft, ResolumeHostDto, ResolumeStatusDto};
 use crate::api::ApiError;
 
-static TEXT: CardText = CardText {
+static TEXT: CardText<ConnectionForm> = CardText {
     editor: EditorSpec {
         role: "host",
         message_id: "resolume-form-status",
         message_role: "form-status",
-        label_placeholder: "Main Arena",
-        host_placeholder: "resolume.lan",
         new_title: "New Resolume connection",
         new_submit: "Add connection",
         edit_title: "Edit Resolume connection",
+        save_text: "Save",
+        cancel_text: "Cancel",
+        unsaved_title: UNSAVED_TITLE,
+        saving_title: SAVING_TITLE,
+    },
+    form: ConnectionSpec {
+        label_placeholder: "Main Arena",
+        host_placeholder: "resolume.lan",
+        // Arena's default web-server port.
+        default_port: 8090,
         extra: None,
     },
-    // Arena's default web-server port.
-    default_port: 8090,
     list_role: "resolume-host-list",
     add_text: "+ Add connection",
     empty_text: "No Resolume connections defined yet.",
     fallback_name: "this connection",
+    edit_text: "Edit",
+    delete_text: "Delete",
+    delete_question: ("Remove ", "? Presenter will stop reconnecting."),
     saving: "Saving changes…",
     creating: "Creating connection…",
     updated: "Updated Resolume connection.",
@@ -59,6 +71,7 @@ fn row_key(h: &ResolumeHostDto) -> (String, String, String, u16, bool) {
 }
 
 impl CardItem for ResolumeHostDto {
+    type Form = ConnectionForm;
     type Status = ResolumeStatusDto;
     type Key = (String, String, String, u16, bool);
 
@@ -74,12 +87,18 @@ impl CardItem for ResolumeHostDto {
         &self.label
     }
 
-    fn host(&self) -> &str {
-        &self.host
+    fn address(&self) -> (String, String) {
+        (self.host.clone(), format!(":{}", self.port))
     }
 
-    fn port(&self) -> u16 {
-        self.port
+    fn values(&self) -> DraftValues {
+        DraftValues {
+            label: self.label.clone(),
+            host: self.host.clone(),
+            port: self.port.to_string(),
+            enabled: self.is_enabled,
+            extra: String::new(),
+        }
     }
 
     fn is_enabled(&self) -> bool {
@@ -90,12 +109,8 @@ impl CardItem for ResolumeHostDto {
         self.status.clone()
     }
 
-    fn created_at(&self) -> &str {
-        &self.created_at
-    }
-
-    fn updated_at(&self) -> &str {
-        &self.updated_at
+    fn timestamps(&self) -> Option<String> {
+        Some(updated_created(&self.updated_at, &self.created_at))
     }
 
     async fn list() -> Result<Vec<Self>, ApiError> {
```

- [ ] **Step 3: Check that the connection cards render exactly as before** — read the patch once more for the two cards: every `data-role`, class, text and placeholder of the old `host_editor` markup (`<role>-editor`, `<role>-editor-title`, `<role>-label` / `-host` / `-port` / `-enabled` / `-submit` / `-cancel` with `<role>` = `host` for Resolume and Android's own prefix, `android-component`, the message line ids, the "Enabled" text, Save / Cancel, "Edit" / "Delete", the delete question, the tooltips) is now rendered from `ConnectionForm::render_fields` / `render_action_lead` or `CardText` with the same strings. The E2E specs listed above select those exact roles and texts.

- [ ] **Step 4: Gates** — `(cd crates/presenter-ui && cargo fmt --check)`; `QC_TARGETS="$(printf '%s\n' crates/presenter-ui/src/pages/settings/{connection_form,host_editor,list_card,resolume,android,mod}.rs)" python3 scripts/dev/fn_length_check.py .`; `bash scripts/dev/count_prod_lines.sh crates/presenter-ui/src/pages/settings/host_editor.rs` → `575`, `connection_form.rs` → `275`.

- [ ] **Step 5: Commit** — paths `crates/presenter-ui/src/pages/settings/{connection_form,host_editor,list_card,resolume,android,mod}.rs`, message `refactor(settings): the list card's editor over any form, the connection form on its own (#852 C1b-1)`.

### Task 4: Playbook, then close lane C1b-1

**Files:**
- Modify: `.claude/rules/settings-ui.md`, `CLAUDE.md` (the router row "Settings tab (#819)")

- [ ] **Step 1: Apply the docs** (patch recipe):

```diff
diff --git a/.claude/rules/settings-ui.md b/.claude/rules/settings-ui.md
index 73f526a..b74adca 100644
--- a/.claude/rules/settings-ui.md
+++ b/.claude/rules/settings-ui.md
@@ -50,20 +50,28 @@ faded the secondary text (3.0:1), the warning (3.9:1) and the status badges
 (3.6–4.5:1) below 4.5:1, while only the label stayed readable. Before adding a
 colour to a row, compute its ratio against `#334155` and `#263346`.
 
-## Resolume / Android lists: `list_card::ListCard` + `host_editor::ListEditor`
-
-The two cards are thin. Each implements `CardItem` for its DTO (id, key, fields,
-status, and the `list` / `save` / `delete` API calls as `async fn`s), gives a
-`static CardText` (copy, roles, `EditorSpec`), and builds the card-specific row
-parts (`RowParts`: status badge + aside, meta lines, warning, extra buttons).
+## List cards: `list_card::ListCard` + `host_editor::ListEditor<F>` over an `EditorForm`
+
+The list cards are thin (Resolume, Android; from #852 C1b-6 the listening
+microphones). Each implements `CardItem` for its DTO (id, key, label, `address()`,
+`values()` for the editor, optional `timestamps()`, status, and the `list` /
+`save` / `delete` API calls as `async fn`s), gives a `static CardText<Form>` (ALL
+copy — buttons, delete question, tooltips, editor texts — plus the form's config),
+and builds the card-specific row parts (`RowParts`: status badge + aside, meta
+lines, warning, extra buttons). What the editor edits is the card's `EditorForm`
+(#852 C1b-1): `connection_form::ConnectionForm` (label / host / port / extra field
+/ "Enabled", config `ConnectionSpec`) for the connection cards. A new kind of item
+gets its own `EditorForm` (values, `validate`, `render_fields`), never fields
+bolted onto `DraftValues`; `host_editor.rs` holds only the shared state machine.
 Everything else is shared: put a new list behaviour or guard in `list_card.rs` /
-`host_editor.rs`, never in one card.
+`host_editor.rs`, never in one card. A failed save / delete line is
+`failure_text` (the server's own message for `ApiError::Refused` / `NotFound`).
 
 - **Edit** renders the editor IN the row; "+ Add …" renders it as the first `<li>`
   (`<role>-new-item`). `EditTarget` = Closed / New / Item(id). The draft lives at
   CARD level (`ListEditor`), outside the keyed rows, so a poll can never reset typing.
 - A card's extra field (Android's launch package) is part of the draft
-  (`DraftValues.extra`, `EditorSpec.extra`): `open_*` loads it, `begin_save`
+  (`DraftValues.extra`, `ConnectionSpec.extra`): `open_*` loads it, `begin_save`
   validates it via `validate_draft`. Never a separate signal with its own paths.
 - Field `data-role`s (`host-label`, `android-component`, …) and the message ids
   (`resolume-form-status`, `android-form-status`) stay unique ONLY because one
@@ -96,9 +104,10 @@ same lock, so a locked trigger can never open the editor:
   disabled and Escape is ignored (`ListEditor::cancel`). The late result can then
   never land on a different editor.
 - **`unsaved`** — the draft differs from what the editor was opened with (the
-  `dirty` Memo: current `DraftValues` != `loaded`, every field incl. the extra
-  one). Every OTHER trigger is locked, with the title "Save or cancel the open
-  editor first". Typing the stored value back unlocks again.
+  `dirty` Memo: the form's current `Values` != `loaded`, every field incl. the
+  extra one). Every OTHER trigger is locked, with the card's
+  `EditorSpec.unsaved_title` ("Save or cancel the open editor first" on the
+  connection cards). Typing the stored value back unlocks again.
 - **`open`** — the trigger's own editor is already open ("+ Add" while adding).
 - A clean editor locks nothing: Edit on another row switches in one click.
 
diff --git a/CLAUDE.md b/CLAUDE.md
index d77da5b..97249c8 100644
--- a/CLAUDE.md
+++ b/CLAUDE.md
@@ -49,7 +49,7 @@ This file provides guidance to Claude Code (claude.ai/code) when working with co
 | Resolume port drift (#564/#744/#813): never adopt a port another host on the same machine owns (same host string or a shared resolved IP, re-checked every tick; a disabled host still owns its configured port, an enabled one also its adopted port; `port_claims.rs` table rebuilt by `set_hosts`, a persisted `active_port` is only a seed: kept unless another host owns it, else dropped before the worker is seeded; check-and-claim on adopt; a running host's sibling-owned port dropped + persisted on `SiblingsChanged`); tests use `free_port_pair()` / `free_port_triple()`, never `free_port()+N` | `.claude/rules/resolume-port-drift.md` (auto-loads on `resolume/port_drift*.rs`, `resolume/port_claims.rs`, `resolume/mod.rs`, `resolume/driver.rs`, `resolume/mapping_refresh.rs`) |
 | Resolume composition fetches (#808): NEVER read `/composition` (16 MB on SNV) on a timer — the 10 s tick (and the Test button) is a `/product` probe; fetch only on missing / error-invalidated (#563b threshold = recovery) / stale-id (push 404 → typed `StaleIdError`, invalidate + retry once; 404 on a fresh mapping → 60 s pause) / manual ("Refresh mapping" endpoint + settings button); every new PUT/connect maps 404 → `StaleIdError`; keep pushes idempotent until success (flip/dedup after OK); one `resolume_http_client()`, no connection reuse; a fetch can hit Arena MID-LOAD (PP: 27 tag-less clips) and a deck switch changes every clip id, both with NO 404 → `provisional_mapping.rs`: per-deck last-good kinds, suspect → follow-ups 2/5/15/30/60 s (select! deadline, never a timer), deck check `decks/by-id/{id}` in `ensure_mapping_for_push` + on the tick (switch → inline refetch, failed check → push anyway), lane refetch re-armed on every follow-up step, a used one waits only 3 s (owner: no text skipped; only a deck accepted as lacking a kind stops refetching), an unchanged composition settles as the deck's content; mock Arenas listing `decks` must serve the deck route | `.claude/rules/resolume-mapping-refresh.md` (auto-loads on `resolume/driver.rs`, `resolume/mod.rs`, `resolume/mapping_refresh*.rs`, `resolume/provisional_*.rs`, `resolume/keepalive_tests.rs`, `handlers.rs`, `bible_clear.rs`, `port_drift.rs`, the mocks + settings card) |
 | NDI `ndi_frames_live` per-session Cell vs shared-signal desync (#757/#732/#500): never write the shared signal `false` without resetting the `FrameStats` Cell; same-source reactivation must not reset the gate; ticker self-heal + pure/clock-wired host-test split | `.claude/rules/ndi-frames-live-desync.md` (auto-loads on `pages/stage.rs`, `pages/stage_events.rs`, `components/stage/ndi_frame_stats.rs`, `ndi_health_ticker.rs`, `ndi_video.rs`) |
-| Settings tab (#819): ONE `.settings__form-row` system (never a per-card override — the 220 px-gap bug), list rows/badges/editor in `settings_lists.css`; Resolume/Android = thin cards over the shared `list_card::ListCard` (fetch/save/delete/rows) + `host_editor::ListEditor` state machine (one editor per card, card-level draft, `trigger_lock` saving/unsaved guards, removed-elsewhere toast); row text ≥4.5:1 on #334155; row key = id + shown/edited fields, never status or `updated_at` (port drift bumps it) | `.claude/rules/settings-ui.md` (auto-loads on `pages/settings/**`, `styles/settings*.css`, `tests/e2e/settings*.spec.ts`) |
+| Settings tab (#819): ONE `.settings__form-row` system (never a per-card override — the 220 px-gap bug), list rows/badges/editor in `settings_lists.css`; list cards = thin cards over the shared `list_card::ListCard` (fetch/save/delete/rows) + `host_editor::ListEditor<F>` state machine over the card's `EditorForm` (#852 C1b-1: `connection_form::ConnectionForm` for Resolume/Android, all copy in `CardText`; one editor per card, card-level draft, `trigger_lock` saving/unsaved guards, removed-elsewhere toast); row text ≥4.5:1 on #334155; row key = id + shown/edited fields, never status or `updated_at` (port drift bumps it) | `.claude/rules/settings-ui.md` (auto-loads on `pages/settings/**`, `styles/settings*.css`, `tests/e2e/settings*.spec.ts`) |
 | Stage displays drop-out (#793): launcher adb reads tri-state (never install/uninstall on an adb error), never churn a `device` target (disconnect only after 3 stuck cycles), stage WS applies EVERY event via a handler (no coalescing last-event signal), snapshot-driven layout reconcile + reconnect resync, `routeWebSocket` lost-frame E2E, WS end-reason / real peer IP / layout-switch logs, per-display layout adopted from heartbeat ACK + diag frames (#797) | `.claude/rules/stage-live-sync.md` (auto-loads on `ws/stage.rs`, `pages/stage*.rs`, `pages/camera.rs`, `live.rs`, `stage_connections.rs`, `android_stage*`) |
 | AI status on `/healthz` + `/ai/status` (#760): ONE shared `evaluate_ai_status`; backend-agnostic `{connected,error,model}` (no OAuth fields, OpenRouter-ready); `/healthz` served through a per-AppState SWR cache (30s TTL, background refresh, single in-flight) so operator-tab fan-out never storms the backend — `/ai/status` stays live; render code in `router/ai_health.rs` + cache in `ai/health_cache.rs`, not the over-cap `router/ai.rs`; deploy gate `connected:false`→non-blocking `::error::`, `modelValid:false`→hard-fail | `.claude/rules/ai-health-endpoint.md` (auto-loads on `router/ai.rs`, `router/ai_health.rs`, `router.rs`) |
 | AI agent loop + bible validator (#784): composer emits the reference the validator sees (`format_verse_range` comma-lists) so `normalize_reference` must accept every composer output; `run_agent_with_guard` seam for race-free budget tests; `##`-marker trick for a deterministic rejection; GaveUp is not an outage (no `AiCallHealth` write); chip fold/probe cross-crate prefix `posledné AI volanie zlyhalo` | `.claude/rules/ai-agent-loop.md` (auto-loads on `ai/agent*.rs`, `ai/bible_validator.rs`, `ai/tools/bible_presentation.rs`, `components/ai_status.rs`) |
```

- [ ] **Step 2: Commit** — paths `.claude/rules/settings-ui.md CLAUDE.md`, message `docs(playbook): list cards over an EditorForm (#852 C1b-1) [no-test: docs only]`.

- [ ] **Step 3: Measure the lane** — `bash "$SCRATCH/lane_measure.sh"`. Expected:

```text
crates/presenter-core/src/listen/mics.rs                                 +19
crates/presenter-ui/src/pages/settings/android.rs                        +30
crates/presenter-ui/src/pages/settings/connection_form.rs                +275
crates/presenter-ui/src/pages/settings/host_editor.rs                    +126
crates/presenter-ui/src/pages/settings/list_card.rs                      +78
crates/presenter-ui/src/pages/settings/mod.rs                            +1
crates/presenter-ui/src/pages/settings/resolume.rs                       +32
lane total: 561 production lines added or changed
```

(± a few lines if `cargo fmt` rewrapped something; a total over 600 means a step went wrong — stop and compare with the patches.)

- [ ] **Step 4: Local gates** (the block in Global Constraints).

- [ ] **Step 5: Push and watch CI** (Global Constraints "Push + CI") — especially the presenter-ui unit tests, the UI build (the WASM crate compiles only there) and the E2E shards carrying the four settings specs.

- [ ] **Step 6: Verify on dev after deploy-dev** (load `post-deploy-verification`): in a real browser (Playwright) open `$DEV/ui/settings`: the footer shows the new version; Resolume and Android cards render; "+ Add connection" opens the editor with port 8090 and "Enabled" checked, Escape closes it; Edit on an existing row opens in place with its values; after typing, another row's Edit is locked with the tooltip "Save or cancel the open editor first"; Cancel; zero console errors. Nothing is saved.

- [ ] **Step 7: Close.** `playbook-review` (`📔 Playbook:` line — the rule landed in Step 1); comment on #852: lane C1b-1 merged (generic list editor, `connection_form.rs`, `mic_id_for_role`), next lane C1b-2.

---

## PR unit C1b-2 — the recorder's contract, its retention and the frame channel (515 lines)

Starts from `origin/dev` with C1b-1 merged. Ships the core contract of the recorder (setting, toggle, status DTOs, WAV header, file layout, retention rule), on the server the recordings folder plus the daily retention pass — BEFORE anything can be recorded (deletion exists before creation) — and the frame channel every chain sends into. Nothing records yet: no writer exists, and nothing subscribes to the channel, so `send_frames` returns at once (`receiver_count() == 0`). Visible on SNV / PP: one supervisor task that finds nothing to delete (Decision 11).

### Task 5: Version check

**Files:** `Cargo.toml`, `Cargo.lock` (Task 1 recipe).

- [ ] **Step 1:** Run Task 1 Step 1. `OK` → done. `BUMP` → Task 1 Steps 2–3.

### Task 6: Core — the recorder's contract and the WAV header

**Files:**
- Create: `crates/presenter-core/src/audio/wav.rs`, `crates/presenter-core/src/listen/recorder.rs`
- Modify: `crates/presenter-core/src/audio/mod.rs` (`pub mod wav;`), `crates/presenter-core/src/listen/mod.rs` (`pub mod recorder;`), `crates/presenter-core/src/listen/mics.rs` (`is_valid_mic_id` becomes `pub(crate)`: the file-name rule reuses it)

**Interfaces** (all `pub`, under `presenter_core::audio::wav` / `presenter_core::listen::recorder`, no root re-export):
- `WAV_HEADER_LEN = 44`, `WAV_SAMPLE_RATE_HZ = 16_000`, `WAV_BLOCK_BYTES = 2`, `WAV_RIFF_SIZE_AT = 4`, `WAV_DATA_SIZE_AT = 40`; `fn wav_header(data_bytes: u32) -> [u8; 44]`; `fn wav_riff_size(data_bytes: u32) -> u32` (`36 + data`, saturating — no `WAV_MAX`, no "full" error, B7).
- `LISTEN_RECORDER_KEY = "listen.recorder"`, `RECORDING_RETENTION_DAYS = 30`, `RECORDING_NAMES_PER_SECOND = 9`.
- `RecorderSetting { enabled_for: Option<NaiveDate> }` (camelCase, `#[serde(default)]`; `records_on(today)`), `RecorderToggle { enabled: bool }`, `RecordingState { Off, Waiting, Recording, Error }` (snake_case), `MicRecording { id, state, file, bytes, dropped_frames, error }`, `RecorderStatus { enabled, enabled_for, root, mics }` (both camelCase).
- `recording_day_dir(NaiveDate) -> String` (`YYYY-MM-DD`), `parse_recording_day(&str) -> Option<NaiveDate>` (canonical spelling only), `recording_expired(day, today) -> bool` (today − day > 30), `recording_file_name(mic_id, NaiveTime, attempt) -> String` (`<id>_<HHMMSS>.wav`, attempt 2 … 9 → `-n`), `is_recording_file_name(&str) -> bool` (only those names, valid mic id).
- Consumed by: Task 7 (retention), lane 3 (status, toggle), lane 4 (files), lane 6–7 (UI).

- [ ] **Step 1: Create `crates/presenter-core/src/audio/wav.rs`:**

```rust
//! The recorder's file format (#852 C1b, spec §5.5): 16 kHz mono s16le — the
//! frames the ASR gets — behind the canonical 44-byte PCM WAV header. The
//! recorder patches the two size fields after every batch it writes, so a
//! crash leaves a file whose header covers the audio up to the last batch. A
//! file never comes near the 4 GiB a u32 size field can hold: one mic's day is at
//! most 2.76 GB (24 h × 32 000 B/s) and the recorder starts new files at midnight.

/// The header length; the audio starts here.
pub const WAV_HEADER_LEN: usize = 44;
/// The recorder's sample rate (spec §5.4: 16 kHz mono s16le).
pub const WAV_SAMPLE_RATE_HZ: u32 = 16_000;
/// Mono s16: one sample is one block of two bytes.
pub const WAV_BLOCK_BYTES: u16 = 2;
/// Where the RIFF size field (`36 + data`) is.
pub const WAV_RIFF_SIZE_AT: u64 = 4;
/// Where the data size field is.
pub const WAV_DATA_SIZE_AT: u64 = 40;

/// The header of a file holding `data_bytes` of audio.
pub fn wav_header(data_bytes: u32) -> [u8; WAV_HEADER_LEN] {
    let byte_rate = WAV_SAMPLE_RATE_HZ * u32::from(WAV_BLOCK_BYTES);
    let mut header = [0u8; WAV_HEADER_LEN];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&wav_riff_size(data_bytes).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    header[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    header[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    header[24..28].copy_from_slice(&WAV_SAMPLE_RATE_HZ.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&WAV_BLOCK_BYTES.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits per sample
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    header
}

/// The RIFF size field of a file holding `data_bytes` of audio.
pub fn wav_riff_size(data_bytes: u32) -> u32 {
    data_bytes.saturating_add(36)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Python's `wave` module writes for one second of 16 kHz mono s16
    /// (Task 6 Step 1).
    const ONE_SECOND: [u8; WAV_HEADER_LEN] = [
        82, 73, 70, 70, 36, 125, 0, 0, 87, 65, 86, 69, 102, 109, 116, 32, 16, 0, 0, 0, 1, 0, 1, 0,
        128, 62, 0, 0, 0, 125, 0, 0, 2, 0, 16, 0, 100, 97, 116, 97, 0, 125, 0, 0,
    ];

    #[test]
    fn one_second_has_the_header_python_writes() {
        assert_eq!(wav_header(32_000), ONE_SECOND);
        assert_eq!(&wav_header(0)[4..8], &36u32.to_le_bytes());
        assert_eq!(&wav_header(0)[40..44], &0u32.to_le_bytes());
    }

    #[test]
    fn the_size_fields_sit_where_the_recorder_patches_them() {
        let header = wav_header(0x0102_0304);
        let riff = WAV_RIFF_SIZE_AT as usize;
        let data = WAV_DATA_SIZE_AT as usize;
        assert_eq!(
            &header[riff..riff + 4],
            &(0x0102_0304u32 + 36).to_le_bytes()
        );
        assert_eq!(&header[data..data + 4], &0x0102_0304u32.to_le_bytes());
    }

    #[test]
    fn the_riff_size_is_the_audio_plus_36_and_never_wraps() {
        assert_eq!(wav_riff_size(0), 36);
        assert_eq!(wav_riff_size(32_000), 32_036);
        assert_eq!(wav_riff_size(u32::MAX), u32::MAX, "saturates, never wraps");
    }
}
```

The header bytes of `one_second_has_the_header_python_writes` were produced while planning with Python's `wave` module (`nchannels=1, sampwidth=2, framerate=16000`, 32 000 zero bytes); the test compares all 44 bytes.

- [ ] **Step 2: Create `crates/presenter-core/src/listen/recorder.rs`:**

```rust
//! The recorder's contract (#852 C1b, spec §5.5, §12): its `app_settings` row,
//! the toggle's request, its part of `GET /listen/status`, and the file layout
//! `<root>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav` with its 30-day retention.
//!
//! The toggle is a per-DAY switch: switching it on stores the server's local
//! date (`enabledFor`); the recorder records while that date is today, so it
//! goes off by itself at midnight and a restart the same day resumes it. Every
//! file is one continuous stretch of a mic's audio, named by the local time of
//! its first frame — the bake-off places the show-log times by it (spec §13).

use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::listen::mics::is_valid_mic_id;

/// The `app_settings` key of the toggle.
pub const LISTEN_RECORDER_KEY: &str = "listen.recorder";
/// Day folders older than this many days are deleted (spec §5.5).
pub const RECORDING_RETENTION_DAYS: i64 = 30;
/// The most files one mic can start within one second (`_HHMMSS`, `-2` … `-9`).
pub const RECORDING_NAMES_PER_SECOND: u32 = 9;
const RECORDING_EXTENSION: &str = ".wav";

/// The stored `listen.recorder` row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderSetting {
    /// The local date the recorder was switched on for; `None` = off.
    #[serde(default)]
    pub enabled_for: Option<NaiveDate>,
}

impl RecorderSetting {
    /// Whether the recorder records on `today`.
    pub fn records_on(&self, today: NaiveDate) -> bool {
        self.enabled_for == Some(today)
    }
}

/// `PUT /listen/recorder`: on = today, off = not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecorderToggle {
    pub enabled: bool,
}

/// One mic's recorder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingState {
    /// Not recording: switched off, the day is over, or never on.
    #[default]
    Off,
    /// Switched on, no audio from the mic yet: a file opens with its first frame.
    Waiting,
    Recording,
    /// A write failed; the writer is started again after a pause.
    Error,
}

/// One mic in the recorder's status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicRecording {
    pub id: String,
    pub state: RecordingState,
    /// The file being written, or the last one.
    pub file: Option<String>,
    /// Audio bytes in that file (16 kHz mono s16: 32 000 per second).
    pub bytes: u64,
    /// Frames lost because the writer fell 12.8 s behind (since the server started).
    pub dropped_frames: u64,
    /// Slovak, for the operator.
    pub error: Option<String>,
}

/// The recorder's part of `GET /listen/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    /// Recording today.
    pub enabled: bool,
    /// The day it was switched on for (possibly over).
    pub enabled_for: Option<NaiveDate>,
    /// Where the day folders go.
    pub root: String,
    /// Every configured mic.
    pub mics: Vec<MicRecording>,
}

/// A day folder's name: `YYYY-MM-DD`.
pub fn recording_day_dir(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

/// The day of a folder the recorder made; `None` for any other name (only the
/// canonical `YYYY-MM-DD` spelling counts).
pub fn parse_recording_day(name: &str) -> Option<NaiveDate> {
    let day = NaiveDate::parse_from_str(name, "%Y-%m-%d").ok()?;
    (recording_day_dir(day) == name).then_some(day)
}

/// A day folder is deleted once it is more than [`RECORDING_RETENTION_DAYS`] old.
pub fn recording_expired(day: NaiveDate, today: NaiveDate) -> bool {
    today.signed_duration_since(day).num_days() > RECORDING_RETENTION_DAYS
}

/// The file of a stretch of `mic_id`'s audio whose first frame came at
/// `started`; `attempt` 2 … 9 names a further stretch started in the same second.
pub fn recording_file_name(mic_id: &str, started: NaiveTime, attempt: u32) -> String {
    let time = started.format("%H%M%S");
    match attempt {
        0 | 1 => format!("{mic_id}_{time}{RECORDING_EXTENSION}"),
        n => format!("{mic_id}_{time}-{n}{RECORDING_EXTENSION}"),
    }
}

/// Whether `name` is a file the recorder writes: `<mic id>_<HHMMSS>[-2…9].wav`
/// (a mic id never holds `_`). Retention deletes nothing else.
pub fn is_recording_file_name(name: &str) -> bool {
    let Some((mic_id, stamp)) = name
        .strip_suffix(RECORDING_EXTENSION)
        .and_then(|stem| stem.rsplit_once('_'))
    else {
        return false;
    };
    let (time, attempt) = match stamp.split_once('-') {
        Some((time, attempt)) => (time, Some(attempt)),
        None => (stamp, None),
    };
    let attempt_ok = matches!(
        attempt,
        None | Some("2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
    );
    let time_ok = time.len() == 6
        && time.bytes().all(|b| b.is_ascii_digit())
        && NaiveTime::parse_from_str(time, "%H%M%S").is_ok();
    attempt_ok && time_ok && is_valid_mic_id(mic_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn time(text: &str) -> NaiveTime {
        text.parse().unwrap()
    }

    #[test]
    fn the_setting_is_off_by_default_and_on_only_for_its_day() {
        assert_eq!(RecorderSetting::default().enabled_for, None);
        let empty: RecorderSetting = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, RecorderSetting::default(), "an empty row = off");
        let on = RecorderSetting {
            enabled_for: Some(day("2026-10-18")),
        };
        assert_eq!(
            serde_json::to_value(on).unwrap(),
            json!({"enabledFor": "2026-10-18"})
        );
        assert!(on.records_on(day("2026-10-18")));
        assert!(
            !on.records_on(day("2026-10-19")),
            "off by itself after midnight"
        );
        assert!(!RecorderSetting::default().records_on(day("2026-10-18")));
        let toggle: RecorderToggle = serde_json::from_value(json!({"enabled": true})).unwrap();
        assert!(toggle.enabled);
    }

    #[test]
    fn the_status_wire_shape_is_exact() {
        let status = RecorderStatus {
            enabled: true,
            enabled_for: Some(day("2026-10-18")),
            root: "/opt/presenter/recordings".into(),
            mics: vec![MicRecording {
                id: "guest".into(),
                state: RecordingState::Recording,
                file: Some("/opt/presenter/recordings/2026-10-18/guest_194205.wav".into()),
                bytes: 32_000,
                dropped_frames: 0,
                error: None,
            }],
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            json!({
                "enabled": true, "enabledFor": "2026-10-18", "root": "/opt/presenter/recordings",
                "mics": [{"id": "guest", "state": "recording",
                    "file": "/opt/presenter/recordings/2026-10-18/guest_194205.wav",
                    "bytes": 32000, "droppedFrames": 0, "error": null}]
            })
        );
        for (state, wire) in [
            (RecordingState::Off, "off"),
            (RecordingState::Waiting, "waiting"),
            (RecordingState::Error, "error"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), json!(wire));
        }
        assert_eq!(RecordingState::default(), RecordingState::Off);
    }

    #[test]
    fn only_a_canonical_date_names_a_day_folder() {
        assert_eq!(recording_day_dir(day("2026-01-05")), "2026-01-05");
        assert_eq!(parse_recording_day("2026-10-18"), Some(day("2026-10-18")));
        for name in [
            "2026-1-5",
            "2026-02-30",
            "recordings",
            "2026-10-18x",
            "",
            "+2026-10-18",
            "2026-10-18 ",
        ] {
            assert_eq!(parse_recording_day(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_day_is_kept_30_days_and_deleted_on_the_31st() {
        let today = day("2026-10-15");
        assert!(
            !recording_expired(day("2026-09-15"), today),
            "30 days old: kept"
        );
        assert!(
            recording_expired(day("2026-09-14"), today),
            "31 days old: deleted"
        );
        assert!(!recording_expired(today, today));
        assert!(
            !recording_expired(day("2026-10-16"), today),
            "a folder dated in the future is never deleted"
        );
    }

    #[test]
    fn a_files_name_carries_its_mic_and_start_time() {
        assert_eq!(
            recording_file_name("guest", time("19:42:05"), 1),
            "guest_194205.wav"
        );
        assert_eq!(
            recording_file_name("host-2", time("07:05:09"), 3),
            "host-2_070509-3.wav"
        );
        for attempt in 1..=RECORDING_NAMES_PER_SECOND {
            let name = recording_file_name("guest", time("19:42:05"), attempt);
            assert!(is_recording_file_name(&name), "{name}");
        }
    }

    #[test]
    fn retention_recognises_only_the_recorders_own_file_names() {
        for name in [
            "guest.wav",
            "Guest_194205.wav",
            "../guest_194205.wav",
            "guest_194205.wav.tmp",
            "guest_194205-1.wav",
            "guest_194205-10.wav",
            "guest_246000.wav",
            "guest_19420.wav",
            "_194205.wav",
            "guest_194205.WAV",
            "notes.txt",
        ] {
            assert!(!is_recording_file_name(name), "{name:?}");
        }
    }
}
```

- [ ] **Step 3: Apply the module lines** (patch recipe):

```diff
diff --git a/crates/presenter-core/src/audio/mod.rs b/crates/presenter-core/src/audio/mod.rs
index c14ecfe..1da2b76 100644
--- a/crates/presenter-core/src/audio/mod.rs
+++ b/crates/presenter-core/src/audio/mod.rs
@@ -1,5 +1,6 @@
 //! Audio ingest building blocks (#852 C1a, spec §5): the VBAN packet codec,
-//! the level meter and the per-stream frame-counter rule. Pure, no I/O — the
+//! the level meter, the per-stream frame-counter rule and the recorder's WAV
+//! header (`wav`, C1b). Pure, no I/O — the
 //! server's `listen` module owns the socket and the resampler.
 
 pub mod level;
@@ -7,3 +8,4 @@ pub mod sequence;
 pub mod vban;
 #[cfg(any(test, feature = "test-helpers"))]
 pub mod vban_encode;
+pub mod wav;
diff --git a/crates/presenter-core/src/listen/mics.rs b/crates/presenter-core/src/listen/mics.rs
index ae5197e..8090f27 100644
--- a/crates/presenter-core/src/listen/mics.rs
+++ b/crates/presenter-core/src/listen/mics.rs
@@ -161,7 +161,7 @@ fn normalize_mic(mut mic: MicConfig) -> Result<MicConfig, MicsValidationError> {
     Ok(mic)
 }
 
-fn is_valid_mic_id(id: &str) -> bool {
+pub(crate) fn is_valid_mic_id(id: &str) -> bool {
     (1..=MIC_ID_MAX_CHARS).contains(&id.len())
         && id
             .bytes()
diff --git a/crates/presenter-core/src/listen/mod.rs b/crates/presenter-core/src/listen/mod.rs
index a0a2041..406f013 100644
--- a/crates/presenter-core/src/listen/mod.rs
+++ b/crates/presenter-core/src/listen/mod.rs
@@ -1,5 +1,7 @@
 //! Sermon listening (#852 / #850 part C): the shared config and status types.
 //! C1a: the microphones ([`mics`]) and `GET /listen/status` ([`status`]).
+//! C1b: the recorder's contract ([`recorder`]).
 
 pub mod mics;
+pub mod recorder;
 pub mod status;
```

- [ ] **Step 4: Gates** — `cargo fmt --all -- --check`; `QC_TARGETS="$(printf '%s\n' crates/presenter-core/src/audio/{mod,wav}.rs crates/presenter-core/src/listen/{mod,recorder,mics}.rs)" python3 scripts/dev/fn_length_check.py .`; `python3 scripts/dev/no_panic_check.py .`.

- [ ] **Step 5: Commit** — paths `crates/presenter-core/src/audio/mod.rs crates/presenter-core/src/audio/wav.rs crates/presenter-core/src/listen/mod.rs crates/presenter-core/src/listen/recorder.rs crates/presenter-core/src/listen/mics.rs`, message `feat(listen): the recorder's contract and the WAV header (#852 C1b-2)`.

### Task 7: Server — the recordings folder and the daily retention pass

**Files:**
- Create: `crates/presenter-server/src/listen/recorder.rs`, `crates/presenter-server/src/listen/recorder/tests.rs`, `crates/presenter-server/src/listen/recording_file.rs`
- Modify: `crates/presenter-server/src/listen/mod.rs` (modules, `recorder_env` in `Inner`, the supervisor spawned by `start`, `recorder_env()` / `#[cfg(test)] set_recorder_env()`), `crates/presenter-server/src/state/stream_assets.rs` (`dir_next_to_db`, `stream_assets_dir_from_db_url` delegates to it)

**Interfaces:**
- Produces (`listen::recorder`, `pub(crate)`): `RECORDER_TARGET = "presenter::listen::recorder"`, `RECORDINGS_DIR_ENV = "PRESENTER_LISTEN_RECORDINGS_DIR"`, `type Clock = Arc<dyn Fn() -> NaiveDateTime + Send + Sync>`, `RecorderEnv { root: PathBuf, clock: Clock }` (`from_env()`, `now()`), `recordings_root(Option<OsString>, &str) -> PathBuf`; `pub(super) async fn supervise(ListenService)` — this lane: retention once per local date on a 60 s tick (`MissedTickBehavior::Delay`, first tick after 60 s; the pass at startup runs before it).
- Produces (`listen::recording_file`): `PruneReport { deleted: Vec<(PathBuf, u64)>, kept: Vec<PathBuf>, failed: Vec<(PathBuf, String)> }`, `prune_recordings(&Path, NaiveDate) -> io::Result<PruneReport>` (a missing root is an empty report).
- Produces (`state::stream_assets`): `pub(crate) fn dir_next_to_db(db_url: &str, name: &str) -> PathBuf`.
- B6: under `cfg(test)` `default_root()` is `<temp dir>/presenter-unit-test-recordings` — a unit test that never calls `set_recorder_env` still never writes into the crate; every recorder test sets its own temp folder and settable clock.

- [ ] **Step 1: Create `crates/presenter-server/src/listen/recording_file.rs`:**

```rust
//! The recorder's files on disk (#852 C1b, spec §5.5, §12) — blocking
//! `std::fs`, run in `spawn_blocking` only.
//!
//! Retention deletes only what the recorder makes: day folders named
//! `YYYY-MM-DD` (canonical spelling) and in them the `<mic id>_<HHMMSS>[-n].wav`
//! regular files; a folder is removed only when that emptied it. Entries are
//! judged by `DirEntry::file_type`, which never follows a symlink, so a linked
//! folder or file is never entered or deleted.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use presenter_core::listen::recorder::{
    is_recording_file_name, parse_recording_day, recording_expired,
};

/// What a retention pass did.
#[derive(Debug, Default)]
pub(crate) struct PruneReport {
    /// Each deleted file and its size.
    pub(crate) deleted: Vec<(PathBuf, u64)>,
    /// An expired folder left in place: it still holds something the recorder
    /// did not write.
    pub(crate) kept: Vec<PathBuf>,
    /// An expired folder that could not be cleaned, and why.
    pub(crate) failed: Vec<(PathBuf, String)>,
}

/// Delete the day folders under `root` older than the retention: their
/// recordings, then the folder when that emptied it. A missing root is nothing
/// to do; one folder's failure never stops the others.
pub(crate) fn prune_recordings(root: &Path, today: NaiveDate) -> io::Result<PruneReport> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(PruneReport::default()),
        Err(err) => return Err(err),
    };
    let mut report = PruneReport::default();
    for entry in entries {
        let entry = entry?;
        let expired = entry.file_type()?.is_dir()
            && entry
                .file_name()
                .to_str()
                .and_then(parse_recording_day)
                .is_some_and(|day| recording_expired(day, today));
        if expired {
            let dir = entry.path();
            if let Err(err) = prune_day(&dir, &mut report) {
                report.failed.push((dir, err.to_string()));
            }
        }
    }
    Ok(report)
}

/// One expired day folder: its recordings, then the folder if nothing else is in it.
fn prune_day(dir: &Path, report: &mut PruneReport) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let recording = entry.file_type()?.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(is_recording_file_name);
        if recording {
            let bytes = entry.metadata()?.len();
            fs::remove_file(entry.path())?;
            report.deleted.push((entry.path(), bytes));
        }
    }
    match fs::remove_dir(dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => {
            report.kept.push(dir.to_path_buf());
            Ok(())
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    /// `root/<folder>/<file>` with `len` bytes.
    fn put(root: &Path, folder: &str, file: &str, len: usize) -> PathBuf {
        let dir = root.join(folder);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(file);
        fs::write(&path, vec![0u8; len]).unwrap();
        path
    }

    #[test]
    fn old_day_folders_go_file_by_file_and_the_30th_day_stays() {
        let root = tempfile::tempdir().unwrap();
        let guest = put(root.path(), "2026-09-14", "guest_194205.wav", 100);
        let interp = put(root.path(), "2026-09-14", "interp_194206-2.wav", 300);
        let kept = put(root.path(), "2026-09-15", "guest_100000.wav", 50);
        let report = prune_recordings(root.path(), day("2026-10-15")).unwrap();
        let mut deleted = report.deleted.clone();
        deleted.sort();
        assert_eq!(deleted, vec![(guest, 100), (interp, 300)]);
        assert!(
            !root.path().join("2026-09-14").exists(),
            "the emptied folder goes"
        );
        assert!(kept.exists(), "30 days old: kept");
        assert!(
            report.kept.is_empty() && report.failed.is_empty(),
            "{report:?}"
        );
    }

    #[test]
    fn a_folder_with_other_files_or_another_name_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let ours = put(root.path(), "2026-09-01", "guest_194205.wav", 10);
        let notes = put(root.path(), "2026-09-01", "notes.txt", 10);
        let renamed = put(root.path(), "2026-09-01", "guest.wav", 10);
        let elsewhere = put(root.path(), "old-stuff", "guest_194205.wav", 10);
        let loose = put(root.path(), ".", "2026-09-02", 10);
        let report = prune_recordings(root.path(), day("2026-10-15")).unwrap();
        assert!(!ours.exists());
        assert!(
            notes.exists() && renamed.exists(),
            "not the recorder's files"
        );
        assert_eq!(report.kept, vec![root.path().join("2026-09-01")]);
        assert!(elsewhere.exists(), "a folder not named like a day");
        assert!(loose.exists(), "a FILE named like a day");
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_folder_is_never_followed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = put(outside.path(), "2026-09-01", "guest_194205.wav", 10);
        std::os::unix::fs::symlink(
            outside.path().join("2026-09-01"),
            root.path().join("2026-09-01"),
        )
        .unwrap();
        let linked_file = put(root.path(), "2026-09-02", "notes.txt", 1);
        std::os::unix::fs::symlink(&target, linked_file.with_file_name("guest_100000.wav"))
            .unwrap();
        let report = prune_recordings(root.path(), day("2026-10-15")).unwrap();
        assert!(target.exists(), "nothing behind a symlink is deleted");
        assert!(report.deleted.is_empty(), "{report:?}");
        assert!(
            root.path()
                .join("2026-09-02")
                .join("guest_100000.wav")
                .symlink_metadata()
                .is_ok(),
            "a link named like a recording is not a regular file: left"
        );
    }

    #[test]
    fn a_missing_root_is_nothing_to_do() {
        let root = tempfile::tempdir().unwrap();
        let report = prune_recordings(&root.path().join("never-made"), day("2026-10-15")).unwrap();
        assert!(report.deleted.is_empty() && report.kept.is_empty() && report.failed.is_empty());
    }
}
```

- [ ] **Step 2: Create `crates/presenter-server/src/listen/recorder.rs`:**

```rust
//! The recorder (#852 C1b, spec §5.5, §12). C1b-2: its folder and its
//! retention — the day folders older than 30 days are deleted once per local
//! date (at startup, then in the first minute after each midnight), so a
//! recording can never outlive 30 days. C1b-4 adds the writers.
//!
//! The folder: `PRESENTER_LISTEN_RECORDINGS_DIR`, else `recordings` next to the
//! sqlite file of `PRESENTER_DB_URL` (prod / PP `/opt/presenter/recordings`, the
//! spec's path; dev `/opt/presenter-dev/recordings`; both inside the units'
//! `ReadWritePaths`), else `./recordings`. One supervisor task, started with the
//! socket task (never in validate mode).

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime};
use presenter_core::listen::recorder::RECORDING_RETENTION_DAYS;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{info, warn};

use super::recording_file::{prune_recordings, PruneReport};
use super::ListenService;

/// The target of every recorder log line.
pub(crate) const RECORDER_TARGET: &str = "presenter::listen::recorder";
/// Overrides the recordings folder (the E2E spec and tests use temp dirs).
pub(crate) const RECORDINGS_DIR_ENV: &str = "PRESENTER_LISTEN_RECORDINGS_DIR";
const RECORDINGS_DIR_NAME: &str = "recordings";
/// The supervisor looks at the date this often.
const RECONCILE_EVERY: Duration = Duration::from_secs(60);

/// The server's local wall time (tests inject a settable one).
pub(crate) type Clock = Arc<dyn Fn() -> NaiveDateTime + Send + Sync>;

/// Where the recorder writes and what time it is.
#[derive(Clone)]
pub(crate) struct RecorderEnv {
    pub(crate) root: PathBuf,
    pub(crate) clock: Clock,
}

impl RecorderEnv {
    /// The folder from the environment and the local clock. A unit test's
    /// default folder is a temp dir, never `./recordings` inside the crate.
    pub(crate) fn from_env() -> Self {
        Self {
            root: default_root(),
            clock: Arc::new(|| chrono::Local::now().naive_local()),
        }
    }

    /// The local wall time now.
    pub(crate) fn now(&self) -> NaiveDateTime {
        (*self.clock)()
    }
}

#[cfg(not(test))]
fn default_root() -> PathBuf {
    recordings_root(
        std::env::var_os(RECORDINGS_DIR_ENV),
        &std::env::var("PRESENTER_DB_URL").unwrap_or_default(),
    )
}

/// A unit test that never sets its own folder (`set_recorder_env`) still never
/// writes into the crate.
#[cfg(test)]
fn default_root() -> PathBuf {
    std::env::temp_dir().join("presenter-unit-test-recordings")
}

/// The recordings folder: `explicit` (`PRESENTER_LISTEN_RECORDINGS_DIR`) when
/// set, else `recordings` next to the sqlite file of `db_url`.
pub(crate) fn recordings_root(explicit: Option<OsString>, db_url: &str) -> PathBuf {
    match explicit.filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => crate::state::stream_assets::dir_next_to_db(db_url, RECORDINGS_DIR_NAME),
    }
}

/// The supervisor: the retention pass once per local date.
pub(super) async fn supervise(service: ListenService) {
    let mut pruned_on: Option<NaiveDate> = None;
    let mut tick = interval(RECONCILE_EVERY);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        prune_if_new_day(&service.recorder_env(), &mut pruned_on).await;
    }
}

/// Retention once per local date: at startup, then after each midnight. A
/// failed pass WARNs and is tried again the next day.
async fn prune_if_new_day(env: &RecorderEnv, pruned_on: &mut Option<NaiveDate>) {
    let today = env.now().date();
    if *pruned_on == Some(today) {
        return;
    }
    *pruned_on = Some(today);
    let root = env.root.clone();
    let shown = root.display().to_string();
    match tokio::task::spawn_blocking(move || prune_recordings(&root, today)).await {
        Ok(Ok(report)) => log_prune(&report),
        Ok(Err(err)) => warn!(
            target: RECORDER_TARGET,
            root = %shown,
            %err,
            "recordings retention failed — next try tomorrow"
        ),
        Err(err) => warn!(
            target: RECORDER_TARGET,
            root = %shown,
            %err,
            "recordings retention was interrupted — next try tomorrow"
        ),
    }
}

/// One INFO per deleted file and per folder kept, a WARN per folder that failed.
fn log_prune(report: &PruneReport) {
    for (file, bytes) in &report.deleted {
        info!(
            target: RECORDER_TARGET,
            file = %file.display(),
            bytes,
            retention_days = RECORDING_RETENTION_DAYS,
            "recording deleted (retention)"
        );
    }
    for dir in &report.kept {
        info!(
            target: RECORDER_TARGET,
            dir = %dir.display(),
            "expired recordings folder kept — it holds files the recorder did not write"
        );
    }
    for (dir, error) in &report.failed {
        warn!(
            target: RECORDER_TARGET,
            dir = %dir.display(),
            error = error.as_str(),
            "an expired recordings folder could not be cleaned — next try tomorrow"
        );
    }
}

#[cfg(test)]
mod tests;
```

- [ ] **Step 3: Create `crates/presenter-server/src/listen/recorder/tests.rs`:**

```rust
//! #852 C1b-2: the recordings folder and the daily retention pass. Folders are
//! temp dirs; the clock is a settable local time.

use super::*;
use crate::tracing_capture::Capture;
use std::path::Path;
use std::sync::Mutex;

fn at(text: &str) -> NaiveDateTime {
    text.parse().unwrap()
}

/// A settable local wall time and the handle that moves it.
fn clock(start: &str) -> (Clock, Arc<Mutex<NaiveDateTime>>) {
    let now = Arc::new(Mutex::new(at(start)));
    let read = Arc::clone(&now);
    let clock: Clock = Arc::new(move || *read.lock().unwrap());
    (clock, now)
}

/// `root/<folder>/<file>` with `len` bytes.
fn put(root: &Path, folder: &str, file: &str, len: usize) -> PathBuf {
    let dir = root.join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(file);
    std::fs::write(&path, vec![0u8; len]).unwrap();
    path
}

#[test]
fn the_recordings_folder_is_the_env_value_or_next_to_the_database() {
    assert_eq!(RECORDINGS_DIR_ENV, "PRESENTER_LISTEN_RECORDINGS_DIR");
    let prod = "sqlite:///opt/presenter/presenter.db";
    assert_eq!(
        recordings_root(Some("/srv/rec".into()), prod),
        PathBuf::from("/srv/rec")
    );
    assert_eq!(
        recordings_root(Some(OsString::new()), prod),
        PathBuf::from("/opt/presenter/recordings"),
        "an empty override is no override"
    );
    assert_eq!(
        recordings_root(None, "sqlite:///opt/presenter-dev/presenter.db?mode=rwc"),
        PathBuf::from("/opt/presenter-dev/recordings")
    );
    assert_eq!(
        recordings_root(None, "sqlite::memory:"),
        PathBuf::from("./recordings")
    );
    assert_eq!(recordings_root(None, ""), PathBuf::from("./recordings"));
    assert!(
        RecorderEnv::from_env()
            .root
            .starts_with(std::env::temp_dir()),
        "a unit test's default folder is never inside the crate"
    );
}

#[tokio::test]
async fn retention_runs_once_per_local_date() {
    let root = tempfile::tempdir().unwrap();
    let (clock, now) = clock("2026-10-15T10:00:00");
    let env = RecorderEnv {
        root: root.path().to_path_buf(),
        clock,
    };
    let expired = put(root.path(), "2026-09-14", "guest_194205.wav", 10);
    let mut pruned_on = None;
    prune_if_new_day(&env, &mut pruned_on).await;
    assert!(!expired.exists(), "the first pass of the day deletes it");
    let later_today = put(root.path(), "2026-09-01", "guest_194205.wav", 10);
    let thirtieth = put(root.path(), "2026-09-15", "guest_194205.wav", 10);
    prune_if_new_day(&env, &mut pruned_on).await;
    assert!(later_today.exists(), "one pass per local date");
    *now.lock().unwrap() = at("2026-10-16T00:00:30");
    prune_if_new_day(&env, &mut pruned_on).await;
    assert!(
        !later_today.exists() && !thirtieth.exists(),
        "the next day's pass"
    );
}

#[tokio::test]
async fn a_started_service_prunes_at_once_and_logs_each_file() {
    let capture = Capture::install();
    let root = tempfile::tempdir().unwrap();
    let (clock, _) = clock("2026-10-15T10:00:00");
    let expired = put(root.path(), "2026-09-01", "guest_194205.wav", 1_234);
    let young = put(root.path(), "2026-10-14", "guest_194205.wav", 10);
    let service = ListenService::new();
    service.set_recorder_env(RecorderEnv {
        root: root.path().to_path_buf(),
        clock,
    });
    assert!(service.start("127.0.0.1:0".parse().unwrap()));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while expired.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no retention pass at startup"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(young.exists());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let lines = loop {
        let lines = capture.events_with_message("recording deleted (retention)");
        if !lines.is_empty() || tokio::time::Instant::now() >= deadline {
            break lines;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].field("bytes"), Some("1234"));
}
```

- [ ] **Step 4: Apply** (patch recipe):

```diff
diff --git a/crates/presenter-server/src/listen/mod.rs b/crates/presenter-server/src/listen/mod.rs
index d954e78..7044406 100644
--- a/crates/presenter-server/src/listen/mod.rs
+++ b/crates/presenter-server/src/listen/mod.rs
@@ -5,10 +5,14 @@
 //! start-once flag and the save lock. Started from `spawn_background_tasks`
 //! only, so validate mode never binds. Each mic has a chain (counter rule →
 //! 16 kHz → level, `mic_chain`); no frame channel yet: it arrives with its
-//! first consumer (C1b recorder / C3 ASR).
+//! first consumer (C1b recorder / C3 ASR). The recorder (`recorder`,
+//! `recording_file`, #852 C1b) has its own supervisor task, started with the
+//! socket task; it deletes day folders older than 30 days once per local date.
 
 pub(crate) mod health;
 mod mic_chain;
+pub(crate) mod recorder;
+mod recording_file;
 mod resample;
 #[cfg(test)]
 mod tests;
@@ -26,6 +30,7 @@ use tokio::sync::watch;
 use tokio::time::Instant;
 
 use self::health::ListenHealth;
+use self::recorder::RecorderEnv;
 
 /// The target of every receiver log line (spec §14 `presenter::listen::mic`).
 pub(crate) const MIC_TARGET: &str = "presenter::listen::mic";
@@ -49,6 +54,8 @@ struct Inner {
     health: Mutex<ListenHealth>,
     started: AtomicBool,
     save_lock: tokio::sync::Mutex<()>,
+    /// Where the recorder writes and what time it is (#852 C1b).
+    recorder_env: Mutex<RecorderEnv>,
 }
 
 impl ListenService {
@@ -60,6 +67,7 @@ impl ListenService {
                 health: Mutex::new(ListenHealth::default()),
                 started: AtomicBool::new(false),
                 save_lock: tokio::sync::Mutex::new(()),
+                recorder_env: Mutex::new(RecorderEnv::from_env()),
             }),
         }
     }
@@ -80,13 +88,15 @@ impl ListenService {
         self.with_health(|health| health.snapshot(&mics, Instant::now()))
     }
 
-    /// Start the socket task once; `false` when it was already started.
+    /// Start the socket task and the recorder's supervisor once; `false` when
+    /// they were already started.
     pub(crate) fn start(&self, bind: SocketAddr) -> bool {
         if self.inner.started.swap(true, Ordering::SeqCst) {
             return false;
         }
         self.with_health(|health| health.socket.bind_addr = Some(bind));
         tokio::spawn(vban_rx::supervise(self.clone(), bind));
+        tokio::spawn(recorder::supervise(self.clone()));
         true
     }
 
@@ -108,6 +118,25 @@ impl ListenService {
     fn config_receiver(&self) -> watch::Receiver<Arc<Vec<MicConfig>>> {
         self.inner.config.subscribe()
     }
+
+    /// The recorder's folder and clock.
+    fn recorder_env(&self) -> RecorderEnv {
+        self.inner
+            .recorder_env
+            .lock()
+            .unwrap_or_else(PoisonError::into_inner)
+            .clone()
+    }
+
+    /// Point the recorder at a test folder and a test clock.
+    #[cfg(test)]
+    pub(crate) fn set_recorder_env(&self, env: RecorderEnv) {
+        *self
+            .inner
+            .recorder_env
+            .lock()
+            .unwrap_or_else(PoisonError::into_inner) = env;
+    }
 }
 
 /// Where the VBAN socket binds: `PRESENTER_LISTEN_VBAN_ADDR`, else 0.0.0.0:6980.
diff --git a/crates/presenter-server/src/state/stream_assets.rs b/crates/presenter-server/src/state/stream_assets.rs
index c7e8f83..36213b0 100644
--- a/crates/presenter-server/src/state/stream_assets.rs
+++ b/crates/presenter-server/src/state/stream_assets.rs
@@ -375,13 +375,20 @@ pub(crate) fn resolve_dir() -> PathBuf {
 /// `db_url`. An in-memory / empty / relative-filename url falls back to
 /// `./stream-assets`.
 pub(crate) fn stream_assets_dir_from_db_url(db_url: &str) -> PathBuf {
+    dir_next_to_db(db_url, "stream-assets")
+}
+
+/// Pure: the folder `name` next to the sqlite file named in `db_url` (#852 C1b:
+/// the listening recordings go there too); `./<name>` for an in-memory / empty /
+/// relative-filename url.
+pub(crate) fn dir_next_to_db(db_url: &str, name: &str) -> PathBuf {
     let parent = sqlite_file_path(db_url)
         .as_deref()
         .and_then(Path::parent)
         .filter(|p| !p.as_os_str().is_empty())
         .map(Path::to_path_buf)
         .unwrap_or_else(|| PathBuf::from("."));
-    parent.join("stream-assets")
+    parent.join(name)
 }
 
 fn sqlite_file_path(db_url: &str) -> Option<PathBuf> {
```

- [ ] **Step 5: Gates** — `cargo fmt --all -- --check`; `QC_TARGETS="$(printf '%s\n' crates/presenter-server/src/listen/{mod,recorder,recording_file}.rs crates/presenter-server/src/state/stream_assets.rs)" python3 scripts/dev/fn_length_check.py .`; `python3 scripts/dev/no_panic_check.py .`; `grep -rn 'stream_assets_dir_from_db_url\|dir_next_to_db' crates/presenter-server/src` (the old function keeps its callers; the new one has two).

- [ ] **Step 6: Commit** — paths `crates/presenter-server/src/listen/mod.rs crates/presenter-server/src/listen/recorder.rs crates/presenter-server/src/listen/recorder/tests.rs crates/presenter-server/src/listen/recording_file.rs crates/presenter-server/src/state/stream_assets.rs`, message `feat(listen): the recordings folder and the 30-day retention pass (#852 C1b-2)`.

### Task 8: The frame channel — every current chain hands its 16 kHz frames on

**Files:**
- Modify: `crates/presenter-server/src/listen/mod.rs` (`MicFrame`, `FRAME_CHANNEL_CAPACITY`, `frames` in `Inner`, `frame_sender()`), `crates/presenter-server/src/listen/mic_chain.rs` (`run_chain` sends; `send_frames`; a test), `crates/presenter-server/src/listen/health.rs` (`record_chain` returns whether the generation was current)

**Interfaces:**
- Produces: `pub(crate) type MicFrame = Arc<[i16]>` (1600 samples), `pub(crate) const FRAME_CHANNEL_CAPACITY: usize = 128` (tokio rounds a broadcast capacity up to a power of two — 128 is one, so the 12.8 s is exact); `ListenService::frame_sender(&str) -> broadcast::Sender<MicFrame>` (private; created on first use, kept for the process — every chain of a mic sends into the same channel); `ListenHealth::record_chain(..) -> bool`.
- Consumed by: lane 4 (`subscribe_frames`), later C3 (ASR).

- [ ] **Step 1: Apply** (patch recipe):

```diff
diff --git a/crates/presenter-server/src/listen/health.rs b/crates/presenter-server/src/listen/health.rs
index 0c1aabd..54a551d 100644
--- a/crates/presenter-server/src/listen/health.rs
+++ b/crates/presenter-server/src/listen/health.rs
@@ -336,6 +336,7 @@ impl ListenHealth {
     }
 
     /// A chain's output; ignored unless `generation` is the mic's current one.
+    /// Returns whether it was (only a current chain hands its frames on).
     pub(crate) fn record_chain(
         &mut self,
         mic_id: &str,
@@ -343,19 +344,20 @@ impl ListenHealth {
         frames: u64,
         level_dbfs: Option<f32>,
         error: Option<&str>,
-    ) {
+    ) -> bool {
         let Some(mic) = self
             .mics
             .get_mut(mic_id)
             .filter(|mic| mic.generation == generation)
         else {
-            return;
+            return false;
         };
         mic.frames_out += frames;
         if level_dbfs.is_some() {
             mic.level_dbfs = level_dbfs;
         }
         mic.error = error.map(str::to_string);
+        true
     }
 
     /// A format the mic's chain switched to; the mic's formats this session
diff --git a/crates/presenter-server/src/listen/mic_chain.rs b/crates/presenter-server/src/listen/mic_chain.rs
index e0e8270..5c87663 100644
--- a/crates/presenter-server/src/listen/mic_chain.rs
+++ b/crates/presenter-server/src/listen/mic_chain.rs
@@ -7,8 +7,9 @@
 //! last one's level; it spends a unit of its coop budget per block, so a full
 //! queue never pins a worker. A format change rebuilds the converter; its log
 //! line goes through the #484 gate on the mic's format count (health). The
-//! frames themselves reach no consumer yet: the channel arrives with C1b's
-//! recorder / C3's ASR.
+//! frames go to the mic's frame channel (`ListenService::frame_sender`) only
+//! while the chain's generation is the mic's current one; `broadcast::send`
+//! never waits — a consumer that falls behind loses the oldest frames.
 
 use std::collections::VecDeque;
 use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
@@ -16,11 +17,11 @@ use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
 use presenter_core::audio::level::rms_dbfs;
 use presenter_core::audio::vban::VbanAudioFormat;
 use presenter_core::listen::mics::MicChannel;
-use tokio::sync::Notify;
+use tokio::sync::{broadcast, Notify};
 use tracing::{info, warn};
 
 use super::resample::Resampler16k;
-use super::{should_log_event, ListenService, MIC_TARGET};
+use super::{should_log_event, ListenService, MicFrame, MIC_TARGET};
 
 /// ≈ 340 ms of 256-frame packets at 48 kHz — far above the ≥ 6 × 256
 /// samples a Voicemeeter receiver keeps (songplayer `vban-out.md`).
@@ -193,6 +194,7 @@ pub(crate) async fn run_chain(
     generation: u64,
     queue: Arc<BlockQueue>,
 ) {
+    let frames_out = service.frame_sender(&mic_id);
     let mut chain = MicChain::new(channel);
     while let Some(block) = queue.pop().await {
         let before = chain.format();
@@ -205,9 +207,12 @@ pub(crate) async fn run_chain(
         }
         let level = frames.last().map(Vec::as_slice).map(rms_dbfs);
         let produced = frames.len() as u64;
-        service.with_health(|health| {
-            health.record_chain(&mic_id, generation, produced, level, chain.error());
+        let current = service.with_health(|health| {
+            health.record_chain(&mic_id, generation, produced, level, chain.error())
         });
+        if current {
+            send_frames(&frames_out, frames);
+        }
         // `pop` touches no tokio resource: while blocks are queued (a sender
         // flooding counter jumps queues a second of silence per packet) the
         // task must still give its worker back once its coop budget is spent.
@@ -215,6 +220,17 @@ pub(crate) async fn run_chain(
     }
 }
 
+/// The frames to every consumer of the mic. Never waits: `broadcast::send`
+/// overwrites the oldest frame a consumer has not read yet.
+fn send_frames(frames_out: &broadcast::Sender<MicFrame>, frames: Vec<Vec<i16>>) {
+    if frames_out.receiver_count() == 0 {
+        return;
+    }
+    for frame in frames {
+        let _receivers = frames_out.send(MicFrame::from(frame));
+    }
+}
+
 /// The converter line of a format the chain switched to; `formats` (the
 /// mic's formats this session, its first one included) passed the #484 gate.
 fn log_format(
@@ -251,7 +267,7 @@ mod tests {
     use crate::listen::health::PacketSeen;
     use crate::tracing_capture::Capture;
     use presenter_core::audio::sequence::SequenceVerdict;
-    use presenter_core::audio::vban::VbanSampleType;
+    use presenter_core::audio::vban::{f32_to_int16, VbanSampleType};
     use presenter_core::audio::vban_encode::tone;
     use presenter_core::listen::mics::{MicConfig, MicLanguage, MicRole};
     use std::time::Duration;
@@ -519,6 +535,41 @@ mod tests {
         );
     }
 
+    #[tokio::test]
+    async fn the_current_chain_hands_its_frames_on_and_a_stale_chain_does_not() {
+        let service = ListenService::new();
+        let stale = service.with_health(|health| health.set_mics(&[], &["guest"]));
+        let current = service.with_health(|health| health.set_mics(&[], &["guest"]));
+        let mut frames = service.frame_sender("guest").subscribe();
+        for generation in [stale, current] {
+            let queue = Arc::new(BlockQueue::new(QUEUE_CAPACITY));
+            // 16 kHz passes through: one block of 1600 samples = one 100 ms frame
+            queue.push(block(MONO_16K, 0, vec![0.25; 1_600]));
+            queue.close();
+            let chain = tokio::spawn(run_chain(
+                service.clone(),
+                "guest".to_string(),
+                MicChannel::Left,
+                generation,
+                Arc::clone(&queue),
+            ));
+            tokio::time::timeout(Duration::from_secs(5), chain)
+                .await
+                .unwrap()
+                .unwrap();
+        }
+        let frame = frames.try_recv().unwrap();
+        assert_eq!(frame.len(), 1_600);
+        assert!(
+            frame.iter().all(|&s| s == f32_to_int16(0.25)),
+            "the current chain's frame"
+        );
+        assert!(
+            frames.try_recv().is_err(),
+            "exactly one frame: the stale chain sent none"
+        );
+    }
+
     fn guest() -> MicConfig {
         MicConfig {
             id: "guest".into(),
diff --git a/crates/presenter-server/src/listen/mod.rs b/crates/presenter-server/src/listen/mod.rs
index 7044406..0af9832 100644
--- a/crates/presenter-server/src/listen/mod.rs
+++ b/crates/presenter-server/src/listen/mod.rs
@@ -4,9 +4,10 @@
 //! (a `watch`), the health (a std mutex, never held across an `.await`), the
 //! start-once flag and the save lock. Started from `spawn_background_tasks`
 //! only, so validate mode never binds. Each mic has a chain (counter rule →
-//! 16 kHz → level, `mic_chain`); no frame channel yet: it arrives with its
-//! first consumer (C1b recorder / C3 ASR). The recorder (`recorder`,
-//! `recording_file`, #852 C1b) has its own supervisor task, started with the
+//! 16 kHz → level, `mic_chain`) that hands its frames to the mic's frame
+//! channel (#852 C1b): one `broadcast` per mic id that never makes the chain
+//! wait, read by the recorder (C1b) and later the ASR (C3). The recorder
+//! (`recorder`, `recording_file`) has its own supervisor task, started with the
 //! socket task; it deletes day folders older than 30 days once per local date.
 
 pub(crate) mod health;
@@ -18,6 +19,7 @@ mod resample;
 mod tests;
 mod vban_rx;
 
+use std::collections::HashMap;
 use std::net::{IpAddr, Ipv4Addr, SocketAddr};
 use std::sync::atomic::{AtomicBool, Ordering};
 use std::sync::{Arc, Mutex, PoisonError};
@@ -26,7 +28,7 @@ use std::time::Duration;
 use presenter_core::audio::vban::VBAN_DEFAULT_PORT;
 use presenter_core::listen::mics::MicConfig;
 use presenter_core::listen::status::ListenStatus;
-use tokio::sync::watch;
+use tokio::sync::{broadcast, watch};
 use tokio::time::Instant;
 
 use self::health::ListenHealth;
@@ -42,6 +44,13 @@ pub(crate) const DEFAULT_VBAN_ADDR: SocketAddr =
 /// A mic whose last packet is this old has no audio ("žiadny zvuk" within 3 s, C-S1).
 pub(crate) const NO_AUDIO_AFTER: Duration = Duration::from_secs(3);
 const BIND_RETRY_CAP: Duration = Duration::from_secs(60);
+/// One 100 ms frame of a mic: 1600 samples of 16 kHz mono s16 — exactly what
+/// the ASR gets (spec §5.4). Shared: every consumer reads the same buffer.
+pub(crate) type MicFrame = Arc<[i16]>;
+/// A consumer that falls this many frames (12.8 s) behind loses the oldest ones
+/// and is told how many (`RecvError::Lagged`): no consumer can slow a chain.
+/// tokio rounds a broadcast capacity up to a power of two, so this is one.
+pub(crate) const FRAME_CHANNEL_CAPACITY: usize = 128;
 
 /// The listen state every `AppState` clone shares.
 #[derive(Clone)]
@@ -54,6 +63,10 @@ struct Inner {
     health: Mutex<ListenHealth>,
     started: AtomicBool,
     save_lock: tokio::sync::Mutex<()>,
+    /// One frame channel per mic id, kept for the life of the process: every
+    /// chain of a mic (a save that changes its stream starts a new one) sends
+    /// into the same channel, so a consumer never re-subscribes (#852 C1b).
+    frames: Mutex<HashMap<String, broadcast::Sender<MicFrame>>>,
     /// Where the recorder writes and what time it is (#852 C1b).
     recorder_env: Mutex<RecorderEnv>,
 }
@@ -67,6 +80,7 @@ impl ListenService {
                 health: Mutex::new(ListenHealth::default()),
                 started: AtomicBool::new(false),
                 save_lock: tokio::sync::Mutex::new(()),
+                frames: Mutex::new(HashMap::new()),
                 recorder_env: Mutex::new(RecorderEnv::from_env()),
             }),
         }
@@ -119,6 +133,19 @@ impl ListenService {
         self.inner.config.subscribe()
     }
 
+    /// The mic's frame channel, created on first use.
+    fn frame_sender(&self, mic_id: &str) -> broadcast::Sender<MicFrame> {
+        let mut frames = self
+            .inner
+            .frames
+            .lock()
+            .unwrap_or_else(PoisonError::into_inner);
+        frames
+            .entry(mic_id.to_string())
+            .or_insert_with(|| broadcast::Sender::new(FRAME_CHANNEL_CAPACITY))
+            .clone()
+    }
+
     /// The recorder's folder and clock.
     fn recorder_env(&self) -> RecorderEnv {
         self.inner
```

`the_current_chain_hands_its_frames_on_and_a_stale_chain_does_not` runs two chains of the same mic — one from an earlier `set_mics` generation, one current — and receives exactly one frame (the current chain's, 1600 samples of `f32_to_int16(0.25)`). The existing chain tests are unchanged: `record_chain`'s new return value is ignored by them (a `bool` is not `#[must_use]`).

- [ ] **Step 2: Gates** — `cargo fmt --all -- --check`; `QC_TARGETS="$(printf '%s\n' crates/presenter-server/src/listen/{mod,mic_chain,health}.rs)" python3 scripts/dev/fn_length_check.py .`; `grep -rn 'record_chain(' crates/presenter-server/src` (every caller still compiles: the chain uses the result, tests ignore it).

- [ ] **Step 3: Commit** — paths `crates/presenter-server/src/listen/mod.rs crates/presenter-server/src/listen/mic_chain.rs crates/presenter-server/src/listen/health.rs`, message `feat(listen): one never-waiting frame channel per microphone (#852 C1b-2)`.

### Task 9: Docs, playbook, then close lane C1b-2

**Files:**
- Modify: `.claude/rules/listen-vban.md`, `docs/configuration.md`, `CLAUDE.md` (env table row, router row "Listening ingest")

- [ ] **Step 1: Apply the docs** (patch recipe):

```diff
diff --git a/.claude/rules/listen-vban.md b/.claude/rules/listen-vban.md
index 0f706d4..c13faea 100644
--- a/.claude/rules/listen-vban.md
+++ b/.claude/rules/listen-vban.md
@@ -67,8 +67,15 @@ paths:
   rubato `=5.0.1` `Fft` + `FixedSync::Both` with **20 ms** chunks (delay 160 samples = 10 ms; a
   whole-100-ms FFT would add 50 ms, over spec §8.5); 16 kHz passes through, 11 025 Hz is refused
   (`error`, mic state `error`); a format change starts a fresh converter. Frames = 1600 s16 samples,
-  counted (`framesOut`) with the last one's level (`levelDbfs`); **no frame channel until its first
-  consumer** (C1b recorder / C3 ASR) adds it.
+  counted (`framesOut`) with the last one's level (`levelDbfs`).
+- **Frame channel** (#852 C1b-2, `ListenService::frame_sender`): ONE `broadcast::Sender<MicFrame>`
+  (`Arc<[i16]>`, 1600 samples) per mic id, created on first use and kept for the life of the process —
+  every chain of the mic (a save that changes its stream starts a new chain) sends into the same
+  channel, so a consumer never re-subscribes. A chain sends only when `record_chain` returned `true`
+  (its generation is current) and only while someone listens (`receiver_count`). Capacity
+  `FRAME_CHANNEL_CAPACITY` = 128 frames (12.8 s; tokio rounds a capacity up to a power of two, so keep
+  one): `send` never waits; a consumer that falls behind gets `Lagged(n)` and counts it. Never make a
+  chain wait for a consumer (no `mpsc::send().await`, no lock held by a consumer).
 - **rubato's lock footprint:** 13 packages, byte-identical to songplayer's lock — incl.
   `audio-codec-algorithms` (a dependency of `audioadapter-sample`, 0BSD OR Apache-2.0). Resolve a
   dependency change with `cargo metadata` (non-compiling) and diff `Cargo.lock`: only `+` lines, no
@@ -109,6 +116,21 @@ paths:
   1 kHz half-scale tone, pulse peak at 4160, 899 crossings) come from a Python mirror of rubato's
   `FftResampler`; if CI measures outside a window, compare `output_delay()` with the measurement before
   touching the test — never widen it.
+- **Recorder — contract and retention** (#852 C1b-2; core `listen/recorder.rs` + `audio/wav.rs`,
+  server `listen/recorder.rs` + `recording_file.rs`): files `<root>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav`
+  (`-2` … `-9` within one second, `RECORDING_NAMES_PER_SECOND`), canonical 44-byte PCM header for
+  16 kHz mono s16 (byte-checked against Python's `wave`; `wav_riff_size` saturates, no 4 GiB guard: a
+  mic's day is ≤ 2.76 GB). Root = `PRESENTER_LISTEN_RECORDINGS_DIR`, else `recordings` next to the
+  sqlite file (`state::stream_assets::dir_next_to_db`: prod / PP `/opt/presenter/recordings`, dev
+  `/opt/presenter-dev/recordings`, inside the units' `ReadWritePaths`); under `cfg(test)` the default
+  is a temp dir, never `./recordings` in the crate — tests set their own folder and a settable clock
+  with `ListenService::set_recorder_env`. ONE supervisor task, spawned by `ListenService::start` with
+  the socket task (never in validate mode), ticks every 60 s even without a mic (a stated deviation
+  from C-S8: on SNV / PP it only runs the daily retention). Retention once per local date (startup,
+  then the first tick after midnight): a canonical `YYYY-MM-DD` folder whose day is more than 30 days
+  ago (`recording_expired`), only `is_recording_file_name` regular files (`DirEntry::file_type` never
+  follows a link; a linked folder is skipped), the folder only once empty; one INFO per deleted file,
+  an INFO for a folder kept, a WARN for one that failed. All I/O in `spawn_blocking`.
 - **Routing day (owner action, spec §20, after C1b):** the owner routes `GUEST_EN` / `INTERP_SK`
   (48 kHz, 16-bit, after the channel mute) to 10.77.9.205:6980; then the two mics are saved (the C1b
   card, or `PUT /listen/mics` with the spec §5.6 list) and the box acceptance runs (spec §5.8): status
diff --git a/CLAUDE.md b/CLAUDE.md
index 97249c8..f37a2bc 100644
--- a/CLAUDE.md
+++ b/CLAUDE.md
@@ -60,7 +60,7 @@ This file provides guidance to Claude Code (claude.ai/code) when working with co
 | Remote Bible translation NLT (#826): `eng-nlt` fetched on demand from Tyndale's NLT API (`api.nlt.to`), never stored/bulk-copied/committed; ONE dispatch in `state/bible_source.rs` (never call the repository's Bible reads elsewhere); books/chapters + book names from `eng-kjv`, listed last and only with eng-kjv; ≤50-verse chunks, the API clamps out-of-range requests (filter!), `1Thes`/`1Jn` abbreviations; `RemoteBibleError` → 503/502 "NLT nedostupné" for NLT as main, while an unavailable NLT SECONDARY loads main-only + a `warning` toast (30 s fail-fast after the API was unreachable); `NLT_LONGER_CHAPTERS` extends the KJV structure (3 John 1, Revelation 12) | `.claude/rules/bible-remote-translations.md` (auto-loads on `bible_remote/**`, `state/bible_source*`, `state/bible.rs`) |
 | AI backend config (#761, bundled proxy removed #762): OpenRouter via env; `PRESENTER_AI_*` WIN over the DB `ai-settings` row on the EFFECTIVE path only (`resolve_effective_settings` + `router/ai_env.rs`), never the raw persist/display path; effective `apiUrl` = env → DB → default `https://openrouter.ai/api/v1` (`DEFAULT_AI_API_URL`); deploy writes 0600 `/etc/presenter/ai.env` (`EnvironmentFile=-`) from `OPENROUTER_API_KEY`+`AI_MODEL` in all three workflows; `DEFAULT_AI_MODEL`=OpenRouter slug; attribution headers in `ai/client.rs` | `.claude/rules/ai-backend-config.md` (auto-loads on `router/ai.rs`, `router/ai_env.rs`, `ai/mod.rs`, `ai/client.rs`, the 3 deploy workflows, the 2 units) |
 | Šepkár tips (#852 S1a/S1b): site-local `interpreter_tips` (no sync), rules + limits once in core `interpreter_tips.rs`, state-layer `TipRefusal` → 422 / 429 + Retry-After, duplicate = pair + its note (or the note alone) within 2 min not dismissed, create = one at a time (`create_lock`), full-list `LiveEvent::InterpreterTips` after every write + the minute boundary task (publishes serialized), injected `TipClock`, retention by `service_date` | `.claude/rules/interpreter-tips.md` (auto-loads on core `interpreter_tips.rs`, the migration, the entity, repository/state/router `interpreter_tips*`, the two pages, `components/interpreter/**`, `ws/passive.rs`, `state/interpreter.rs`, the two stylesheets, the spec) |
-| Listening ingest (#852 C1a): VBAN codec ported from songplayer `vban_packet.rs` @ dd551b7d + typed-reject decode, mic list ships EMPTY (no socket until saved, validate mode never binds, startup read under the save lock), the receive loop never waits (drop-oldest chain queue), sender IP parsed once and peer IP canonicalised, counter rule gap ≤ 1 s = silence / late = drop / > 1 s = resync, a second sender locked out until the first is silent 3 s, a rename keeps the mic and its chain (generation-gated chain writes), rubato `=5.0.1` `Fft` 20 ms chunks → 16 kHz frames counted with their level (no frame channel until C1b / C3), WARN gates read the health counters, whole-list `GET|PUT /listen/mics`, quiet `GET /listen/status` | `.claude/rules/listen-vban.md` (auto-loads on core `audio/**` + `listen/**`, server `listen/**`, `state/listen.rs`, `router/listen.rs`, `scripts/dev/listen/**`) |
+| Listening ingest (#852 C1a): VBAN codec ported from songplayer `vban_packet.rs` @ dd551b7d + typed-reject decode, mic list ships EMPTY (no socket until saved, validate mode never binds, startup read under the save lock), the receive loop never waits (drop-oldest chain queue), sender IP parsed once and peer IP canonicalised, counter rule gap ≤ 1 s = silence / late = drop / > 1 s = resync, a second sender locked out until the first is silent 3 s, a rename keeps the mic and its chain (generation-gated chain writes), rubato `=5.0.1` `Fft` 20 ms chunks → 16 kHz frames counted with their level and sent to ONE never-waiting `broadcast` per mic id (C1b-2, 128 frames), WARN gates read the health counters, whole-list `GET|PUT /listen/mics`, quiet `GET /listen/status`; recorder (C1b): `<root>/<YYYY-MM-DD>/<mic>_<HHMMSS>.wav`, root env or `recordings` next to the DB, 30-day retention once per local date (own files only, never a link) | `.claude/rules/listen-vban.md` (auto-loads on core `audio/**` + `listen/**`, server `listen/**`, `state/listen.rs`, `router/listen.rs`, `scripts/dev/listen/**`) |
 | Request log + `presenter::bible::show` (#850 O1): TraceLayer span on tower-http's `make_span` target, quiet poll/asset paths = no span, 5xx = WARN, client strings quoted, client IP = the #793 rule, `sqlx=warn` in every default `RUST_LOG`, drop-ins override `RUST_LOG` (dev's `gst-debug.conf` hides request lines; deploy.yml/release.yml fail when the running service's `RUST_LOG` is not the unit's), timer-polled GETs quiet (writes on the same paths stay logged), a silent WHEP poll (POST 204) has no response line, the request span prefixes every handler line, `src/tracing_capture.rs` = the ONE log-test helper | `.claude/rules/observability.md` (auto-loads on `router/request_log.rs`, `tracing_capture.rs`, `router/integrations/ndi_whep.rs`, `router.rs`, `state/bible_show_log.rs`, `main.rs`, the units, `deploy.yml`, `release.yml`) |
 | Cloudflare Access reconcile (#850 §9): config outside the public repo, dry run first (4× `ok`), never deletes, `update_app` unverified live | `.claude/rules/cloudflare-access.md` (auto-loads on `ops/cloudflare/**`) |
 | Bible reference resolver (#850 A): generated name table (never hand-edit; generator `--check` in CI), Python↔Rust fold parity via `fold-cases.json`, data decisions (Ab = Habakkuk, P.Š. no-fold, Mr); A1b: structure filter, SHORT_DEFAULTS + `<=` ambiguity, corpus alternatives rule | `.claude/rules/bible-resolver.md` (auto-loads on `bible/resolver/**`, `canonical.rs`, the generator, `data/bible-book-names/**`) |
@@ -303,6 +303,7 @@ crates/
 | `PRESENTER_DRIVE_DOWNLOAD_URL` | `https://drive.usercontent.google.com` | Google Drive download base URL for the playback mirror (#843; same test treatment). |
 | `PRESENTER_PLAYBACK_MAX_GB`   | `20`                    | Byte budget of the whole playback mirror in GiB (#843); a download that would pass it is refused with a Slovak status error. |
 | `PRESENTER_LISTEN_VBAN_ADDR`  | `0.0.0.0:6980`          | Bind address of the VBAN receive socket for the listening microphones (#852 C1a); bound only while `listen.mics` is non-empty (it ships empty). The E2E harness uses `127.0.0.1:0`. |
+| `PRESENTER_LISTEN_RECORDINGS_DIR` | `recordings` next to the sqlite file | Folder of the listening recordings (#852 C1b): `<dir>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav`, day folders deleted after 30 days. Prod / PP `/opt/presenter/recordings`, dev `/opt/presenter-dev/recordings`. |
 | `RUST_LOG`                    | `info,tower_http=debug,sqlx=warn` | Tracing filter                                                       |
 
 ---
diff --git a/docs/configuration.md b/docs/configuration.md
index b1e9755..e062c7e 100644
--- a/docs/configuration.md
+++ b/docs/configuration.md
@@ -105,9 +105,14 @@ for a refused sample rate), the streams seen but not assigned and the senders wh
 refused. Senders: VB-Matrix or Voicemeeter on the FOH PC, 48 kHz, 16-bit, stream names
 `GUEST_EN` / `INTERP_SK` (spec §5.7). `scripts/dev/listen/vban_send.py` sends a test tone.
 
-| Variable                     | Default        | Purpose |
-| ---------------------------- | -------------- | ------- |
-| `PRESENTER_LISTEN_VBAN_ADDR` | `0.0.0.0:6980` | Bind address of the VBAN socket. The E2E harness uses `127.0.0.1:0`, so a test server never takes the dev service's port. |
+Recordings (spec §5.5, §12) live in day folders `<recordings folder>/<YYYY-MM-DD>/`; a day folder is
+deleted 30 days after its day (once per local date: at startup and after each midnight), only the
+recorder's own `<mic id>_<HHMMSS>.wav` files, never a link or a foreign file.
+
+| Variable                          | Default                               | Purpose |
+| --------------------------------- | ------------------------------------- | ------- |
+| `PRESENTER_LISTEN_VBAN_ADDR`      | `0.0.0.0:6980`                        | Bind address of the VBAN socket. The E2E harness uses `127.0.0.1:0`, so a test server never takes the dev service's port. |
+| `PRESENTER_LISTEN_RECORDINGS_DIR` | `recordings` next to the sqlite file  | Folder of the listening recordings: prod / PP `/opt/presenter/recordings`, dev `/opt/presenter-dev/recordings` (inside the units' `ReadWritePaths`). The E2E spec uses a temp folder. |
 
 ### Companion Integration
 
```

- [ ] **Step 2: Commit** — paths `.claude/rules/listen-vban.md docs/configuration.md CLAUDE.md`, message `docs(listen): frame channel, recordings folder and retention (#852 C1b-2) [no-test: docs only]`.

- [ ] **Step 3: Measure the lane** — `bash "$SCRATCH/lane_measure.sh"`. Expected:

```text
crates/presenter-core/src/audio/mod.rs                                   +3
crates/presenter-core/src/audio/wav.rs                                   +43
crates/presenter-core/src/listen/mics.rs                                 +1
crates/presenter-core/src/listen/mod.rs                                  +2
crates/presenter-core/src/listen/recorder.rs                             +139
crates/presenter-server/src/listen/health.rs                             +4
crates/presenter-server/src/listen/mic_chain.rs                          +22
crates/presenter-server/src/listen/mod.rs                                +60
crates/presenter-server/src/listen/recorder.rs                           +150
crates/presenter-server/src/listen/recording_file.rs                     +83
crates/presenter-server/src/state/stream_assets.rs                       +8
lane total: 515 production lines added or changed
```

- [ ] **Step 4: Local gates** (Global Constraints).

- [ ] **Step 5: Push and watch CI** — every job; the new tests are in `cargo test -p presenter-core` (wav, recorder) and `-p presenter-server` (`listen::recorder::tests`, `listen::recording_file::tests`, `listen::mic_chain::tests`). The retention tests make temp dirs and a symlink (`std::os::unix::fs::symlink`; CI is Linux).

- [ ] **Step 6: Verify on dev after deploy-dev** (read-only): `ssh` is not needed — dev runs on this box. `journalctl -u presenter-dev --since "-10 min" | grep -c 'presenter::listen::recorder'` → 0 lines are expected (no folder → nothing deleted, nothing kept, no WARN); `test -e /opt/presenter-dev/recordings && echo exists || echo absent` → `absent` (the recorder creates nothing before it records); `curl -s "$DEV/healthz"` shows the new version; `curl -s "$DEV/listen/status" | python3 -m json.tool | head -5` unchanged shape (the `recorder` field arrives in lane 3).

- [ ] **Step 7: Close.** `playbook-review` (`📔 Playbook:` — the rule landed in Step 1); comment on #852: lane C1b-2 merged (recorder contract, retention, frame channel), next lane C1b-3.

---

## PR unit C1b-3 — the per-day toggle and the recorder's status (140 lines)

Starts from `origin/dev` with C1b-2 merged. Adds `PUT /listen/recorder`, the `listen.recorder` row (read at startup under the save lock, audited on every write) and `ListenStatus.recorder`. Still nothing records: there is no writer yet, so every configured mic shows `off` even with the switch on (the router test says so). The UI arrives in lane 7; until then the switch is reachable only through the API, and the plan never uses it on SNV or PP.

### Task 10: Version check

**Files:** `Cargo.toml`, `Cargo.lock` (Task 1 recipe).

- [ ] **Step 1:** Run Task 1 Step 1. `OK` → done. `BUMP` → Task 1 Steps 2–3.

### Task 11: The toggle — `PUT /listen/recorder`, startup read, audit, status

**Files:**
- Modify: `crates/presenter-core/src/listen/recorder.rs` (`MicRecording::off`), `crates/presenter-core/src/listen/status.rs` (`ListenStatus.recorder`; the wire-shape test grows by that field), `crates/presenter-server/src/listen/health.rs` (the snapshot lists every configured mic `off`), `crates/presenter-server/src/listen/mod.rs` (the toggle `watch<Option<NaiveDate>>`, `apply_recorder`, `today`, the status fills `enabled` / `enabledFor` / `root`), `crates/presenter-server/src/listen/recorder/tests.rs` (one test), `crates/presenter-server/src/state/listen.rs` (`listen_recorder`, `set_listen_recorder`, the startup read, `audit_listen_setting` shared with `listen.mics`), `crates/presenter-server/src/router/listen.rs` (the route + a test)

**Interfaces:**
- Produces: `PUT /listen/recorder` body `{"enabled": bool}` → `200 {"enabledFor": "YYYY-MM-DD" | null}`; actor from the request (`extract_actor`) in the INFO and the audit row. `GET /listen/status` gains `"recorder": {"enabled", "enabledFor", "root", "mics": [{"id", "state", "file", "bytes", "droppedFrames", "error"}]}`.
- `ListenService::apply_recorder(Option<NaiveDate>)`, `ListenService::today() -> NaiveDate` (the recorder's clock), `AppState::set_listen_recorder(enabled, actor) -> anyhow::Result<RecorderSetting>`; `status()` decides "records today" with core `RecorderSetting::records_on` (one rule).
- A stored row that does not parse: startup WARNs `listen.recorder cannot be read — the recorder stays off until it is switched again` and leaves it off; a `PUT` overwrites it.
- The one edit to an existing test: `status.rs`'s exact wire-shape test adds `recorder: RecorderStatus::default()` and its `"recorder"` JSON (the shape grew).

- [ ] **Step 1: Apply** (patch recipe):

```diff
diff --git a/crates/presenter-core/src/listen/recorder.rs b/crates/presenter-core/src/listen/recorder.rs
index 3b05b8f..5aa4428 100644
--- a/crates/presenter-core/src/listen/recorder.rs
+++ b/crates/presenter-core/src/listen/recorder.rs
@@ -73,6 +73,20 @@ pub struct MicRecording {
     pub error: Option<String>,
 }
 
+impl MicRecording {
+    /// A mic no writer ran for.
+    pub fn off(id: &str) -> Self {
+        Self {
+            id: id.to_string(),
+            state: RecordingState::Off,
+            file: None,
+            bytes: 0,
+            dropped_frames: 0,
+            error: None,
+        }
+    }
+}
+
 /// The recorder's part of `GET /listen/status`.
 #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
 #[serde(rename_all = "camelCase")]
diff --git a/crates/presenter-core/src/listen/status.rs b/crates/presenter-core/src/listen/status.rs
index d6390a0..014dc3f 100644
--- a/crates/presenter-core/src/listen/status.rs
+++ b/crates/presenter-core/src/listen/status.rs
@@ -1,11 +1,13 @@
 //! `GET /listen/status` (#852 C1a, spec §5.4 "Health", §14): the socket, each
 //! configured mic's counters, the streams seen but not used (they help the
-//! first setup) and the senders whose datagrams were refused. Read by
+//! first setup), the senders whose datagrams were refused and the recorder
+//! (C1b). Read by
 //! the C1b Settings card and the box acceptance (§5.8: counters read before
 //! and after a capture and diffed).
 
 use crate::audio::vban::VbanAudioFormat;
 use crate::listen::mics::MicConfig;
+use crate::listen::recorder::RecorderStatus;
 use serde::{Deserialize, Serialize};
 
 /// The VBAN receive socket's life cycle.
@@ -103,6 +105,8 @@ pub struct ListenStatus {
     pub mics: Vec<MicStatus>,
     pub unmatched: Vec<UnmatchedStream>,
     pub rejected: Vec<RejectedSender>,
+    /// The recorder (#852 C1b): the per-day toggle, the folder, each mic's file.
+    pub recorder: RecorderStatus,
 }
 
 #[cfg(test)]
@@ -177,6 +181,7 @@ mod tests {
                 corrupt: 0,
                 last_packet_age_ms: 10,
             }],
+            recorder: RecorderStatus::default(),
         };
         let wire = serde_json::to_value(&status).unwrap();
         assert_eq!(
@@ -198,7 +203,8 @@ mod tests {
                     "packets": 3, "lastPacketAgeMs": 40}],
                 "rejected": [{"sender": "198.51.100.7", "notVban": 1, "nonAudio": 2,
                     "unsupportedCodec": 0, "unsupportedType": 0, "corrupt": 0,
-                    "lastPacketAgeMs": 10}]
+                    "lastPacketAgeMs": 10}],
+                "recorder": {"enabled": false, "enabledFor": null, "root": "", "mics": []}
             })
         );
         let back: ListenStatus = serde_json::from_value(wire).unwrap();
diff --git a/crates/presenter-server/src/listen/health.rs b/crates/presenter-server/src/listen/health.rs
index 54a551d..f183d00 100644
--- a/crates/presenter-server/src/listen/health.rs
+++ b/crates/presenter-server/src/listen/health.rs
@@ -13,6 +13,7 @@ use presenter_core::audio::level::is_voiced;
 use presenter_core::audio::sequence::SequenceVerdict;
 use presenter_core::audio::vban::{VbanAudioFormat, VbanReject};
 use presenter_core::listen::mics::MicConfig;
+use presenter_core::listen::recorder::{MicRecording, RecorderStatus};
 use presenter_core::listen::status::{
     ListenStatus, MicState, MicStatus, RejectedSender, SocketState, SocketStatus, UnmatchedStream,
 };
@@ -446,6 +447,11 @@ impl ListenHealth {
                 .collect(),
             unmatched: self.unmatched_streams(now),
             rejected: self.rejected_senders(now),
+            // every configured mic, off; `ListenService::status` adds the toggle
+            recorder: RecorderStatus {
+                mics: mics.iter().map(|mic| MicRecording::off(&mic.id)).collect(),
+                ..RecorderStatus::default()
+            },
         }
     }
 
diff --git a/crates/presenter-server/src/listen/mod.rs b/crates/presenter-server/src/listen/mod.rs
index 0af9832..543ffb1 100644
--- a/crates/presenter-server/src/listen/mod.rs
+++ b/crates/presenter-server/src/listen/mod.rs
@@ -25,8 +25,10 @@ use std::sync::atomic::{AtomicBool, Ordering};
 use std::sync::{Arc, Mutex, PoisonError};
 use std::time::Duration;
 
+use chrono::NaiveDate;
 use presenter_core::audio::vban::VBAN_DEFAULT_PORT;
 use presenter_core::listen::mics::MicConfig;
+use presenter_core::listen::recorder::RecorderSetting;
 use presenter_core::listen::status::ListenStatus;
 use tokio::sync::{broadcast, watch};
 use tokio::time::Instant;
@@ -69,11 +71,14 @@ struct Inner {
     frames: Mutex<HashMap<String, broadcast::Sender<MicFrame>>>,
     /// Where the recorder writes and what time it is (#852 C1b).
     recorder_env: Mutex<RecorderEnv>,
+    /// The day the recorder records (`listen.recorder`); `None` = off.
+    recorder: watch::Sender<Option<NaiveDate>>,
 }
 
 impl ListenService {
     pub(crate) fn new() -> Self {
         let (config, _) = watch::channel(Arc::new(Vec::new()));
+        let (recorder, _) = watch::channel(None);
         Self {
             inner: Arc::new(Inner {
                 config,
@@ -82,6 +87,7 @@ impl ListenService {
                 save_lock: tokio::sync::Mutex::new(()),
                 frames: Mutex::new(HashMap::new()),
                 recorder_env: Mutex::new(RecorderEnv::from_env()),
+                recorder,
             }),
         }
     }
@@ -96,10 +102,27 @@ impl ListenService {
         let _previous = self.inner.config.send_replace(Arc::new(mics));
     }
 
-    /// `GET /listen/status`, ages measured now.
+    /// Hand the stored recorder toggle to the recorder (`None` = off).
+    pub(crate) fn apply_recorder(&self, enabled_for: Option<NaiveDate>) {
+        let _previous = self.inner.recorder.send_replace(enabled_for);
+    }
+
+    /// Today's local date, as the recorder counts days.
+    pub(crate) fn today(&self) -> NaiveDate {
+        self.recorder_env().now().date()
+    }
+
+    /// `GET /listen/status`, ages measured now; the recorder records while its
+    /// day is today.
     pub(crate) fn status(&self) -> ListenStatus {
         let mics = self.mics();
-        self.with_health(|health| health.snapshot(&mics, Instant::now()))
+        let mut status = self.with_health(|health| health.snapshot(&mics, Instant::now()));
+        let env = self.recorder_env();
+        let enabled_for = *self.inner.recorder.borrow();
+        status.recorder.enabled = RecorderSetting { enabled_for }.records_on(env.now().date());
+        status.recorder.enabled_for = enabled_for;
+        status.recorder.root = env.root.display().to_string();
+        status
     }
 
     /// Start the socket task and the recorder's supervisor once; `false` when
diff --git a/crates/presenter-server/src/listen/recorder/tests.rs b/crates/presenter-server/src/listen/recorder/tests.rs
index 86223f2..8d08032 100644
--- a/crates/presenter-server/src/listen/recorder/tests.rs
+++ b/crates/presenter-server/src/listen/recorder/tests.rs
@@ -114,3 +114,23 @@ async fn a_started_service_prunes_at_once_and_logs_each_file() {
     assert_eq!(lines.len(), 1);
     assert_eq!(lines[0].field("bytes"), Some("1234"));
 }
+
+#[test]
+fn the_status_says_whether_the_recorder_records_today() {
+    let service = ListenService::new();
+    let (clock, now) = clock("2026-10-18T19:42:05");
+    service.set_recorder_env(RecorderEnv {
+        root: PathBuf::from("/srv/recordings"),
+        clock,
+    });
+    let off = service.status().recorder;
+    assert_eq!((off.enabled, off.enabled_for), (false, None));
+    assert_eq!(off.root, "/srv/recordings");
+    let day: NaiveDate = "2026-10-18".parse().unwrap();
+    service.apply_recorder(Some(day));
+    let on = service.status().recorder;
+    assert_eq!((on.enabled, on.enabled_for), (true, Some(day)));
+    *now.lock().unwrap() = at("2026-10-19T00:00:01");
+    assert!(!service.status().recorder.enabled, "the day is over");
+    assert_eq!(service.today(), "2026-10-19".parse::<NaiveDate>().unwrap());
+}
diff --git a/crates/presenter-server/src/router/listen.rs b/crates/presenter-server/src/router/listen.rs
index 66edf2d..86ac7b1 100644
--- a/crates/presenter-server/src/router/listen.rs
+++ b/crates/presenter-server/src/router/listen.rs
@@ -1,14 +1,16 @@
 //! #852 C1a: `GET|PUT /listen/mics` (the whole list; 422 with the Slovak rule)
-//! and `GET /listen/status` (polled → a quiet GET in `request_log.rs`).
+//! and `GET /listen/status` (polled → a quiet GET in `request_log.rs`). C1b:
+//! `PUT /listen/recorder`, the recorder's per-day toggle (a person's write, logged).
 
 use super::integrations::extract_actor;
 use super::AppError;
 use crate::state::AppState;
 use axum::extract::State;
 use axum::http::HeaderMap;
-use axum::routing::get;
+use axum::routing::{get, put};
 use axum::{Json, Router};
 use presenter_core::listen::mics::{MicConfig, MicsValidationError};
+use presenter_core::listen::recorder::{RecorderSetting, RecorderToggle};
 use presenter_core::listen::status::ListenStatus;
 use tracing::instrument;
 
@@ -17,6 +19,7 @@ pub fn router() -> Router<AppState> {
     Router::new()
         .route("/listen/mics", get(get_mics).put(put_mics))
         .route("/listen/status", get(get_status))
+        .route("/listen/recorder", put(put_recorder))
 }
 
 #[instrument(skip_all)]
@@ -42,6 +45,18 @@ async fn get_status(State(state): State<AppState>) -> Json<ListenStatus> {
     Json(state.listen_status())
 }
 
+#[instrument(skip_all)]
+async fn put_recorder(
+    State(state): State<AppState>,
+    headers: HeaderMap,
+    Json(toggle): Json<RecorderToggle>,
+) -> Result<Json<RecorderSetting>, AppError> {
+    let actor = extract_actor(&headers);
+    Ok(Json(
+        state.set_listen_recorder(toggle.enabled, &actor).await?,
+    ))
+}
+
 /// `MicsValidationError` → 422 with its Slovak text; everything else → the
 /// central mapping (a corrupt stored list → 500).
 fn map_mics_error(err: anyhow::Error) -> AppError {
@@ -60,6 +75,7 @@ mod tests {
     use presenter_core::audio::vban::VbanSampleType;
     use presenter_core::audio::vban_encode::{tone, VbanEncoder};
     use presenter_core::listen::mics::LISTEN_MICS_KEY;
+    use presenter_core::listen::recorder::LISTEN_RECORDER_KEY;
     use presenter_core::listen::status::SocketState;
     use serde_json::{json, Value};
     use std::net::SocketAddr;
@@ -308,6 +324,73 @@ mod tests {
         assert_eq!(status["unmatched"][0]["format"]["channels"], json!(2));
     }
 
+    #[tokio::test]
+    async fn put_listen_recorder_switches_it_on_for_today_audits_and_shows_in_the_status() {
+        let (state, app) = app().await;
+        state
+            .set_listen_mics(serde_json::from_str(SPEC_MICS).unwrap(), "test")
+            .await
+            .unwrap();
+        let (status, reply) = send(
+            &app,
+            Method::PUT,
+            "/listen/recorder",
+            Some(json!({"enabled": true})),
+        )
+        .await;
+        assert_eq!(status, StatusCode::OK, "{reply}");
+        let day = reply["enabledFor"].as_str().unwrap().to_string();
+        assert!(
+            chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d").is_ok(),
+            "{reply}"
+        );
+        let (_, live) = send(&app, Method::GET, "/listen/status", None).await;
+        assert_eq!(
+            (
+                live["recorder"]["enabled"].clone(),
+                live["recorder"]["enabledFor"].clone()
+            ),
+            (json!(true), json!(day))
+        );
+        assert_eq!(
+            live["recorder"]["mics"],
+            json!([
+                {"id": "guest", "state": "off", "file": null, "bytes": 0, "droppedFrames": 0, "error": null},
+                {"id": "interp", "state": "off", "file": null, "bytes": 0, "droppedFrames": 0, "error": null}
+            ]),
+            "every configured mic, off until the writers arrive (C1b-4)"
+        );
+        let stored = state
+            .repository()
+            .get_app_setting(LISTEN_RECORDER_KEY)
+            .await
+            .unwrap()
+            .unwrap();
+        assert_eq!(
+            serde_json::from_str::<Value>(&stored).unwrap(),
+            json!({"enabledFor": day})
+        );
+        assert_eq!(
+            send(
+                &app,
+                Method::PUT,
+                "/listen/recorder",
+                Some(json!({"enabled": false}))
+            )
+            .await,
+            (StatusCode::OK, json!({"enabledFor": null}))
+        );
+        let (_, live) = send(&app, Method::GET, "/listen/status", None).await;
+        assert_eq!(live["recorder"]["enabled"], json!(false));
+        let audit = state
+            .repository()
+            .list_settings_audit(Some("app_settings"), Some(LISTEN_RECORDER_KEY), None, 10)
+            .await
+            .unwrap();
+        assert_eq!(audit.len(), 2, "one audit row per toggle");
+        assert!(audit.iter().all(|row| row.actor == "203.0.113.9"));
+    }
+
     /// C1a-3: the counter rule's and the chain's fields of the integration
     /// test's mic (counter 10 was left out; a 1 kHz tone at half scale).
     fn assert_chain_fields(mic: &Value) {
diff --git a/crates/presenter-server/src/state/listen.rs b/crates/presenter-server/src/state/listen.rs
index edfae68..c5a3907 100644
--- a/crates/presenter-server/src/state/listen.rs
+++ b/crates/presenter-server/src/state/listen.rs
@@ -1,10 +1,14 @@
 //! #852 C1a: `listen.mics` ↔ the receiver. Startup only READS it (never in
 //! validate mode); `set_listen_mics` is the only writer (+ a best-effort
-//! `settings_audit` row, the `ai-settings` pattern, #661).
+//! `settings_audit` row, the `ai-settings` pattern, #661). C1b: the recorder's
+//! per-day toggle `listen.recorder` the same way — read at startup, written by
+//! `set_listen_recorder`.
 
 use super::AppState;
+use crate::listen::recorder::RECORDER_TARGET;
 use crate::listen::{vban_bind_addr, MIC_TARGET};
 use presenter_core::listen::mics::{validate_mics, MicConfig, LISTEN_MICS_KEY};
+use presenter_core::listen::recorder::{RecorderSetting, LISTEN_RECORDER_KEY};
 use presenter_core::listen::status::ListenStatus;
 use presenter_persistence::SettingsAuditSource;
 use std::net::SocketAddr;
@@ -17,6 +21,11 @@ const LISTEN_AUDIT_TABLE: &str = "app_settings";
 #[error("Uložené nastavenie mikrofónov (listen.mics) je poškodené: {0}")]
 struct StoredMicsCorrupt(String);
 
+/// A stored `listen.recorder` that is not a valid row: the recorder stays off.
+#[derive(Debug, thiserror::Error)]
+#[error("Uložené nastavenie nahrávania (listen.recorder) je poškodené: {0}")]
+struct StoredRecorderCorrupt(String);
+
 impl AppState {
     /// The stored list (empty when never saved).
     pub(crate) async fn listen_mics(&self) -> anyhow::Result<Vec<MicConfig>> {
@@ -47,16 +56,59 @@ impl AppState {
             actor,
             "listen mics saved"
         );
-        self.audit_listen_mics(before, &mics, actor).await;
+        let after = serde_json::to_value(&mics).unwrap_or_default();
+        self.audit_listen_setting(LISTEN_MICS_KEY, before, after, actor)
+            .await;
         Ok(mics)
     }
 
+    /// The stored toggle (off when never saved).
+    async fn listen_recorder(&self) -> anyhow::Result<RecorderSetting> {
+        let Some(raw) = self.repository.get_app_setting(LISTEN_RECORDER_KEY).await? else {
+            return Ok(RecorderSetting::default());
+        };
+        serde_json::from_str(&raw)
+            .map_err(|err| anyhow::Error::new(StoredRecorderCorrupt(err.to_string())))
+    }
+
+    /// `PUT /listen/recorder`: on = today's local date, off = none; stored,
+    /// applied, audited.
+    pub(crate) async fn set_listen_recorder(
+        &self,
+        enabled: bool,
+        actor: &str,
+    ) -> anyhow::Result<RecorderSetting> {
+        let setting = RecorderSetting {
+            enabled_for: enabled.then(|| self.listen.today()),
+        };
+        let _save = self.listen.save_guard().await;
+        let before = self.repository.get_app_setting(LISTEN_RECORDER_KEY).await?;
+        self.repository
+            .set_app_setting(LISTEN_RECORDER_KEY, &serde_json::to_string(&setting)?)
+            .await?;
+        self.listen.apply_recorder(setting.enabled_for);
+        match setting.enabled_for {
+            Some(day) => tracing::info!(
+                target: RECORDER_TARGET,
+                %day,
+                actor,
+                "listen recorder switched on for today"
+            ),
+            None => tracing::info!(target: RECORDER_TARGET, actor, "listen recorder switched off"),
+        }
+        let after = serde_json::to_value(setting).unwrap_or_default();
+        self.audit_listen_setting(LISTEN_RECORDER_KEY, before, after, actor)
+            .await;
+        Ok(setting)
+    }
+
     /// `GET /listen/status`.
     pub(crate) fn listen_status(&self) -> ListenStatus {
         self.listen.status()
     }
 
-    /// Load `listen.mics` into the receiver and start its socket task (once).
+    /// Load `listen.mics` and the recorder toggle into the receiver and start its
+    /// socket task (once).
     /// The read and the apply run under the save lock: a save that comes
     /// during startup waits and is applied after it, never overwritten by
     /// this older read (review 1a).
@@ -71,6 +123,14 @@ impl AppState {
                     "listen.mics cannot be read — no microphone is received until it is saved again"
                 ),
             }
+            match self.listen_recorder().await {
+                Ok(setting) => self.listen.apply_recorder(setting.enabled_for),
+                Err(err) => tracing::warn!(
+                    target: RECORDER_TARGET,
+                    error = %err,
+                    "listen.recorder cannot be read — the recorder stays off until it is switched again"
+                ),
+            }
         }
         self.listen.start(bind)
     }
@@ -84,16 +144,21 @@ impl AppState {
     }
 
     /// Best effort: the write already succeeded, an audit failure only WARNs.
-    async fn audit_listen_mics(&self, before: Option<String>, after: &[MicConfig], actor: &str) {
+    async fn audit_listen_setting(
+        &self,
+        key: &str,
+        before: Option<String>,
+        after: serde_json::Value,
+        actor: &str,
+    ) {
         let before = before.map(|raw| {
             serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::Value::String(raw))
         });
-        let after = serde_json::to_value(after).unwrap_or_default();
         let recorded = self
             .repository
             .record_settings_audit(
                 LISTEN_AUDIT_TABLE,
-                LISTEN_MICS_KEY,
+                key,
                 SettingsAuditSource::HttpSetter,
                 actor,
                 before,
@@ -104,7 +169,8 @@ impl AppState {
             tracing::warn!(
                 target: MIC_TARGET,
                 ?err,
-                "failed to record the listen.mics audit row (the save itself succeeded)"
+                key,
+                "failed to record a listen setting's audit row (the save itself succeeded)"
             );
         }
     }
@@ -113,7 +179,10 @@ impl AppState {
 #[cfg(test)]
 mod tests {
     use super::*;
+    use crate::listen::recorder::RecorderEnv;
+    use chrono::NaiveDateTime;
     use presenter_core::listen::status::SocketState;
+    use std::sync::Arc;
     use std::time::Duration;
 
     #[tokio::test]
@@ -141,4 +210,57 @@ mod tests {
             Some("127.0.0.1:0")
         );
     }
+
+    fn fixed(at: &'static str) -> RecorderEnv {
+        let now: NaiveDateTime = at.parse().unwrap();
+        RecorderEnv {
+            root: std::env::temp_dir().join("presenter-recorder-never-written"),
+            clock: Arc::new(move || now),
+        }
+    }
+
+    #[tokio::test]
+    async fn the_startup_resumes_a_recorder_switched_on_today_but_not_yesterdays() {
+        let row = r#"{"enabledFor":"2026-10-18"}"#;
+        let same_day = AppState::in_memory().await.unwrap();
+        same_day
+            .listen
+            .set_recorder_env(fixed("2026-10-18T19:42:05"));
+        same_day
+            .repository()
+            .set_app_setting(LISTEN_RECORDER_KEY, row)
+            .await
+            .unwrap();
+        assert!(same_day.start_listen("127.0.0.1:0".parse().unwrap()).await);
+        assert!(
+            same_day.listen_status().recorder.enabled,
+            "a restart the same day (an auto-deploy mid-sermon) records on"
+        );
+        let next_day = AppState::in_memory().await.unwrap();
+        next_day
+            .listen
+            .set_recorder_env(fixed("2026-10-19T08:00:00"));
+        next_day
+            .repository()
+            .set_app_setting(LISTEN_RECORDER_KEY, row)
+            .await
+            .unwrap();
+        assert!(next_day.start_listen("127.0.0.1:0".parse().unwrap()).await);
+        let status = next_day.listen_status().recorder;
+        assert!(!status.enabled, "yesterday's switch never records today");
+        assert_eq!(status.enabled_for, Some("2026-10-18".parse().unwrap()));
+    }
+
+    #[tokio::test]
+    async fn a_corrupt_recorder_row_keeps_the_recorder_off() {
+        let state = AppState::in_memory().await.unwrap();
+        state
+            .repository()
+            .set_app_setting(LISTEN_RECORDER_KEY, "not a row")
+            .await
+            .unwrap();
+        assert!(state.start_listen("127.0.0.1:0".parse().unwrap()).await);
+        let status = state.listen_status().recorder;
+        assert_eq!((status.enabled, status.enabled_for), (false, None));
+    }
 }
```

Tests in this task: `the_status_says_whether_the_recorder_records_today` (settable clock: on for the 18th, off at 00:00:01 on the 19th), `the_startup_resumes_a_recorder_switched_on_today_but_not_yesterdays` (Review Focus 1 and 3), `a_corrupt_recorder_row_keeps_the_recorder_off`, `put_listen_recorder_switches_it_on_for_today_audits_and_shows_in_the_status` (stored row, two audit rows with the RFC 5737 actor, every mic `off`).

- [ ] **Step 2: Grep the construction sites** (`.claude/rules/quality-gates.md`, E0063): `git grep -n 'ListenStatus {' -- crates tests` → only `health.rs` (snapshot) and `status.rs` (the test), both in the patch.

- [ ] **Step 3: Gates** — `cargo fmt --all -- --check`; `QC_TARGETS="$(printf '%s\n' crates/presenter-core/src/listen/{recorder,status}.rs crates/presenter-server/src/listen/{health,mod}.rs crates/presenter-server/src/state/listen.rs crates/presenter-server/src/router/listen.rs)" python3 scripts/dev/fn_length_check.py .`; `python3 scripts/dev/no_panic_check.py .`.

- [ ] **Step 4: Commit** — the seven paths above, message `feat(listen): the recorder's per-day toggle and its status (#852 C1b-3)`.

### Task 12: Docs, playbook, then close lane C1b-3

**Files:**
- Modify: `.claude/rules/listen-vban.md`, `docs/configuration.md`, `CLAUDE.md` (router row)

- [ ] **Step 1: Apply the docs** (patch recipe):

```diff
diff --git a/.claude/rules/listen-vban.md b/.claude/rules/listen-vban.md
index c13faea..b4f1671 100644
--- a/.claude/rules/listen-vban.md
+++ b/.claude/rules/listen-vban.md
@@ -131,6 +131,17 @@ paths:
   ago (`recording_expired`), only `is_recording_file_name` regular files (`DirEntry::file_type` never
   follows a link; a linked folder is skipped), the folder only once empty; one INFO per deleted file,
   an INFO for a folder kept, a WARN for one that failed. All I/O in `spawn_blocking`.
+- **Recorder toggle** (#852 C1b-3) — a per-DAY switch: `app_settings` `listen.recorder` =
+  `{"enabledFor": "YYYY-MM-DD"}` or `{"enabledFor": null}` (`RecorderSetting`), written only by
+  `PUT /listen/recorder` `{"enabled": bool}` (on = the server's local date today) under the listen save
+  lock, with one best-effort `settings_audit` row (`audit_listen_setting`, shared with `listen.mics`)
+  and an INFO naming the actor; read at startup with `listen.mics`, under the same lock (a corrupt row
+  → off + WARN). `ListenService` holds it in a `watch<Option<NaiveDate>>`; the recorder records only
+  while that date is TODAY — a restart (auto-deploy) the same day resumes, the next day it is off, a
+  forgotten switch costs at most one day (VB-Matrix sends silence all day). Never make it a plain
+  on/off flag. `ListenStatus.recorder` (`RecorderStatus`): `enabled` (records today), `enabledFor`,
+  `root`, and one `MicRecording` per configured mic (`off` / `waiting` / `recording` / `error`, file,
+  bytes, `droppedFrames`, Slovak `error`).
 - **Routing day (owner action, spec §20, after C1b):** the owner routes `GUEST_EN` / `INTERP_SK`
   (48 kHz, 16-bit, after the channel mute) to 10.77.9.205:6980; then the two mics are saved (the C1b
   card, or `PUT /listen/mics` with the spec §5.6 list) and the box acceptance runs (spec §5.8): status
diff --git a/CLAUDE.md b/CLAUDE.md
index f37a2bc..80e50e0 100644
--- a/CLAUDE.md
+++ b/CLAUDE.md
@@ -60,7 +60,7 @@ This file provides guidance to Claude Code (claude.ai/code) when working with co
 | Remote Bible translation NLT (#826): `eng-nlt` fetched on demand from Tyndale's NLT API (`api.nlt.to`), never stored/bulk-copied/committed; ONE dispatch in `state/bible_source.rs` (never call the repository's Bible reads elsewhere); books/chapters + book names from `eng-kjv`, listed last and only with eng-kjv; ≤50-verse chunks, the API clamps out-of-range requests (filter!), `1Thes`/`1Jn` abbreviations; `RemoteBibleError` → 503/502 "NLT nedostupné" for NLT as main, while an unavailable NLT SECONDARY loads main-only + a `warning` toast (30 s fail-fast after the API was unreachable); `NLT_LONGER_CHAPTERS` extends the KJV structure (3 John 1, Revelation 12) | `.claude/rules/bible-remote-translations.md` (auto-loads on `bible_remote/**`, `state/bible_source*`, `state/bible.rs`) |
 | AI backend config (#761, bundled proxy removed #762): OpenRouter via env; `PRESENTER_AI_*` WIN over the DB `ai-settings` row on the EFFECTIVE path only (`resolve_effective_settings` + `router/ai_env.rs`), never the raw persist/display path; effective `apiUrl` = env → DB → default `https://openrouter.ai/api/v1` (`DEFAULT_AI_API_URL`); deploy writes 0600 `/etc/presenter/ai.env` (`EnvironmentFile=-`) from `OPENROUTER_API_KEY`+`AI_MODEL` in all three workflows; `DEFAULT_AI_MODEL`=OpenRouter slug; attribution headers in `ai/client.rs` | `.claude/rules/ai-backend-config.md` (auto-loads on `router/ai.rs`, `router/ai_env.rs`, `ai/mod.rs`, `ai/client.rs`, the 3 deploy workflows, the 2 units) |
 | Šepkár tips (#852 S1a/S1b): site-local `interpreter_tips` (no sync), rules + limits once in core `interpreter_tips.rs`, state-layer `TipRefusal` → 422 / 429 + Retry-After, duplicate = pair + its note (or the note alone) within 2 min not dismissed, create = one at a time (`create_lock`), full-list `LiveEvent::InterpreterTips` after every write + the minute boundary task (publishes serialized), injected `TipClock`, retention by `service_date` | `.claude/rules/interpreter-tips.md` (auto-loads on core `interpreter_tips.rs`, the migration, the entity, repository/state/router `interpreter_tips*`, the two pages, `components/interpreter/**`, `ws/passive.rs`, `state/interpreter.rs`, the two stylesheets, the spec) |
-| Listening ingest (#852 C1a): VBAN codec ported from songplayer `vban_packet.rs` @ dd551b7d + typed-reject decode, mic list ships EMPTY (no socket until saved, validate mode never binds, startup read under the save lock), the receive loop never waits (drop-oldest chain queue), sender IP parsed once and peer IP canonicalised, counter rule gap ≤ 1 s = silence / late = drop / > 1 s = resync, a second sender locked out until the first is silent 3 s, a rename keeps the mic and its chain (generation-gated chain writes), rubato `=5.0.1` `Fft` 20 ms chunks → 16 kHz frames counted with their level and sent to ONE never-waiting `broadcast` per mic id (C1b-2, 128 frames), WARN gates read the health counters, whole-list `GET|PUT /listen/mics`, quiet `GET /listen/status`; recorder (C1b): `<root>/<YYYY-MM-DD>/<mic>_<HHMMSS>.wav`, root env or `recordings` next to the DB, 30-day retention once per local date (own files only, never a link) | `.claude/rules/listen-vban.md` (auto-loads on core `audio/**` + `listen/**`, server `listen/**`, `state/listen.rs`, `router/listen.rs`, `scripts/dev/listen/**`) |
+| Listening ingest (#852 C1a): VBAN codec ported from songplayer `vban_packet.rs` @ dd551b7d + typed-reject decode, mic list ships EMPTY (no socket until saved, validate mode never binds, startup read under the save lock), the receive loop never waits (drop-oldest chain queue), sender IP parsed once and peer IP canonicalised, counter rule gap ≤ 1 s = silence / late = drop / > 1 s = resync, a second sender locked out until the first is silent 3 s, a rename keeps the mic and its chain (generation-gated chain writes), rubato `=5.0.1` `Fft` 20 ms chunks → 16 kHz frames counted with their level and sent to ONE never-waiting `broadcast` per mic id (C1b-2, 128 frames), WARN gates read the health counters, whole-list `GET|PUT /listen/mics`, quiet `GET /listen/status`; recorder (C1b): `<root>/<YYYY-MM-DD>/<mic>_<HHMMSS>.wav`, root env or `recordings` next to the DB, 30-day retention once per local date (own files only, never a link), per-DAY toggle `listen.recorder` `{enabledFor}` via `PUT /listen/recorder` (a restart resumes only the same day; audited) | `.claude/rules/listen-vban.md` (auto-loads on core `audio/**` + `listen/**`, server `listen/**`, `state/listen.rs`, `router/listen.rs`, `scripts/dev/listen/**`) |
 | Request log + `presenter::bible::show` (#850 O1): TraceLayer span on tower-http's `make_span` target, quiet poll/asset paths = no span, 5xx = WARN, client strings quoted, client IP = the #793 rule, `sqlx=warn` in every default `RUST_LOG`, drop-ins override `RUST_LOG` (dev's `gst-debug.conf` hides request lines; deploy.yml/release.yml fail when the running service's `RUST_LOG` is not the unit's), timer-polled GETs quiet (writes on the same paths stay logged), a silent WHEP poll (POST 204) has no response line, the request span prefixes every handler line, `src/tracing_capture.rs` = the ONE log-test helper | `.claude/rules/observability.md` (auto-loads on `router/request_log.rs`, `tracing_capture.rs`, `router/integrations/ndi_whep.rs`, `router.rs`, `state/bible_show_log.rs`, `main.rs`, the units, `deploy.yml`, `release.yml`) |
 | Cloudflare Access reconcile (#850 §9): config outside the public repo, dry run first (4× `ok`), never deletes, `update_app` unverified live | `.claude/rules/cloudflare-access.md` (auto-loads on `ops/cloudflare/**`) |
 | Bible reference resolver (#850 A): generated name table (never hand-edit; generator `--check` in CI), Python↔Rust fold parity via `fold-cases.json`, data decisions (Ab = Habakkuk, P.Š. no-fold, Mr); A1b: structure filter, SHORT_DEFAULTS + `<=` ambiguity, corpus alternatives rule | `.claude/rules/bible-resolver.md` (auto-loads on `bible/resolver/**`, `canonical.rs`, the generator, `data/bible-book-names/**`) |
diff --git a/docs/configuration.md b/docs/configuration.md
index e062c7e..4d17fd8 100644
--- a/docs/configuration.md
+++ b/docs/configuration.md
@@ -107,7 +107,11 @@ refused. Senders: VB-Matrix or Voicemeeter on the FOH PC, 48 kHz, 16-bit, stream
 
 Recordings (spec §5.5, §12) live in day folders `<recordings folder>/<YYYY-MM-DD>/`; a day folder is
 deleted 30 days after its day (once per local date: at startup and after each midnight), only the
-recorder's own `<mic id>_<HHMMSS>.wav` files, never a link or a foreign file.
+recorder's own `<mic id>_<HHMMSS>.wav` files, never a link or a foreign file. The recorder is OFF by
+default and switched per day (`PUT /listen/recorder` `{"enabled": true}` stores today's date in
+`listen.recorder`; Settings → "Počúvanie" → "Nahrávať dnes"): it records only on that date, so a
+restart the same day resumes and the switch turns itself off at midnight. `GET /listen/status` →
+`recorder` shows the switch, the folder and each microphone's file, bytes, dropped frames and error.
 
 | Variable                          | Default                               | Purpose |
 | --------------------------------- | ------------------------------------- | ------- |
```

- [ ] **Step 2: Commit** — paths `.claude/rules/listen-vban.md docs/configuration.md CLAUDE.md`, message `docs(listen): the recorder's per-day toggle (#852 C1b-3) [no-test: docs only]`.

- [ ] **Step 3: Measure the lane** — `bash "$SCRATCH/lane_measure.sh"`. Expected:

```text
crates/presenter-core/src/listen/recorder.rs                             +14
crates/presenter-core/src/listen/status.rs                               +5
crates/presenter-server/src/listen/health.rs                             +6
crates/presenter-server/src/listen/mod.rs                                +25
crates/presenter-server/src/router/listen.rs                             +17
crates/presenter-server/src/state/listen.rs                              +73
lane total: 140 production lines added or changed
```

- [ ] **Step 4: Local gates** (Global Constraints).

- [ ] **Step 5: Push and watch CI** — every job.

- [ ] **Step 6: Verify on dev after deploy-dev** (read-only, the switch is NOT touched on dev either — dev copies prod's DB on deploy): `curl -s "$DEV/listen/status" | python3 -c 'import json,sys; print(json.load(sys.stdin)["recorder"])'` → `{'enabled': False, 'enabledFor': None, 'root': '/opt/presenter-dev/recordings', 'mics': []}`. Same read on SNV and PP after the next release (`$SNV`, `$PP`, root `/opt/presenter/recordings`).

- [ ] **Step 7: Close.** `playbook-review`; comment on #852: lane C1b-3 merged (toggle, status), next lane C1b-4.

---

## PR unit C1b-4 — the recording file and the writers (594 lines)

Starts from `origin/dev` with C1b-3 merged. With this lane the switch records: one writer per configured mic while the toggle names today, one file per writer start (named by its first frame), a header that covers every batch, lag counted, failures shown and WARNed. Not yet here (lane 5): a new file on a drift, the midnight split of a batch (this lane drops the whole first batch heard after midnight and ends), the restart back-off (this lane restarts a failed writer on the next 60 s tick) and the cleanup of a file whose header write failed. Both lanes are safe on prod as they stand: the switch stays off at SNV and PP until the owner's consent (spec §19), and C1b-5 is expected to merge before that.

### Task 13: Version check

**Files:** `Cargo.toml`, `Cargo.lock` (Task 1 recipe).

- [ ] **Step 1:** Run Task 1 Step 1. `OK` → done. `BUMP` → Task 1 Steps 2–3.

### Task 14: The recording file — create, append, finish

**Files:**
- Modify: `crates/presenter-server/src/listen/recording_file.rs` (`RecordError`, `RecordingFile` + tests)

**Interfaces** (`pub(crate)`, consumed by Task 15):
- `enum RecordError { Io { path, source }, Linked { path }, NamesTaken { path } }` — every `Display` is Slovak and names the path (the status shows it).
- `RecordingFile::create(root: &Path, started: NaiveDateTime, mic_id: &str) -> Result<RecordingFile, RecordError>` — makes `<root>/<YYYY-MM-DD>` (a linked day folder is refused), opens `<mic id>_<HHMMSS>[-n].wav` with `create_new` (an existing name — a symlink too — is skipped to the next `-n`; the 10th in one second is refused, never overwritten), writes the 44-byte header for 0 bytes.
- `append(&mut self, &[MicFrame]) -> Result<(), RecordError>` (samples little-endian, then BOTH sizes patched; a failed write changes no size — the old ones stay valid), `finish(self) -> Result<u32, RecordError>` (sizes + `sync_all`, returns the audio bytes), `data_bytes() -> u32`, `path_text() -> String`.

- [ ] **Step 1: Apply** (patch recipe):

```diff
diff --git a/crates/presenter-server/src/listen/recording_file.rs b/crates/presenter-server/src/listen/recording_file.rs
index 10c683d..ea2dc27 100644
--- a/crates/presenter-server/src/listen/recording_file.rs
+++ b/crates/presenter-server/src/listen/recording_file.rs
@@ -1,21 +1,147 @@
 //! The recorder's files on disk (#852 C1b, spec §5.5, §12) — blocking
 //! `std::fs`, run in `spawn_blocking` only.
 //!
+//! A recording ([`RecordingFile`]) is one continuous stretch of a mic's audio:
+//! `<root>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav` (`-2` … `-9` for more stretches
+//! started in the same second), 16 kHz mono s16le behind the 44-byte header,
+//! whose two sizes are patched after every batch — a crash leaves a file whose
+//! header covers the audio up to the last batch. A file is only ever created
+//! (`create_new`: an existing name, a symlink included, is never opened) and
+//! never inside a linked day folder.
+//!
 //! Retention deletes only what the recorder makes: day folders named
 //! `YYYY-MM-DD` (canonical spelling) and in them the `<mic id>_<HHMMSS>[-n].wav`
 //! regular files; a folder is removed only when that emptied it. Entries are
 //! judged by `DirEntry::file_type`, which never follows a symlink, so a linked
 //! folder or file is never entered or deleted.
 
-use std::fs;
-use std::io;
+use std::fs::{self, File, OpenOptions};
+use std::io::{self, Seek, SeekFrom, Write};
 use std::path::{Path, PathBuf};
 
-use chrono::NaiveDate;
+use chrono::{NaiveDate, NaiveDateTime};
+use presenter_core::audio::wav::{
+    wav_header, wav_riff_size, WAV_DATA_SIZE_AT, WAV_HEADER_LEN, WAV_RIFF_SIZE_AT,
+};
 use presenter_core::listen::recorder::{
-    is_recording_file_name, parse_recording_day, recording_expired,
+    is_recording_file_name, parse_recording_day, recording_day_dir, recording_expired,
+    recording_file_name, RECORDING_NAMES_PER_SECOND,
 };
 
+use super::MicFrame;
+
+/// Why a recording could not be written (Slovak: the status shows it).
+#[derive(Debug, thiserror::Error)]
+pub(crate) enum RecordError {
+    #[error("zápis nahrávky {path} zlyhal: {source}")]
+    Io { path: String, source: io::Error },
+    #[error("priečinok nahrávok {path} je odkaz — nenahrávam doň")]
+    Linked { path: String },
+    #[error("v {path} už je 9 nahrávok z tej istej sekundy")]
+    NamesTaken { path: String },
+}
+
+fn io_error(path: &Path, source: io::Error) -> RecordError {
+    RecordError::Io {
+        path: path.display().to_string(),
+        source,
+    }
+}
+
+/// One continuous stretch of a mic's audio, in its own file.
+#[derive(Debug)]
+pub(crate) struct RecordingFile {
+    path: PathBuf,
+    file: File,
+    /// Audio bytes written (whole samples). One mic's day is at most 2.76 GB,
+    /// far below what the u32 size fields hold: the day ends at midnight.
+    data_bytes: u32,
+}
+
+impl RecordingFile {
+    /// A new file for `mic_id`'s audio whose first sample was heard at
+    /// `started` (local time); its header says "no audio yet".
+    pub(crate) fn create(
+        root: &Path,
+        started: NaiveDateTime,
+        mic_id: &str,
+    ) -> Result<Self, RecordError> {
+        let dir = root.join(recording_day_dir(started.date()));
+        fs::create_dir_all(&dir).map_err(|err| io_error(&dir, err))?;
+        if fs::symlink_metadata(&dir).is_ok_and(|meta| meta.file_type().is_symlink()) {
+            return Err(RecordError::Linked {
+                path: dir.display().to_string(),
+            });
+        }
+        for attempt in 1..=RECORDING_NAMES_PER_SECOND {
+            let path = dir.join(recording_file_name(mic_id, started.time(), attempt));
+            match OpenOptions::new().write(true).create_new(true).open(&path) {
+                Ok(file) => {
+                    let mut recording = Self {
+                        path,
+                        file,
+                        data_bytes: 0,
+                    };
+                    recording.io(|file| file.write_all(&wav_header(0)))?;
+                    return Ok(recording);
+                }
+                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
+                Err(err) => return Err(io_error(&path, err)),
+            }
+        }
+        Err(RecordError::NamesTaken {
+            path: dir.display().to_string(),
+        })
+    }
+
+    /// Append `frames`, then patch the header sizes: a crash after this leaves
+    /// a file whose header covers them. A failed write changes no size.
+    pub(crate) fn append(&mut self, frames: &[MicFrame]) -> Result<(), RecordError> {
+        let mut bytes = Vec::with_capacity(frames.iter().map(|frame| frame.len() * 2).sum());
+        for sample in frames.iter().flat_map(|frame| frame.iter()) {
+            bytes.extend_from_slice(&sample.to_le_bytes());
+        }
+        let at = WAV_HEADER_LEN as u64 + u64::from(self.data_bytes);
+        self.io(|file| {
+            file.seek(SeekFrom::Start(at))?;
+            file.write_all(&bytes)
+        })?;
+        let added = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
+        self.data_bytes = self.data_bytes.saturating_add(added);
+        self.patch_sizes()
+    }
+
+    /// The header and the audio on disk (`fsync`); the audio bytes written.
+    pub(crate) fn finish(mut self) -> Result<u32, RecordError> {
+        self.patch_sizes()?;
+        self.io(|file| file.sync_all())?;
+        Ok(self.data_bytes)
+    }
+
+    pub(crate) fn data_bytes(&self) -> u32 {
+        self.data_bytes
+    }
+
+    pub(crate) fn path_text(&self) -> String {
+        self.path.display().to_string()
+    }
+
+    fn patch_sizes(&mut self) -> Result<(), RecordError> {
+        let riff = wav_riff_size(self.data_bytes).to_le_bytes();
+        let data = self.data_bytes.to_le_bytes();
+        self.io(|file| {
+            file.seek(SeekFrom::Start(WAV_RIFF_SIZE_AT))?;
+            file.write_all(&riff)?;
+            file.seek(SeekFrom::Start(WAV_DATA_SIZE_AT))?;
+            file.write_all(&data)
+        })
+    }
+
+    fn io<T>(&mut self, op: impl FnOnce(&mut File) -> io::Result<T>) -> Result<T, RecordError> {
+        op(&mut self.file).map_err(|err| io_error(&self.path, err))
+    }
+}
+
 /// What a retention pass did.
 #[derive(Debug, Default)]
 pub(crate) struct PruneReport {
@@ -171,4 +297,126 @@ mod tests {
         let report = prune_recordings(&root.path().join("never-made"), day("2026-10-15")).unwrap();
         assert!(report.deleted.is_empty() && report.kept.is_empty() && report.failed.is_empty());
     }
+
+    fn at(text: &str) -> NaiveDateTime {
+        text.parse().unwrap()
+    }
+
+    fn frame(value: i16) -> MicFrame {
+        std::sync::Arc::from(vec![value; 1_600])
+    }
+
+    #[test]
+    fn every_batch_leaves_a_header_that_covers_it() {
+        let root = tempfile::tempdir().unwrap();
+        let mut file =
+            RecordingFile::create(root.path(), at("2026-10-18T19:42:05"), "guest").unwrap();
+        let path = root.path().join("2026-10-18").join("guest_194205.wav");
+        assert_eq!(file.path_text(), path.display().to_string());
+        assert_eq!(fs::read(&path).unwrap(), wav_header(0));
+        file.append(&[frame(1), frame(-2)]).unwrap();
+        // no finish: exactly what a crash right now would leave on disk
+        let bytes = fs::read(&path).unwrap();
+        assert_eq!(bytes.len(), 44 + 6_400);
+        assert_eq!(&bytes[..44], &wav_header(6_400)[..]);
+        assert_eq!(&bytes[44..46], &1i16.to_le_bytes());
+        assert_eq!(&bytes[44 + 3_200..44 + 3_202], &(-2i16).to_le_bytes());
+        file.append(&[frame(3)]).unwrap();
+        assert_eq!(&fs::read(&path).unwrap()[..44], &wav_header(9_600)[..]);
+        assert_eq!(file.finish().unwrap(), 9_600);
+    }
+
+    #[test]
+    fn a_second_file_in_the_same_second_never_touches_the_first() {
+        let root = tempfile::tempdir().unwrap();
+        let started = at("2026-10-18T19:42:05");
+        let mut first = RecordingFile::create(root.path(), started, "guest").unwrap();
+        first.append(&[frame(7)]).unwrap();
+        assert_eq!(first.finish().unwrap(), 3_200);
+        let second = RecordingFile::create(root.path(), started, "guest").unwrap();
+        assert!(
+            second.path_text().ends_with("guest_194205-2.wav"),
+            "{}",
+            second.path_text()
+        );
+        let old = fs::read(root.path().join("2026-10-18").join("guest_194205.wav")).unwrap();
+        assert_eq!(old.len(), 44 + 3_200);
+        assert_eq!(&old[44..46], &7i16.to_le_bytes());
+    }
+
+    #[test]
+    fn a_tenth_file_in_one_second_is_refused_not_overwritten() {
+        let root = tempfile::tempdir().unwrap();
+        let started = at("2026-10-18T19:42:05");
+        for _ in 0..RECORDING_NAMES_PER_SECOND {
+            RecordingFile::create(root.path(), started, "guest").unwrap();
+        }
+        let err = RecordingFile::create(root.path(), started, "guest").unwrap_err();
+        assert!(matches!(err, RecordError::NamesTaken { .. }), "{err}");
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn a_link_is_never_written_through() {
+        let root = tempfile::tempdir().unwrap();
+        let outside = tempfile::tempdir().unwrap();
+        std::os::unix::fs::symlink(outside.path(), root.path().join("2026-10-18")).unwrap();
+        let err =
+            RecordingFile::create(root.path(), at("2026-10-18T19:42:05"), "guest").unwrap_err();
+        assert!(matches!(err, RecordError::Linked { .. }), "{err}");
+        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
+        let target = outside.path().join("keep.wav");
+        fs::write(&target, b"keep").unwrap();
+        fs::create_dir_all(root.path().join("2026-10-19")).unwrap();
+        let link = root.path().join("2026-10-19").join("guest_194205.wav");
+        std::os::unix::fs::symlink(&target, link).unwrap();
+        let file = RecordingFile::create(root.path(), at("2026-10-19T19:42:05"), "guest").unwrap();
+        assert!(
+            file.path_text().ends_with("guest_194205-2.wav"),
+            "a link at the name is a taken name"
+        );
+        assert_eq!(fs::read(&target).unwrap(), b"keep");
+    }
+
+    #[test]
+    fn a_failing_write_after_a_good_create_changes_no_size_and_says_why() {
+        let root = tempfile::tempdir().unwrap();
+        let mut good =
+            RecordingFile::create(root.path(), at("2026-10-18T19:42:05"), "guest").unwrap();
+        good.append(&[frame(1)]).unwrap();
+        // the disk refuses the next write: the same file through a handle that cannot write
+        let mut full = RecordingFile {
+            path: good.path.clone(),
+            file: File::open(&good.path).unwrap(),
+            data_bytes: good.data_bytes(),
+        };
+        let err = full.append(&[frame(2)]).unwrap_err();
+        assert!(matches!(err, RecordError::Io { .. }), "{err}");
+        assert!(err.to_string().starts_with("zápis nahrávky "), "{err}");
+        assert_eq!(
+            full.data_bytes(),
+            3_200,
+            "nothing counted that was not written"
+        );
+        let bytes = fs::read(&good.path).unwrap();
+        assert_eq!(
+            &bytes[..44],
+            &wav_header(3_200)[..],
+            "the header still covers the audio"
+        );
+        assert_eq!(bytes.len(), 44 + 3_200);
+    }
+
+    #[test]
+    fn a_folder_that_cannot_be_made_is_a_slovak_error_naming_it() {
+        let dir = tempfile::tempdir().unwrap();
+        let root = dir.path().join("not-a-folder");
+        fs::write(&root, b"x").unwrap();
+        let err = RecordingFile::create(&root, at("2026-10-18T19:42:05"), "guest").unwrap_err();
+        let text = err.to_string();
+        assert!(
+            text.starts_with("zápis nahrávky ") && text.contains("not-a-folder"),
+            "{text}"
+        );
+    }
 }
```

Tests in this task: `every_batch_leaves_a_header_that_covers_it` (Review Focus 1: after each append the file on disk is a valid WAV of exactly the audio written), `a_second_file_in_the_same_second_never_touches_the_first`, `a_tenth_file_in_one_second_is_refused_not_overwritten`, `a_link_is_never_written_through`, `a_failing_write_after_a_good_create_changes_no_size_and_says_why` (B1: a file created fine, then a read-only handle — the append fails with a Slovak error naming the file and the header still says 0 bytes), `a_folder_that_cannot_be_made_is_a_slovak_error_naming_it`.

- [ ] **Step 2: Gates** — `cargo fmt --all -- --check`; `QC_TARGETS="crates/presenter-server/src/listen/recording_file.rs" python3 scripts/dev/fn_length_check.py .`; `python3 scripts/dev/no_panic_check.py .`.

- [ ] **Step 3: Commit** — path `crates/presenter-server/src/listen/recording_file.rs`, message `feat(listen): the recording file — create-new only, a header after every batch (#852 C1b-4)`. (`RecordingFile` has no production caller until Task 15; the two commits land in one push, so CI never sees the dead-code state alone — commit Task 15 before pushing.)

### Task 15: The writers — one per configured mic while the toggle names today

**Files:**
- Modify: `crates/presenter-server/src/listen/recorder.rs` (`Writer`, `Writers::reconcile`, the supervisor now also watches the mic list and the toggle, `run_writer`, `collect_batch`, `write_batch`, `note_*`, `close`), `crates/presenter-server/src/listen/health.rs` (`RecorderHealth` in `ListenHealth`, its snapshot in the status), `crates/presenter-server/src/listen/mod.rs` (`subscribe_frames`, `recorder_receiver`), `crates/presenter-server/src/listen/recorder/tests.rs` (the writer tests)

**Interfaces:**
- `ListenService::subscribe_frames(&str) -> broadcast::Receiver<MicFrame>` (private) — a writer subscribes BEFORE `writer_started` marks it `waiting`, so no frame after that is missed.
- `RecorderHealth` (`pub(crate)`, field `ListenHealth::recorder`, not cleared by `end_session`): `writer_started`, `wrote(mic, file, bytes)`, `dropped(mic, frames) -> (lag events, frames lost)`, `failed(mic, error) -> failures in a row`, `writer_stopped`, `all_stopped`, `retain(&[&str])`, `snapshot(&[MicConfig]) -> RecorderStatus`.
- Writer: frames → batch of ≤ 50 (`MAX_BATCH`, 5 s) → ONE `spawn_blocking` per batch (create lazily + append); the batch began `now − (batch + frames still queued) × 100 ms` (lane 5 uses this for the drift); INFO `recording started` (mic, file, `started` `%H:%M:%S%.3f`) and `recording stopped` (mic, file, bytes, why); WARN `recorder fell behind — dropped frames` on power-of-two lag events and `recording write failed …` on power-of-two failures in a row (the #484 gate `should_log_event`).
- Supervisor: `select!` on the 60 s tick, a mic-list change, a toggle change; `Writers::reconcile(service, env, day, mics)` stops writers of removed mics or when the day is not today (closing their files), collects ended ones, starts missing ones; off → `all_stopped` (every error forgotten).

- [ ] **Step 1: Apply** (patch recipe):

```diff
diff --git a/crates/presenter-server/src/listen/health.rs b/crates/presenter-server/src/listen/health.rs
index f183d00..e11c578 100644
--- a/crates/presenter-server/src/listen/health.rs
+++ b/crates/presenter-server/src/listen/health.rs
@@ -13,7 +13,7 @@ use presenter_core::audio::level::is_voiced;
 use presenter_core::audio::sequence::SequenceVerdict;
 use presenter_core::audio::vban::{VbanAudioFormat, VbanReject};
 use presenter_core::listen::mics::MicConfig;
-use presenter_core::listen::recorder::{MicRecording, RecorderStatus};
+use presenter_core::listen::recorder::{MicRecording, RecorderStatus, RecordingState};
 use presenter_core::listen::status::{
     ListenStatus, MicState, MicStatus, RejectedSender, SocketState, SocketStatus, UnmatchedStream,
 };
@@ -162,6 +162,114 @@ pub(crate) struct RejectSeen {
     pub(crate) non_audio: bool,
 }
 
+/// The recorder's part of the status (#852 C1b): one entry per configured mic
+/// a writer ran for. Not part of the socket session (`end_session` keeps it).
+#[derive(Debug, Default)]
+pub(crate) struct RecorderHealth {
+    mics: HashMap<String, MicRecorder>,
+}
+
+#[derive(Debug, Default)]
+struct MicRecorder {
+    state: RecordingState,
+    file: Option<String>,
+    bytes: u64,
+    dropped_frames: u64,
+    /// Times the writer fell behind: the #484 gate of the "fell behind" WARN.
+    lag_events: u64,
+    error: Option<String>,
+    /// Failed writes in a row: the #484 gate of the "write failed" WARN.
+    failures: u64,
+}
+
+impl MicRecorder {
+    fn stop(&mut self) {
+        self.state = RecordingState::Off;
+        self.error = None;
+        self.failures = 0;
+    }
+}
+
+impl RecorderHealth {
+    /// A writer subscribed to the mic's frames: waiting for audio. A failed
+    /// one keeps its error on show until a write succeeds.
+    pub(crate) fn writer_started(&mut self, mic_id: &str) {
+        let mic = self.mics.entry(mic_id.to_string()).or_default();
+        if mic.state != RecordingState::Error {
+            mic.state = RecordingState::Waiting;
+        }
+    }
+
+    /// A batch reached the file: recording, `bytes` of audio in it.
+    pub(crate) fn wrote(&mut self, mic_id: &str, file: &str, bytes: u32) {
+        let mic = self.mics.entry(mic_id.to_string()).or_default();
+        mic.state = RecordingState::Recording;
+        mic.file = Some(file.to_string());
+        mic.bytes = u64::from(bytes);
+        mic.error = None;
+        mic.failures = 0;
+    }
+
+    /// The writer fell behind and lost `frames`: (lag events, frames lost) so far.
+    pub(crate) fn dropped(&mut self, mic_id: &str, frames: u64) -> (u64, u64) {
+        let mic = self.mics.entry(mic_id.to_string()).or_default();
+        mic.dropped_frames += frames;
+        mic.lag_events += 1;
+        (mic.lag_events, mic.dropped_frames)
+    }
+
+    /// A write failed (`error` in Slovak); the failures in a row.
+    pub(crate) fn failed(&mut self, mic_id: &str, error: &str) -> u64 {
+        let mic = self.mics.entry(mic_id.to_string()).or_default();
+        mic.state = RecordingState::Error;
+        mic.error = Some(error.to_string());
+        mic.failures += 1;
+        mic.failures
+    }
+
+    /// The writer ended (switched off, the day over): off; the last file and
+    /// its size stay on show.
+    pub(crate) fn writer_stopped(&mut self, mic_id: &str) {
+        if let Some(mic) = self.mics.get_mut(mic_id) {
+            mic.stop();
+        }
+    }
+
+    /// The recorder was switched off (or its day ended): every mic off, every
+    /// error and failure streak forgotten.
+    pub(crate) fn all_stopped(&mut self) {
+        self.mics.values_mut().for_each(MicRecorder::stop);
+    }
+
+    /// Only configured mics keep an entry.
+    pub(crate) fn retain(&mut self, mic_ids: &[&str]) {
+        self.mics.retain(|id, _| mic_ids.contains(&id.as_str()));
+    }
+
+    /// Every configured mic, `off` for one no writer ran for. `ListenService`
+    /// fills in `enabled`, `enabledFor` and `root`.
+    pub(crate) fn snapshot(&self, mics: &[MicConfig]) -> RecorderStatus {
+        let mics = mics
+            .iter()
+            .map(|config| match self.mics.get(&config.id) {
+                Some(mic) => MicRecording {
+                    id: config.id.clone(),
+                    state: mic.state,
+                    file: mic.file.clone(),
+                    bytes: mic.bytes,
+                    dropped_frames: mic.dropped_frames,
+                    error: mic.error.clone(),
+                },
+                None => MicRecording::off(&config.id),
+            })
+            .collect();
+        RecorderStatus {
+            mics,
+            ..RecorderStatus::default()
+        }
+    }
+}
+
 /// A map that keeps the [`RECENT_CAP`] most recently seen keys.
 #[derive(Debug)]
 struct Recent<K, V> {
@@ -225,6 +333,8 @@ pub(crate) struct ListenHealth {
     /// WARN-class rejects this session (non-audio left out): the second gate of
     /// a repeat line.
     reject_total: u64,
+    /// The recorder's writers (#852 C1b); not part of the socket session.
+    pub(crate) recorder: RecorderHealth,
 }
 
 impl ListenHealth {
@@ -447,11 +557,7 @@ impl ListenHealth {
                 .collect(),
             unmatched: self.unmatched_streams(now),
             rejected: self.rejected_senders(now),
-            // every configured mic, off; `ListenService::status` adds the toggle
-            recorder: RecorderStatus {
-                mics: mics.iter().map(|mic| MicRecording::off(&mic.id)).collect(),
-                ..RecorderStatus::default()
-            },
+            recorder: self.recorder.snapshot(mics),
         }
     }
 
diff --git a/crates/presenter-server/src/listen/mod.rs b/crates/presenter-server/src/listen/mod.rs
index 543ffb1..13903f8 100644
--- a/crates/presenter-server/src/listen/mod.rs
+++ b/crates/presenter-server/src/listen/mod.rs
@@ -169,6 +169,15 @@ impl ListenService {
             .clone()
     }
 
+    /// A consumer's receiver of the mic's frames (the recorder; C3: the ASR).
+    fn subscribe_frames(&self, mic_id: &str) -> broadcast::Receiver<MicFrame> {
+        self.frame_sender(mic_id).subscribe()
+    }
+
+    fn recorder_receiver(&self) -> watch::Receiver<Option<NaiveDate>> {
+        self.inner.recorder.subscribe()
+    }
+
     /// The recorder's folder and clock.
     fn recorder_env(&self) -> RecorderEnv {
         self.inner
diff --git a/crates/presenter-server/src/listen/recorder.rs b/crates/presenter-server/src/listen/recorder.rs
index f668c29..0649112 100644
--- a/crates/presenter-server/src/listen/recorder.rs
+++ b/crates/presenter-server/src/listen/recorder.rs
@@ -1,34 +1,50 @@
-//! The recorder (#852 C1b, spec §5.5, §12). C1b-2: its folder and its
-//! retention — the day folders older than 30 days are deleted once per local
-//! date (at startup, then in the first minute after each midnight), so a
-//! recording can never outlive 30 days. C1b-4 adds the writers.
+//! The recorder (#852 C1b, spec §5.5, §12): with the per-day toggle on, every
+//! configured mic's 16 kHz frames — exactly what the ASR gets — go to
+//! `<root>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav` (`recording_file`). A writer per
+//! mic reads the mic's frame channel and never slows its chain (12.8 s behind,
+//! the oldest frames go and are counted); each batch (≤ 5 s) is written in
+//! `spawn_blocking`. A file opens with the mic's first frame (a mic without
+//! audio leaves no file) and is named by the time that frame was heard; the
+//! writer ends at the first frame after midnight; a failed write ends it too,
+//! and the supervisor starts it again. The day folders older than 30 days go
+//! once per local date. (C1b-5 adds a new file when the audio drifts from the
+//! wall clock, the midnight split of a batch and the restart back-off.)
 //!
 //! The folder: `PRESENTER_LISTEN_RECORDINGS_DIR`, else `recordings` next to the
-//! sqlite file of `PRESENTER_DB_URL` (prod / PP `/opt/presenter/recordings`, the
-//! spec's path; dev `/opt/presenter-dev/recordings`; both inside the units'
-//! `ReadWritePaths`), else `./recordings`. One supervisor task, started with the
-//! socket task (never in validate mode).
+//! sqlite file of `PRESENTER_DB_URL` (prod / PP `/opt/presenter/recordings`;
+//! dev `/opt/presenter-dev/recordings`). One supervisor task, started with the
+//! socket task (never in validate mode), reconciles the writers with the mic
+//! list and the toggle on every change and every minute.
 
+use std::collections::HashMap;
 use std::ffi::OsString;
-use std::path::PathBuf;
+use std::path::{Path, PathBuf};
 use std::sync::Arc;
 use std::time::Duration;
 
-use chrono::{NaiveDate, NaiveDateTime};
+use chrono::{NaiveDate, NaiveDateTime, TimeDelta};
+use presenter_core::listen::mics::MicConfig;
 use presenter_core::listen::recorder::RECORDING_RETENTION_DAYS;
+use tokio::sync::broadcast::error::{RecvError, TryRecvError};
+use tokio::sync::{broadcast, oneshot};
+use tokio::task::JoinHandle;
 use tokio::time::{interval, MissedTickBehavior};
 use tracing::{info, warn};
 
-use super::recording_file::{prune_recordings, PruneReport};
-use super::ListenService;
+use super::recording_file::{prune_recordings, PruneReport, RecordError, RecordingFile};
+use super::{should_log_event, ListenService, MicFrame};
 
 /// The target of every recorder log line.
 pub(crate) const RECORDER_TARGET: &str = "presenter::listen::recorder";
 /// Overrides the recordings folder (the E2E spec and tests use temp dirs).
 pub(crate) const RECORDINGS_DIR_ENV: &str = "PRESENTER_LISTEN_RECORDINGS_DIR";
 const RECORDINGS_DIR_NAME: &str = "recordings";
-/// The supervisor looks at the date this often.
+/// The supervisor reconciles at least this often.
 const RECONCILE_EVERY: Duration = Duration::from_secs(60);
+/// At most this many frames (5 s) per blocking write.
+const MAX_BATCH: usize = 50;
+/// One frame is 100 ms.
+const FRAME_MS: i64 = 100;
 
 /// The server's local wall time (tests inject a settable one).
 pub(crate) type Clock = Arc<dyn Fn() -> NaiveDateTime + Send + Sync>;
@@ -80,14 +96,325 @@ pub(crate) fn recordings_root(explicit: Option<OsString>, db_url: &str) -> PathB
     }
 }
 
-/// The supervisor: the retention pass once per local date.
+/// The wall-time length of `frames` frames.
+fn frames_duration(frames: usize) -> TimeDelta {
+    let frames = i64::try_from(frames).unwrap_or(i64::MAX);
+    TimeDelta::milliseconds(frames.saturating_mul(FRAME_MS))
+}
+
+/// A running writer and the way to stop it.
+struct Writer {
+    stop: oneshot::Sender<()>,
+    task: JoinHandle<()>,
+}
+
+impl Writer {
+    /// Subscribe to the mic's frames — BEFORE it is marked waiting, so no frame
+    /// after that is missed — and spawn its task.
+    fn start(service: &ListenService, mic_id: &str, day: NaiveDate, root: &Path) -> Self {
+        let frames = service.subscribe_frames(mic_id);
+        service.with_health(|health| health.recorder.writer_started(mic_id));
+        let (stop, stopped) = oneshot::channel();
+        let job = WriterJob {
+            mic_id: mic_id.to_string(),
+            day,
+            root: root.to_path_buf(),
+        };
+        let task = tokio::spawn(run_writer(service.clone(), job, frames, stopped));
+        Self { stop, task }
+    }
+
+    /// Ask the writer to close its file and wait until it has.
+    async fn stop(self) {
+        let _unheard = self.stop.send(());
+        if let Err(err) = self.task.await {
+            warn!(target: RECORDER_TARGET, %err, "a recorder writer ended abnormally");
+        }
+    }
+}
+
+/// The supervisor's writers.
+#[derive(Default)]
+struct Writers {
+    running: HashMap<String, Writer>,
+}
+
+impl Writers {
+    /// One writer per configured mic while recording `day`, none otherwise; a
+    /// writer that ended (a failed write, its day over) is replaced.
+    async fn reconcile(
+        &mut self,
+        service: &ListenService,
+        env: &RecorderEnv,
+        day: Option<NaiveDate>,
+        mics: &[MicConfig],
+    ) {
+        let configured: Vec<&str> = mics.iter().map(|mic| mic.id.as_str()).collect();
+        let wanted: &[&str] = if day.is_some() { &configured } else { &[] };
+        let ended: Vec<String> = self
+            .running
+            .iter()
+            .filter(|(id, writer)| !wanted.contains(&id.as_str()) || writer.task.is_finished())
+            .map(|(id, _)| id.clone())
+            .collect();
+        for id in ended {
+            if let Some(writer) = self.running.remove(&id) {
+                writer.stop().await;
+            }
+        }
+        service.with_health(|health| health.recorder.retain(&configured));
+        let Some(day) = day else {
+            service.with_health(|health| health.recorder.all_stopped());
+            return;
+        };
+        for id in wanted {
+            if !self.running.contains_key(*id) {
+                let writer = Writer::start(service, id, day, &env.root);
+                self.running.insert((*id).to_string(), writer);
+            }
+        }
+    }
+
+    async fn stop_all(self) {
+        for writer in self.running.into_values() {
+            writer.stop().await;
+        }
+    }
+}
+
+/// The supervisor: retention once per local date, then the writers matching
+/// the mic list and the toggle.
 pub(super) async fn supervise(service: ListenService) {
+    let mut config = service.config_receiver();
+    let mut toggle = service.recorder_receiver();
+    let mut writers = Writers::default();
     let mut pruned_on: Option<NaiveDate> = None;
     let mut tick = interval(RECONCILE_EVERY);
     tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
+    tick.reset();
     loop {
-        tick.tick().await;
-        prune_if_new_day(&service.recorder_env(), &mut pruned_on).await;
+        let env = service.recorder_env();
+        prune_if_new_day(&env, &mut pruned_on).await;
+        let today = env.now().date();
+        let day = (*toggle.borrow_and_update()).filter(|day| *day == today);
+        let mics = config.borrow_and_update().clone();
+        writers.reconcile(&service, &env, day, &mics).await;
+        let gone = tokio::select! {
+            _ = tick.tick() => false,
+            changed = config.changed() => changed.is_err(),
+            changed = toggle.changed() => changed.is_err(),
+        };
+        if gone {
+            break;
+        }
+    }
+    writers.stop_all().await;
+}
+
+/// What a writer writes.
+struct WriterJob {
+    mic_id: String,
+    day: NaiveDate,
+    root: PathBuf,
+}
+
+/// An open file and the wall time its first sample was heard.
+struct OpenFile {
+    file: RecordingFile,
+    started: NaiveDateTime,
+}
+
+impl OpenFile {
+    fn create(root: &Path, started: NaiveDateTime, mic_id: &str) -> Result<Self, RecordError> {
+        let file = RecordingFile::create(root, started, mic_id)?;
+        Ok(Self { file, started })
+    }
+}
+
+/// Why a writer ended or a file was closed.
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+enum Ended {
+    Stopped,
+    DayOver,
+    Failed,
+}
+
+impl Ended {
+    fn as_str(self) -> &'static str {
+        match self {
+            Self::Stopped => "switched off or the mic was removed",
+            Self::DayOver => "the day ended",
+            Self::Failed => "a write failed",
+        }
+    }
+}
+
+/// One mic's writer: frames in, batches written off the runtime. Ends when told
+/// to stop, at its first frame after midnight, or on a failed write (the
+/// supervisor starts it again).
+async fn run_writer(
+    service: ListenService,
+    job: WriterJob,
+    mut frames: broadcast::Receiver<MicFrame>,
+    mut stop: oneshot::Receiver<()>,
+) {
+    let mut open: Option<OpenFile> = None;
+    let ended = loop {
+        let received = tokio::select! {
+            _ = &mut stop => Err(RecvError::Closed),
+            received = frames.recv() => received,
+        };
+        let first = match received {
+            Ok(frame) => frame,
+            Err(RecvError::Lagged(missed)) => {
+                note_dropped(&service, &job.mic_id, missed);
+                continue;
+            }
+            Err(RecvError::Closed) => break Ended::Stopped,
+        };
+        let batch = collect_batch(&service, &job.mic_id, first, &mut frames);
+        // the batch began its own length, plus whatever is still queued, ago
+        let behind = frames_duration(batch.len() + frames.len());
+        let now = service.recorder_env().now();
+        let start = now.checked_sub_signed(behind).unwrap_or(now);
+        if now.date() != job.day {
+            break Ended::DayOver;
+        }
+        match write_batch(open.take(), &job, start, batch).await {
+            Ok((written, created)) => {
+                note_written(&service, &job, &written, created);
+                open = Some(written);
+            }
+            Err(error) => {
+                note_failed(&service, &job, &error);
+                break Ended::Failed;
+            }
+        }
+    };
+    if let Some(last) = open {
+        close(&job, last.file, ended).await;
+    }
+    if ended != Ended::Failed {
+        service.with_health(|health| health.recorder.writer_stopped(&job.mic_id));
+    }
+}
+
+/// The frame just received and whatever else is already waiting, at most
+/// [`MAX_BATCH`].
+fn collect_batch(
+    service: &ListenService,
+    mic_id: &str,
+    first: MicFrame,
+    frames: &mut broadcast::Receiver<MicFrame>,
+) -> Vec<MicFrame> {
+    let mut batch = vec![first];
+    while batch.len() < MAX_BATCH {
+        match frames.try_recv() {
+            Ok(frame) => batch.push(frame),
+            Err(TryRecvError::Lagged(missed)) => note_dropped(service, mic_id, missed),
+            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
+        }
+    }
+    batch
+}
+
+/// Write a batch off the runtime, into the open file or into a new one whose
+/// first sample was heard at `start`; the Slovak reason when it failed (the
+/// file is then dropped, its header covering the last good batch).
+async fn write_batch(
+    open: Option<OpenFile>,
+    job: &WriterJob,
+    start: NaiveDateTime,
+    batch: Vec<MicFrame>,
+) -> Result<(OpenFile, bool), String> {
+    let (root, mic_id) = (job.root.clone(), job.mic_id.clone());
+    let joined = tokio::task::spawn_blocking(move || -> Result<(OpenFile, bool), RecordError> {
+        let created = open.is_none();
+        let mut open = match open {
+            Some(open) => open,
+            None => OpenFile::create(&root, start, &mic_id)?,
+        };
+        open.file.append(&batch)?;
+        Ok((open, created))
+    })
+    .await;
+    match joined {
+        Ok(result) => result.map_err(|err| err.to_string()),
+        Err(err) => Err(format!("zápis nahrávky sa prerušil: {err}")),
+    }
+}
+
+/// The status after a batch; one INFO with the millisecond start time when the
+/// batch opened a file (the bake-off places the show log by it).
+fn note_written(service: &ListenService, job: &WriterJob, written: &OpenFile, created: bool) {
+    let file = written.file.path_text();
+    let bytes = written.file.data_bytes();
+    service.with_health(|health| health.recorder.wrote(&job.mic_id, &file, bytes));
+    if created {
+        info!(
+            target: RECORDER_TARGET,
+            mic = job.mic_id.as_str(),
+            file = %file,
+            started = %written.started.format("%H:%M:%S%.3f"),
+            "recording started"
+        );
+    }
+}
+
+/// Frames a lagging writer lost: counted; WARN on power-of-two lag events.
+fn note_dropped(service: &ListenService, mic_id: &str, missed: u64) {
+    let (events, dropped_frames) =
+        service.with_health(|health| health.recorder.dropped(mic_id, missed));
+    if should_log_event(events) {
+        warn!(
+            target: RECORDER_TARGET,
+            mic = mic_id,
+            missed,
+            dropped_frames,
+            events,
+            "recorder fell behind — dropped frames"
+        );
+    }
+}
+
+/// A failed write: the mic's state `error`; WARN on power-of-two failures in a row.
+fn note_failed(service: &ListenService, job: &WriterJob, error: &str) {
+    let failures = service.with_health(|health| health.recorder.failed(&job.mic_id, error));
+    if should_log_event(failures) {
+        warn!(
+            target: RECORDER_TARGET,
+            mic = job.mic_id.as_str(),
+            day = %job.day,
+            error,
+            failures,
+            "recording write failed — the writer starts again within a minute"
+        );
+    }
+}
+
+/// Close a file — header and `fsync` off the runtime — and log it.
+async fn close(job: &WriterJob, file: RecordingFile, why: Ended) {
+    let path = file.path_text();
+    let finished = match tokio::task::spawn_blocking(move || file.finish()).await {
+        Ok(result) => result.map_err(|err| err.to_string()),
+        Err(err) => Err(err.to_string()),
+    };
+    match finished {
+        Ok(bytes) => info!(
+            target: RECORDER_TARGET,
+            mic = job.mic_id.as_str(),
+            file = %path,
+            bytes,
+            why = why.as_str(),
+            "recording stopped"
+        ),
+        Err(error) => warn!(
+            target: RECORDER_TARGET,
+            mic = job.mic_id.as_str(),
+            file = %path,
+            error,
+            "closing the recording failed — its header covers the last batch written"
+        ),
     }
 }
 
diff --git a/crates/presenter-server/src/listen/recorder/tests.rs b/crates/presenter-server/src/listen/recorder/tests.rs
index 8d08032..aaf9f97 100644
--- a/crates/presenter-server/src/listen/recorder/tests.rs
+++ b/crates/presenter-server/src/listen/recorder/tests.rs
@@ -1,10 +1,21 @@
-//! #852 C1b-2: the recordings folder and the daily retention pass. Folders are
-//! temp dirs; the clock is a settable local time.
+//! #852 C1b: the recordings folder, the retention pass, the toggle's status and
+//! the writers (driven through the real supervisor, frames sent into the mic's
+//! channel or, end to end, real VBAN over loopback UDP). Folders are temp
+//! dirs; the clock is a settable local time.
 
 use super::*;
+use crate::listen::health::ListenHealth;
 use crate::tracing_capture::Capture;
+use presenter_core::audio::level::rms_dbfs;
+use presenter_core::audio::vban::VbanSampleType;
+use presenter_core::audio::vban_encode::{tone, VbanEncoder};
+use presenter_core::audio::wav::wav_header;
+use presenter_core::listen::mics::{MicChannel, MicLanguage, MicRole};
+use presenter_core::listen::recorder::{MicRecording, RecordingState};
+use presenter_core::listen::status::{ListenStatus, SocketState};
 use std::path::Path;
 use std::sync::Mutex;
+use tokio::time::Instant;
 
 fn at(text: &str) -> NaiveDateTime {
     text.parse().unwrap()
@@ -134,3 +145,486 @@ fn the_status_says_whether_the_recorder_records_today() {
     assert!(!service.status().recorder.enabled, "the day is over");
     assert_eq!(service.today(), "2026-10-19".parse::<NaiveDate>().unwrap());
 }
+
+// ---- the writers (C1b-4) ------------------------------------------------------
+
+const DAY: &str = "2026-10-18";
+
+fn day(text: &str) -> NaiveDate {
+    text.parse().unwrap()
+}
+
+fn mic(id: &str, stream: &str) -> MicConfig {
+    MicConfig {
+        id: id.to_string(),
+        name: id.to_string(),
+        stream_name: stream.to_string(),
+        sender_ip: None,
+        channel: MicChannel::Left,
+        language: MicLanguage::En,
+        role: MicRole::Guest,
+    }
+}
+
+fn recording(status: &ListenStatus, id: &str) -> MicRecording {
+    status
+        .recorder
+        .mics
+        .iter()
+        .find(|m| m.id == id)
+        .cloned()
+        .unwrap()
+}
+
+/// Poll `service.status()` until `done` holds; fails with the last status after 5 s.
+async fn wait_for(
+    service: &ListenService,
+    what: &str,
+    done: impl Fn(&ListenStatus) -> bool,
+) -> ListenStatus {
+    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
+    loop {
+        let status = service.status();
+        if done(&status) {
+            return status;
+        }
+        assert!(
+            tokio::time::Instant::now() < deadline,
+            "{what}: {status:#?}"
+        );
+        tokio::time::sleep(Duration::from_millis(10)).await;
+    }
+}
+
+/// Poll `done` for at most 5 s.
+async fn wait_until(what: &str, done: impl Fn() -> bool) {
+    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
+    while !done() {
+        assert!(tokio::time::Instant::now() < deadline, "{what}");
+        tokio::time::sleep(Duration::from_millis(10)).await;
+    }
+}
+
+fn frame(value: i16) -> MicFrame {
+    Arc::from(vec![value; 1_600])
+}
+
+fn send(service: &ListenService, mic_id: &str, value: i16) {
+    service.frame_sender(mic_id).send(frame(value)).unwrap();
+}
+
+fn day_file(root: &Path, name: &str) -> PathBuf {
+    root.join(DAY).join(name)
+}
+
+/// The names of the files in the day folder, sorted.
+fn day_files(root: &Path) -> Vec<String> {
+    let mut names: Vec<String> = std::fs::read_dir(root.join(DAY))
+        .unwrap()
+        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
+        .collect();
+    names.sort();
+    names
+}
+
+/// A file's header and its samples.
+fn read_wav(path: &Path) -> (Vec<u8>, Vec<i16>) {
+    let bytes = std::fs::read(path).unwrap();
+    let samples = bytes[44..]
+        .chunks_exact(2)
+        .map(|b| i16::from_le_bytes([b[0], b[1]]))
+        .collect();
+    (bytes[..44].to_vec(), samples)
+}
+
+/// A service recording `mics` into `root` on `start`'s day, its supervisor
+/// running (no socket), every writer subscribed; and the clock's handle.
+async fn recording_service(
+    root: &Path,
+    start: &str,
+    mics: Vec<MicConfig>,
+) -> (ListenService, Arc<Mutex<NaiveDateTime>>) {
+    let (clock, now) = clock(start);
+    let service = ListenService::new();
+    service.set_recorder_env(RecorderEnv {
+        root: root.to_path_buf(),
+        clock,
+    });
+    let ids: Vec<String> = mics.iter().map(|m| m.id.clone()).collect();
+    service.apply(mics);
+    service.apply_recorder(Some(at(start).date()));
+    tokio::spawn(supervise(service.clone()));
+    for id in &ids {
+        wait_for(&service, "a writer per mic", |s| {
+            recording(s, id).state == RecordingState::Waiting
+        })
+        .await;
+    }
+    (service, now)
+}
+
+/// 75 packets of a 1 kHz half-scale tone = 4 frames of 16 kHz (C1a-3: the 4th
+/// completes with the 75th packet).
+async fn send_tone(ready: &ListenStatus) {
+    let vban: std::net::SocketAddr = ready.socket.local_addr.as_deref().unwrap().parse().unwrap();
+    let tx = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
+    let mut encoder = VbanEncoder::new("GUEST_EN", 48_000, 1, VbanSampleType::Int16);
+    for (k, chunk) in tone(48_000, 1_000.0, 0.5, 0, 75 * 256, 1)
+        .chunks(256)
+        .enumerate()
+    {
+        tx.send_to(&encoder.packet(chunk).unwrap(), vban)
+            .await
+            .unwrap();
+        if k % 4 == 3 {
+            tokio::time::sleep(Duration::from_millis(5)).await;
+        }
+    }
+}
+
+#[test]
+fn the_recorder_status_lists_every_configured_mic_and_keeps_the_last_file() {
+    let mics = vec![mic("guest", "GUEST_EN"), mic("interp", "INTERP_SK")];
+    let mut health = ListenHealth::default();
+    let file = "/r/2026-10-18/guest_194205.wav";
+    health.recorder.writer_started("guest");
+    health.recorder.wrote("guest", file, 6_400);
+    assert_eq!(
+        health.recorder.dropped("guest", 22),
+        (1, 22),
+        "the first lag event"
+    );
+    assert_eq!(
+        health.recorder.failed("interp", "zápis nahrávky x zlyhal"),
+        1
+    );
+    assert_eq!(
+        health.recorder.failed("interp", "zápis nahrávky x zlyhal"),
+        2
+    );
+    health.recorder.writer_started("interp"); // a restart keeps the error on show
+    let status = health.snapshot(&mics, Instant::now()).recorder;
+    assert_eq!(
+        status.mics[0],
+        MicRecording {
+            id: "guest".into(),
+            state: RecordingState::Recording,
+            file: Some(file.into()),
+            bytes: 6_400,
+            dropped_frames: 22,
+            error: None,
+        }
+    );
+    assert_eq!(status.mics[1].state, RecordingState::Error);
+    health.end_session(); // the socket closed: the recorder's status stays
+    health.recorder.all_stopped(); // switched off: every mic off, errors forgotten
+    let status = health.snapshot(&mics, Instant::now()).recorder;
+    assert_eq!(
+        (
+            status.mics[0].state,
+            status.mics[0].file.as_deref(),
+            status.mics[0].bytes
+        ),
+        (RecordingState::Off, Some(file), 6_400),
+        "the last file stays on show"
+    );
+    assert_eq!(
+        (status.mics[1].state, status.mics[1].error.clone()),
+        (RecordingState::Off, None)
+    );
+    health.recorder.retain(&["interp"]);
+    let status = health.snapshot(&mics, Instant::now()).recorder;
+    assert_eq!(
+        status.mics[0],
+        MicRecording::off("guest"),
+        "a removed mic's entry goes"
+    );
+}
+
+/// The unit's integration test: real VBAN over loopback UDP → the chain → the
+/// frame channel → the writer → a WAV any reader opens.
+#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
+async fn real_vban_lands_in_todays_wav_exactly_as_the_chain_made_it() {
+    let root = tempfile::tempdir().unwrap();
+    let (clock, _) = clock("2026-10-18T19:42:05");
+    let service = ListenService::new();
+    service.set_recorder_env(RecorderEnv {
+        root: root.path().to_path_buf(),
+        clock,
+    });
+    service.apply(vec![mic("guest", "GUEST_EN")]);
+    service.apply_recorder(Some(day(DAY)));
+    assert!(service.start("127.0.0.1:0".parse().unwrap()));
+    let ready = wait_for(&service, "bound and waiting", |s| {
+        s.socket.state == SocketState::Bound
+            && recording(s, "guest").state == RecordingState::Waiting
+    })
+    .await;
+    send_tone(&ready).await;
+    let status = wait_for(&service, "4 frames recorded", |s| {
+        recording(s, "guest").bytes == 4 * 3_200
+    })
+    .await;
+    // the clock stands at 19:42:05; the 4 frames began at most 0.4 s before it
+    let path = day_file(root.path(), "guest_194204.wav");
+    assert_eq!(day_files(root.path()), vec!["guest_194204.wav"]);
+    let rec = recording(&status, "guest");
+    assert_eq!(rec.state, RecordingState::Recording);
+    assert_eq!(
+        rec.file.as_deref(),
+        Some(path.display().to_string().as_str())
+    );
+    assert!(status.recorder.enabled);
+    let (header, samples) = read_wav(&path);
+    assert_eq!(
+        header,
+        wav_header(12_800).to_vec(),
+        "the header covers the audio before any close"
+    );
+    assert_eq!(samples.len(), 6_400);
+    let level = rms_dbfs(&samples[4_800..]);
+    assert!(
+        (-9.6..=-8.5).contains(&level),
+        "the 4th frame of a 1 kHz half-scale tone ≈ −9.03 dBFS: {level}"
+    );
+    assert_eq!(
+        status.mics[0].frames_out, 4,
+        "every frame the chain made is in the file"
+    );
+}
+
+#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
+async fn the_recorder_is_off_by_default_and_writes_nothing() {
+    let root = tempfile::tempdir().unwrap();
+    let (clock, _) = clock("2026-10-18T19:42:05");
+    let service = ListenService::new();
+    service.set_recorder_env(RecorderEnv {
+        root: root.path().to_path_buf(),
+        clock,
+    });
+    service.apply(vec![mic("guest", "GUEST_EN")]);
+    assert!(service.start("127.0.0.1:0".parse().unwrap()));
+    let ready = wait_for(&service, "bound", |s| s.socket.state == SocketState::Bound).await;
+    send_tone(&ready).await;
+    let status = wait_for(&service, "4 frames made", |s| {
+        s.mics[0].frames_out == 4 && s.mics[0].packets == 75
+    })
+    .await;
+    assert_eq!(
+        (status.recorder.enabled, status.recorder.enabled_for),
+        (false, None)
+    );
+    assert_eq!(recording(&status, "guest").state, RecordingState::Off);
+    assert!(!root.path().join(DAY).exists(), "nothing written");
+}
+
+#[tokio::test]
+async fn a_restart_the_same_day_records_into_a_new_file_and_keeps_the_old_one() {
+    let root = tempfile::tempdir().unwrap();
+    // one-frame batches begin 0.1 s before the clock: at 19:42:05.000
+    let start = "2026-10-18T19:42:05.100";
+    let (service, _) = recording_service(root.path(), start, vec![mic("guest", "GUEST_EN")]).await;
+    send(&service, "guest", 100);
+    wait_for(&service, "first file", |s| {
+        recording(s, "guest").bytes == 3_200
+    })
+    .await;
+    service.apply_recorder(None);
+    wait_for(&service, "switched off", |s| {
+        recording(s, "guest").state == RecordingState::Off
+    })
+    .await;
+    service.apply_recorder(Some(day(DAY))); // a new writer, in the same second
+    wait_for(&service, "a new writer", |s| {
+        recording(s, "guest").state == RecordingState::Waiting
+    })
+    .await;
+    send(&service, "guest", -100);
+    let second = day_file(root.path(), "guest_194205-2.wav")
+        .display()
+        .to_string();
+    wait_for(&service, "second file", |s| {
+        recording(s, "guest").file.as_deref() == Some(second.as_str())
+    })
+    .await;
+    let (header, samples) = read_wav(&day_file(root.path(), "guest_194205.wav"));
+    assert_eq!(
+        (header, samples.len(), samples[0]),
+        (wav_header(3_200).to_vec(), 1_600, 100),
+        "the first file: closed and untouched"
+    );
+    let (header, samples) = read_wav(Path::new(&second));
+    assert_eq!((header, samples[0]), (wav_header(3_200).to_vec(), -100));
+}
+
+#[tokio::test]
+async fn a_writer_that_falls_behind_loses_the_oldest_frames_and_counts_them() {
+    let capture = Capture::install();
+    let root = tempfile::tempdir().unwrap();
+    let start = "2026-10-18T19:42:05.100";
+    let (service, _) = recording_service(root.path(), start, vec![mic("guest", "GUEST_EN")]).await;
+    let frames = service.frame_sender("guest");
+    // a disk stall: 15 s of frames while the writer cannot run (nothing is
+    // awaited in between on this current-thread runtime); every send returns at once
+    for k in 0..150i16 {
+        frames.send(frame(k)).unwrap();
+    }
+    wait_for(
+        &service,
+        "the channel's 12.8 s written, the rest counted",
+        |s| {
+            let rec = recording(s, "guest");
+            rec.bytes == 128 * 3_200 && rec.dropped_frames == 22
+        },
+    )
+    .await;
+    // the 128 frames left began 12.8 s before the clock: 19:41:52.300
+    assert_eq!(day_files(root.path()), vec!["guest_194152.wav"]);
+    let (_, samples) = read_wav(&day_file(root.path(), "guest_194152.wav"));
+    assert_eq!(samples[0], 22, "the oldest 22 frames went");
+    assert_eq!(samples[samples.len() - 1], 149, "the newest are all there");
+    assert_eq!(capture.events_with_message("recorder fell behind").len(), 1);
+}
+
+#[tokio::test]
+async fn the_recording_ends_at_midnight_and_no_writer_starts_the_next_day() {
+    let root = tempfile::tempdir().unwrap();
+    let mics = vec![mic("guest", "GUEST_EN")];
+    let start = "2026-10-18T23:59:58.100";
+    let (service, now) = recording_service(root.path(), start, mics.clone()).await;
+    send(&service, "guest", 1);
+    wait_for(&service, "before midnight", |s| {
+        recording(s, "guest").bytes == 3_200
+    })
+    .await;
+    *now.lock().unwrap() = at("2026-10-19T00:00:01.100");
+    send(&service, "guest", 2);
+    wait_for(&service, "the day ended", |s| {
+        recording(s, "guest").state == RecordingState::Off
+    })
+    .await;
+    service.apply(mics); // a reconcile on the new day starts nothing
+    tokio::time::sleep(Duration::from_millis(100)).await;
+    let frames = service.frame_sender("guest");
+    assert_eq!(
+        frames.receiver_count(),
+        0,
+        "no writer listens on the next day"
+    );
+    let _unheard = frames.send(frame(3));
+    tokio::time::sleep(Duration::from_millis(100)).await;
+    assert!(
+        !root.path().join("2026-10-19").exists(),
+        "nothing recorded after midnight"
+    );
+    let (header, samples) = read_wav(&day_file(root.path(), "guest_235958.wav"));
+    assert_eq!(
+        (header, samples.len()),
+        (wav_header(3_200).to_vec(), 1_600),
+        "closed with its header; the frame after midnight was not written"
+    );
+    let status = service.status().recorder;
+    assert_eq!(
+        (status.enabled, status.enabled_for),
+        (false, Some(day(DAY)))
+    );
+}
+
+#[tokio::test]
+async fn a_mic_removed_from_the_list_stops_its_writer_and_leaves_the_status() {
+    let root = tempfile::tempdir().unwrap();
+    let guest = mic("guest", "GUEST_EN");
+    let mics = vec![guest.clone(), mic("interp", "INTERP_SK")];
+    let start = "2026-10-18T19:42:05.100";
+    let (service, _) = recording_service(root.path(), start, mics).await;
+    send(&service, "interp", 5);
+    wait_for(&service, "interp records", |s| {
+        recording(s, "interp").bytes == 3_200
+    })
+    .await;
+    service.apply(vec![guest]);
+    let interp = service.frame_sender("interp");
+    wait_until("interp's writer ended", || interp.receiver_count() == 0).await;
+    let status = service.status();
+    assert_eq!(status.recorder.mics.len(), 1);
+    assert_eq!(
+        recording(&status, "guest").state,
+        RecordingState::Waiting,
+        "the other mic records on"
+    );
+    let (header, _) = read_wav(&day_file(root.path(), "interp_194205.wav"));
+    assert_eq!(header, wav_header(3_200).to_vec());
+}
+
+/// One failed write in a writer: wait for it to listen, send a frame, wait
+/// for it to fail and end.
+async fn fail_once(frames: &broadcast::Sender<MicFrame>) {
+    wait_until("a writer listens", || frames.receiver_count() == 1).await;
+    frames.send(frame(1)).unwrap();
+    wait_until("the write failed and the writer ended", || {
+        frames.receiver_count() == 0
+    })
+    .await;
+}
+
+/// Let the supervisor collect the ended writer, move the clock past its pause
+/// (C1b-5's back-off), and reconcile again.
+async fn restart_after(
+    service: &ListenService,
+    mics: &[MicConfig],
+    now: &Mutex<NaiveDateTime>,
+    pause: TimeDelta,
+) {
+    service.apply(mics.to_vec());
+    tokio::time::sleep(Duration::from_millis(100)).await;
+    let later = *now.lock().unwrap() + pause + TimeDelta::seconds(1);
+    *now.lock().unwrap() = later;
+    service.apply(mics.to_vec());
+}
+
+#[tokio::test]
+async fn a_failed_write_is_shown_warned_on_power_of_two_failures_and_restarted() {
+    let capture = Capture::install();
+    let dir = tempfile::tempdir().unwrap();
+    let root = dir.path().join("not-a-folder");
+    std::fs::write(&root, b"a file where the folder should be").unwrap();
+    let mics = vec![mic("guest", "GUEST_EN")];
+    let (service, now) = recording_service(&root, "2026-10-18T19:42:05", mics.clone()).await;
+    let frames = service.frame_sender("guest");
+    fail_once(&frames).await;
+    restart_after(&service, &mics, &now, TimeDelta::minutes(1)).await;
+    fail_once(&frames).await;
+    restart_after(&service, &mics, &now, TimeDelta::minutes(2)).await;
+    fail_once(&frames).await;
+    let rec = recording(&service.status(), "guest");
+    assert_eq!(rec.state, RecordingState::Error);
+    assert!(
+        rec.error
+            .as_deref()
+            .is_some_and(|e| e.starts_with("zápis nahrávky ")),
+        "{rec:?}"
+    );
+    let warned: Vec<String> = capture
+        .events_with_message("recording write failed")
+        .iter()
+        .filter_map(|line| line.field("failures").map(str::to_string))
+        .collect();
+    assert_eq!(
+        warned,
+        vec!["1", "2"],
+        "3 failures in a row: 2 lines, not 3"
+    );
+    std::fs::remove_file(&root).unwrap(); // the disk is fine again
+    restart_after(&service, &mics, &now, TimeDelta::minutes(4)).await;
+    wait_until("a writer listens", || frames.receiver_count() == 1).await;
+    frames.send(frame(7)).unwrap();
+    let status = wait_for(&service, "recording again", |s| {
+        recording(s, "guest").state == RecordingState::Recording
+    })
+    .await;
+    assert_eq!(
+        recording(&status, "guest").error,
+        None,
+        "a good write clears the error"
+    );
+}
```

Tests in this task (all with a temp folder and the settable clock):
- `the_recorder_status_lists_every_configured_mic_and_keeps_the_last_file` — `RecorderHealth` unit test.
- `real_vban_lands_in_todays_wav_exactly_as_the_chain_made_it` — the real chain: VBAN over loopback UDP → 16 kHz frames → `guest_194204.wav` (the clock stands at 19:42:05, the frames began ≤ 0.4 s before), header covering the 12 800 bytes BEFORE any close, the 4th frame ≈ −9.03 dBFS (the C1a tone level), `framesOut` = 4.
- `the_recorder_is_off_by_default_and_writes_nothing` — the same audio with the switch off: no day folder, every mic `off`.
- `a_restart_the_same_day_records_into_a_new_file_and_keeps_the_old_one` (Review Focus 1) — a second service on the same folder writes `guest_194205.wav` next to the first, whose bytes are unchanged.
- `a_writer_that_falls_behind_loses_the_oldest_frames_and_counts_them` (Review Focus 2) — 150 frames sent before the writer runs: one file `guest_194152.wav`, `droppedFrames` 22, one WARN.
- `the_recording_ends_at_midnight_and_no_writer_starts_the_next_day` (Review Focus 3).
- `a_mic_removed_from_the_list_stops_its_writer_and_leaves_the_status`.
- `a_failed_write_is_shown_warned_on_power_of_two_failures_and_restarted` (Review Focus 2) — the recordings folder is a FILE, so `create` fails: state `error`, the Slovak text, WARNs at failures 1 and 2 (not 3), and a restart once `restart_after` moves the clock (this lane: the next tick; lane 5: after the back-off — the helper moves the clock far enough for both).

- [ ] **Step 2: Gates** — `cargo fmt --all -- --check`; `QC_TARGETS="$(printf '%s\n' crates/presenter-server/src/listen/{recorder,health,mod,recording_file}.rs)" python3 scripts/dev/fn_length_check.py .`; `bash scripts/dev/count_prod_lines.sh crates/presenter-server/src/listen/recorder.rs` → `477`, `health.rs` → `650`; `python3 scripts/dev/no_panic_check.py .`.

- [ ] **Step 3: Commit** — the four paths above, message `feat(listen): record each microphone's frames while the switch names today (#852 C1b-4)`.

### Task 16: Docs, playbook, then close lane C1b-4

**Files:**
- Modify: `.claude/rules/listen-vban.md`, `docs/configuration.md`, `CLAUDE.md` (router row)

- [ ] **Step 1: Apply the docs** (patch recipe):

```diff
diff --git a/.claude/rules/listen-vban.md b/.claude/rules/listen-vban.md
index b4f1671..bc912ba 100644
--- a/.claude/rules/listen-vban.md
+++ b/.claude/rules/listen-vban.md
@@ -142,6 +142,19 @@ paths:
   on/off flag. `ListenStatus.recorder` (`RecorderStatus`): `enabled` (records today), `enabledFor`,
   `root`, and one `MicRecording` per configured mic (`off` / `waiting` / `recording` / `error`, file,
   bytes, `droppedFrames`, Slovak `error`).
+- **Writers** (#852 C1b-4, server `listen/recorder.rs`): while the toggle names today,
+  `Writers::reconcile` keeps ONE writer per configured mic (on every mic-list / toggle change and every
+  60 s tick). A writer subscribes (`subscribe_frames`) BEFORE the mic is marked `waiting`, takes up to
+  50 frames per batch and writes each batch with ONE `spawn_blocking` — never a file op on the runtime,
+  never a wait in the chain. The file opens with the first frame (a mic without audio leaves no file):
+  `RecordingFile::create` makes the day folder, refuses a linked one, opens `create_new` only (a taken
+  name or a symlink at it → the next `-n`; the 10th in one second is refused, never overwritten). A
+  batch began `now − (its frames + the frames still queued) × 100 ms`. The header sizes are patched
+  after EVERY batch (a crash leaves a file readable up to its last batch), `fsync` on close; a failed
+  write changes no size. `Lagged(n)` → `droppedFrames` += n, WARN on power-of-two lag events; a failed
+  write → state `error` + Slovak text, WARN on power-of-two failures in a row, the writer ends.
+  Switching off or removing the mic closes the file (INFO `recording stopped`: bytes + why) and the
+  status entry. `RecorderHealth` lives in `ListenHealth` but is not part of the socket session.
 - **Routing day (owner action, spec §20, after C1b):** the owner routes `GUEST_EN` / `INTERP_SK`
   (48 kHz, 16-bit, after the channel mute) to 10.77.9.205:6980; then the two mics are saved (the C1b
   card, or `PUT /listen/mics` with the spec §5.6 list) and the box acceptance runs (spec §5.8): status
diff --git a/CLAUDE.md b/CLAUDE.md
index 80e50e0..df4f7b6 100644
--- a/CLAUDE.md
+++ b/CLAUDE.md
@@ -60,7 +60,7 @@ This file provides guidance to Claude Code (claude.ai/code) when working with co
 | Remote Bible translation NLT (#826): `eng-nlt` fetched on demand from Tyndale's NLT API (`api.nlt.to`), never stored/bulk-copied/committed; ONE dispatch in `state/bible_source.rs` (never call the repository's Bible reads elsewhere); books/chapters + book names from `eng-kjv`, listed last and only with eng-kjv; ≤50-verse chunks, the API clamps out-of-range requests (filter!), `1Thes`/`1Jn` abbreviations; `RemoteBibleError` → 503/502 "NLT nedostupné" for NLT as main, while an unavailable NLT SECONDARY loads main-only + a `warning` toast (30 s fail-fast after the API was unreachable); `NLT_LONGER_CHAPTERS` extends the KJV structure (3 John 1, Revelation 12) | `.claude/rules/bible-remote-translations.md` (auto-loads on `bible_remote/**`, `state/bible_source*`, `state/bible.rs`) |
 | AI backend config (#761, bundled proxy removed #762): OpenRouter via env; `PRESENTER_AI_*` WIN over the DB `ai-settings` row on the EFFECTIVE path only (`resolve_effective_settings` + `router/ai_env.rs`), never the raw persist/display path; effective `apiUrl` = env → DB → default `https://openrouter.ai/api/v1` (`DEFAULT_AI_API_URL`); deploy writes 0600 `/etc/presenter/ai.env` (`EnvironmentFile=-`) from `OPENROUTER_API_KEY`+`AI_MODEL` in all three workflows; `DEFAULT_AI_MODEL`=OpenRouter slug; attribution headers in `ai/client.rs` | `.claude/rules/ai-backend-config.md` (auto-loads on `router/ai.rs`, `router/ai_env.rs`, `ai/mod.rs`, `ai/client.rs`, the 3 deploy workflows, the 2 units) |
 | Šepkár tips (#852 S1a/S1b): site-local `interpreter_tips` (no sync), rules + limits once in core `interpreter_tips.rs`, state-layer `TipRefusal` → 422 / 429 + Retry-After, duplicate = pair + its note (or the note alone) within 2 min not dismissed, create = one at a time (`create_lock`), full-list `LiveEvent::InterpreterTips` after every write + the minute boundary task (publishes serialized), injected `TipClock`, retention by `service_date` | `.claude/rules/interpreter-tips.md` (auto-loads on core `interpreter_tips.rs`, the migration, the entity, repository/state/router `interpreter_tips*`, the two pages, `components/interpreter/**`, `ws/passive.rs`, `state/interpreter.rs`, the two stylesheets, the spec) |
-| Listening ingest (#852 C1a): VBAN codec ported from songplayer `vban_packet.rs` @ dd551b7d + typed-reject decode, mic list ships EMPTY (no socket until saved, validate mode never binds, startup read under the save lock), the receive loop never waits (drop-oldest chain queue), sender IP parsed once and peer IP canonicalised, counter rule gap ≤ 1 s = silence / late = drop / > 1 s = resync, a second sender locked out until the first is silent 3 s, a rename keeps the mic and its chain (generation-gated chain writes), rubato `=5.0.1` `Fft` 20 ms chunks → 16 kHz frames counted with their level and sent to ONE never-waiting `broadcast` per mic id (C1b-2, 128 frames), WARN gates read the health counters, whole-list `GET|PUT /listen/mics`, quiet `GET /listen/status`; recorder (C1b): `<root>/<YYYY-MM-DD>/<mic>_<HHMMSS>.wav`, root env or `recordings` next to the DB, 30-day retention once per local date (own files only, never a link), per-DAY toggle `listen.recorder` `{enabledFor}` via `PUT /listen/recorder` (a restart resumes only the same day; audited) | `.claude/rules/listen-vban.md` (auto-loads on core `audio/**` + `listen/**`, server `listen/**`, `state/listen.rs`, `router/listen.rs`, `scripts/dev/listen/**`) |
+| Listening ingest (#852 C1a): VBAN codec ported from songplayer `vban_packet.rs` @ dd551b7d + typed-reject decode, mic list ships EMPTY (no socket until saved, validate mode never binds, startup read under the save lock), the receive loop never waits (drop-oldest chain queue), sender IP parsed once and peer IP canonicalised, counter rule gap ≤ 1 s = silence / late = drop / > 1 s = resync, a second sender locked out until the first is silent 3 s, a rename keeps the mic and its chain (generation-gated chain writes), rubato `=5.0.1` `Fft` 20 ms chunks → 16 kHz frames counted with their level and sent to ONE never-waiting `broadcast` per mic id (C1b-2, 128 frames), WARN gates read the health counters, whole-list `GET|PUT /listen/mics`, quiet `GET /listen/status`; recorder (C1b): `<root>/<YYYY-MM-DD>/<mic>_<HHMMSS>.wav`, root env or `recordings` next to the DB, 30-day retention once per local date (own files only, never a link), per-DAY toggle `listen.recorder` `{enabledFor}` via `PUT /listen/recorder` (a restart resumes only the same day; audited), one writer per mic: lazy file, ≤ 50-frame batches in `spawn_blocking`, header patched per batch, `create_new` only, lag counted + gated WARN | `.claude/rules/listen-vban.md` (auto-loads on core `audio/**` + `listen/**`, server `listen/**`, `state/listen.rs`, `router/listen.rs`, `scripts/dev/listen/**`) |
 | Request log + `presenter::bible::show` (#850 O1): TraceLayer span on tower-http's `make_span` target, quiet poll/asset paths = no span, 5xx = WARN, client strings quoted, client IP = the #793 rule, `sqlx=warn` in every default `RUST_LOG`, drop-ins override `RUST_LOG` (dev's `gst-debug.conf` hides request lines; deploy.yml/release.yml fail when the running service's `RUST_LOG` is not the unit's), timer-polled GETs quiet (writes on the same paths stay logged), a silent WHEP poll (POST 204) has no response line, the request span prefixes every handler line, `src/tracing_capture.rs` = the ONE log-test helper | `.claude/rules/observability.md` (auto-loads on `router/request_log.rs`, `tracing_capture.rs`, `router/integrations/ndi_whep.rs`, `router.rs`, `state/bible_show_log.rs`, `main.rs`, the units, `deploy.yml`, `release.yml`) |
 | Cloudflare Access reconcile (#850 §9): config outside the public repo, dry run first (4× `ok`), never deletes, `update_app` unverified live | `.claude/rules/cloudflare-access.md` (auto-loads on `ops/cloudflare/**`) |
 | Bible reference resolver (#850 A): generated name table (never hand-edit; generator `--check` in CI), Python↔Rust fold parity via `fold-cases.json`, data decisions (Ab = Habakkuk, P.Š. no-fold, Mr); A1b: structure filter, SHORT_DEFAULTS + `<=` ambiguity, corpus alternatives rule | `.claude/rules/bible-resolver.md` (auto-loads on `bible/resolver/**`, `canonical.rs`, the generator, `data/bible-book-names/**`) |
diff --git a/docs/configuration.md b/docs/configuration.md
index 4d17fd8..69ebb04 100644
--- a/docs/configuration.md
+++ b/docs/configuration.md
@@ -112,6 +112,9 @@ default and switched per day (`PUT /listen/recorder` `{"enabled": true}` stores
 `listen.recorder`; Settings → "Počúvanie" → "Nahrávať dnes"): it records only on that date, so a
 restart the same day resumes and the switch turns itself off at midnight. `GET /listen/status` →
 `recorder` shows the switch, the folder and each microphone's file, bytes, dropped frames and error.
+While it is on, every microphone that receives audio is written to
+`<folder>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav` (16 kHz mono 16-bit, exactly what the speech recognition
+gets; ≈ 115 MB per hour), named by the time its first sample was heard.
 
 | Variable                          | Default                               | Purpose |
 | --------------------------------- | ------------------------------------- | ------- |
```

- [ ] **Step 2: Commit** — paths `.claude/rules/listen-vban.md docs/configuration.md CLAUDE.md`, message `docs(listen): the recorder's writers (#852 C1b-4) [no-test: docs only]`.

- [ ] **Step 3: Measure the lane** — `bash "$SCRATCH/lane_measure.sh"`. Expected:

```text
crates/presenter-server/src/listen/health.rs                             +112
crates/presenter-server/src/listen/mod.rs                                +9
crates/presenter-server/src/listen/recorder.rs                           +343
crates/presenter-server/src/listen/recording_file.rs                     +130
lane total: 594 production lines added or changed
```

The tightest lane: a fix commit that adds more than ~6 production lines here must move something to lane 5 instead.

- [ ] **Step 4: Local gates** (Global Constraints).

- [ ] **Step 5: Push and watch CI** — every job; `listen::recorder::tests` binds loopback UDP sockets and writes temp files.

- [ ] **Step 6: Verify on dev after deploy-dev** (dev only; the switch is never touched on SNV / PP): `curl -s "$DEV/listen/status" | python3 -m json.tool | grep -A3 '"recorder"'` → off, `mics: []` (dev has no mic). Then one real recording on dev with the C1a test sender, switched on and off within a minute, and removed again:

```bash
curl -s -X PUT "$DEV/listen/mics" -H 'content-type: application/json' \
  -d '[{"id":"guest","name":"Test","streamName":"TEST_EN","senderIp":null,"channel":"left","language":"en","role":"guest"}]'
curl -s -X PUT "$DEV/listen/recorder" -H 'content-type: application/json' -d '{"enabled":true}'
python3 scripts/dev/listen/vban_send.py --to "$DEV_HOST:6980" --stream TEST_EN --seconds 10
curl -s "$DEV/listen/status" | python3 -c 'import json,sys; print(json.load(sys.stdin)["recorder"]["mics"])'
curl -s -X PUT "$DEV/listen/recorder" -H 'content-type: application/json' -d '{"enabled":false}'
ls -l /opt/presenter-dev/recordings/*/
python3 -c 'import wave,glob,sys; f=wave.open(sorted(glob.glob("/opt/presenter-dev/recordings/*/guest_*.wav"))[-1]); print(f.getnchannels(), f.getsampwidth(), f.getframerate(), f.getnframes())'
curl -s -X PUT "$DEV/listen/mics" -H 'content-type: application/json' -d '[]'
```

Expected: state `recording` with growing bytes while sending, one `guest_<HHMMSS>.wav`, `1 2 16000 ≈160000` frames (10 s), the log lines themselves are pinned by the unit tests (dev's `gst-debug.conf` hides the `presenter::listen::*` targets, listen-vban rule). Leave the test WAV (it expires with the retention) or delete it — it is our own test tone on dev. The PUTs write audit rows on dev only.

- [ ] **Step 7: Close.** `playbook-review`; comment on #852: lane C1b-4 merged (writers), next lane C1b-5.

---

## PR unit C1b-5 — continuous files, the midnight split, the back-off (118 lines)

Starts from `origin/dev` with C1b-4 merged. Makes every file one continuous stretch of audio named by its start (Y2), keeps the frames of a batch heard before midnight (B5), backs a failed writer off 1, 2, 4 … 60 min (B1) and removes a new file whose header could not be written (B1). Server only; no wire change.

### Task 17: Version check

**Files:** `Cargo.toml`, `Cargo.lock` (Task 1 recipe).

- [ ] **Step 1:** Run Task 1 Step 1. `OK` → done. `BUMP` → Task 1 Steps 2–3.

### Task 18: A new file on a drift, the midnight split, the restart back-off, no empty files

**Files:**
- Modify: `crates/presenter-server/src/listen/recorder.rs` (`drifted`, `split_at_midnight`, `restart_delay`, `Writers::retry_at`, `Ended::Gap`, `run_writer`), `crates/presenter-server/src/listen/health.rs` (`RecorderHealth::failures`), `crates/presenter-server/src/listen/recording_file.rs` (`RecordingFile::begin`), `crates/presenter-server/src/listen/recorder/tests.rs`

**Interfaces** (all private to `listen::recorder` unless noted):
- `fn drifted(file_start: NaiveDateTime, audio_bytes: u32, batch_start: NaiveDateTime) -> bool` — `batch_start − (file_start + audio_bytes / 32 ms) > 1 s` (`MAX_DRIFT_MS`). Exactly 1 s keeps the file; audio AHEAD of the clock (a sender catching up) keeps it too. There is no gap-between-batches rule (no `ROTATE_AFTER_GAP`): 0.1–2 s sender pauses would otherwise either split constantly or add up unseen.
- `fn split_at_midnight(batch: Vec<MicFrame>, start: NaiveDateTime, day: NaiveDate) -> (Vec<MicFrame>, bool)` — frame `i` is heard at `start + i × 100 ms`; frames heard after `day` are dropped, `true` = some were (the writer then ends `DayOver` after writing the rest). An all-after-midnight batch writes nothing and ends.
- `fn restart_delay(failures: u64) -> TimeDelta` — 1, 2, 4, 8, 16, 32, 60, 60 … minutes; `Writers::retry_at: HashMap<String, NaiveDateTime>` (wall clock, so tests move the settable clock), cleared when switched off and for removed mics; the "write failed" WARN carries `retry_in_min`.
- `RecorderHealth::failures(&str) -> u64` (`pub(crate)`).
- `RecordingFile::begin(path, File)` (private) writes the header of a just-created file and removes the file again when that fails (a full disk leaves no empty file); `create` calls it.
- On a drift the open file is closed (`recording stopped`, why `a gap in the audio`) and the batch opens a new file named at its own start; `recording started` carries `started` = `%H:%M:%S%.3f`.

- [ ] **Step 1: Apply** (patch recipe):

```diff
diff --git a/crates/presenter-server/src/listen/health.rs b/crates/presenter-server/src/listen/health.rs
index e11c578..79ba2ab 100644
--- a/crates/presenter-server/src/listen/health.rs
+++ b/crates/presenter-server/src/listen/health.rs
@@ -178,7 +178,8 @@ struct MicRecorder {
     /// Times the writer fell behind: the #484 gate of the "fell behind" WARN.
     lag_events: u64,
     error: Option<String>,
-    /// Failed writes in a row: the #484 gate of the "write failed" WARN.
+    /// Failed writes in a row: the #484 gate of the "write failed" WARN and the
+    /// restart pause (1, 2, 4 … 60 min).
     failures: u64,
 }
 
@@ -241,6 +242,10 @@ impl RecorderHealth {
         self.mics.values_mut().for_each(MicRecorder::stop);
     }
 
+    pub(crate) fn failures(&self, mic_id: &str) -> u64 {
+        self.mics.get(mic_id).map_or(0, |mic| mic.failures)
+    }
+
     /// Only configured mics keep an entry.
     pub(crate) fn retain(&mut self, mic_ids: &[&str]) {
         self.mics.retain(|id, _| mic_ids.contains(&id.as_str()));
diff --git a/crates/presenter-server/src/listen/recorder.rs b/crates/presenter-server/src/listen/recorder.rs
index 0649112..69f9e17 100644
--- a/crates/presenter-server/src/listen/recorder.rs
+++ b/crates/presenter-server/src/listen/recorder.rs
@@ -3,12 +3,11 @@
 //! `<root>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav` (`recording_file`). A writer per
 //! mic reads the mic's frame channel and never slows its chain (12.8 s behind,
 //! the oldest frames go and are counted); each batch (≤ 5 s) is written in
-//! `spawn_blocking`. A file opens with the mic's first frame (a mic without
-//! audio leaves no file) and is named by the time that frame was heard; the
-//! writer ends at the first frame after midnight; a failed write ends it too,
-//! and the supervisor starts it again. The day folders older than 30 days go
-//! once per local date. (C1b-5 adds a new file when the audio drifts from the
-//! wall clock, the midnight split of a batch and the restart back-off.)
+//! `spawn_blocking`. Every file is continuous audio from the time in its name:
+//! a batch that begins more than 1 s after the end of the file's audio (a pause
+//! of the sender, a resync, frames lost) starts a new file. Frames heard after
+//! midnight are not written; a failed write ends the writer, restarted after
+//! 1, 2, 4 … 60 min. The day folders older than 30 days go once per local date.
 //!
 //! The folder: `PRESENTER_LISTEN_RECORDINGS_DIR`, else `recordings` next to the
 //! sqlite file of `PRESENTER_DB_URL` (prod / PP `/opt/presenter/recordings`;
@@ -43,8 +42,14 @@ const RECORDINGS_DIR_NAME: &str = "recordings";
 const RECONCILE_EVERY: Duration = Duration::from_secs(60);
 /// At most this many frames (5 s) per blocking write.
 const MAX_BATCH: usize = 50;
-/// One frame is 100 ms.
+/// One frame is 100 ms; one millisecond of audio is 32 bytes (16 kHz mono s16).
 const FRAME_MS: i64 = 100;
+const BYTES_PER_MS: i64 = 32;
+/// A batch that begins more than this after the end of the file's audio
+/// starts a new file.
+const MAX_DRIFT_MS: i64 = 1_000;
+/// The longest pause before a failed writer is restarted.
+const RESTART_CAP_MINUTES: i64 = 60;
 
 /// The server's local wall time (tests inject a settable one).
 pub(crate) type Clock = Arc<dyn Fn() -> NaiveDateTime + Send + Sync>;
@@ -102,6 +107,42 @@ fn frames_duration(frames: usize) -> TimeDelta {
     TimeDelta::milliseconds(frames.saturating_mul(FRAME_MS))
 }
 
+/// Whether a batch that began at `batch_start` cannot follow the file's audio
+/// without a gap of more than 1 s: the file's first sample was heard at
+/// `file_start` and it holds `audio_bytes`.
+fn drifted(file_start: NaiveDateTime, audio_bytes: u32, batch_start: NaiveDateTime) -> bool {
+    let audio = TimeDelta::milliseconds(i64::from(audio_bytes) / BYTES_PER_MS);
+    let audio_end = file_start
+        .checked_add_signed(audio)
+        .unwrap_or(NaiveDateTime::MAX);
+    batch_start.signed_duration_since(audio_end) > TimeDelta::milliseconds(MAX_DRIFT_MS)
+}
+
+/// The frames of a batch that began at `start` heard on `day`, and whether
+/// frames from after midnight were left out.
+fn split_at_midnight(
+    mut batch: Vec<MicFrame>,
+    start: NaiveDateTime,
+    day: NaiveDate,
+) -> (Vec<MicFrame>, bool) {
+    let keep = (0..batch.len())
+        .take_while(|index| {
+            let heard = start.checked_add_signed(frames_duration(*index));
+            heard.is_some_and(|heard| heard.date() <= day)
+        })
+        .count();
+    let cut = keep < batch.len();
+    batch.truncate(keep);
+    (batch, cut)
+}
+
+/// The pause before a writer is restarted after `failures` failed writes in a
+/// row: 1, 2, 4 … 60 minutes.
+fn restart_delay(failures: u64) -> TimeDelta {
+    let shift = u32::try_from(failures.saturating_sub(1).min(16)).unwrap_or(16);
+    TimeDelta::minutes((1i64 << shift).min(RESTART_CAP_MINUTES))
+}
+
 /// A running writer and the way to stop it.
 struct Writer {
     stop: oneshot::Sender<()>,
@@ -133,15 +174,18 @@ impl Writer {
     }
 }
 
-/// The supervisor's writers.
+/// The supervisor's writers, and when each failed one may start again.
 #[derive(Default)]
 struct Writers {
     running: HashMap<String, Writer>,
+    /// A mic whose writer failed is not restarted before this wall time.
+    retry_at: HashMap<String, NaiveDateTime>,
 }
 
 impl Writers {
-    /// One writer per configured mic while recording `day`, none otherwise; a
-    /// writer that ended (a failed write, its day over) is replaced.
+    /// One writer per configured mic while recording `day`, none otherwise. A
+    /// writer that ended is collected; a failed one starts again only after its
+    /// back-off.
     async fn reconcile(
         &mut self,
         service: &ListenService,
@@ -151,6 +195,7 @@ impl Writers {
     ) {
         let configured: Vec<&str> = mics.iter().map(|mic| mic.id.as_str()).collect();
         let wanted: &[&str] = if day.is_some() { &configured } else { &[] };
+        let now = env.now();
         let ended: Vec<String> = self
             .running
             .iter()
@@ -161,17 +206,28 @@ impl Writers {
             if let Some(writer) = self.running.remove(&id) {
                 writer.stop().await;
             }
+            let failures = service.with_health(|health| health.recorder.failures(&id));
+            if failures > 0 {
+                let at = now.checked_add_signed(restart_delay(failures));
+                self.retry_at.insert(id, at.unwrap_or(NaiveDateTime::MAX));
+            }
         }
+        self.retry_at
+            .retain(|id, _| configured.contains(&id.as_str()));
         service.with_health(|health| health.recorder.retain(&configured));
         let Some(day) = day else {
+            self.retry_at.clear();
             service.with_health(|health| health.recorder.all_stopped());
             return;
         };
         for id in wanted {
-            if !self.running.contains_key(*id) {
-                let writer = Writer::start(service, id, day, &env.root);
-                self.running.insert((*id).to_string(), writer);
+            let backing_off = self.retry_at.get(*id).is_some_and(|at| now < *at);
+            if self.running.contains_key(*id) || backing_off {
+                continue;
             }
+            self.retry_at.remove(*id);
+            let writer = Writer::start(service, id, day, &env.root);
+            self.running.insert((*id).to_string(), writer);
         }
     }
 
@@ -235,6 +291,7 @@ impl OpenFile {
 #[derive(Debug, Clone, Copy, PartialEq, Eq)]
 enum Ended {
     Stopped,
+    Gap,
     DayOver,
     Failed,
 }
@@ -243,15 +300,17 @@ impl Ended {
     fn as_str(self) -> &'static str {
         match self {
             Self::Stopped => "switched off or the mic was removed",
+            Self::Gap => "a gap in the audio",
             Self::DayOver => "the day ended",
             Self::Failed => "a write failed",
         }
     }
 }
 
-/// One mic's writer: frames in, batches written off the runtime. Ends when told
-/// to stop, at its first frame after midnight, or on a failed write (the
-/// supervisor starts it again).
+/// One mic's writer: frames in, batches written off the runtime, a new file
+/// whenever the audio drifts more than 1 s from the wall clock. Ends when told
+/// to stop, after its day's last frame, or on a failed write (the supervisor
+/// restarts it after its back-off).
 async fn run_writer(
     service: ListenService,
     job: WriterJob,
@@ -277,8 +336,17 @@ async fn run_writer(
         let behind = frames_duration(batch.len() + frames.len());
         let now = service.recorder_env().now();
         let start = now.checked_sub_signed(behind).unwrap_or(now);
-        if now.date() != job.day {
-            break Ended::DayOver;
+        let (batch, day_over) = split_at_midnight(batch, start, job.day);
+        if batch.is_empty() {
+            break Ended::DayOver; // every frame of it was heard after midnight
+        }
+        if open
+            .as_ref()
+            .is_some_and(|o| drifted(o.started, o.file.data_bytes(), start))
+        {
+            if let Some(gone) = open.take() {
+                close(&job, gone.file, Ended::Gap).await;
+            }
         }
         match write_batch(open.take(), &job, start, batch).await {
             Ok((written, created)) => {
@@ -290,6 +358,9 @@ async fn run_writer(
                 break Ended::Failed;
             }
         }
+        if day_over {
+            break Ended::DayOver;
+        }
     };
     if let Some(last) = open {
         close(&job, last.file, ended).await;
@@ -387,7 +458,8 @@ fn note_failed(service: &ListenService, job: &WriterJob, error: &str) {
             day = %job.day,
             error,
             failures,
-            "recording write failed — the writer starts again within a minute"
+            retry_in_min = restart_delay(failures).num_minutes(),
+            "recording write failed — the writer restarts after a pause"
         );
     }
 }
diff --git a/crates/presenter-server/src/listen/recorder/tests.rs b/crates/presenter-server/src/listen/recorder/tests.rs
index aaf9f97..31299c2 100644
--- a/crates/presenter-server/src/listen/recorder/tests.rs
+++ b/crates/presenter-server/src/listen/recorder/tests.rs
@@ -5,7 +5,7 @@
 
 use super::*;
 use crate::listen::health::ListenHealth;
-use crate::tracing_capture::Capture;
+use crate::tracing_capture::{quoted, Capture};
 use presenter_core::audio::level::rms_dbfs;
 use presenter_core::audio::vban::VbanSampleType;
 use presenter_core::audio::vban_encode::{tone, VbanEncoder};
@@ -628,3 +628,188 @@ async fn a_failed_write_is_shown_warned_on_power_of_two_failures_and_restarted()
         "a good write clears the error"
     );
 }
+
+// ---- continuous files, the midnight split, the back-off (C1b-5) -------------
+
+#[test]
+fn a_batch_more_than_a_second_after_the_files_audio_drifted() {
+    let started = at("2026-10-18T19:42:05");
+    // the file holds 2 s of audio: it ends at 19:42:07
+    assert!(!drifted(started, 64_000, at("2026-10-18T19:42:07.900")));
+    assert!(
+        !drifted(started, 64_000, at("2026-10-18T19:42:08")),
+        "exactly 1 s: kept"
+    );
+    assert!(drifted(started, 64_000, at("2026-10-18T19:42:08.001")));
+    assert!(
+        !drifted(started, 64_000, at("2026-10-18T19:42:01")),
+        "audio ahead: kept"
+    );
+}
+
+#[test]
+fn only_the_frames_heard_before_midnight_stay_in_a_batch() {
+    let batch: Vec<MicFrame> = (0..30).map(frame).collect();
+    let (kept, cut) = split_at_midnight(batch.clone(), at("2026-10-18T23:59:58.100"), day(DAY));
+    assert_eq!(
+        (kept.len(), cut),
+        (19, true),
+        "frame 19 is heard at 00:00:00.000"
+    );
+    assert_eq!(kept[18][0], 18);
+    let (kept, cut) = split_at_midnight(batch, at("2026-10-18T12:00:00"), day(DAY));
+    assert_eq!((kept.len(), cut), (30, false));
+}
+
+#[test]
+fn a_failed_writer_waits_1_2_4_up_to_60_minutes() {
+    let minutes: Vec<i64> = (1..=8).map(|f| restart_delay(f).num_minutes()).collect();
+    assert_eq!(minutes, vec![1, 2, 4, 8, 16, 32, 60, 60]);
+    assert_eq!(restart_delay(u64::MAX).num_minutes(), 60);
+}
+
+#[tokio::test]
+async fn a_pause_of_the_sender_starts_a_new_file_named_by_its_start() {
+    let capture = Capture::install();
+    let root = tempfile::tempdir().unwrap();
+    let start = "2026-10-18T19:42:05.100";
+    let (service, now) =
+        recording_service(root.path(), start, vec![mic("guest", "GUEST_EN")]).await;
+    send(&service, "guest", 1);
+    wait_for(&service, "first frame", |s| {
+        recording(s, "guest").bytes == 3_200
+    })
+    .await;
+    *now.lock().unwrap() = at("2026-10-18T19:42:05.800"); // a 0.6 s pause: the same file
+    send(&service, "guest", 2);
+    wait_for(&service, "second frame", |s| {
+        recording(s, "guest").bytes == 6_400
+    })
+    .await;
+    *now.lock().unwrap() = at("2026-10-18T19:42:07.300"); // a 1.5 s pause: a new file
+    send(&service, "guest", 3);
+    let second = day_file(root.path(), "guest_194207.wav")
+        .display()
+        .to_string();
+    wait_for(&service, "a new file", |s| {
+        recording(s, "guest").file.as_deref() == Some(second.as_str())
+    })
+    .await;
+    let (header, samples) = read_wav(&day_file(root.path(), "guest_194205.wav"));
+    assert_eq!((header, samples[1_600]), (wav_header(6_400).to_vec(), 2));
+    assert_eq!(
+        read_wav(Path::new(&second)).1[0],
+        3,
+        "the audio after the pause is kept"
+    );
+    let started: Vec<String> = capture
+        .events_with_message("recording started")
+        .iter()
+        .filter_map(|line| line.field("started").map(str::to_string))
+        .collect();
+    assert_eq!(
+        started,
+        vec!["19:42:05.000", "19:42:07.200"],
+        "the millisecond start"
+    );
+    let stopped = capture.events_with_message("recording stopped");
+    assert_eq!(stopped.len(), 1);
+    assert_eq!(
+        stopped[0].field("why"),
+        Some(quoted("a gap in the audio").as_str())
+    );
+}
+
+#[tokio::test]
+async fn a_lag_in_the_middle_of_a_file_starts_a_new_one_at_the_right_time() {
+    let root = tempfile::tempdir().unwrap();
+    let start = "2026-10-18T19:42:05.100";
+    let (service, now) =
+        recording_service(root.path(), start, vec![mic("guest", "GUEST_EN")]).await;
+    send(&service, "guest", -1);
+    wait_for(&service, "first frame", |s| {
+        recording(s, "guest").bytes == 3_200
+    })
+    .await;
+    // 15 s later the writer gets 15 s of frames at once (a stall): 22 are lost
+    *now.lock().unwrap() = at("2026-10-18T19:42:20.100");
+    let frames = service.frame_sender("guest");
+    for k in 0..150i16 {
+        frames.send(frame(k)).unwrap();
+    }
+    wait_for(&service, "the rest written", |s| {
+        let rec = recording(s, "guest");
+        rec.bytes == 128 * 3_200 && rec.dropped_frames == 22
+    })
+    .await;
+    // the 128 frames kept began 12.8 s before the clock: 19:42:07.300
+    assert_eq!(
+        day_files(root.path()),
+        vec!["guest_194205.wav", "guest_194207.wav"]
+    );
+    let (_, first) = read_wav(&day_file(root.path(), "guest_194205.wav"));
+    assert_eq!(
+        first.len(),
+        1_600,
+        "the file before the lag ends where its audio ended"
+    );
+    assert_eq!(
+        read_wav(&day_file(root.path(), "guest_194207.wav")).1[0],
+        22
+    );
+}
+
+#[tokio::test]
+async fn a_batch_across_midnight_keeps_only_the_frames_heard_before_it() {
+    let root = tempfile::tempdir().unwrap();
+    let start = "2026-10-18T23:59:58.100";
+    let (service, now) =
+        recording_service(root.path(), start, vec![mic("guest", "GUEST_EN")]).await;
+    send(&service, "guest", -1);
+    wait_for(&service, "first frame", |s| {
+        recording(s, "guest").bytes == 3_200
+    })
+    .await;
+    // 3 s of frames arrive at once at 00:00:01.100: they began at 23:59:58.100
+    *now.lock().unwrap() = at("2026-10-19T00:00:01.100");
+    let frames = service.frame_sender("guest");
+    for k in 0..30i16 {
+        frames.send(frame(k)).unwrap();
+    }
+    wait_for(&service, "the day ended", |s| {
+        recording(s, "guest").state == RecordingState::Off
+    })
+    .await;
+    let (header, samples) = read_wav(&day_file(root.path(), "guest_235958.wav"));
+    assert_eq!(
+        header,
+        wav_header(20 * 3_200).to_vec(),
+        "1 + the 19 frames before midnight"
+    );
+    assert_eq!(samples[samples.len() - 1], 18);
+    assert!(!root.path().join("2026-10-19").exists());
+}
+
+#[tokio::test]
+async fn a_failed_writer_is_not_restarted_before_its_pause() {
+    let dir = tempfile::tempdir().unwrap();
+    let root = dir.path().join("not-a-folder");
+    std::fs::write(&root, b"x").unwrap();
+    let mics = vec![mic("guest", "GUEST_EN")];
+    let (service, now) = recording_service(&root, "2026-10-18T19:42:05", mics.clone()).await;
+    let frames = service.frame_sender("guest");
+    fail_once(&frames).await;
+    service.apply(mics.clone()); // collected: back off 1 min
+    tokio::time::sleep(Duration::from_millis(100)).await;
+    assert_eq!(frames.receiver_count(), 0, "no restart at once");
+    *now.lock().unwrap() = at("2026-10-18T19:42:35");
+    service.apply(mics.clone());
+    tokio::time::sleep(Duration::from_millis(100)).await;
+    assert_eq!(frames.receiver_count(), 0, "30 s later: still backing off");
+    *now.lock().unwrap() = at("2026-10-18T19:43:06");
+    service.apply(mics);
+    wait_until("restarted after its minute", || {
+        frames.receiver_count() == 1
+    })
+    .await;
+}
diff --git a/crates/presenter-server/src/listen/recording_file.rs b/crates/presenter-server/src/listen/recording_file.rs
index ea2dc27..ac1ca5c 100644
--- a/crates/presenter-server/src/listen/recording_file.rs
+++ b/crates/presenter-server/src/listen/recording_file.rs
@@ -6,8 +6,9 @@
 //! started in the same second), 16 kHz mono s16le behind the 44-byte header,
 //! whose two sizes are patched after every batch — a crash leaves a file whose
 //! header covers the audio up to the last batch. A file is only ever created
-//! (`create_new`: an existing name, a symlink included, is never opened) and
-//! never inside a linked day folder.
+//! (`create_new`: an existing name, a symlink included, is never opened), never
+//! inside a linked day folder, and removed again when not even its header can
+//! be written (a full disk).
 //!
 //! Retention deletes only what the recorder makes: day folders named
 //! `YYYY-MM-DD` (canonical spelling) and in them the `<mic id>_<HHMMSS>[-n].wav`
@@ -76,15 +77,7 @@ impl RecordingFile {
         for attempt in 1..=RECORDING_NAMES_PER_SECOND {
             let path = dir.join(recording_file_name(mic_id, started.time(), attempt));
             match OpenOptions::new().write(true).create_new(true).open(&path) {
-                Ok(file) => {
-                    let mut recording = Self {
-                        path,
-                        file,
-                        data_bytes: 0,
-                    };
-                    recording.io(|file| file.write_all(&wav_header(0)))?;
-                    return Ok(recording);
-                }
+                Ok(file) => return Self::begin(path, file),
                 Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                 Err(err) => return Err(io_error(&path, err)),
             }
@@ -94,6 +87,23 @@ impl RecordingFile {
         })
     }
 
+    /// The header of a file just created. When not even that can be written
+    /// (a full disk), the empty file is removed again, best effort.
+    fn begin(path: PathBuf, file: File) -> Result<Self, RecordError> {
+        let mut recording = Self {
+            path,
+            file,
+            data_bytes: 0,
+        };
+        match recording.io(|file| file.write_all(&wav_header(0))) {
+            Ok(()) => Ok(recording),
+            Err(err) => {
+                let _removed = fs::remove_file(&recording.path);
+                Err(err)
+            }
+        }
+    }
+
     /// Append `frames`, then patch the header sizes: a crash after this leaves
     /// a file whose header covers them. A failed write changes no size.
     pub(crate) fn append(&mut self, frames: &[MicFrame]) -> Result<(), RecordError> {
@@ -407,6 +417,16 @@ mod tests {
         assert_eq!(bytes.len(), 44 + 3_200);
     }
 
+    #[test]
+    fn a_header_that_cannot_be_written_leaves_no_file_behind() {
+        let root = tempfile::tempdir().unwrap();
+        let path = root.path().join("guest_194205.wav");
+        fs::write(&path, b"").unwrap();
+        let err = RecordingFile::begin(path.clone(), File::open(&path).unwrap()).unwrap_err();
+        assert!(matches!(err, RecordError::Io { .. }), "{err}");
+        assert!(!path.exists(), "a full disk leaves no empty file");
+    }
+
     #[test]
     fn a_folder_that_cannot_be_made_is_a_slovak_error_naming_it() {
         let dir = tempfile::tempdir().unwrap();
```

Tests in this task:
- pure: `a_batch_more_than_a_second_after_the_files_audio_drifted`, `only_the_frames_heard_before_midnight_stay_in_a_batch` (30 frames from 23:59:58.100 → 19 kept, frame 19 is heard at 00:00:00.000), `a_failed_writer_waits_1_2_4_up_to_60_minutes`;
- `a_header_that_cannot_be_written_leaves_no_file_behind` (`recording_file.rs`: `begin` on a read-only handle → `Io` error, the file is gone);
- writers (Review Focus 5 and 3, B1): `a_pause_of_the_sender_starts_a_new_file_named_by_its_start` (a 0.6 s pause keeps `guest_194205.wav`; the next pause brings the drift to 2 s → `guest_194207.wav`; `started` = `19:42:05.000`, `19:42:07.200`; one `recording stopped` with why `a gap in the audio`), `a_lag_in_the_middle_of_a_file_starts_a_new_one_at_the_right_time` (one frame at 19:42:05, a 15 s stall, 22 frames lost: the first file keeps exactly its 1 600 samples, the second is `guest_194207.wav` starting with frame 22), `a_batch_across_midnight_keeps_only_the_frames_heard_before_it` (`guest_235958.wav` holds 1 + 19 frames, no `2026-10-19` folder), `a_failed_writer_is_not_restarted_before_its_pause` (no subscriber at once nor 30 s later, one after 61 s).
- The lane-4 tests still pass unchanged: their clock stands still while frames arrive in real time, so no drift; `a_failed_write_is_shown_warned_on_power_of_two_failures_and_restarted` moves the clock 1, 2 and 4 minutes (+1 s) between failures — exactly the back-off.

- [ ] **Step 2: Gates** — `cargo fmt --all -- --check`; `QC_TARGETS="$(printf '%s\n' crates/presenter-server/src/listen/{recorder,health,recording_file}.rs)" python3 scripts/dev/fn_length_check.py .` (`run_writer` is the longest function, well under 80); `bash scripts/dev/count_prod_lines.sh crates/presenter-server/src/listen/recorder.rs` → `549`; `python3 scripts/dev/no_panic_check.py .`.

- [ ] **Step 3: Commit** — the four paths above, message `feat(listen): one file per continuous stretch, the midnight split and a restart back-off (#852 C1b-5)`.

### Task 19: Docs, playbook, then close lane C1b-5

**Files:**
- Modify: `.claude/rules/listen-vban.md`, `docs/configuration.md`, `CLAUDE.md` (router row)

- [ ] **Step 1: Apply the docs** (patch recipe):

```diff
diff --git a/.claude/rules/listen-vban.md b/.claude/rules/listen-vban.md
index bc912ba..68d6daf 100644
--- a/.claude/rules/listen-vban.md
+++ b/.claude/rules/listen-vban.md
@@ -155,6 +155,18 @@ paths:
   write → state `error` + Slovak text, WARN on power-of-two failures in a row, the writer ends.
   Switching off or removing the mic closes the file (INFO `recording stopped`: bytes + why) and the
   status entry. `RecorderHealth` lives in `ListenHealth` but is not part of the socket session.
+- **Continuous files** (#852 C1b-5): before each batch, `drifted(file start, data bytes, batch
+  start)` — more than 1 s between the end of the file's audio and the batch's start (a pause of the
+  sender, a resync, a lag, a slow disk) — closes the file (why `a gap in the audio`) and opens a new
+  one named at the batch's start; the `recording started` INFO carries `started` = `%H:%M:%S%.3f`. So
+  every file is continuous audio from the time in its name (the bake-off places the
+  `presenter::bible::show` times by it); never reopen, append to or truncate an existing file, and never
+  rotate on the wall-clock gap between two batches alone (sender pauses of 0.1–2 s add up).
+  `split_at_midnight`: the frames of a batch heard after midnight are dropped and the writer ends; no
+  writer starts until the toggle names the new day. A failed writer restarts after `restart_delay`
+  1, 2, 4 … 60 min (`Writers::retry_at`, wall clock — tests move the settable clock); switching off
+  clears the errors and the pauses. `RecordingFile::begin` removes a new file whose header cannot be
+  written (a full disk): no empty files.
 - **Routing day (owner action, spec §20, after C1b):** the owner routes `GUEST_EN` / `INTERP_SK`
   (48 kHz, 16-bit, after the channel mute) to 10.77.9.205:6980; then the two mics are saved (the C1b
   card, or `PUT /listen/mics` with the spec §5.6 list) and the box acceptance runs (spec §5.8): status
diff --git a/CLAUDE.md b/CLAUDE.md
index df4f7b6..9e7df3f 100644
--- a/CLAUDE.md
+++ b/CLAUDE.md
@@ -60,7 +60,7 @@ This file provides guidance to Claude Code (claude.ai/code) when working with co
 | Remote Bible translation NLT (#826): `eng-nlt` fetched on demand from Tyndale's NLT API (`api.nlt.to`), never stored/bulk-copied/committed; ONE dispatch in `state/bible_source.rs` (never call the repository's Bible reads elsewhere); books/chapters + book names from `eng-kjv`, listed last and only with eng-kjv; ≤50-verse chunks, the API clamps out-of-range requests (filter!), `1Thes`/`1Jn` abbreviations; `RemoteBibleError` → 503/502 "NLT nedostupné" for NLT as main, while an unavailable NLT SECONDARY loads main-only + a `warning` toast (30 s fail-fast after the API was unreachable); `NLT_LONGER_CHAPTERS` extends the KJV structure (3 John 1, Revelation 12) | `.claude/rules/bible-remote-translations.md` (auto-loads on `bible_remote/**`, `state/bible_source*`, `state/bible.rs`) |
 | AI backend config (#761, bundled proxy removed #762): OpenRouter via env; `PRESENTER_AI_*` WIN over the DB `ai-settings` row on the EFFECTIVE path only (`resolve_effective_settings` + `router/ai_env.rs`), never the raw persist/display path; effective `apiUrl` = env → DB → default `https://openrouter.ai/api/v1` (`DEFAULT_AI_API_URL`); deploy writes 0600 `/etc/presenter/ai.env` (`EnvironmentFile=-`) from `OPENROUTER_API_KEY`+`AI_MODEL` in all three workflows; `DEFAULT_AI_MODEL`=OpenRouter slug; attribution headers in `ai/client.rs` | `.claude/rules/ai-backend-config.md` (auto-loads on `router/ai.rs`, `router/ai_env.rs`, `ai/mod.rs`, `ai/client.rs`, the 3 deploy workflows, the 2 units) |
 | Šepkár tips (#852 S1a/S1b): site-local `interpreter_tips` (no sync), rules + limits once in core `interpreter_tips.rs`, state-layer `TipRefusal` → 422 / 429 + Retry-After, duplicate = pair + its note (or the note alone) within 2 min not dismissed, create = one at a time (`create_lock`), full-list `LiveEvent::InterpreterTips` after every write + the minute boundary task (publishes serialized), injected `TipClock`, retention by `service_date` | `.claude/rules/interpreter-tips.md` (auto-loads on core `interpreter_tips.rs`, the migration, the entity, repository/state/router `interpreter_tips*`, the two pages, `components/interpreter/**`, `ws/passive.rs`, `state/interpreter.rs`, the two stylesheets, the spec) |
-| Listening ingest (#852 C1a): VBAN codec ported from songplayer `vban_packet.rs` @ dd551b7d + typed-reject decode, mic list ships EMPTY (no socket until saved, validate mode never binds, startup read under the save lock), the receive loop never waits (drop-oldest chain queue), sender IP parsed once and peer IP canonicalised, counter rule gap ≤ 1 s = silence / late = drop / > 1 s = resync, a second sender locked out until the first is silent 3 s, a rename keeps the mic and its chain (generation-gated chain writes), rubato `=5.0.1` `Fft` 20 ms chunks → 16 kHz frames counted with their level and sent to ONE never-waiting `broadcast` per mic id (C1b-2, 128 frames), WARN gates read the health counters, whole-list `GET|PUT /listen/mics`, quiet `GET /listen/status`; recorder (C1b): `<root>/<YYYY-MM-DD>/<mic>_<HHMMSS>.wav`, root env or `recordings` next to the DB, 30-day retention once per local date (own files only, never a link), per-DAY toggle `listen.recorder` `{enabledFor}` via `PUT /listen/recorder` (a restart resumes only the same day; audited), one writer per mic: lazy file, ≤ 50-frame batches in `spawn_blocking`, header patched per batch, `create_new` only, lag counted + gated WARN | `.claude/rules/listen-vban.md` (auto-loads on core `audio/**` + `listen/**`, server `listen/**`, `state/listen.rs`, `router/listen.rs`, `scripts/dev/listen/**`) |
+| Listening ingest (#852 C1a): VBAN codec ported from songplayer `vban_packet.rs` @ dd551b7d + typed-reject decode, mic list ships EMPTY (no socket until saved, validate mode never binds, startup read under the save lock), the receive loop never waits (drop-oldest chain queue), sender IP parsed once and peer IP canonicalised, counter rule gap ≤ 1 s = silence / late = drop / > 1 s = resync, a second sender locked out until the first is silent 3 s, a rename keeps the mic and its chain (generation-gated chain writes), rubato `=5.0.1` `Fft` 20 ms chunks → 16 kHz frames counted with their level and sent to ONE never-waiting `broadcast` per mic id (C1b-2, 128 frames), WARN gates read the health counters, whole-list `GET|PUT /listen/mics`, quiet `GET /listen/status`; recorder (C1b): `<root>/<YYYY-MM-DD>/<mic>_<HHMMSS>.wav`, root env or `recordings` next to the DB, 30-day retention once per local date (own files only, never a link), per-DAY toggle `listen.recorder` `{enabledFor}` via `PUT /listen/recorder` (a restart resumes only the same day; audited), one writer per mic: lazy file, ≤ 50-frame batches in `spawn_blocking`, header patched per batch, `create_new` only, lag counted + gated WARN; a new file when the audio drifts > 1 s from the wall clock (never on the batch gap alone), frames after midnight dropped, restart back-off 1…60 min | `.claude/rules/listen-vban.md` (auto-loads on core `audio/**` + `listen/**`, server `listen/**`, `state/listen.rs`, `router/listen.rs`, `scripts/dev/listen/**`) |
 | Request log + `presenter::bible::show` (#850 O1): TraceLayer span on tower-http's `make_span` target, quiet poll/asset paths = no span, 5xx = WARN, client strings quoted, client IP = the #793 rule, `sqlx=warn` in every default `RUST_LOG`, drop-ins override `RUST_LOG` (dev's `gst-debug.conf` hides request lines; deploy.yml/release.yml fail when the running service's `RUST_LOG` is not the unit's), timer-polled GETs quiet (writes on the same paths stay logged), a silent WHEP poll (POST 204) has no response line, the request span prefixes every handler line, `src/tracing_capture.rs` = the ONE log-test helper | `.claude/rules/observability.md` (auto-loads on `router/request_log.rs`, `tracing_capture.rs`, `router/integrations/ndi_whep.rs`, `router.rs`, `state/bible_show_log.rs`, `main.rs`, the units, `deploy.yml`, `release.yml`) |
 | Cloudflare Access reconcile (#850 §9): config outside the public repo, dry run first (4× `ok`), never deletes, `update_app` unverified live | `.claude/rules/cloudflare-access.md` (auto-loads on `ops/cloudflare/**`) |
 | Bible reference resolver (#850 A): generated name table (never hand-edit; generator `--check` in CI), Python↔Rust fold parity via `fold-cases.json`, data decisions (Ab = Habakkuk, P.Š. no-fold, Mr); A1b: structure filter, SHORT_DEFAULTS + `<=` ambiguity, corpus alternatives rule | `.claude/rules/bible-resolver.md` (auto-loads on `bible/resolver/**`, `canonical.rs`, the generator, `data/bible-book-names/**`) |
diff --git a/docs/configuration.md b/docs/configuration.md
index 69ebb04..92d0ddb 100644
--- a/docs/configuration.md
+++ b/docs/configuration.md
@@ -114,7 +114,10 @@ restart the same day resumes and the switch turns itself off at midnight. `GET /
 `recorder` shows the switch, the folder and each microphone's file, bytes, dropped frames and error.
 While it is on, every microphone that receives audio is written to
 `<folder>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav` (16 kHz mono 16-bit, exactly what the speech recognition
-gets; ≈ 115 MB per hour), named by the time its first sample was heard.
+gets; ≈ 115 MB per hour), named by the time its first sample was heard. A gap of more than 1 s in the
+audio (the sender paused or the recorder fell behind) starts a new file, so every file is continuous
+audio from the time in its name; recording stops at midnight. A failed write is retried after 1, 2,
+4 … 60 minutes.
 
 | Variable                          | Default                               | Purpose |
 | --------------------------------- | ------------------------------------- | ------- |
```

- [ ] **Step 2: Commit** — paths `.claude/rules/listen-vban.md docs/configuration.md CLAUDE.md`, message `docs(listen): continuous recording files, midnight, back-off (#852 C1b-5) [no-test: docs only]`.

- [ ] **Step 3: Measure the lane** — `bash "$SCRATCH/lane_measure.sh"`. Expected:

```text
crates/presenter-server/src/listen/health.rs                             +6
crates/presenter-server/src/listen/recorder.rs                           +91
crates/presenter-server/src/listen/recording_file.rs                     +21
lane total: 118 production lines added or changed
```

- [ ] **Step 4: Local gates** (Global Constraints).

- [ ] **Step 5: Push and watch CI** — every job.

- [ ] **Step 6: Verify on dev after deploy-dev** — repeat Task 16 Step 6 with a pause in the middle: `vban_send.py … --seconds 5`, wait 5 s (a new sender process has a new source port; C1a accepts it once the first one is silent ≥ 3 s), `vban_send.py … --seconds 5` again, then switch off: TWO `guest_<HHMMSS>.wav` files in today's folder, about 10 s apart in their names, each ≈ 5 s long (`wave` frames ≈ 80 000). Reset the mics to `[]`.

- [ ] **Step 7: Close.** `playbook-review`; comment on #852: lane C1b-5 merged (continuous files, midnight split, back-off), next lane C1b-6.

---

## PR unit C1b-6 — the Počúvanie card: microphones (500 lines)

Starts from `origin/dev` with C1b-5 merged. Settings gets the "Počúvanie" card between Android and Ableton: the microphone list with the shared inline editor (`MicForm`), the server's 422 rule inline, ids from the role shown in each row, delete with a Slovak confirm. No health, no switch yet (lane 7). `GET /listen/mics` becomes a quiet GET (the list card polls it every 5 s).

### Task 20: Version check

**Files:** `Cargo.toml`, `Cargo.lock` (Task 1 recipe).

- [ ] **Step 1:** Run Task 1 Step 1. `OK` → done. `BUMP` → Task 1 Steps 2–3.

### Task 21: The listen client and the quiet microphone list

**Files:**
- Create: `crates/presenter-ui/src/api/listen.rs`
- Modify: `crates/presenter-ui/src/api/mod.rs` (`pub mod listen;`, `put_json_refusal`), `crates/presenter-server/src/router/request_log.rs` (`/listen/mics` in `POLLED_GETS`)

**Interfaces:**
- `pub async fn put_json_refusal<B: Serialize, T: DeserializeOwned>(path, &B) -> Result<T, ApiError>` — like `post_json_refusal`: a non-2xx whose body is the server's JSON error is `ApiError::Refused(status, message)` (the existing `status_with_refusal`).
- `api::listen::{get_mics() -> Result<Vec<MicConfig>, ApiError>, put_mics(&[MicConfig]) -> Result<Vec<MicConfig>, ApiError>}`.
- Request log: `GET /listen/mics` quiet (no span, no response line); its `PUT` stays logged. The existing request-log test changes in two lines because the shape of the polled set changed: `/listen/mics` moves from the "never quiet" list to the "quiet by GET" list, and `/listen/recorder` takes its place in the "never quiet" list.

- [ ] **Step 1: Create `crates/presenter-ui/src/api/listen.rs`:**

```rust
//! Client for the listening API (#852 C1b): the microphones as a whole list.
//! The write uses `put_json_refusal`: a 422 whose body is the server's JSON
//! error is an `ApiError::Refused` carrying the Slovak rule.

use super::{get_json, put_json_refusal, ApiError};
use presenter_core::listen::mics::MicConfig;

pub async fn get_mics() -> Result<Vec<MicConfig>, ApiError> {
    get_json("/listen/mics").await
}

pub async fn put_mics(mics: &[MicConfig]) -> Result<Vec<MicConfig>, ApiError> {
    put_json_refusal("/listen/mics", &mics).await
}
```

- [ ] **Step 2: Apply** (patch recipe):

```diff
diff --git a/crates/presenter-server/src/router/request_log.rs b/crates/presenter-server/src/router/request_log.rs
index be34326..cf63bdf 100644
--- a/crates/presenter-server/src/router/request_log.rs
+++ b/crates/presenter-server/src/router/request_log.rs
@@ -167,11 +167,12 @@ pub(super) fn is_quiet_request(method: &Method, path: &str) -> bool {
 /// presentation list (`pages/tablet.rs`) and the Settings tab's 5 s pollers (the
 /// Resolume and Android lists in `list_card.rs`, AbleSet + OSC status in
 /// `ableton.rs`, NDI source status in `video_sources.rs`) and the listening
-/// microphones' status (`/listen/status`, #852 C1a). Only the GET is
+/// card's microphone list + status (`/listen/mics` 5 s, `/listen/status` 2 s,
+/// #852 C1b). Only the GET is
 /// quiet: the same paths take a person's POST / PUT / DELETE (create, edit,
 /// remove), and those stay logged.
 pub(super) fn is_quiet_get_path(path: &str) -> bool {
-    const POLLED_GETS: [&str; 7] = [
+    const POLLED_GETS: [&str; 8] = [
         "/bible/presentations",
         "/integrations/resolume/hosts",
         "/integrations/android-stage/displays",
@@ -179,6 +180,7 @@ pub(super) fn is_quiet_get_path(path: &str) -> bool {
         "/integrations/osc/status",
         "/integrations/video-sources/status",
         "/listen/status",
+        "/listen/mics",
     ];
     POLLED_GETS.contains(&path)
 }
@@ -359,6 +361,7 @@ mod tests {
             "/integrations/osc/status",
             "/integrations/video-sources/status",
             "/listen/status",
+            "/listen/mics",
         ] {
             assert!(is_quiet_request(&Method::GET, path), "GET {path} is polled");
             assert!(!is_quiet_path(path), "{path} is quiet by GET only");
@@ -382,7 +385,7 @@ mod tests {
             "/integrations/ableset/settings",
             "/integrations/osc/status/extra",
             "/settings/features",
-            "/listen/mics",
+            "/listen/recorder",
         ] {
             assert!(
                 !is_quiet_request(&Method::GET, path),
diff --git a/crates/presenter-ui/src/api/mod.rs b/crates/presenter-ui/src/api/mod.rs
index 0dc9fb0..05f4914 100644
--- a/crates/presenter-ui/src/api/mod.rs
+++ b/crates/presenter-ui/src/api/mod.rs
@@ -2,6 +2,7 @@ pub mod ai;
 pub mod bible;
 pub mod interpreter_tips;
 pub mod libraries;
+pub mod listen;
 pub mod ndi;
 pub mod network;
 pub mod playlists;
@@ -245,6 +246,24 @@ pub async fn post_json_refusal<B: serde::Serialize, T: DeserializeOwned>(
     response.json().await.map_err(ApiError::Deserialize)
 }
 
+/// `put_json`, but a non-2xx whose body is the server's JSON error body is an
+/// [`ApiError::Refused`] carrying that message (see [`status_with_refusal`]).
+pub async fn put_json_refusal<B: serde::Serialize, T: DeserializeOwned>(
+    path: &str,
+    body: &B,
+) -> Result<T, ApiError> {
+    let response = Request::put(&api_url(path))
+        .json(body)
+        .map_err(ApiError::Serialize)?
+        .send()
+        .await
+        .map_err(ApiError::Network)?;
+    if !response.ok() {
+        return Err(status_with_refusal(response).await);
+    }
+    response.json().await.map_err(ApiError::Deserialize)
+}
+
 /// `post_json`, but a non-2xx carries the server error-body message (see
 /// [`status_with_detail`]).
 pub async fn post_json_detail<B: serde::Serialize, T: DeserializeOwned>(
@@ -307,7 +326,8 @@ pub enum ApiError {
     Network(gloo_net::Error),
     Status(u16, String),
     /// A non-2xx whose body was the server's own JSON error body: the text is
-    /// the server's message (only [`post_json_refusal`] produces it).
+    /// the server's message (only [`post_json_refusal`] / [`put_json_refusal`]
+    /// produce it).
     Refused(u16, String),
     Serialize(gloo_net::Error),
     Deserialize(gloo_net::Error),
```

- [ ] **Step 3: Gates** — `cargo fmt --all -- --check && (cd crates/presenter-ui && cargo fmt --check)`; `QC_TARGETS="$(printf '%s\n' crates/presenter-ui/src/api/{mod,listen}.rs crates/presenter-server/src/router/request_log.rs)" python3 scripts/dev/fn_length_check.py .`.

- [ ] **Step 4: Commit** — paths `crates/presenter-ui/src/api/listen.rs crates/presenter-ui/src/api/mod.rs crates/presenter-server/src/router/request_log.rs`, message `feat(listen): the UI client and a quiet microphone list poll (#852 C1b-6)`. (`api::listen` has no caller until Task 22; presenter-ui is clippy-excluded and both commits go out in one push.)

### Task 22: The card — microphones on the shared list card

**Files:**
- Create: `crates/presenter-ui/src/pages/settings/listen.rs`
- Modify: `crates/presenter-ui/src/pages/settings/mod.rs` (`mod listen;`, `<ListenCard toast=toast />` after `<AndroidCard>`), `crates/presenter-ui/styles/settings.css` (`select` gets the shared form-row input style and focus ring — the one form system, no per-card class)

**Interfaces:**
- `MicDraft` (name, stream name, sender IP as typed, channel, language, role) / `MicForm` (`EditorForm`, `Config = ()`; Názov, VBAN stream, IP odosielateľa (nepovinné); Kanál / Jazyk / Rola `<select>`s, `prop:selected` per option — the #827 rule); `validate_mic` checks only a name and a stream (trimmed); every other rule is the server's 422.
- `impl CardItem for MicConfig` (`Status = ()`, key = id + every shown or edited field, `address()` = stream + " · od <ip>" / " · od ktoréhokoľvek odosielateľa", no timestamps); `save` / `delete` = `get_mics` → `apply_draft` / `retain` → `put_mics` (a save of a mic deleted meanwhile → `ApiError::NotFound(TEXT.removed_elsewhere)`, never re-added); `apply_draft` gives a new mic `mic_id_for_role(role, taken)` and keeps an existing mic's id.
- Row aside (B2): `"<jazyk> · <rola> · kanál <kanál> · id <id>"`. All `data-role`s start with `listen-` (`listen-card`, `listen-add`, `listen-new-item`, `listen-editor`, `listen-name`, `listen-stream`, `listen-sender-ip`, `listen-channel`, `listen-language`, `listen-role`, `listen-submit`, `listen-form-status`, `listen-mic-list`, `listen-config`, `listen-count`, `listen-edit`, `listen-delete`, `listen-empty`).
- `pub fn ListenCard(toast: ToastHandle)` (Leptos component).

- [ ] **Step 1: Create `crates/presenter-ui/src/pages/settings/listen.rs`:**

```rust
//! Settings → "Počúvanie" (#852 C1b, spec §5.6): the listening microphones on
//! the shared `list_card::ListCard`, edited in place by [`MicForm`]. The API is
//! the whole list (`GET|PUT /listen/mics`): a save or a delete reads the list,
//! changes one mic and writes the list back ([`apply_draft`]) — one round trip
//! apart, in one Settings card, for at most 4 mics; the server validates the
//! whole list and its Slovak 422 rule shows in the editor. A new mic's id comes
//! from its role (`mic_id_for_role`: `guest`, `interp`, `preacher`, the spec
//! §5.6 ids) and never changes: it names the mic's recordings. All copy is
//! Slovak.

use leptos::prelude::*;
use presenter_core::listen::mics::{mic_id_for_role, MicChannel, MicConfig, MicLanguage, MicRole};

use super::host_editor::{EditorForm, EditorSpec};
use super::list_card::{CardItem, CardText, ListCard, RowParts};
use super::ToastHandle;
use crate::api::{listen, ApiError};

static TEXT: CardText<MicForm> = CardText {
    editor: EditorSpec {
        role: "listen",
        message_id: "listen-form-status",
        message_role: "listen-form-status",
        new_title: "Nový mikrofón",
        new_submit: "Pridať mikrofón",
        edit_title: "Upraviť mikrofón",
        save_text: "Uložiť",
        cancel_text: "Zrušiť",
        unsaved_title: "Najprv ulož alebo zruš otvorenú úpravu",
        saving_title: "Počkaj, kým sa uloženie dokončí",
    },
    form: (),
    list_role: "listen-mic-list",
    add_text: "+ Pridať mikrofón",
    empty_text: "Zatiaľ žiadny mikrofón — Presenter nepočúva a neotvorí žiadny port.",
    fallback_name: "tento mikrofón",
    edit_text: "Upraviť",
    delete_text: "Odstrániť",
    delete_question: ("Odstrániť mikrofón „", "“? Presenter ho prestane počúvať."),
    saving: "Ukladám mikrofón…",
    creating: "Pridávam mikrofón…",
    updated: "Mikrofón je uložený.",
    added: "Mikrofón je pridaný.",
    deleted: "Mikrofón je odstránený.",
    save_failed: "Mikrofón sa nepodarilo uložiť.",
    delete_failed: "Mikrofón sa nepodarilo odstrániť.",
    removed_elsewhere: "Tento mikrofón medzitým niekto odstránil.",
};

/// (value, the wire name = the `<option>` value, the caption).
const CHANNELS: &[(MicChannel, &str, &str)] = &[
    (MicChannel::Left, "left", "ľavý (1)"),
    (MicChannel::Right, "right", "pravý (2)"),
    (MicChannel::Mix, "mix", "zmes"),
];
const LANGUAGES: &[(MicLanguage, &str, &str)] = &[
    (MicLanguage::En, "en", "angličtina"),
    (MicLanguage::Sk, "sk", "slovenčina"),
    (MicLanguage::Off, "off", "neprepisovať"),
];
const ROLES: &[(MicRole, &str, &str)] = &[
    (MicRole::Guest, "guest", "hosť"),
    (MicRole::Interpreter, "interpreter", "tlmočník"),
    (MicRole::Preacher, "preacher", "kazateľ"),
];

/// What the microphone editor holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MicDraft {
    name: String,
    stream_name: String,
    /// As typed; blank = any sender.
    sender_ip: String,
    channel: MicChannel,
    language: MicLanguage,
    role: MicRole,
}

impl Default for MicDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            stream_name: String::new(),
            sender_ip: String::new(),
            channel: MicChannel::Left,
            language: MicLanguage::En,
            role: MicRole::Guest,
        }
    }
}

impl MicDraft {
    fn of(mic: &MicConfig) -> Self {
        Self {
            name: mic.name.clone(),
            stream_name: mic.stream_name.clone(),
            sender_ip: mic.sender_ip.clone().unwrap_or_default(),
            channel: mic.channel,
            language: mic.language,
            role: mic.role,
        }
    }

    /// The mic with this draft's values; `id` names its recordings and stays.
    fn into_config(self, id: String) -> MicConfig {
        MicConfig {
            id,
            name: self.name,
            stream_name: self.stream_name,
            sender_ip: Some(self.sender_ip).filter(|ip| !ip.is_empty()),
            channel: self.channel,
            language: self.language,
            role: self.role,
        }
    }
}

/// The editor's own checks: a name and a stream name. Every other rule is the
/// server's (its 422 text shows in the same line).
fn validate_mic(values: &MicDraft) -> Result<MicDraft, &'static str> {
    let name = values.name.trim();
    if name.is_empty() {
        return Err("Zadaj názov mikrofónu.");
    }
    let stream_name = values.stream_name.trim();
    if stream_name.is_empty() {
        return Err("Zadaj názov VBAN streamu.");
    }
    Ok(MicDraft {
        name: name.to_string(),
        stream_name: stream_name.to_string(),
        sender_ip: values.sender_ip.trim().to_string(),
        ..values.clone()
    })
}

/// The microphone editor's fields. `Copy` (signals only).
#[derive(Clone, Copy)]
pub(super) struct MicForm {
    name: RwSignal<String>,
    stream_name: RwSignal<String>,
    sender_ip: RwSignal<String>,
    channel: RwSignal<MicChannel>,
    language: RwSignal<MicLanguage>,
    role: RwSignal<MicRole>,
}

impl EditorForm for MicForm {
    type Values = MicDraft;
    type Submission = MicDraft;
    type Config = ();

    fn new(_config: ()) -> Self {
        let blank = MicDraft::default();
        Self {
            name: RwSignal::new(blank.name),
            stream_name: RwSignal::new(blank.stream_name),
            sender_ip: RwSignal::new(blank.sender_ip),
            channel: RwSignal::new(blank.channel),
            language: RwSignal::new(blank.language),
            role: RwSignal::new(blank.role),
        }
    }

    fn new_values(self) -> MicDraft {
        MicDraft::default()
    }

    fn load(self, values: &MicDraft) {
        self.name.set(values.name.clone());
        self.stream_name.set(values.stream_name.clone());
        self.sender_ip.set(values.sender_ip.clone());
        self.channel.set(values.channel);
        self.language.set(values.language);
        self.role.set(values.role);
    }

    fn values(self) -> MicDraft {
        MicDraft {
            name: self.name.get(),
            stream_name: self.stream_name.get(),
            sender_ip: self.sender_ip.get(),
            channel: self.channel.get(),
            language: self.language.get(),
            role: self.role.get(),
        }
    }

    fn values_untracked(self) -> MicDraft {
        MicDraft {
            name: self.name.get_untracked(),
            stream_name: self.stream_name.get_untracked(),
            sender_ip: self.sender_ip.get_untracked(),
            channel: self.channel.get_untracked(),
            language: self.language.get_untracked(),
            role: self.role.get_untracked(),
        }
    }

    fn validate(self, values: &MicDraft) -> Result<MicDraft, &'static str> {
        validate_mic(values)
    }

    fn render_fields(
        self,
        spec: EditorSpec,
        first: NodeRef<leptos::html::Input>,
        invalid: Signal<bool>,
    ) -> AnyView {
        let role = move |field: &str| format!("{}-{field}", spec.role);
        let aria_invalid = move || invalid.get().to_string();
        view! {
            <>
                <div class="settings__form-row">
                    <label>
                        <span>"Názov"</span>
                        <input type="text" node_ref=first data-role=role("name") placeholder="Hosť"
                            required aria-required="true"
                            aria-describedby=spec.message_id aria-invalid=aria_invalid
                            prop:value=move || self.name.get()
                            on:input=move |ev| self.name.set(event_target_value(&ev)) />
                    </label>
                    <label>
                        <span>"VBAN stream"</span>
                        <input type="text" data-role=role("stream") placeholder="GUEST_EN"
                            required aria-required="true"
                            aria-describedby=spec.message_id aria-invalid=aria_invalid
                            prop:value=move || self.stream_name.get()
                            on:input=move |ev| self.stream_name.set(event_target_value(&ev)) />
                    </label>
                    <label>
                        <span>"IP odosielateľa (nepovinné)"</span>
                        <input type="text" data-role=role("sender-ip") placeholder="ktorýkoľvek"
                            aria-describedby=spec.message_id aria-invalid=aria_invalid
                            prop:value=move || self.sender_ip.get()
                            on:input=move |ev| self.sender_ip.set(event_target_value(&ev)) />
                    </label>
                </div>
                <div class="settings__form-row">
                    {render_select("Kanál", role("channel"), spec.message_id, invalid, self.channel, CHANNELS)}
                    {render_select("Jazyk", role("language"), spec.message_id, invalid, self.language, LANGUAGES)}
                    {render_select("Rola", role("role"), spec.message_id, invalid, self.role, ROLES)}
                </div>
            </>
        }
        .into_any()
    }

    fn render_action_lead(self, _spec: EditorSpec) -> AnyView {
        ().into_any()
    }
}

/// A `<select>` over `options`; the selection lives on the options
/// (`prop:selected`, the #827 rule).
fn render_select<T>(
    caption: &'static str,
    data_role: String,
    message_id: &'static str,
    invalid: Signal<bool>,
    value: RwSignal<T>,
    options: &'static [(T, &'static str, &'static str)],
) -> impl IntoView
where
    T: Copy + PartialEq + Send + Sync + 'static,
{
    let on_change = move |ev: leptos::ev::Event| {
        let picked = event_target_value(&ev);
        if let Some((option, _, _)) = options.iter().find(|(_, key, _)| *key == picked) {
            value.set(*option);
        }
    };
    view! {
        <label>
            <span>{caption}</span>
            <select data-role=data_role aria-describedby=message_id
                aria-invalid=move || invalid.get().to_string() on:change=on_change>
                {options
                    .iter()
                    .map(|(option, key, text)| {
                        let option = *option;
                        view! {
                            <option value=*key prop:selected=move || value.get() == option>{*text}</option>
                        }
                    })
                    .collect_view()}
            </select>
        </label>
    }
}

/// An option's caption.
fn caption<T: PartialEq>(options: &[(T, &'static str, &'static str)], value: T) -> &'static str {
    options
        .iter()
        .find(|(option, _, _)| *option == value)
        .map_or("?", |(_, _, text)| *text)
}

/// The row's aside: language · role · channel · the id (its recordings' name).
fn mic_aside(mic: &MicConfig) -> String {
    format!(
        "{} · {} · kanál {} · id {}",
        caption(LANGUAGES, mic.language),
        caption(ROLES, mic.role),
        caption(CHANNELS, mic.channel),
        mic.id
    )
}

type MicKey = (
    String,
    String,
    String,
    Option<String>,
    MicChannel,
    MicLanguage,
    MicRole,
);

/// A row's `<For>` key: the id plus every field the row shows or edits — never
/// live health (a poll must not rebuild a row).
fn row_key(mic: &MicConfig) -> MicKey {
    (
        mic.id.clone(),
        mic.name.clone(),
        mic.stream_name.clone(),
        mic.sender_ip.clone(),
        mic.channel,
        mic.language,
        mic.role,
    )
}

impl CardItem for MicConfig {
    type Form = MicForm;
    /// Health is the card's own business (C1b-7), not the list's.
    type Status = ();
    type Key = MicKey;

    fn id(&self) -> &str {
        &self.id
    }

    fn key(&self) -> MicKey {
        row_key(self)
    }

    fn label(&self) -> &str {
        &self.name
    }

    fn address(&self) -> (String, String) {
        let from = match &self.sender_ip {
            Some(ip) => format!(" · od {ip}"),
            None => " · od ktoréhokoľvek odosielateľa".to_string(),
        };
        (self.stream_name.clone(), from)
    }

    fn values(&self) -> MicDraft {
        MicDraft::of(self)
    }

    fn status(&self) -> Option<()> {
        None
    }

    fn timestamps(&self) -> Option<String> {
        None
    }

    async fn list() -> Result<Vec<Self>, ApiError> {
        listen::get_mics().await
    }

    async fn save(id: Option<String>, draft: MicDraft) -> Result<(), ApiError> {
        let mics = listen::get_mics().await?;
        let mics = apply_draft(mics, id.as_deref(), draft)
            .ok_or_else(|| ApiError::NotFound(TEXT.removed_elsewhere.to_string()))?;
        listen::put_mics(&mics).await.map(drop)
    }

    async fn delete(id: String) -> Result<(), ApiError> {
        let mut mics = listen::get_mics().await?;
        mics.retain(|mic| mic.id != id);
        listen::put_mics(&mics).await.map(drop)
    }
}

/// `mics` with `draft` applied: the mic `id` changed in place (its id kept), or
/// a new mic appended with an id from its role; `None` when `id` is gone (a mic
/// removed meanwhile is never added back).
fn apply_draft(
    mut mics: Vec<MicConfig>,
    id: Option<&str>,
    draft: MicDraft,
) -> Option<Vec<MicConfig>> {
    match id {
        Some(id) => {
            let mic = mics.iter_mut().find(|mic| mic.id == id)?;
            *mic = draft.into_config(id.to_string());
        }
        None => {
            let taken: Vec<&str> = mics.iter().map(|mic| mic.id.as_str()).collect();
            let new_id = mic_id_for_role(draft.role, &taken);
            mics.push(draft.into_config(new_id));
        }
    }
    Some(mics)
}

/// A mic row's own part: its language, role, channel and id.
fn mic_row(mic: &MicConfig) -> RowParts {
    RowParts {
        line: view! {
            <span class="settings__list-aside" data-role="listen-config">{mic_aside(mic)}</span>
        }
        .into_any(),
        meta: None,
        warning: ().into_any(),
        actions: ().into_any(),
    }
}

/// The Počúvanie card.
#[component]
pub fn ListenCard(toast: ToastHandle) -> impl IntoView {
    let card = ListCard::<MicConfig>::new(toast, &TEXT);
    let row = move |mic: &MicConfig, _: Memo<Option<()>>| mic_row(mic);
    let mics = card.items();

    view! {
        <section class="settings__card" data-role="listen-card">
            <header class="settings__card-header">
                <div>
                    <h2>"Počúvanie"</h2>
                    <p>"Mikrofóny hosťa a tlmočníka cez VBAN (prepis kázne). Bez mikrofónu Presenter nepočúva a neotvorí žiadny port."</p>
                </div>
                <div class="settings__badge-group">
                    <span class="settings__badge" data-role="listen-count">
                        {move || mics.with(Vec::len).to_string()}
                    </span>
                    <span class="settings__badge-label">"Mikrofóny"</span>
                </div>
            </header>
            {card.render_list(row)}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mic(id: &str, name: &str, role: MicRole) -> MicConfig {
        MicConfig {
            id: id.into(),
            name: name.into(),
            stream_name: "GUEST_EN".into(),
            sender_ip: None,
            channel: MicChannel::Left,
            language: MicLanguage::En,
            role,
        }
    }

    fn draft(name: &str, ip: &str, role: MicRole) -> MicDraft {
        MicDraft {
            name: name.into(),
            stream_name: "INTERP_SK".into(),
            sender_ip: ip.into(),
            channel: MicChannel::Right,
            language: MicLanguage::Sk,
            role,
        }
    }

    #[test]
    fn a_save_changes_one_mic_in_place_and_a_new_one_gets_its_roles_id() {
        let mics = vec![
            mic("guest", "Hosť", MicRole::Guest),
            mic("interp", "Tlmočník", MicRole::Interpreter),
        ];
        let renamed = draft("Tlmočník booth", "192.0.2.10", MicRole::Interpreter);
        let changed = apply_draft(mics.clone(), Some("interp"), renamed).unwrap();
        assert_eq!(changed[0], mics[0], "the other mic is untouched");
        assert_eq!(changed[1].id, "interp", "the id never changes on a rename");
        assert_eq!(changed[1].name, "Tlmočník booth");
        assert_eq!(changed[1].sender_ip.as_deref(), Some("192.0.2.10"));
        let added = apply_draft(mics, None, draft("Druhý hosť", "", MicRole::Guest)).unwrap();
        assert_eq!(added.len(), 3);
        assert_eq!(
            added[2].id, "guest-2",
            "the role's id is taken: the next one"
        );
        assert_eq!(added[2].sender_ip, None, "a blank IP = any sender");
        let preacher = apply_draft(Vec::new(), None, draft("Kazateľ", "", MicRole::Preacher));
        assert_eq!(preacher.unwrap()[0].id, "preacher");
    }

    #[test]
    fn a_save_of_a_mic_removed_meanwhile_never_adds_it_back() {
        let mics = vec![mic("guest", "Hosť", MicRole::Guest)];
        assert_eq!(
            apply_draft(mics, Some("gone"), draft("X", "", MicRole::Guest)),
            None
        );
    }

    #[test]
    fn the_editor_needs_a_name_and_a_stream_and_trims_them() {
        let blank_name = draft("   ", "", MicRole::Guest);
        assert_eq!(validate_mic(&blank_name), Err("Zadaj názov mikrofónu."));
        let mut blank_stream = draft("Hosť", "", MicRole::Guest);
        blank_stream.stream_name = "  ".into();
        assert_eq!(
            validate_mic(&blank_stream),
            Err("Zadaj názov VBAN streamu.")
        );
        let mut padded = draft("  Hosť ", " 192.0.2.10 ", MicRole::Guest);
        padded.stream_name = " GUEST_EN ".into();
        let valid = validate_mic(&padded).unwrap();
        assert_eq!(
            (
                valid.name.as_str(),
                valid.stream_name.as_str(),
                valid.sender_ip.as_str()
            ),
            ("Hosť", "GUEST_EN", "192.0.2.10")
        );
        let round_trip = MicDraft::of(&valid.clone().into_config("guest".into()));
        assert_eq!(round_trip, valid);
    }

    #[test]
    fn the_row_shows_the_id_and_its_key_every_field_it_shows_or_edits() {
        let base = mic("interp", "Tlmočník", MicRole::Interpreter);
        assert_eq!(
            mic_aside(&base),
            "angličtina · tlmočník · kanál ľavý (1) · id interp"
        );
        let mut renamed = base.clone();
        renamed.name = "Tlmočník 2".into();
        let mut mixed = base.clone();
        mixed.channel = MicChannel::Mix;
        assert_ne!(row_key(&base), row_key(&renamed));
        assert_ne!(row_key(&base), row_key(&mixed));
        assert_eq!(row_key(&base), row_key(&base.clone()));
    }
}
```

- [ ] **Step 2: Apply** (patch recipe):

```diff
diff --git a/crates/presenter-ui/src/pages/settings/mod.rs b/crates/presenter-ui/src/pages/settings/mod.rs
index c246d38..94ffaf9 100644
--- a/crates/presenter-ui/src/pages/settings/mod.rs
+++ b/crates/presenter-ui/src/pages/settings/mod.rs
@@ -4,8 +4,9 @@
 //! Reproduces every card the vanilla-JS page managed — Companion (feature
 //! flags), operator Preferences (line-limit pref persisted in localStorage,
 //! #272), Resolume hosts, Android stage launchers, Ableton/OSC, and NDI video
-//! sources — using Leptos signals + `on:*` handlers + the `api::settings` /
-//! `api::ndi` client fns. A 5s interval polls live status (matching the old
+//! sources, plus (#852 C1b) the listening microphones ("Počúvanie") — using
+//! Leptos signals + `on:*` handlers + the `api::settings` / `api::ndi` client
+//! fns. A 5s interval polls live status (matching the old
 //! `STATUS_REFRESH_MS`), and a toast (the original `settings__toast` element)
 //! gives feedback. The server `/integrations/*` + `/settings/features`
 //! contracts are unchanged.
@@ -21,6 +22,7 @@ mod connection_form;
 mod host_editor;
 mod list_card;
 mod list_sync;
+mod listen;
 mod preferences;
 mod resolume;
 mod row_status;
@@ -30,6 +32,7 @@ mod video_sources;
 use ableton::AbletonCard;
 use android::AndroidCard;
 use companion::CompanionCard;
+use listen::ListenCard;
 use preferences::PreferencesCard;
 use resolume::ResolumeCard;
 use trash::TrashCard;
@@ -158,6 +161,7 @@ pub fn SettingsPage(#[prop(optional)] embedded: bool) -> impl IntoView {
                 <PreferencesCard />
                 <ResolumeCard toast=toast />
                 <AndroidCard toast=toast />
+                <ListenCard toast=toast />
                 <AbletonCard toast=toast />
                 <VideoSourcesCard toast=toast />
                 <TrashCard toast=toast />
diff --git a/crates/presenter-ui/styles/settings.css b/crates/presenter-ui/styles/settings.css
index c2de522..96746c7 100644
--- a/crates/presenter-ui/styles/settings.css
+++ b/crates/presenter-ui/styles/settings.css
@@ -211,7 +211,8 @@ body.settings {
 }
 
 .settings__form-row input[type="text"],
-.settings__form-row input[type="number"] {
+.settings__form-row input[type="number"],
+.settings__form-row select {
   box-sizing: border-box;
   width: 100%;
   min-width: 0;
@@ -227,7 +228,8 @@ body.settings {
     box-shadow 0.2s ease;
 }
 
-.settings__form-row input:focus {
+.settings__form-row input:focus,
+.settings__form-row select:focus {
   outline: none;
   border-color: #6366f1;
   box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.12);
```

Unit tests in this task (presenter-ui, run by CI's "Run presenter-ui unit tests"): `a_save_changes_one_mic_in_place_and_a_new_one_gets_its_roles_id`, `a_save_of_a_mic_removed_meanwhile_never_adds_it_back`, `the_editor_needs_a_name_and_a_stream_and_trims_them`, `the_row_shows_the_id_and_its_key_every_field_it_shows_or_edits`.

- [ ] **Step 3: Row contrast and layout check** (`.claude/rules/settings-ui.md`): the card adds no colour (rows, badges and the editor reuse `settings_lists.css`); the `select` rule only joins the existing input selectors. The 360 px check is in the E2E (Task 23).

- [ ] **Step 4: Gates** — `(cd crates/presenter-ui && cargo fmt --check)`; `QC_TARGETS="$(printf '%s\n' crates/presenter-ui/src/pages/settings/{listen,mod}.rs)" python3 scripts/dev/fn_length_check.py .`; `bash scripts/dev/count_prod_lines.sh crates/presenter-ui/src/pages/settings/listen.rs` → `451`.

- [ ] **Step 5: Commit** — paths `crates/presenter-ui/src/pages/settings/listen.rs crates/presenter-ui/src/pages/settings/mod.rs crates/presenter-ui/styles/settings.css`, message `feat(settings): the Počúvanie card — listening microphones (#852 C1b-6)`.

### Task 23: E2E — add, refuse, save, edit, delete, 360 px

**Files:**
- Create: `tests/e2e/settings-listen.spec.ts`

The spec runs its own server (`startTestServer`, like every spec) with `PRESENTER_LISTEN_RECORDINGS_DIR` set to a temp folder around the start (lane 7's recording test writes there; removed in `afterAll`), resets the mics and the recorder through `request.put` before each test, and ends every test with the console assertion. Test 1 count-asserts the ONE `Failed to load resource … 422` line of its refused save (a real 422 is the behaviour under test, `.claude/rules/settings-ui.md`). Seeded data uses RFC 5737 `192.0.2.10`.

- [ ] **Step 1: Create `tests/e2e/settings-listen.spec.ts`:**

```typescript
import { test, expect, type Page } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #852 C1b — Settings → "Počúvanie": the listening microphones card.
// A real browser adds a microphone in the shared inline editor, gets the server's
// Slovak 422 rule inline, saves (the id comes from the role), edits a mic in place
// and removes one; the card fits a 360 px phone. This file's server records into
// its own temp folder (the recording test, C1b-7, writes there).

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let recordings: string;
const STREAM = "E2E_GUEST";

test.describe.configure({ mode: "serial", timeout: 120_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  recordings = mkdtempSync(path.join(os.tmpdir(), "presenter-recordings-"));
  process.env.PRESENTER_LISTEN_RECORDINGS_DIR = recordings;
  try {
    serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
  } finally {
    delete process.env.PRESENTER_LISTEN_RECORDINGS_DIR;
  }
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
  rmSync(recordings, { recursive: true, force: true });
});

test.beforeEach(async ({ request }) => {
  expect((await request.put(url("/listen/recorder"), { data: { enabled: false } })).ok()).toBe(true);
  expect((await request.put(url("/listen/mics"), { data: [] })).ok()).toBe(true);
});

function url(p: string): string {
  return new URL(p, baseURL).toString();
}

async function storedMics(page: Page): Promise<unknown[]> {
  const reply = await page.request.get(url("/listen/mics"));
  expect(reply.ok()).toBe(true);
  return reply.json();
}

async function openSettings(page: Page): Promise<void> {
  await page.goto(url("/ui/settings"));
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
}

test("a microphone is added, refused inline, saved and removed", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await openSettings(page);
  const card = page.locator('[data-role="listen-card"]');
  await expect(card.locator('[data-role="listen-empty"]')).toBeVisible();

  // "+ Pridať mikrofón" opens the shared inline editor at the top of the list
  await card.locator('[data-role="listen-add"]').click();
  const editor = card.locator('[data-role="listen-new-item"] [data-role="listen-editor"]');
  await expect(editor.locator('[data-role="listen-name"]')).toBeFocused();
  await editor.locator('[data-role="listen-name"]').fill("Hosť E2E");
  await editor.locator('[data-role="listen-stream"]').fill("HOSŤ_EN"); // not ASCII: the server refuses
  await editor.locator('[data-role="listen-submit"]').click();
  const message = card.locator('[data-role="listen-form-status"]');
  await expect(message).toHaveAttribute("data-state", "error");
  await expect(message).toContainText("názov VBAN streamu môže mať 1–16 znakov ASCII");
  expect(await storedMics(page)).toEqual([]);

  // corrected: saved; a guest's id is "guest" (spec §5.6)
  await editor.locator('[data-role="listen-stream"]').fill(STREAM);
  await editor.locator('[data-role="listen-language"]').selectOption("en");
  await editor.locator('[data-role="listen-role"]').selectOption("guest");
  await editor.locator('[data-role="listen-channel"]').selectOption("left");
  await editor.locator('[data-role="listen-submit"]').click();
  const row = card.locator('[data-role="listen-mic-list"] li[data-id="guest"]');
  await expect(row.locator(".settings__host-label")).toHaveText("Hosť E2E");
  await expect(row.locator('[data-role="listen-config"]')).toHaveText(
    "angličtina · hosť · kanál ľavý (1) · id guest",
  );
  await expect(card.locator('[data-role="listen-count"]')).toHaveText("1");
  expect(await storedMics(page)).toEqual([
    { id: "guest", name: "Hosť E2E", streamName: STREAM, senderIp: null, channel: "left", language: "en", role: "guest" },
  ]);

  // removed: the confirm names the mic, the list is empty again
  page.once("dialog", async (dialog) => {
    expect(dialog.message()).toBe("Odstrániť mikrofón „Hosť E2E“? Presenter ho prestane počúvať.");
    await dialog.accept();
  });
  await row.locator('[data-role="listen-delete"]').click();
  await expect(card.locator('[data-role="listen-empty"]')).toBeVisible();
  expect(await storedMics(page)).toEqual([]);

  // exactly the one real 422 of the refused save; nothing else
  const refused = /Failed to load resource: the server responded with a status of 422\b/;
  expect(errors.filter((line) => refused.test(line))).toHaveLength(1);
  expect(errors.filter((line) => !refused.test(line))).toEqual([]);
});

test("a microphone is edited in place and the card fits a 360 px phone", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const seeded = await page.request.put(url("/listen/mics"), {
    data: [
      { id: "interp", name: "Tlmočník", streamName: "INTERP_SK", senderIp: "192.0.2.10", channel: "right", language: "sk", role: "interpreter" },
    ],
  });
  expect(seeded.ok()).toBe(true);
  await page.setViewportSize({ width: 360, height: 800 });
  await openSettings(page);
  const card = page.locator('[data-role="listen-card"]');
  const row = () => card.locator('li[data-id="interp"]');
  await expect(row().locator('[data-role="listen-config"]')).toHaveText(
    "slovenčina · tlmočník · kanál pravý (2) · id interp",
  );
  await expect(row().locator(".settings__host-addr")).toContainText("INTERP_SK · od 192.0.2.10");

  await row().locator('[data-role="listen-edit"]').click();
  const editor = row().locator('[data-role="listen-editor"]');
  await expect(editor.locator('[data-role="listen-sender-ip"]')).toHaveValue("192.0.2.10");
  await expect(editor.locator('[data-role="listen-channel"]')).toHaveValue("right");
  await expect(editor.locator('[data-role="listen-language"]')).toHaveValue("sk");
  await editor.locator('[data-role="listen-name"]').fill("Tlmočník booth");
  await editor.locator('[data-role="listen-sender-ip"]').fill("");
  await editor.locator('[data-role="listen-submit"]').click();
  await expect(row().locator(".settings__host-label")).toHaveText("Tlmočník booth");
  expect(await storedMics(page)).toEqual([
    { id: "interp", name: "Tlmočník booth", streamName: "INTERP_SK", senderIp: null, channel: "right", language: "sk", role: "interpreter" },
  ]);

  const fit = await page.evaluate(async () => {
    await document.fonts.ready;
    const layout = document.querySelector(".settings-layout");
    if (!layout) throw new Error("no .settings-layout");
    return { scrollWidth: layout.scrollWidth, clientWidth: layout.clientWidth };
  });
  expect(fit.scrollWidth).toBeLessThanOrEqual(fit.clientWidth);
  expect(errors).toEqual([]);
});
```

- [ ] **Step 2: Type-check** — `node node_modules/typescript/bin/tsc --noEmit -p tsconfig.json` (CI job "TypeScript Type Check"; in a worktree use the main checkout's `node_modules`, `.claude/rules/quality-gates.md`).

- [ ] **Step 3: Commit** — path `tests/e2e/settings-listen.spec.ts`, message `test(e2e): the Počúvanie card — add, refuse, save, edit, delete (#852 C1b-6)`.

### Task 24: Playbook, docs, then close lane C1b-6

**Files:**
- Modify: `.claude/rules/settings-ui.md` (new section "Listening microphones card", incl. the whole-list read-modify-write race line, B7), `.claude/rules/listen-vban.md` (quiet `GET /listen/mics`), `CLAUDE.md` (router row "Settings tab")

- [ ] **Step 1: Apply the docs** (patch recipe):

```diff
diff --git a/.claude/rules/listen-vban.md b/.claude/rules/listen-vban.md
index 68d6daf..a69d1e9 100644
--- a/.claude/rules/listen-vban.md
+++ b/.claude/rules/listen-vban.md
@@ -100,6 +100,7 @@ paths:
   `[a-z0-9-]{1,20}` unique, name 1–40, stream 1–16 printable ASCII without edge spaces, IP canonical,
   no two mics one packet could match) → 422 Slovak text; a corrupt stored value → 500, receiver idle.
   Every save writes one best-effort `settings_audit` row (`app_settings` / `listen.mics`).
+  `GET /listen/mics` is a quiet GET too (the Settings card polls it every 5 s, C1b-6).
 - **Logs** (`presenter::listen::mic`): INFO socket bound / closed, `listen mics saved`, `mic receives
   VBAN` (first packet or after ≥ 3 s silence), `mic lost audio`, format change, unassigned stream (gated);
   WARN through the #484 power-of-two gate (lost / late / resync / dropped / second sender per mic, rejects
diff --git a/.claude/rules/settings-ui.md b/.claude/rules/settings-ui.md
index b74adca..434701e 100644
--- a/.claude/rules/settings-ui.md
+++ b/.claude/rules/settings-ui.md
@@ -94,6 +94,32 @@ Everything else is shared: put a new list behaviour or guard in `list_card.rs` /
   matches the editor: same open generation AND same target (`save_is_current`,
   unit-tested); otherwise the card toasts the error.
 
+## Listening microphones card (#852 C1b-6): whole list behind per-mic save / delete
+
+`settings/listen.rs` is a thin `ListCard<MicConfig>` with its own `EditorForm`
+(`MicForm`: name, VBAN stream, optional sender IP, channel / language / role
+`<select>`s with `prop:selected` per option, the #827 rule) and Slovak copy in its
+`CardText` (incl. `unsaved_title` / `saving_title`).
+
+- The API is the WHOLE list (`GET|PUT /listen/mics`). `CardItem::save` / `delete`
+  read the list, change one mic (`apply_draft`) and `PUT` it back. Two tabs saving
+  within that one round trip can lose one edit: accepted for one Settings card and
+  ≤ 4 mics — do not add per-mic endpoints or ETags for it. A save of a mic deleted
+  meanwhile is refused with `ApiError::NotFound("Tento mikrofón medzitým niekto
+  odstránil.")`, never re-added.
+- The server validates the whole list; its 422 text shows in the editor line
+  (`put_json_refusal` → `ApiError::Refused` → `failure_text`). The card checks only
+  name and stream itself.
+- A new mic's id is `mic_id_for_role` (`guest` / `interp` / `preacher`, then `-2` …),
+  never derived from the name; it never changes on edit (it names the recordings)
+  and the row's aside ends with "id <id>".
+- `GET /listen/mics` is a quiet GET (`request_log.rs`): the card's 5 s list poll.
+- E2E `settings-listen.spec.ts` count-asserts the ONE `Failed to load resource …
+  422` line of its refused save, then `toEqual([])` on the rest; it resets the mics
+  and the recorder through `request.put` before each test, and its server records
+  into a temp folder (`PRESENTER_LISTEN_RECORDINGS_DIR` set around
+  `startTestServer`).
+
 ## The editor's guards: `trigger_lock` (pure, unit-tested)
 
 Every row's Edit and "+ Add" read `ListEditor::trigger_lock(target)` through a
diff --git a/CLAUDE.md b/CLAUDE.md
index 9e7df3f..4534f1a 100644
--- a/CLAUDE.md
+++ b/CLAUDE.md
@@ -49,7 +49,7 @@ This file provides guidance to Claude Code (claude.ai/code) when working with co
 | Resolume port drift (#564/#744/#813): never adopt a port another host on the same machine owns (same host string or a shared resolved IP, re-checked every tick; a disabled host still owns its configured port, an enabled one also its adopted port; `port_claims.rs` table rebuilt by `set_hosts`, a persisted `active_port` is only a seed: kept unless another host owns it, else dropped before the worker is seeded; check-and-claim on adopt; a running host's sibling-owned port dropped + persisted on `SiblingsChanged`); tests use `free_port_pair()` / `free_port_triple()`, never `free_port()+N` | `.claude/rules/resolume-port-drift.md` (auto-loads on `resolume/port_drift*.rs`, `resolume/port_claims.rs`, `resolume/mod.rs`, `resolume/driver.rs`, `resolume/mapping_refresh.rs`) |
 | Resolume composition fetches (#808): NEVER read `/composition` (16 MB on SNV) on a timer — the 10 s tick (and the Test button) is a `/product` probe; fetch only on missing / error-invalidated (#563b threshold = recovery) / stale-id (push 404 → typed `StaleIdError`, invalidate + retry once; 404 on a fresh mapping → 60 s pause) / manual ("Refresh mapping" endpoint + settings button); every new PUT/connect maps 404 → `StaleIdError`; keep pushes idempotent until success (flip/dedup after OK); one `resolume_http_client()`, no connection reuse; a fetch can hit Arena MID-LOAD (PP: 27 tag-less clips) and a deck switch changes every clip id, both with NO 404 → `provisional_mapping.rs`: per-deck last-good kinds, suspect → follow-ups 2/5/15/30/60 s (select! deadline, never a timer), deck check `decks/by-id/{id}` in `ensure_mapping_for_push` + on the tick (switch → inline refetch, failed check → push anyway), lane refetch re-armed on every follow-up step, a used one waits only 3 s (owner: no text skipped; only a deck accepted as lacking a kind stops refetching), an unchanged composition settles as the deck's content; mock Arenas listing `decks` must serve the deck route | `.claude/rules/resolume-mapping-refresh.md` (auto-loads on `resolume/driver.rs`, `resolume/mod.rs`, `resolume/mapping_refresh*.rs`, `resolume/provisional_*.rs`, `resolume/keepalive_tests.rs`, `handlers.rs`, `bible_clear.rs`, `port_drift.rs`, the mocks + settings card) |
 | NDI `ndi_frames_live` per-session Cell vs shared-signal desync (#757/#732/#500): never write the shared signal `false` without resetting the `FrameStats` Cell; same-source reactivation must not reset the gate; ticker self-heal + pure/clock-wired host-test split | `.claude/rules/ndi-frames-live-desync.md` (auto-loads on `pages/stage.rs`, `pages/stage_events.rs`, `components/stage/ndi_frame_stats.rs`, `ndi_health_ticker.rs`, `ndi_video.rs`) |
-| Settings tab (#819): ONE `.settings__form-row` system (never a per-card override — the 220 px-gap bug), list rows/badges/editor in `settings_lists.css`; list cards = thin cards over the shared `list_card::ListCard` (fetch/save/delete/rows) + `host_editor::ListEditor<F>` state machine over the card's `EditorForm` (#852 C1b-1: `connection_form::ConnectionForm` for Resolume/Android, all copy in `CardText`; one editor per card, card-level draft, `trigger_lock` saving/unsaved guards, removed-elsewhere toast); row text ≥4.5:1 on #334155; row key = id + shown/edited fields, never status or `updated_at` (port drift bumps it) | `.claude/rules/settings-ui.md` (auto-loads on `pages/settings/**`, `styles/settings*.css`, `tests/e2e/settings*.spec.ts`) |
+| Settings tab (#819): ONE `.settings__form-row` system (never a per-card override — the 220 px-gap bug), list rows/badges/editor in `settings_lists.css`; list cards = thin cards over the shared `list_card::ListCard` (fetch/save/delete/rows) + `host_editor::ListEditor<F>` state machine over the card's `EditorForm` (#852 C1b-1: `connection_form::ConnectionForm` for Resolume/Android, all copy in `CardText`; one editor per card, card-level draft, `trigger_lock` saving/unsaved guards, removed-elsewhere toast); row text ≥4.5:1 on #334155; row key = id + shown/edited fields, never status or `updated_at` (port drift bumps it); listening mics card (#852 C1b-6) = `MicForm` over the whole-list `GET|PUT /listen/mics` (read-change-PUT per mic, race accepted for ≤ 4 mics, server 422 inline, ids from the role, quiet `GET /listen/mics`) | `.claude/rules/settings-ui.md` (auto-loads on `pages/settings/**`, `styles/settings*.css`, `tests/e2e/settings*.spec.ts`) |
 | Stage displays drop-out (#793): launcher adb reads tri-state (never install/uninstall on an adb error), never churn a `device` target (disconnect only after 3 stuck cycles), stage WS applies EVERY event via a handler (no coalescing last-event signal), snapshot-driven layout reconcile + reconnect resync, `routeWebSocket` lost-frame E2E, WS end-reason / real peer IP / layout-switch logs, per-display layout adopted from heartbeat ACK + diag frames (#797) | `.claude/rules/stage-live-sync.md` (auto-loads on `ws/stage.rs`, `pages/stage*.rs`, `pages/camera.rs`, `live.rs`, `stage_connections.rs`, `android_stage*`) |
 | AI status on `/healthz` + `/ai/status` (#760): ONE shared `evaluate_ai_status`; backend-agnostic `{connected,error,model}` (no OAuth fields, OpenRouter-ready); `/healthz` served through a per-AppState SWR cache (30s TTL, background refresh, single in-flight) so operator-tab fan-out never storms the backend — `/ai/status` stays live; render code in `router/ai_health.rs` + cache in `ai/health_cache.rs`, not the over-cap `router/ai.rs`; deploy gate `connected:false`→non-blocking `::error::`, `modelValid:false`→hard-fail | `.claude/rules/ai-health-endpoint.md` (auto-loads on `router/ai.rs`, `router/ai_health.rs`, `router.rs`) |
 | AI agent loop + bible validator (#784): composer emits the reference the validator sees (`format_verse_range` comma-lists) so `normalize_reference` must accept every composer output; `run_agent_with_guard` seam for race-free budget tests; `##`-marker trick for a deterministic rejection; GaveUp is not an outage (no `AiCallHealth` write); chip fold/probe cross-crate prefix `posledné AI volanie zlyhalo` | `.claude/rules/ai-agent-loop.md` (auto-loads on `ai/agent*.rs`, `ai/bible_validator.rs`, `ai/tools/bible_presentation.rs`, `components/ai_status.rs`) |
```

- [ ] **Step 2: Commit** — paths `.claude/rules/settings-ui.md .claude/rules/listen-vban.md CLAUDE.md`, message `docs(playbook): the listening microphones card (#852 C1b-6) [no-test: docs only]`.

- [ ] **Step 3: Measure the lane** — `bash "$SCRATCH/lane_measure.sh"`. Expected:

```text
crates/presenter-server/src/router/request_log.rs                        +4
crates/presenter-ui/src/api/listen.rs                                    +14
crates/presenter-ui/src/api/mod.rs                                       +21
crates/presenter-ui/src/pages/settings/listen.rs                         +451
crates/presenter-ui/src/pages/settings/mod.rs                            +6
crates/presenter-ui/styles/settings.css                                  +4
lane total: 500 production lines added or changed
```

- [ ] **Step 4: Local gates** (Global Constraints) + the `tsc` check.

- [ ] **Step 5: Push and watch CI** — every job; especially the presenter-ui unit tests, the UI build, the TypeScript check and the E2E shard running `settings-listen.spec.ts` (and the four older settings specs: the card adds a section to the same page; `settings-inline-edit.spec.ts`'s layout guard measures every form-row label).

- [ ] **Step 6: Verify on dev after deploy-dev** (load `post-deploy-verification`; Playwright, real browser): `$DEV/ui/settings` shows the footer version and the "Počúvanie" card with "Zatiaľ žiadny mikrofón …" (dev has no mic); "+ Pridať mikrofón" opens the editor with the name field focused and the three selects (ľavý (1) / angličtina / hosť); Escape closes it; nothing is saved; zero console errors. At 360 px the `.settings-layout` does not scroll sideways. On SNV / PP after the next release: the card renders with an empty list (read-only look, nothing added).

- [ ] **Step 7: Close.** `playbook-review` (the rule landed in Step 1); comment on #852: lane C1b-6 merged (Počúvanie card: microphones), next lane C1b-7.

---

## PR unit C1b-7 — the Počúvanie card: health, socket and the recorder switch (341 lines)

Starts from `origin/dev` with C1b-6 merged. Each mic row gets its state badge, health line and recorder line; the card gets the socket line and the per-day switch "Nahrávať dnes" with its state line — all from one 2 s poll of `GET /listen/status` while the card is on screen. The E2E proves the whole path with a real VBAN sender: badge → health → switch on → WAV on disk → switch off.

### Task 25: Version check

**Files:** `Cargo.toml`, `Cargo.lock` (Task 1 recipe).

- [ ] **Step 1:** Run Task 1 Step 1. `OK` → done. `BUMP` → Task 1 Steps 2–3.

### Task 26: Health, socket and recorder lines; the 2 s poll; the switch

**Files:**
- Modify: `crates/presenter-ui/src/api/listen.rs` (`status()`, `set_recorder(enabled)`), `crates/presenter-ui/src/pages/settings/listen.rs`

**Interfaces:**
- `api::listen::status() -> Result<ListenStatus, ApiError>`, `api::listen::set_recorder(bool) -> Result<RecorderSetting, ApiError>` (`put_json_refusal`).
- `start_status_poll(status: RwSignal<Option<ListenStatus>>, section: NodeRef<Section>) -> impl Fn() + Copy` — fetch now, then every 2 s (`LISTEN_STATUS_REFRESH_MS`) only while the section has an `offsetParent`; numbered by `ResponseOrder`; the `Interval` is `forget()`-ed like every poller on the page (`video_sources.rs` explains why). `after_poll(shown, failures, fresh) -> (shown, failures)` (pure): a fresh status resets the count; one failure keeps the last status; the 2nd in a row → `None` (`video_sources::is_stale`, Y3) and ONE `console.warn` at that transition (the video-sources pattern; the poll never fails in the E2E).
- `recorder_switch(toast, switching, switch_ref, refresh)` — `PUT /listen/recorder`, a Slovak toast, then an immediate refresh; on failure the checkbox goes back to what the server has. The checkbox: `prop:checked` = `recording_on: Memo<bool>` (`recorder.enabled` only, B4 — a poll that changes bytes or files never touches it), `prop:disabled` = switching, or `locked: Memo<bool>` (no status yet, or off with no mic, B3; an ON switch can always be switched off).
- Lines (pure, unit-tested): `mic_badge` (receiving → `live` "prijíma", no_audio → `not-broadcasting` "žiadny zvuk", error → `error` "chyba", waiting → `checking` "čaká na zvuk", none → `unknown` "—" — existing `.settings__status--*` modifiers only), `health_line` (packets/s, level dBFS ● hlas / ○ ticho, lost · late · resync, dropped blocks, other sender, frames, last packet age, format "48000 Hz · 1 kan. · int16", sender), `recording_line` ("Nahráva: <file> · m:ss · MB · zahodené rámce N", "Nahrávanie: čaká na zvuk.", the error line, "Posledná nahrávka: …"), `socket_line`, `recorder_line` ("Nahráva sa do <root>/<day> — vypne sa samo o polnoci. Nahrávky sa mažú po 30 dňoch." / "Nahrávanie je vypnuté. Najprv pridaj mikrofón — bez neho niet čo nahrávať." / "Nahrávanie je vypnuté. Zapína sa vždy len na dnešný deň; …"). A mic's or the recorder's error shows as the row's warning (`listen-error`).
- New `data-role`s: `listen-socket`, `listen-recorder-toggle`, `listen-recorder-state`, `listen-state` (with `data-state`), `listen-health`, `listen-recording`, `listen-error`.

- [ ] **Step 1: Apply** (patch recipe):

```diff
diff --git a/crates/presenter-ui/src/api/listen.rs b/crates/presenter-ui/src/api/listen.rs
index 741637c..4ff0a89 100644
--- a/crates/presenter-ui/src/api/listen.rs
+++ b/crates/presenter-ui/src/api/listen.rs
@@ -1,9 +1,12 @@
-//! Client for the listening API (#852 C1b): the microphones as a whole list.
-//! The write uses `put_json_refusal`: a 422 whose body is the server's JSON
-//! error is an `ApiError::Refused` carrying the Slovak rule.
+//! Client for the listening API (#852 C1b): the microphones as a whole list,
+//! the status and the recorder's per-day toggle. Writes use
+//! `put_json_refusal`: a 422 whose body is the server's JSON error is an
+//! `ApiError::Refused` carrying the Slovak rule.
 
 use super::{get_json, put_json_refusal, ApiError};
 use presenter_core::listen::mics::MicConfig;
+use presenter_core::listen::recorder::{RecorderSetting, RecorderToggle};
+use presenter_core::listen::status::ListenStatus;
 
 pub async fn get_mics() -> Result<Vec<MicConfig>, ApiError> {
     get_json("/listen/mics").await
@@ -12,3 +15,11 @@ pub async fn get_mics() -> Result<Vec<MicConfig>, ApiError> {
 pub async fn put_mics(mics: &[MicConfig]) -> Result<Vec<MicConfig>, ApiError> {
     put_json_refusal("/listen/mics", &mics).await
 }
+
+pub async fn status() -> Result<ListenStatus, ApiError> {
+    get_json("/listen/status").await
+}
+
+pub async fn set_recorder(enabled: bool) -> Result<RecorderSetting, ApiError> {
+    put_json_refusal("/listen/recorder", &RecorderToggle { enabled }).await
+}
diff --git a/crates/presenter-ui/src/pages/settings/listen.rs b/crates/presenter-ui/src/pages/settings/listen.rs
index 2833b77..ece0c10 100644
--- a/crates/presenter-ui/src/pages/settings/listen.rs
+++ b/crates/presenter-ui/src/pages/settings/listen.rs
@@ -5,17 +5,30 @@
 //! apart, in one Settings card, for at most 4 mics; the server validates the
 //! whole list and its Slovak 422 rule shows in the editor. A new mic's id comes
 //! from its role (`mic_id_for_role`: `guest`, `interp`, `preacher`, the spec
-//! §5.6 ids) and never changes: it names the mic's recordings. All copy is
-//! Slovak.
+//! §5.6 ids) and never changes: it names the mic's recordings. Each row's
+//! health and recorder line, the socket line and the recorder's per-day switch
+//! come from `GET /listen/status`, polled every 2 s while the card is on screen
+//! (C1b-7). All copy is Slovak.
 
 use leptos::prelude::*;
 use presenter_core::listen::mics::{mic_id_for_role, MicChannel, MicConfig, MicLanguage, MicRole};
+use presenter_core::listen::recorder::{MicRecording, RecorderStatus, RecordingState};
+use presenter_core::listen::status::{
+    ListenStatus, MicState, MicStatus, SocketState, SocketStatus,
+};
 
 use super::host_editor::{EditorForm, EditorSpec};
-use super::list_card::{CardItem, CardText, ListCard, RowParts};
+use super::list_card::{failure_text, CardItem, CardText, ListCard, RowParts};
+use super::list_sync::ResponseOrder;
+use super::video_sources::is_stale;
 use super::ToastHandle;
 use crate::api::{listen, ApiError};
 
+/// The health poll: fast enough to watch a level move; a quiet GET (#850 O1).
+const LISTEN_STATUS_REFRESH_MS: u32 = 2_000;
+/// Audio bytes per second of a recording (16 kHz mono s16).
+const RECORDING_BYTES_PER_SEC: u64 = 32_000;
+
 static TEXT: CardText<MicForm> = CardText {
     editor: EditorSpec {
         role: "listen",
@@ -334,7 +347,7 @@ fn row_key(mic: &MicConfig) -> MicKey {
 
 impl CardItem for MicConfig {
     type Form = MicForm;
-    /// Health is the card's own business (C1b-7), not the list's.
+    /// Health comes from the card's own `/listen/status` poll, not the list.
     type Status = ();
     type Key = MicKey;
 
@@ -410,15 +423,291 @@ fn apply_draft(
     Some(mics)
 }
 
-/// A mic row's own part: its language, role, channel and id.
-fn mic_row(mic: &MicConfig) -> RowParts {
+/// A mic's badge: (CSS state modifier, Slovak text, `data-state`).
+fn mic_badge(state: Option<MicState>) -> (&'static str, &'static str, &'static str) {
+    match state {
+        Some(MicState::Receiving) => ("live", "prijíma", "receiving"),
+        Some(MicState::NoAudio) => ("not-broadcasting", "žiadny zvuk", "no_audio"),
+        Some(MicState::Error) => ("error", "chyba", "error"),
+        Some(MicState::Waiting) => ("checking", "čaká na zvuk", "waiting"),
+        None => ("unknown", "—", "unknown"),
+    }
+}
+
+/// One decimal, Slovak comma.
+fn decimal(value: f32) -> String {
+    format!("{value:.1}").replace('.', ",")
+}
+
+/// A mic's health line (spec §5.4).
+fn health_line(mic: &MicStatus) -> String {
+    if mic.packets == 0 {
+        return "Zatiaľ žiadny paket VBAN.".to_string();
+    }
+    let mut parts = Vec::new();
+    if mic.packets_per_sec > 0.0 {
+        parts.push(format!("{} paketov/s", decimal(mic.packets_per_sec)));
+    }
+    if let Some(level) = mic.level_dbfs {
+        let voice = if mic.voiced { "● hlas" } else { "○ ticho" };
+        parts.push(format!("{} dBFS {voice}", decimal(level)));
+    }
+    parts.push(format!(
+        "stratené {} · neskoré {} · resync {}",
+        mic.lost_packets, mic.late_packets, mic.resyncs
+    ));
+    if mic.dropped_blocks > 0 {
+        parts.push(format!("zahodené bloky {}", mic.dropped_blocks));
+    }
+    if mic.other_sender_packets > 0 {
+        parts.push(format!("iný odosielateľ {}", mic.other_sender_packets));
+    }
+    parts.push(format!("rámce {}", mic.frames_out));
+    if let Some(age) = mic.last_packet_age_ms {
+        parts.push(format!(
+            "posledný paket pred {} s",
+            decimal(age as f32 / 1000.0)
+        ));
+    }
+    if let Some(format) = mic.format {
+        parts.push(format!(
+            "{} Hz · {} kan. · {}",
+            format.rate_hz,
+            format.channels,
+            format.sample_type.as_str()
+        ));
+    }
+    if let Some(sender) = &mic.sender {
+        parts.push(format!("od {sender}"));
+    }
+    parts.join(" · ")
+}
+
+/// Minutes:seconds and megabytes of audio in `bytes`.
+fn audio_size(bytes: u64) -> String {
+    let seconds = bytes / RECORDING_BYTES_PER_SEC;
+    let megabytes = decimal(bytes as f32 / 1_000_000.0);
+    format!("{}:{:02} · {megabytes} MB", seconds / 60, seconds % 60)
+}
+
+/// A mic's recorder line; empty when there is nothing to say.
+fn recording_line(rec: &MicRecording) -> String {
+    let file = rec
+        .file
+        .as_deref()
+        .and_then(|path| path.rsplit('/').next())
+        .unwrap_or_default();
+    match rec.state {
+        RecordingState::Recording => format!(
+            "Nahráva: {file} · {} · zahodené rámce {}",
+            audio_size(rec.bytes),
+            rec.dropped_frames
+        ),
+        RecordingState::Waiting => "Nahrávanie: čaká na zvuk.".to_string(),
+        RecordingState::Error => {
+            "Nahrávanie zlyhalo — ďalší pokus o pár minút (najneskôr o hodinu).".to_string()
+        }
+        RecordingState::Off if rec.file.is_some() => {
+            format!("Posledná nahrávka: {file} · {}", audio_size(rec.bytes))
+        }
+        RecordingState::Off => String::new(),
+    }
+}
+
+/// The socket line (spec §5.1, C-S8).
+fn socket_line(socket: &SocketStatus) -> String {
+    match socket.state {
+        SocketState::NotStarted => "Príjem VBAN nebeží.".to_string(),
+        SocketState::Idle => {
+            "Príjem VBAN je nečinný — bez mikrofónu nie je otvorený žiadny port.".to_string()
+        }
+        SocketState::Bound => format!(
+            "Počúva na {} (UDP).",
+            socket
+                .local_addr
+                .as_deref()
+                .or(socket.bind_addr.as_deref())
+                .unwrap_or("?")
+        ),
+        SocketState::Retrying => format!(
+            "Port sa nedá otvoriť — skúšam znova. {}",
+            socket.error.as_deref().unwrap_or_default()
+        ),
+    }
+}
+
+/// The line under the recorder switch.
+fn recorder_line(recorder: &RecorderStatus) -> String {
+    match (recorder.enabled, recorder.enabled_for) {
+        (true, Some(day)) => format!(
+            "Nahráva sa do {}/{} — vypne sa samo o polnoci. Nahrávky sa mažú po 30 dňoch.",
+            recorder.root,
+            day.format("%Y-%m-%d")
+        ),
+        _ if recorder.mics.is_empty() => {
+            "Nahrávanie je vypnuté. Najprv pridaj mikrofón — bez neho niet čo nahrávať.".to_string()
+        }
+        _ => {
+            "Nahrávanie je vypnuté. Zapína sa vždy len na dnešný deň; nahrávky sa mažú po 30 dňoch."
+                .to_string()
+        }
+    }
+}
+
+/// What the card shows after a status poll whose answer is `fresh`: the fresh
+/// status; the last one after a single failure (a blip — a page torn down
+/// mid-fetch); nothing after two in a row, so the badges read "—" rather than
+/// stale numbers (`video_sources::is_stale`). Also the failures in a row.
+fn after_poll(
+    shown: Option<ListenStatus>,
+    failures: u32,
+    fresh: Option<ListenStatus>,
+) -> (Option<ListenStatus>, u32) {
+    match fresh {
+        Some(fresh) => (Some(fresh), 0),
+        None => {
+            let failures = failures.saturating_add(1);
+            (if is_stale(failures) { None } else { shown }, failures)
+        }
+    }
+}
+
+/// Fetch `/listen/status` now and every 2 s while `section` is on screen (the
+/// operator's Settings panel stays mounted while hidden); returns the fetch for
+/// an immediate refresh. Numbered: an older answer never replaces a newer one.
+fn start_status_poll(
+    status: RwSignal<Option<ListenStatus>>,
+    section: NodeRef<leptos::html::Section>,
+) -> impl Fn() + Copy + 'static {
+    let order = ResponseOrder::default();
+    let failures = StoredValue::new(0u32);
+    let fetch = move || {
+        leptos::task::spawn_local(async move {
+            let seq = order.begin();
+            let answer = listen::status().await;
+            if !order.accept(seq) {
+                return;
+            }
+            let fresh = answer.as_ref().ok().cloned();
+            let (shown, count) = after_poll(status.get_untracked(), failures.get_value(), fresh);
+            if let Err(err) = &answer {
+                if is_stale(count) && !is_stale(count - 1) {
+                    leptos::logging::warn!(
+                        "listen status poll failed {count}x in a row — showing it as unknown: {err}"
+                    );
+                }
+            }
+            failures.set_value(count);
+            status.set(shown);
+        });
+    };
+    fetch();
+    gloo_timers::callback::Interval::new(LISTEN_STATUS_REFRESH_MS, move || {
+        if section
+            .get_untracked()
+            .is_some_and(|el| el.offset_parent().is_some())
+        {
+            fetch();
+        }
+    })
+    .forget();
+    fetch
+}
+
+/// The recorder switch: `PUT /listen/recorder`, a toast, a fresh status. On a
+/// failure the box goes back to what the server has, not the click.
+fn recorder_switch<R>(
+    toast: ToastHandle,
+    switching: RwSignal<bool>,
+    switch_ref: NodeRef<leptos::html::Input>,
+    refresh: R,
+) -> impl Fn(leptos::ev::Event) + 'static
+where
+    R: Fn() + Copy + 'static,
+{
+    move |ev| {
+        let enabled = event_target_checked(&ev);
+        switching.set(true);
+        leptos::task::spawn_local(async move {
+            match listen::set_recorder(enabled).await {
+                Ok(_) => toast.show(
+                    if enabled {
+                        "Nahrávanie je zapnuté na dnešný deň."
+                    } else {
+                        "Nahrávanie je vypnuté."
+                    },
+                    "success",
+                ),
+                Err(err) => {
+                    if let Some(el) = switch_ref.get_untracked() {
+                        el.set_checked(!enabled);
+                    }
+                    let message = failure_text("Nahrávanie sa nepodarilo prepnúť.", &err);
+                    toast.show(&message, "error");
+                }
+            }
+            switching.set(false);
+            refresh();
+        });
+    }
+}
+
+/// A mic row's own parts: the state badge and the mic's language / role /
+/// channel / id, the health and recorder lines, the mic's or the recorder's error.
+fn mic_row(mic: &MicConfig, status: RwSignal<Option<ListenStatus>>) -> RowParts {
+    let health = {
+        let id = mic.id.clone();
+        Memo::new(move |_| {
+            status.with(|s| {
+                s.as_ref()
+                    .and_then(|s| s.mics.iter().find(|m| m.config.id == id).cloned())
+            })
+        })
+    };
+    let recording = {
+        let id = mic.id.clone();
+        Memo::new(move |_| {
+            status.with(|s| {
+                s.as_ref()
+                    .and_then(|s| s.recorder.mics.iter().find(|r| r.id == id).cloned())
+            })
+        })
+    };
+    let badge = move || mic_badge(health.with(|h| h.as_ref().map(|h| h.state)));
+    let warning = move || {
+        let mic_error = health.with(|h| h.as_ref().and_then(|h| h.error.clone()));
+        let recorder_error = recording.with(|r| r.as_ref().and_then(|r| r.error.clone()));
+        mic_error.or(recorder_error).map(|error| {
+            view! {
+                <p class="settings__list-meta settings__list-meta--warning"
+                    data-role="listen-error">{format!("⚠ {error}")}</p>
+            }
+        })
+    };
+    let aside = mic_aside(mic);
     RowParts {
         line: view! {
-            <span class="settings__list-aside" data-role="listen-config">{mic_aside(mic)}</span>
+            <>
+                <span class=move || format!("settings__status settings__status--{}", badge().0)
+                    data-role="listen-state" data-state=move || badge().2>{move || badge().1}</span>
+                <span class="settings__list-aside" data-role="listen-config">{aside}</span>
+            </>
         }
         .into_any(),
-        meta: None,
-        warning: ().into_any(),
+        meta: Some(
+            view! {
+                <>
+                    <p class="settings__list-meta settings__list-meta--muted" data-role="listen-health">
+                        {move || health.with(|h| h.as_ref().map(health_line).unwrap_or_default())}
+                    </p>
+                    <p class="settings__list-meta settings__list-meta--muted" data-role="listen-recording">
+                        {move || recording.with(|r| r.as_ref().map(recording_line).unwrap_or_default())}
+                    </p>
+                </>
+            }
+            .into_any(),
+        ),
+        warning: warning.into_any(),
         actions: ().into_any(),
     }
 }
@@ -427,11 +716,23 @@ fn mic_row(mic: &MicConfig) -> RowParts {
 #[component]
 pub fn ListenCard(toast: ToastHandle) -> impl IntoView {
     let card = ListCard::<MicConfig>::new(toast, &TEXT);
-    let row = move |mic: &MicConfig, _: Memo<Option<()>>| mic_row(mic);
+    let status = RwSignal::new(None::<ListenStatus>);
+    let section = NodeRef::<leptos::html::Section>::new();
+    let refresh = start_status_poll(status, section);
+    let recorder = Memo::new(move |_| status.with(|s| s.as_ref().map(|s| s.recorder.clone())));
+    let recording_on = Memo::new(move |_| recorder.with(|r| r.as_ref().is_some_and(|r| r.enabled)));
+    // no switch without a status, nor ON without a mic (it stays switchable OFF)
+    let locked = Memo::new(move |_| {
+        recorder.with(|r| r.as_ref().is_none_or(|r| !r.enabled && r.mics.is_empty()))
+    });
+    let switching = RwSignal::new(false);
+    let switch_ref = NodeRef::<leptos::html::Input>::new();
+    let on_switch = recorder_switch(toast, switching, switch_ref, refresh);
+    let row = move |mic: &MicConfig, _: Memo<Option<()>>| mic_row(mic, status);
     let mics = card.items();
 
     view! {
-        <section class="settings__card" data-role="listen-card">
+        <section class="settings__card" data-role="listen-card" node_ref=section>
             <header class="settings__card-header">
                 <div>
                     <h2>"Počúvanie"</h2>
@@ -444,6 +745,21 @@ pub fn ListenCard(toast: ToastHandle) -> impl IntoView {
                     <span class="settings__badge-label">"Mikrofóny"</span>
                 </div>
             </header>
+            <p class="settings__list-meta" data-role="listen-socket">
+                {move || status.with(|s| s.as_ref().map(|s| socket_line(&s.socket)).unwrap_or_default())}
+            </p>
+            <div class="settings__form-row settings__form-row--inline">
+                <label class="settings__form-checkbox">
+                    <input type="checkbox" node_ref=switch_ref data-role="listen-recorder-toggle"
+                        prop:checked=move || recording_on.get()
+                        prop:disabled=move || switching.get() || locked.get()
+                        on:change=on_switch />
+                    <span>"Nahrávať dnes"</span>
+                </label>
+            </div>
+            <p class="settings__list-meta settings__list-meta--muted" data-role="listen-recorder-state">
+                {move || recorder.with(|r| r.as_ref().map(recorder_line).unwrap_or_default())}
+            </p>
             {card.render_list(row)}
         </section>
     }
@@ -548,4 +864,139 @@ mod tests {
         assert_ne!(row_key(&base), row_key(&mixed));
         assert_eq!(row_key(&base), row_key(&base.clone()));
     }
+
+    fn receiving() -> MicStatus {
+        MicStatus {
+            config: mic("guest", "Hosť", MicRole::Guest),
+            state: MicState::Receiving,
+            sender: Some("192.0.2.10:6980".into()),
+            format: Some(presenter_core::audio::vban::VbanAudioFormat {
+                rate_hz: 48_000,
+                channels: 1,
+                sample_type: presenter_core::audio::vban::VbanSampleType::Int16,
+            }),
+            frames_per_packet: Some(256),
+            packets: 936,
+            packets_per_sec: 187.5,
+            last_packet_age_ms: Some(120),
+            lost_packets: 1,
+            late_packets: 0,
+            resyncs: 0,
+            dropped_blocks: 0,
+            other_sender_packets: 0,
+            frames_out: 49,
+            level_dbfs: Some(-9.03),
+            voiced: true,
+            error: None,
+        }
+    }
+
+    fn status_with(mics: Vec<MicStatus>, recorder: RecorderStatus) -> ListenStatus {
+        ListenStatus {
+            socket: SocketStatus {
+                state: SocketState::Bound,
+                bind_addr: Some("0.0.0.0:6980".into()),
+                local_addr: Some("0.0.0.0:6980".into()),
+                error: None,
+            },
+            mics,
+            unmatched: Vec::new(),
+            rejected: Vec::new(),
+            recorder,
+        }
+    }
+
+    #[test]
+    fn the_health_line_reads_the_status_in_slovak() {
+        assert_eq!(
+            health_line(&receiving()),
+            "187,5 paketov/s · -9,0 dBFS ● hlas · stratené 1 · neskoré 0 · resync 0 · rámce 49 · \
+             posledný paket pred 0,1 s · 48000 Hz · 1 kan. · int16 · od 192.0.2.10:6980"
+        );
+        let waiting = MicStatus {
+            packets: 0,
+            ..receiving()
+        };
+        assert_eq!(health_line(&waiting), "Zatiaľ žiadny paket VBAN.");
+        assert_eq!(
+            mic_badge(Some(MicState::NoAudio)),
+            ("not-broadcasting", "žiadny zvuk", "no_audio")
+        );
+        assert_eq!(mic_badge(None).1, "—", "no status: no claim");
+    }
+
+    #[test]
+    fn the_recorder_lines_follow_the_state() {
+        let mut rec = MicRecording {
+            id: "guest".into(),
+            state: RecordingState::Recording,
+            file: Some("/opt/presenter/recordings/2026-10-18/guest_194205.wav".into()),
+            bytes: 2_400_000,
+            dropped_frames: 0,
+            error: None,
+        };
+        assert_eq!(
+            recording_line(&rec),
+            "Nahráva: guest_194205.wav · 1:15 · 2,4 MB · zahodené rámce 0"
+        );
+        rec.state = RecordingState::Off;
+        assert_eq!(
+            recording_line(&rec),
+            "Posledná nahrávka: guest_194205.wav · 1:15 · 2,4 MB"
+        );
+        rec.file = None;
+        assert_eq!(recording_line(&rec), "");
+        let on = RecorderStatus {
+            enabled: true,
+            enabled_for: Some("2026-10-18".parse().unwrap()),
+            root: "/opt/presenter/recordings".into(),
+            mics: vec![MicRecording::off("guest")],
+        };
+        assert!(
+            recorder_line(&on).starts_with("Nahráva sa do /opt/presenter/recordings/2026-10-18 —")
+        );
+        let off = RecorderStatus {
+            enabled: false,
+            ..on.clone()
+        };
+        assert!(recorder_line(&off).starts_with("Nahrávanie je vypnuté. Zapína sa"));
+        assert!(
+            recorder_line(&RecorderStatus::default()).contains("Najprv pridaj mikrofón"),
+            "no mic: the hint the disabled switch needs"
+        );
+    }
+
+    #[test]
+    fn the_socket_line_names_the_state() {
+        let status = status_with(Vec::new(), RecorderStatus::default());
+        assert_eq!(socket_line(&status.socket), "Počúva na 0.0.0.0:6980 (UDP).");
+        let idle = SocketStatus {
+            state: SocketState::Idle,
+            local_addr: None,
+            ..status.socket.clone()
+        };
+        assert!(socket_line(&idle).contains("nečinný"));
+    }
+
+    #[test]
+    fn one_failed_poll_keeps_the_last_status_and_two_show_nothing() {
+        let first = status_with(vec![receiving()], RecorderStatus::default());
+        assert_eq!(
+            after_poll(None, 3, Some(first.clone())),
+            (Some(first.clone()), 0)
+        );
+        let (kept, failures) = after_poll(Some(first.clone()), 0, None);
+        assert_eq!((kept, failures), (Some(first.clone()), 1), "a blip");
+        let (shown, failures) = after_poll(Some(first.clone()), 1, None);
+        assert_eq!(
+            (shown, failures),
+            (None, 2),
+            "two in a row: the badges read —"
+        );
+        assert_eq!(
+            after_poll(None, 2, Some(first.clone())),
+            (Some(first), 0),
+            "back"
+        );
+    }
 }
```

Unit tests added: `the_health_line_reads_the_status_in_slovak`, `the_recorder_lines_follow_the_state`, `the_socket_line_names_the_state`, `one_failed_poll_keeps_the_last_status_and_two_show_nothing` (Y3).

- [ ] **Step 2: Gates** — `(cd crates/presenter-ui && cargo fmt --check)`; `QC_TARGETS="$(printf '%s\n' crates/presenter-ui/src/api/listen.rs crates/presenter-ui/src/pages/settings/listen.rs)" python3 scripts/dev/fn_length_check.py .`; `bash scripts/dev/count_prod_lines.sh crates/presenter-ui/src/pages/settings/listen.rs` → `767` (under 800; the next listening UI unit splits the card's status part into `settings/listen_status.rs`).

- [ ] **Step 3: Commit** — paths `crates/presenter-ui/src/api/listen.rs crates/presenter-ui/src/pages/settings/listen.rs`, message `feat(settings): microphone health, the socket line and the per-day recorder switch (#852 C1b-7)`.

### Task 27: E2E — a real sender, the health row and today's WAV

**Files:**
- Modify: `tests/e2e/settings-listen.spec.ts` (imports, two helpers, test 3)

Test 3: with no mic the socket line says "nečinný" and the switch is disabled with "Najprv pridaj mikrofón" (B3); a mic is saved through the API; the row goes `waiting`, the socket "Počúva na 127.0.0.1:…"; `scripts/dev/listen/vban_send.py` (C1a's sender, Python stdlib) plays a 1 kHz half-scale tone for up to 40 s into the port read from `/listen/status`; the row goes `receiving`, the health line shows "dBFS ● hlas" and "48000 Hz · 1 kan. · int16"; `check()` → "vypne sa samo o polnoci", "Nahráva: guest_<HHMMSS>.wav"; **then `expect.poll` until `recorder.mics[0].bytes > 16 000` (Y4) before `uncheck()`**; off → "Nahrávanie je vypnuté", state `off`; the sender stops; ONE day folder with ONE `guest_<HHMMSS>.wav` whose header says PCM 16 kHz and whose sizes match the file (`length = 44 + data`, RIFF = 36 + data, data > 16 000, = the status `bytes`), and the row shows "Posledná nahrávka:". Zero console errors.

If CI ever shows TWO files here, the sender stalled for more than 1 s on a loaded runner and the drift rule split the file as designed: investigate the runner load first; never loosen the assertion or the 1 s rule.

- [ ] **Step 1: Apply** (patch recipe):

```diff
diff --git a/tests/e2e/settings-listen.spec.ts b/tests/e2e/settings-listen.spec.ts
index 0a862d8..e9f06df 100644
--- a/tests/e2e/settings-listen.spec.ts
+++ b/tests/e2e/settings-listen.spec.ts
@@ -1,8 +1,11 @@
 import { test, expect, type Page } from "@playwright/test";
-import { mkdtempSync, rmSync } from "node:fs";
+import { spawn, type ChildProcess } from "node:child_process";
+import { once } from "node:events";
+import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
 import os from "node:os";
 import path from "node:path";
 import {
+  REPO_ROOT,
   attachConsoleErrorCollector,
   deriveTestConfig,
   refreshDevData,
@@ -14,8 +17,10 @@ import {
 // #852 C1b — Settings → "Počúvanie": the listening microphones card.
 // A real browser adds a microphone in the shared inline editor, gets the server's
 // Slovak 422 rule inline, saves (the id comes from the role), edits a mic in place
-// and removes one; the card fits a 360 px phone. This file's server records into
-// its own temp folder (the recording test, C1b-7, writes there).
+// and removes one; the card fits a 360 px phone. The last test (C1b-7) shows the
+// row's health while a real VBAN sender (scripts/dev/listen/vban_send.py) plays a
+// 1 kHz tone into the test server's loopback socket, switches the recorder on for
+// today and off again, and reads the WAV back from this file's temp folder.
 
 let serverHandle: ServerHandle | undefined;
 let baseURL: string;
@@ -153,3 +158,97 @@ test("a microphone is edited in place and the card fits a 360 px phone", async (
   expect(fit.scrollWidth).toBeLessThanOrEqual(fit.clientWidth);
   expect(errors).toEqual([]);
 });
+
+async function listenStatus(page: Page): Promise<any> {
+  const reply = await page.request.get(url("/listen/status"));
+  expect(reply.ok()).toBe(true);
+  return reply.json();
+}
+
+/** A 1 kHz half-scale tone, 48 kHz INT16, into the test server's VBAN socket. */
+function startSender(port: number, seconds: number): ChildProcess {
+  return spawn(
+    "python3",
+    [
+      path.join(REPO_ROOT, "scripts", "dev", "listen", "vban_send.py"),
+      "--to",
+      `127.0.0.1:${port}`,
+      "--stream",
+      STREAM,
+      "--seconds",
+      String(seconds),
+    ],
+    { stdio: "inherit" },
+  );
+}
+
+async function stopSender(sender: ChildProcess): Promise<void> {
+  if (sender.exitCode !== null || sender.signalCode !== null) return;
+  sender.kill("SIGTERM");
+  await once(sender, "exit");
+}
+
+test("a received microphone shows its health and the recorder writes today's WAV", async ({ page }) => {
+  const errors: string[] = [];
+  attachConsoleErrorCollector(page, errors);
+  await openSettings(page);
+  const card = page.locator('[data-role="listen-card"]');
+  const toggle = card.locator('[data-role="listen-recorder-toggle"]');
+  // no mic: no socket, and nothing to record — the switch says so
+  await expect(card.locator('[data-role="listen-socket"]')).toContainText("nečinný");
+  await expect(card.locator('[data-role="listen-recorder-state"]')).toContainText("Najprv pridaj mikrofón");
+  await expect(toggle).toBeDisabled();
+
+  const saved = await page.request.put(url("/listen/mics"), {
+    data: [{ id: "guest", name: "Hosť E2E", streamName: STREAM, senderIp: null, channel: "left", language: "en", role: "guest" }],
+  });
+  expect(saved.ok()).toBe(true);
+  const row = card.locator('li[data-id="guest"]');
+  await expect(row.locator('[data-role="listen-state"]')).toHaveAttribute("data-state", "waiting", { timeout: 10_000 });
+  await expect(card.locator('[data-role="listen-socket"]')).toContainText("Počúva na 127.0.0.1:", { timeout: 10_000 });
+  await expect(toggle).toBeEnabled();
+  const port = Number(String((await listenStatus(page)).socket.localAddr).split(":").pop());
+  const sender = startSender(port, 40);
+  try {
+    await expect(row.locator('[data-role="listen-state"]')).toHaveAttribute("data-state", "receiving", { timeout: 10_000 });
+    await expect(row.locator('[data-role="listen-health"]')).toContainText("dBFS ● hlas", { timeout: 10_000 });
+    await expect(row.locator('[data-role="listen-health"]')).toContainText("48000 Hz · 1 kan. · int16");
+
+    // the recorder: on for today
+    await toggle.check();
+    await expect(card.locator('[data-role="listen-recorder-state"]')).toContainText("vypne sa samo o polnoci", { timeout: 10_000 });
+    await expect(row.locator('[data-role="listen-recording"]')).toContainText(/Nahráva: guest_\d{6}\.wav/, { timeout: 10_000 });
+    expect((await listenStatus(page)).recorder.enabled).toBe(true);
+    // more than half a second recorded before it is switched off
+    await expect
+      .poll(async () => (await listenStatus(page)).recorder.mics[0].bytes, { timeout: 10_000 })
+      .toBeGreaterThan(16_000);
+
+    // off again: the file is closed with its header
+    await toggle.uncheck();
+    await expect(card.locator('[data-role="listen-recorder-state"]')).toContainText("Nahrávanie je vypnuté", { timeout: 10_000 });
+    await expect.poll(async () => (await listenStatus(page)).recorder.mics[0].state, { timeout: 10_000 }).toBe("off");
+  } finally {
+    await stopSender(sender);
+  }
+
+  // the WAV on disk: one day folder, one file, a header that matches its audio
+  const days = readdirSync(recordings);
+  expect(days).toHaveLength(1);
+  expect(days[0]).toMatch(/^\d{4}-\d{2}-\d{2}$/);
+  const files = readdirSync(path.join(recordings, days[0]));
+  expect(files).toHaveLength(1);
+  expect(files[0]).toMatch(/^guest_\d{6}\.wav$/);
+  const wav = readFileSync(path.join(recordings, days[0], files[0]));
+  expect(wav.subarray(0, 4).toString("ascii")).toBe("RIFF");
+  expect(wav.subarray(8, 16).toString("ascii")).toBe("WAVEfmt ");
+  expect(wav.readUInt16LE(22)).toBe(1);
+  expect(wav.readUInt32LE(24)).toBe(16_000);
+  const dataBytes = wav.readUInt32LE(40);
+  expect(dataBytes).toBeGreaterThan(16_000);
+  expect(wav.length).toBe(44 + dataBytes);
+  expect(wav.readUInt32LE(4)).toBe(36 + dataBytes);
+  expect((await listenStatus(page)).recorder.mics[0].bytes).toBe(dataBytes);
+  await expect(row.locator('[data-role="listen-recording"]')).toContainText("Posledná nahrávka:", { timeout: 10_000 });
+  expect(errors).toEqual([]);
+});
```

- [ ] **Step 2: Type-check** — `node node_modules/typescript/bin/tsc --noEmit -p tsconfig.json`.

- [ ] **Step 3: Commit** — path `tests/e2e/settings-listen.spec.ts`, message `test(e2e): a received microphone's health and today's WAV (#852 C1b-7)`.

### Task 28: Playbook, docs, then close lane C1b-7

**Files:**
- Modify: `.claude/rules/settings-ui.md`, `.claude/rules/listen-vban.md` (routing day: the card), `CLAUDE.md` (router row "Settings tab")

- [ ] **Step 1: Apply the docs** (patch recipe):

```diff
diff --git a/.claude/rules/listen-vban.md b/.claude/rules/listen-vban.md
index a69d1e9..745300c 100644
--- a/.claude/rules/listen-vban.md
+++ b/.claude/rules/listen-vban.md
@@ -169,8 +169,8 @@ paths:
   clears the errors and the pauses. `RecordingFile::begin` removes a new file whose header cannot be
   written (a full disk): no empty files.
 - **Routing day (owner action, spec §20, after C1b):** the owner routes `GUEST_EN` / `INTERP_SK`
-  (48 kHz, 16-bit, after the channel mute) to 10.77.9.205:6980; then the two mics are saved (the C1b
-  card, or `PUT /listen/mics` with the spec §5.6 list) and the box acceptance runs (spec §5.8): status
+  (48 kHz, 16-bit, after the channel mute) to 10.77.9.205:6980; then the two mics are saved (Settings →
+  "Počúvanie", or `PUT /listen/mics` with the spec §5.6 list; the card's ids are `guest` / `interp`) and the box acceptance runs (spec §5.8): status
   before / after, a 10-min `tcpdump`, `scripts/dev/listen/vban_capture_check.py` (added at that step).
 - **FOH routing** (VB-Matrix on fohabl, songplayer `vban-out.md`): only with the owner's go; `Mute` only
   (VB-Matrix ignores `dBGain=-inf`); record the prior values; read back every point after each write.
diff --git a/.claude/rules/settings-ui.md b/.claude/rules/settings-ui.md
index 434701e..b711642 100644
--- a/.claude/rules/settings-ui.md
+++ b/.claude/rules/settings-ui.md
@@ -120,6 +120,24 @@ Everything else is shared: put a new list behaviour or guard in `list_card.rs` /
   into a temp folder (`PRESENTER_LISTEN_RECORDINGS_DIR` set around
   `startTestServer`).
 
+Health, socket and recorder (#852 C1b-7):
+- One `GET /listen/status` poll every 2 s (`start_status_poll`), only while the card
+  is on screen (`offsetParent` — the operator keeps the Settings panel mounted when
+  hidden), numbered by `ResponseOrder`. After 2 failed polls in a row the status is
+  `None` (`video_sources::is_stale`, one console warn when it goes stale): every
+  badge shows "—" rather than old numbers. The status never enters a row's key.
+- The recorder switch "Nahrávať dnes" is a checkbox bound to a `Memo<bool>` of
+  `recorder.enabled` ONLY (`prop:checked`), so a poll that changes bytes or files
+  never re-renders it; it PUTs `/listen/recorder` and the next poll confirms. It is
+  disabled with no status, and when it is off and no mic exists ("Najprv pridaj
+  mikrofón") — an ON switch always stays switchable off.
+- Badges reuse the existing `.settings__status--<state>` modifiers (`live`,
+  `not-broadcasting`, `error`, `checking`, `unknown`); add none for the card.
+- The E2E plays a real tone with `scripts/dev/listen/vban_send.py` into the test
+  server's loopback socket (port from `/listen/status` `socket.localAddr`), waits
+  until the status shows > 16 000 recorded bytes BEFORE it switches off, then reads
+  the WAV header from the temp folder.
+
 ## The editor's guards: `trigger_lock` (pure, unit-tested)
 
 Every row's Edit and "+ Add" read `ListEditor::trigger_lock(target)` through a
diff --git a/CLAUDE.md b/CLAUDE.md
index 4534f1a..07565da 100644
--- a/CLAUDE.md
+++ b/CLAUDE.md
@@ -49,7 +49,7 @@ This file provides guidance to Claude Code (claude.ai/code) when working with co
 | Resolume port drift (#564/#744/#813): never adopt a port another host on the same machine owns (same host string or a shared resolved IP, re-checked every tick; a disabled host still owns its configured port, an enabled one also its adopted port; `port_claims.rs` table rebuilt by `set_hosts`, a persisted `active_port` is only a seed: kept unless another host owns it, else dropped before the worker is seeded; check-and-claim on adopt; a running host's sibling-owned port dropped + persisted on `SiblingsChanged`); tests use `free_port_pair()` / `free_port_triple()`, never `free_port()+N` | `.claude/rules/resolume-port-drift.md` (auto-loads on `resolume/port_drift*.rs`, `resolume/port_claims.rs`, `resolume/mod.rs`, `resolume/driver.rs`, `resolume/mapping_refresh.rs`) |
 | Resolume composition fetches (#808): NEVER read `/composition` (16 MB on SNV) on a timer — the 10 s tick (and the Test button) is a `/product` probe; fetch only on missing / error-invalidated (#563b threshold = recovery) / stale-id (push 404 → typed `StaleIdError`, invalidate + retry once; 404 on a fresh mapping → 60 s pause) / manual ("Refresh mapping" endpoint + settings button); every new PUT/connect maps 404 → `StaleIdError`; keep pushes idempotent until success (flip/dedup after OK); one `resolume_http_client()`, no connection reuse; a fetch can hit Arena MID-LOAD (PP: 27 tag-less clips) and a deck switch changes every clip id, both with NO 404 → `provisional_mapping.rs`: per-deck last-good kinds, suspect → follow-ups 2/5/15/30/60 s (select! deadline, never a timer), deck check `decks/by-id/{id}` in `ensure_mapping_for_push` + on the tick (switch → inline refetch, failed check → push anyway), lane refetch re-armed on every follow-up step, a used one waits only 3 s (owner: no text skipped; only a deck accepted as lacking a kind stops refetching), an unchanged composition settles as the deck's content; mock Arenas listing `decks` must serve the deck route | `.claude/rules/resolume-mapping-refresh.md` (auto-loads on `resolume/driver.rs`, `resolume/mod.rs`, `resolume/mapping_refresh*.rs`, `resolume/provisional_*.rs`, `resolume/keepalive_tests.rs`, `handlers.rs`, `bible_clear.rs`, `port_drift.rs`, the mocks + settings card) |
 | NDI `ndi_frames_live` per-session Cell vs shared-signal desync (#757/#732/#500): never write the shared signal `false` without resetting the `FrameStats` Cell; same-source reactivation must not reset the gate; ticker self-heal + pure/clock-wired host-test split | `.claude/rules/ndi-frames-live-desync.md` (auto-loads on `pages/stage.rs`, `pages/stage_events.rs`, `components/stage/ndi_frame_stats.rs`, `ndi_health_ticker.rs`, `ndi_video.rs`) |
-| Settings tab (#819): ONE `.settings__form-row` system (never a per-card override — the 220 px-gap bug), list rows/badges/editor in `settings_lists.css`; list cards = thin cards over the shared `list_card::ListCard` (fetch/save/delete/rows) + `host_editor::ListEditor<F>` state machine over the card's `EditorForm` (#852 C1b-1: `connection_form::ConnectionForm` for Resolume/Android, all copy in `CardText`; one editor per card, card-level draft, `trigger_lock` saving/unsaved guards, removed-elsewhere toast); row text ≥4.5:1 on #334155; row key = id + shown/edited fields, never status or `updated_at` (port drift bumps it); listening mics card (#852 C1b-6) = `MicForm` over the whole-list `GET|PUT /listen/mics` (read-change-PUT per mic, race accepted for ≤ 4 mics, server 422 inline, ids from the role, quiet `GET /listen/mics`) | `.claude/rules/settings-ui.md` (auto-loads on `pages/settings/**`, `styles/settings*.css`, `tests/e2e/settings*.spec.ts`) |
+| Settings tab (#819): ONE `.settings__form-row` system (never a per-card override — the 220 px-gap bug), list rows/badges/editor in `settings_lists.css`; list cards = thin cards over the shared `list_card::ListCard` (fetch/save/delete/rows) + `host_editor::ListEditor<F>` state machine over the card's `EditorForm` (#852 C1b-1: `connection_form::ConnectionForm` for Resolume/Android, all copy in `CardText`; one editor per card, card-level draft, `trigger_lock` saving/unsaved guards, removed-elsewhere toast); row text ≥4.5:1 on #334155; row key = id + shown/edited fields, never status or `updated_at` (port drift bumps it); listening mics card (#852 C1b-6) = `MicForm` over the whole-list `GET|PUT /listen/mics` (read-change-PUT per mic, race accepted for ≤ 4 mics, server 422 inline, ids from the role, quiet `GET /listen/mics`; C1b-7: 2 s `/listen/status` poll only while on screen, stale after 2 failures, per-day recorder switch bound to a `Memo<bool>`, locked without a mic) | `.claude/rules/settings-ui.md` (auto-loads on `pages/settings/**`, `styles/settings*.css`, `tests/e2e/settings*.spec.ts`) |
 | Stage displays drop-out (#793): launcher adb reads tri-state (never install/uninstall on an adb error), never churn a `device` target (disconnect only after 3 stuck cycles), stage WS applies EVERY event via a handler (no coalescing last-event signal), snapshot-driven layout reconcile + reconnect resync, `routeWebSocket` lost-frame E2E, WS end-reason / real peer IP / layout-switch logs, per-display layout adopted from heartbeat ACK + diag frames (#797) | `.claude/rules/stage-live-sync.md` (auto-loads on `ws/stage.rs`, `pages/stage*.rs`, `pages/camera.rs`, `live.rs`, `stage_connections.rs`, `android_stage*`) |
 | AI status on `/healthz` + `/ai/status` (#760): ONE shared `evaluate_ai_status`; backend-agnostic `{connected,error,model}` (no OAuth fields, OpenRouter-ready); `/healthz` served through a per-AppState SWR cache (30s TTL, background refresh, single in-flight) so operator-tab fan-out never storms the backend — `/ai/status` stays live; render code in `router/ai_health.rs` + cache in `ai/health_cache.rs`, not the over-cap `router/ai.rs`; deploy gate `connected:false`→non-blocking `::error::`, `modelValid:false`→hard-fail | `.claude/rules/ai-health-endpoint.md` (auto-loads on `router/ai.rs`, `router/ai_health.rs`, `router.rs`) |
 | AI agent loop + bible validator (#784): composer emits the reference the validator sees (`format_verse_range` comma-lists) so `normalize_reference` must accept every composer output; `run_agent_with_guard` seam for race-free budget tests; `##`-marker trick for a deterministic rejection; GaveUp is not an outage (no `AiCallHealth` write); chip fold/probe cross-crate prefix `posledné AI volanie zlyhalo` | `.claude/rules/ai-agent-loop.md` (auto-loads on `ai/agent*.rs`, `ai/bible_validator.rs`, `ai/tools/bible_presentation.rs`, `components/ai_status.rs`) |
```

- [ ] **Step 2: Commit** — paths `.claude/rules/settings-ui.md .claude/rules/listen-vban.md CLAUDE.md`, message `docs(playbook): the listening card's health poll and recorder switch (#852 C1b-7) [no-test: docs only]`.

- [ ] **Step 3: Measure the lane** — `bash "$SCRATCH/lane_measure.sh"`. Expected:

```text
crates/presenter-ui/src/api/listen.rs                                    +14
crates/presenter-ui/src/pages/settings/listen.rs                         +327
lane total: 341 production lines added or changed
```

- [ ] **Step 4: Local gates** (Global Constraints) + the `tsc` check.

- [ ] **Step 5: Push and watch CI** — every job; the E2E shard running `settings-listen.spec.ts` spawns `python3` (stdlib only) — the `e2e` job runs on `ubuntu-latest`, whose image ships it — and sends loopback UDP to the test server's own port (`127.0.0.1:0`, never the dev service's 6980).

- [ ] **Step 6: Verify on dev after deploy-dev** (load `post-deploy-verification`; Playwright, real browser, dev only): `$DEV/ui/settings` → the card shows the socket line "Príjem VBAN je nečinný …" and a DISABLED "Nahrávať dnes" with "Najprv pridaj mikrofón"; zero console errors. Then the Task 16 Step 6 sequence through the UI instead of curl: add a mic "Test" / `TEST_EN` / hosť in the card, start `vban_send.py --to "$DEV_HOST:6980" --stream TEST_EN --seconds 30`, watch the row go "prijíma" with "−9,0 dBFS ● hlas" (± a few tenths: the sender's half-scale 1 kHz tone), check "Nahrávať dnes", see "Nahráva: guest_… · 0:0x", uncheck after ~10 s, see "Posledná nahrávka: …", delete the mic (the confirm names it). The WAV on disk reads back with `wave` as 1 / 2 / 16000. Zero console errors throughout. On SNV / PP after the next release: the card shows "nečinný" and the locked switch — read-only, nothing clicked.

- [ ] **Step 7: Close.** `playbook-review`; comment on #852: C1b complete (seven lanes, links to the PRs), the recorder ships OFF everywhere, and the owner actions below are next.

---

## After C1b: the routing day and the recordings (owner actions, not lanes)

1. **Routing** (spec §20, C1a plan "Routing day"): the owner routes `GUEST_EN` / `INTERP_SK` to 10.77.9.205:6980; the two mics are saved in Settings → Počúvanie (ids `guest` / `interp` from their roles) or with `PUT /listen/mics` and the spec §5.6 list; box acceptance per spec §5.8 (the C1a plan's capture checker).
2. **Consent** (spec §19) — the supervisor's question to the owner names how the switch works, so no separate decision about the per-day toggle is needed: *"Nahrávanie sa zapína v Nastaveniach → Počúvanie → „Nahrávať dnes" vždy len na jeden deň (napr. ráno pred službou s hosťom) a o polnoci sa samo vypne; nahráva iba mikrofóny hosťa a tlmočníka (16 kHz, to isté, čo dostane prepis), ukladá ich na serveri do /opt/presenter/recordings/<dátum>/ a po 30 dňoch ich samo zmaže (alebo skôr, hneď po vyhodnotení, ak povieš)."*
3. **Recordings** only after the owner's "áno, nahrávaj": on each guest service "Nahrávať dnes" is switched on (by the operator, or by Claude through `PUT /listen/recorder` on the service day); the files are `/opt/presenter/recordings/<date>/{guest,interp}_<HHMMSS>.wav`, each a continuous stretch named by its start; the bake-off (C2) copies them to dev2 and places the `presenter::bible::show` times by the file names. After the bake-off report the day folders are deleted with the owner's go, or left to the 30-day retention.

## Self-review against the spec

| Spec requirement | Task (lane) |
|---|---|
| §5.4 16 kHz mono s16le 100 ms frames — now leaving the chain for a consumer | 8 (2) |
| §5.5 recorder off by default; Settings toggle | 6, 11 (2, 3); 26 (7) — per-day toggle (Decision 5) |
| §5.5 each mic's 16 kHz stream to `/opt/presenter/recordings/<date>/<mic>…wav`, exactly what the ASR gets | 7, 14, 15 (2, 4) — the root next to the DB = `/opt/presenter/recordings` on prod; one file per stretch (Decision 7), 18 (5) |
| §5.5 ~115 MB per mic-hour | 32 000 B/s × 3600 = 115.2 MB (Task 6's header: 16 kHz, 2 bytes) |
| §5.5 deleted after 30 days | 6, 7 (2) — before anything records |
| §5.5 "or right after the bake-off report" | owner-approved manual deletion (Decision 12); "After C1b" |
| §5.6 Settings → "Počúvanie" edits `listen.mics` | 3 (1), 21, 22 (6) |
| §5.6 ids `guest` / `interp`; language en / sk / off, role guest / interpreter / preacher, channel | 2 (1), 22 (6) (selects; the server validates) |
| §5.6 empty list = idle, no socket | unchanged (C1a); the card says so (22, 26) |
| §11 "VBAN sender off" / packet loss visible to the operator | 26 (7) (badge "žiadny zvuk", health counters) |
| §12 audio stored only with the recorder on; owner consent | Decisions 5, 14; `the_recorder_is_off_by_default_and_writes_nothing` (15), the startup test (11); SNV / PP never switched on by this plan |
| §14 INFO on state changes, WARN through the #484 gate, `/listen/status` per mic | 7, 15, 18 (`presenter::listen::recorder`); 11 (status) |
| §15 Playwright for the UI feature, zero console errors | 23, 27 |
| §15 server integration: VBAN over loopback → … → a consumer records the frames | 15 (`real_vban_lands_in_todays_wav_exactly_as_the_chain_made_it`), 27 (the same through the browser) |
| §16 C1b content, ≤ 600 LoC per PR, version bump first | seven measured lanes (Global Constraints), Tasks 1 / 5 / 10 / 13 / 17 / 20 / 25 |
| C-S8 (no mic = nothing runs) | unchanged for the socket; the recorder supervisor's 60 s tick runs without mics but only prunes (stated deviation, Decision 11) |
| C-S10 zero console errors in every new spec | 23, 27 |

Placeholder scan: no "TBD" / "TODO" / "similar to Task N"; every code step carries its code as a file or a patch generated from the checked scratch copy. Type consistency (the patches are cumulative, so later lanes compile against the earlier ones by construction): `EditorForm` / `ListEditor<F>` / `CardItem::{Form, address, values, timestamps}` / `CardText<F>` / `failure_text` (Task 3) are what `MicForm` / `impl CardItem for MicConfig` implement (22); `mic_id_for_role` (2) → `apply_draft` (22); `MicFrame` / `FRAME_CHANNEL_CAPACITY` / `frame_sender` (8) → `subscribe_frames` (15); `RecordingFile::{create, append, finish, data_bytes, path_text}` / `RecordError` (14) → the writer (15) → `begin` (18); `RecorderHealth::{writer_started, wrote, dropped, failed, writer_stopped, all_stopped, retain, snapshot}` (15) → `failures` (18); `apply_recorder` / `today` (11) → `set_listen_recorder` (11) and the supervisor's `recorder_receiver` (15); `RecorderEnv { root, clock }` / `Clock` / `recordings_root` / `prune_if_new_day` / `RECORDER_TARGET` (7) unchanged by 15 and 18; core `RecorderSetting::records_on` / `RecorderToggle` / `RecordingState` / `MicRecording::off` / `RecorderStatus` / `recording_file_name` / `is_recording_file_name` / `RECORDING_NAMES_PER_SECOND` (6, 11) are used with the same names in 7, 11, 14, 15, 26. Every lane head compiles without an orphan: lane 1's items have callers (the two connection cards) or are `pub` core API; lane 2's supervisor is spawned by `start` and `frame_sender` is called by `run_chain`; lane 3's `apply_recorder` / `today` are called by `state::listen`; lane 4's `RecordingFile` by the writer; lane 6's client by the card.

## Spec gaps and contradictions found while planning (decided — the owner may override any)

1. **"shared `ListCard`" (§16) does not fit as it is.** `ListCard` / `ListEditor` are hard-wired to host / port connections with English copy. Lane 1 generalises them over an `EditorForm` (no visible change to Resolume / Android) instead of a private second list editor (Decision 1).
2. **Size (§16: C1b ≈ 350 + 150 LoC).** Measured total 2 769 production lines added or changed (561 + 515 + 140 + 594 + 118 + 500 + 341), in seven PRs each ≤ 600: the generic editor and the connection-form split (a refactor, 561), the frame channel C1a deferred to its first consumer, a crash-safe writer with status, retention, back-off and the API (≈ 1 370 server + core), and the card (837). The review's six-lane cut put the drift rotation (with the B1 / B5 fixes) into the writers' lane, which measured 681; it is its own lane (C1b-5) and can fold into C1b-7 (459) if the supervisor prefers six PRs.
3. **File naming (§5.5 `<date>/<mic>.wav`).** One file per continuous stretch, `<date>/<mic id>_<HHMMSS>.wav`, a new one after > 1 s of drift: crash- and restart-safe without ever reopening a file, and every file carries its own start time for the bake-off's alignment with the show log (§13) (Decision 7).
4. **The toggle's lifetime (§5.5 says only "a Settings toggle").** A per-day switch: off by itself at midnight, resumed by a same-day restart (auto-deploys happen any time), so a forgotten switch costs one day, not 166 GB (Decision 5). The consent question names it ("After C1b" 2).
5. **"Delete right after the bake-off report" (§5.5).** No button; an owner-approved manual deletion or the 30-day retention (Decision 12).
6. **C-S8 and the supervisor.** The recorder supervisor ticks every 60 s on SNV and PP even without a mic, for the daily retention only (Decision 11); no socket, no writer.
7. **Language of the Settings page.** The existing cards (Resolume, Android, Ableton, NDI) are English; the new card is Slovak as the brief and the spec's operator texts ask. The page is mixed until someone decides to translate the rest (not in scope; nothing is filed — say so if a ticket is wanted).
8. **Observability target.** §14 names `presenter::listen::{mic, asr, suggest}`; the recorder logs under a new `presenter::listen::recorder`.
9. **Two different "dropped" numbers.** `droppedBlocks` (C1a: the chain's input queue, receive side) and `droppedFrames` (C1b: the recorder lagging behind the frame channel) are separate counters by design; the card shows both.

## Owner questions

None new. The recordings themselves wait for the spec §19 consent question, which the supervisor asks separately (wording in "After C1b" 2); this plan ships the recorder OFF everywhere and never switches it on at SNV or PP.

## Addendum — plan review round 2 (verification: 0 🔴 / 0 🟡 / 4 🔵), applied by the supervisor

The implementer of the named lane applies these on top of the task text (they override it where they differ):

- **N1 (Task 18, lane C1b-5) — drift in BOTH directions.** `drifted` compares the ABSOLUTE difference:
  `batch_start.signed_duration_since(audio_end).abs() > TimeDelta::milliseconds(MAX_DRIFT_MS)`. A sender clock
  100 ppm fast otherwise runs ~1 s ahead after 3 h (≈ 6 s by midnight) and file names stop matching the show-log
  times. Add a pure test: audio 1.2 s AHEAD of the clock rotates; exactly 1 s ahead keeps the file. Update the
  doc comment ("audio AHEAD of the clock keeps it" is no longer true) and the rule file sentence.
- **N2 (Task 27, lane C1b-7) — the real-sender E2E accepts ≥ 1 file.** A stall on a loaded runner can split the
  recording; assert `files.length >= 1` and that the NEWEST file's WAV data size equals the status `bytes` of the
  open/last file (or that the sum of all files' data sizes equals the total written), never `toHaveLength(1)`.
  Keep `days` at exactly 1.
- **N3 (Task 18, lane C1b-5) — back-off counted from the failure, not from the reconcile.** The writer records the
  failure time in `RecorderHealth` when its write fails; `retry_at = failed_at + restart_delay(failures)`, so a
  reconcile up to one 60 s tick later does not stretch the pause, and the WARN's `retry_in_min` is exact. The
  existing `a_failed_writer_is_not_restarted_before_its_pause` test keeps its expectations; add one asserting the
  restart happens at `failed_at + 1 min` even when the next reconcile runs 50 s after the failure.
- **N4 (lane C1b-7) — `settings/listen.rs` lands at ~767 lines** (warning at 800). Leave it; the next UI unit that
  touches the card splits it (`listen_health.rs`). Do not add to it in C1b beyond the plan.

## Addendum 2 — what lanes C1b-1 / C1b-2 changed after their reviews (later lanes adapt)

- **C1b-1:** `mic_id_for_role` iterates `1..=taken.len() + 1` (never a taken id); the connection cards' `UNSAVED_TITLE` /
  `SAVING_TITLE` live in `settings/connection_form.rs` (not `host_editor.rs`).
- **C1b-2:** `RecorderEnv` gained `first_pass_delay: Duration` (production 10 min, `cfg(test)` default zero — the first
  retention pass waits for NTP after a boot). Every `RecorderEnv { root, clock }` literal in lanes C1b-3 … C1b-5 (plan
  lines ~3856, ~4165, ~5340, ~5442, ~5492) must add `first_pass_delay: Duration::ZERO` (or use
  `..RecorderEnv::from_env()`), else E0063. The retention scan is `prune_scan(root, opened, today)` over an entry
  iterator; per-entry errors go to `report.failed` and the pass continues. `.gitignore` has `/recordings/` and
  `/stream-assets/`. Lane C1b-2 measures 592 prod lines.
- **C1b-4 must keep the first-pass delay (review of C1b-2, 🟡).** The plan's C1b-4 `supervise` patch (~L4971–4995) was
  written before `first_pass_delay` existed: it calls `prune_if_new_day` at the top of the loop before the `select!`
  and resets the tick. Do NOT apply it that way. Retention keeps its own gate (no pass before `start + first_pass_delay`,
  then once per new local date); writer reconciliation is a separate path that still starts IMMEDIATELY after a restart
  (a same-day restart resumes recording at once). The supervise-level paused-clock test added in C1b-2 (no deletion
  before the delay, deletion after it) must stay green unchanged.
- **C1b-3 changed (review):** the startup read clears a FUTURE `enabledFor` (clock was ahead when switched on): off,
  row stored as `{"enabledFor":null}`, one WARN on `RECORDER_TARGET`, a system audit row (`apply_stored_recorder`).
  **C1b-4's reconcile tick applies the same rule at runtime** (NTP can step the clock back while the server runs:
  an in-memory day after today = off + clear the row + WARN), with a test using the injectable clock. The audit-failure
  WARN picks its target by key (`RECORDER_TARGET` for `listen.recorder`). The dev-deploy scrub in `pipeline.yml`
  deletes `listen.recorder` so dev never inherits prod's switch. C1b-4 / C1b-7 update the "what each unit delivers"
  list in `docs/configuration.md` and the rule when they land. Lane C1b-3 measures 202 prod lines.
