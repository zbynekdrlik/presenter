# Mutation Score Improvement & Summary Parsing Fix

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix mutation testing CI summary parsing (#208) and raise mutation score from 0% by writing comprehensive unit tests (#207).

**Architecture:** Two independent workstreams — (1) pipeline YAML fix to parse outcomes.json instead of grepping stdout, (2) unit tests across three crates targeting untested functions with complex logic. Tests use TDD: write failing test, verify fail, implement nothing (functions exist), verify pass.

**Tech Stack:** Rust, cargo-mutants v27, jq, GitHub Actions

**Spec:** `docs/superpowers/specs/2026-04-06-mutation-score-improvement-design.md`

---

## Context

Mutation testing was added to CI (PR #206) as an advisory job. Baseline: 0% score (0 caught / 107 missed in shard 1/12 of 2396 mutants). The summary parsing greps stdout but cargo-mutants v27 only prints MISSED lines by default — CAUGHT is silent. This causes garbled output.

The 0% score means existing tests execute code but don't verify return values tightly enough to catch mutations (e.g., `>` changed to `>=`, return value replaced with default).

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `.github/workflows/pipeline.yml` | Fix mutation summary parsing to use outcomes.json |
| `crates/presenter-core/src/ableset.rs` | Add `#[cfg(test)] mod tests` with `extract_song_prefix` tests |
| `crates/presenter-core/src/resolume.rs` | Add `#[cfg(test)] mod tests` with `validate()` tests |
| `crates/presenter-core/src/osc.rs` | Add `#[cfg(test)] mod tests` with `validate()` tests |
| `crates/presenter-core/src/search.rs` | Add `#[cfg(test)] mod tests` with normalisation + tokenisation tests |
| `crates/presenter-core/src/timer.rs` | Add boundary tests to existing `mod tests` |
| `crates/presenter-core/src/bible/tests/reference.rs` | Add chapter=0 and verse=0 validation tests |
| `crates/presenter-core/src/playlist.rs` | Strengthen MidiBinding test in existing `mod tests` |
| `crates/presenter-core/src/slide.rs` | Add text-preservation assertion to existing test |
| `crates/presenter-bible/src/parsers.rs` | Add `#[cfg(test)] mod tests` with book code, book name, sanitize tests |
| `crates/presenter-importer/src/lib.rs` | Add/strengthen tests in existing `mod tests` |

---

## Task 1: Fix Mutation Summary Parsing (#208)

**Files:**
- Modify: `.github/workflows/pipeline.yml:322-345`

- [ ] **Step 1: Replace grep-based parsing with outcomes.json parsing**

In `.github/workflows/pipeline.yml`, replace the "Run mutation testing" step (lines 322-337) with:

```yaml
      - name: Run mutation testing (baseline — advisory only)
        run: |
          cargo mutants --workspace --timeout 120 --no-shuffle \
            --shard 1/12 --output mutants.out 2>&1 | tee mutants-output.txt || true

          echo "=== Mutation Testing Summary ==="
          if [ -f mutants.out/outcomes.json ]; then
            CAUGHT=$(jq '[.[] | select(.summary == "CaughtMutant")] | length' mutants.out/outcomes.json)
            MISSED=$(jq '[.[] | select(.summary == "MissedMutant")] | length' mutants.out/outcomes.json)
            TIMEOUT=$(jq '[.[] | select(.summary == "Timeout")] | length' mutants.out/outcomes.json)
            TOTAL=$((CAUGHT + MISSED + TIMEOUT))
            echo "Caught: $CAUGHT / $TOTAL"
            echo "Missed: $MISSED / $TOTAL"
            echo "Timeout: $TIMEOUT / $TOTAL"
            if [ "$TOTAL" -gt 0 ]; then
              SCORE=$(( CAUGHT * 100 / TOTAL ))
              echo "Mutation score: ${SCORE}%"
            fi
          else
            echo "No outcomes.json found — cargo-mutants may have failed to run"
            cat mutants-output.txt
          fi
```

- [ ] **Step 2: Update artifact upload to include full mutants.out directory**

Replace the "Upload mutation log" step (lines 339-345) with:

```yaml
      - name: Upload mutation results
        uses: actions/upload-artifact@v4
        if: always()
        with:
          name: mutation-results-${{ github.run_id }}
          path: |
            mutants.out/
            mutants-output.txt
          retention-days: 14
```

- [ ] **Step 3: Verify jq is available on ubuntu-latest**

`jq` is pre-installed on `ubuntu-latest` GitHub Actions runners. No additional install step needed.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/pipeline.yml
git commit -m "fix(ci): parse outcomes.json for mutation testing summary (#208)

Replaces fragile grep-based stdout parsing with jq queries on
mutants.out/outcomes.json. cargo-mutants v27 only prints MISSED
to stdout by default, making grep-based CAUGHT counts unreliable."
```

---

## Task 2: presenter-core — Test `extract_song_prefix()`

**Files:**
- Modify: `crates/presenter-core/src/ableset.rs` (add test module at end of file)

- [ ] **Step 1: Write tests**

Add at the end of `crates/presenter-core/src/ableset.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_song_prefix_returns_digits_of_given_length() {
        assert_eq!(
            extract_song_prefix("123 Song Title", 3),
            Some("123".to_string())
        );
    }

    #[test]
    fn extract_song_prefix_truncates_longer_digit_run() {
        assert_eq!(
            extract_song_prefix("12345 Song", 3),
            Some("123".to_string())
        );
    }

    #[test]
    fn extract_song_prefix_returns_none_for_insufficient_digits() {
        assert_eq!(extract_song_prefix("12 Song", 3), None);
    }

    #[test]
    fn extract_song_prefix_returns_none_for_no_digits() {
        assert_eq!(extract_song_prefix("Song Title", 3), None);
    }

    #[test]
    fn extract_song_prefix_returns_none_for_empty_string() {
        assert_eq!(extract_song_prefix("", 3), None);
    }

    #[test]
    fn extract_song_prefix_returns_none_for_zero_length() {
        assert_eq!(extract_song_prefix("123 Song", 0), None);
    }

    #[test]
    fn extract_song_prefix_trims_leading_whitespace() {
        assert_eq!(
            extract_song_prefix("  456 Song", 3),
            Some("456".to_string())
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p presenter-core --lib -- ableset::tests`
Expected: All 7 tests PASS (function already exists and is correct).

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-core/src/ableset.rs
git commit -m "test(core): add unit tests for extract_song_prefix

Covers: digit extraction, truncation, insufficient digits, no digits,
empty string, zero length, and leading whitespace trimming."
```

---

## Task 3: presenter-core — Test `ResolumeHostDraft::validate()` and `OscSettingsDraft::validate()`

**Files:**
- Modify: `crates/presenter-core/src/resolume.rs` (add test module)
- Modify: `crates/presenter-core/src/osc.rs` (add test module)

- [ ] **Step 1: Write Resolume validation tests**

Add at the end of `crates/presenter-core/src/resolume.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn valid_draft() -> ResolumeHostDraft {
        ResolumeHostDraft::new("Main", "192.168.1.100", 7000)
    }

    #[test]
    fn validate_accepts_valid_draft() {
        assert!(valid_draft().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_label() {
        let mut draft = valid_draft();
        draft.label = "  ".to_string();
        assert_eq!(
            draft.validate().unwrap_err(),
            ResolumeHostValidationError::EmptyLabel
        );
    }

    #[test]
    fn validate_rejects_empty_host() {
        let mut draft = valid_draft();
        draft.host = "".to_string();
        assert_eq!(
            draft.validate().unwrap_err(),
            ResolumeHostValidationError::EmptyHost
        );
    }

    #[test]
    fn validate_rejects_zero_port() {
        let mut draft = valid_draft();
        draft.port = 0;
        assert_eq!(
            draft.validate().unwrap_err(),
            ResolumeHostValidationError::InvalidPort
        );
    }
}
```

- [ ] **Step 2: Write OSC validation tests**

Add at the end of `crates/presenter-core/src/osc.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_default_settings() {
        let settings = OscSettingsDraft::default();
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn validate_rejects_zero_port() {
        let settings = OscSettingsDraft {
            listen_port: 0,
            ..OscSettingsDraft::default()
        };
        assert_eq!(
            settings.validate().unwrap_err(),
            OscSettingsValidationError::InvalidPort
        );
    }

    #[test]
    fn validate_rejects_empty_address() {
        let settings = OscSettingsDraft {
            address_pattern: "  ".to_string(),
            ..OscSettingsDraft::default()
        };
        assert_eq!(
            settings.validate().unwrap_err(),
            OscSettingsValidationError::EmptyAddress
        );
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p presenter-core --lib -- resolume::tests osc::tests`
Expected: All 7 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-core/src/resolume.rs crates/presenter-core/src/osc.rs
git commit -m "test(core): add validation tests for Resolume and OSC settings

Covers all error paths: empty label, empty host, zero port (Resolume),
zero port, empty address (OSC)."
```

---

## Task 4: presenter-core — Test `normalise_for_search()` and `query_tokens()`

**Files:**
- Modify: `crates/presenter-core/src/search.rs` (add test module)

- [ ] **Step 1: Write search function tests**

Add at the end of `crates/presenter-core/src/search.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_preserves_ascii() {
        assert_eq!(normalise_for_search("hello"), "hello");
    }

    #[test]
    fn normalise_lowercases() {
        assert_eq!(normalise_for_search("Hello WORLD"), "hello world");
    }

    #[test]
    fn normalise_strips_diacritics() {
        assert_eq!(normalise_for_search("ěščřžýáíé"), "escrzyaie");
    }

    #[test]
    fn normalise_handles_empty_string() {
        assert_eq!(normalise_for_search(""), "");
    }

    #[test]
    fn normalise_strips_combined_marks() {
        // ň = n + combining caron
        assert_eq!(normalise_for_search("ň"), "n");
    }

    #[test]
    fn query_tokens_splits_on_non_alphanumeric() {
        assert_eq!(
            query_tokens("hello world"),
            vec!["hello".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn query_tokens_filters_empty_segments() {
        assert_eq!(
            query_tokens("  hello  "),
            vec!["hello".to_string()]
        );
    }

    #[test]
    fn query_tokens_handles_special_chars() {
        assert_eq!(
            query_tokens("rock & roll"),
            vec!["rock".to_string(), "roll".to_string()]
        );
    }

    #[test]
    fn query_tokens_empty_input() {
        let result: Vec<String> = query_tokens("");
        assert!(result.is_empty());
    }

    #[test]
    fn query_tokens_normalises_diacritics_before_splitting() {
        assert_eq!(
            query_tokens("Žalm 23"),
            vec!["zalm".to_string(), "23".to_string()]
        );
    }

    #[test]
    fn fold_query_joins_normalised_tokens() {
        assert_eq!(fold_query("Ježíš Kristus"), "jezis kristus");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p presenter-core --lib -- search::tests`
Expected: All 11 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-core/src/search.rs
git commit -m "test(core): add unit tests for search normalisation and tokenisation

Covers: ASCII passthrough, lowercasing, diacritics stripping,
empty input, token splitting, special chars, fold_query."
```

---

## Task 5: presenter-core — Strengthen Timer, BibleReference, MidiBinding, and Slide Tests

**Files:**
- Modify: `crates/presenter-core/src/timer.rs` (add to existing `mod tests`)
- Modify: `crates/presenter-core/src/bible/tests/reference.rs` (add tests)
- Modify: `crates/presenter-core/src/playlist.rs` (strengthen existing test)
- Modify: `crates/presenter-core/src/slide.rs` (add assertion to existing test)

- [ ] **Step 1: Add timer boundary test**

In `crates/presenter-core/src/timer.rs`, add to the existing `mod tests` block (after the last test):

```rust
    #[test]
    fn countdown_remaining_returns_zero_at_exact_target() {
        let now = Utc::now();
        let target = now + TimeDelta::try_minutes(5).unwrap();
        let timer = CountdownTimer::new_with_now(target, now).unwrap();
        // At exact target time, remaining should be zero
        assert_eq!(timer.remaining(target), Duration::zero());
    }

    #[test]
    fn countdown_remaining_returns_zero_past_target() {
        let now = Utc::now();
        let target = now + TimeDelta::try_minutes(5).unwrap();
        let timer = CountdownTimer::new_with_now(target, now).unwrap();
        let past_target = target + TimeDelta::try_seconds(10).unwrap();
        assert_eq!(timer.remaining(past_target), Duration::zero());
    }

    #[test]
    fn preach_timer_elapsed_accumulates_across_pause_resume() {
        let now = Utc::now();
        let mut timer = PreachTimer::new();
        timer.start(now);
        timer.pause(now + TimeDelta::try_seconds(30).unwrap());
        assert_eq!(timer.elapsed(now).num_seconds(), 30);
        timer.start(now + TimeDelta::try_seconds(60).unwrap());
        let elapsed = timer.elapsed(now + TimeDelta::try_seconds(70).unwrap());
        assert_eq!(elapsed.num_seconds(), 40); // 30 accumulated + 10 running
    }

    #[test]
    fn preach_timer_reset_clears_accumulated() {
        let now = Utc::now();
        let mut timer = PreachTimer::new();
        timer.start(now);
        timer.pause(now + TimeDelta::try_seconds(30).unwrap());
        timer.reset();
        assert_eq!(timer.elapsed(now).num_seconds(), 0);
        assert_eq!(timer.state, TimerState::Idle);
    }
```

- [ ] **Step 2: Add BibleReference validation edge cases**

In `crates/presenter-core/src/bible/tests/reference.rs`, add after the existing tests:

```rust
#[test]
fn rejects_zero_chapter() {
    assert_eq!(
        BibleReference::new("Genesis", 0, 1, 5).unwrap_err(),
        BibleReferenceError::InvalidChapter
    );
}

#[test]
fn rejects_zero_verse_start() {
    assert_eq!(
        BibleReference::new("John", 3, 0, 5).unwrap_err(),
        BibleReferenceError::InvalidVerseRange
    );
}

#[test]
fn rejects_zero_verse_end() {
    assert_eq!(
        BibleReference::new("John", 3, 1, 0).unwrap_err(),
        BibleReferenceError::InvalidVerseRange
    );
}

#[test]
fn accepts_single_verse() {
    let reference = BibleReference::new("John", 3, 16, 16).unwrap();
    assert_eq!(reference.chapter, 3);
    assert_eq!(reference.verse_start, 16);
    assert_eq!(reference.verse_end, 16);
}

#[test]
fn new_with_code_stores_code_and_number() {
    let reference =
        BibleReference::new_with_code("Genesis", "GEN", 1, 1, 1, 3).unwrap();
    assert_eq!(reference.book_code.as_deref(), Some("GEN"));
    assert_eq!(reference.book_number, Some(1));
}
```

- [ ] **Step 3: Strengthen MidiBinding test**

In `crates/presenter-core/src/playlist.rs`, replace the `midi_binding_range_check` test with:

```rust
    #[test]
    fn midi_binding_range_check() {
        let binding = MidiBinding::new(127).unwrap();
        assert_eq!(binding.note(), 127);

        let binding_zero = MidiBinding::new(0).unwrap();
        assert_eq!(binding_zero.note(), 0);

        assert_eq!(
            MidiBinding::new(128).unwrap_err(),
            PlaylistError::MidiOutOfRange(128)
        );

        assert_eq!(
            MidiBinding::new(255).unwrap_err(),
            PlaylistError::MidiOutOfRange(255)
        );
    }
```

- [ ] **Step 4: Strengthen slide resolve_sequence test**

In `crates/presenter-core/src/slide.rs`, in the `resolve_sequence_propagates_groups_forward` test, add after the existing `assert_eq!(groups, ...)` assertion:

```rust
        // Verify text content is preserved through resolution
        assert_eq!(resolved[0].main.value(), "A");
        assert_eq!(resolved[0].translation.value(), "A tr");
        assert_eq!(resolved[1].main.value(), "B");
        assert_eq!(resolved[2].stage.value(), "C stage");
```

- [ ] **Step 5: Run all presenter-core tests**

Run: `cargo test -p presenter-core --lib`
Expected: All tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-core/src/timer.rs crates/presenter-core/src/bible/tests/reference.rs crates/presenter-core/src/playlist.rs crates/presenter-core/src/slide.rs
git commit -m "test(core): strengthen timer, bible reference, midi, and slide tests

Timer: boundary tests for remaining() at exact target and past target,
preach timer accumulation across pause/resume, reset clears state.
Bible: chapter=0, verse=0 rejection, single verse, new_with_code.
Midi: verify note() value, not just is_ok(). Slide: verify text
content preservation through resolve_sequence."
```

---

## Task 6: presenter-bible — Test `mysword_book_code()`, `default_book_name()`, `sanitize_text()`, `sanitise_url()`

**Files:**
- Modify: `crates/presenter-bible/src/parsers.rs` (add `#[cfg(test)] mod tests` at end of file)

- [ ] **Step 1: Write tests**

Add at the end of `crates/presenter-bible/src/parsers.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // ---- mysword_book_code ----

    #[test]
    fn mysword_book_code_genesis() {
        assert_eq!(mysword_book_code(1), "GEN");
    }

    #[test]
    fn mysword_book_code_psalms() {
        assert_eq!(mysword_book_code(19), "PSA");
    }

    #[test]
    fn mysword_book_code_matthew() {
        assert_eq!(mysword_book_code(40), "MAT");
    }

    #[test]
    fn mysword_book_code_revelation() {
        assert_eq!(mysword_book_code(66), "REV");
    }

    #[test]
    fn mysword_book_code_unknown() {
        assert_eq!(mysword_book_code(0), "UNK");
        assert_eq!(mysword_book_code(67), "UNK");
    }

    #[test]
    fn mysword_book_code_ot_nt_boundary() {
        assert_eq!(mysword_book_code(39), "MAL"); // last OT
        assert_eq!(mysword_book_code(40), "MAT"); // first NT
    }

    #[test]
    fn mysword_book_code_numbered_books() {
        assert_eq!(mysword_book_code(9), "1SA");
        assert_eq!(mysword_book_code(10), "2SA");
        assert_eq!(mysword_book_code(46), "1CO");
        assert_eq!(mysword_book_code(47), "2CO");
        assert_eq!(mysword_book_code(62), "1JN");
        assert_eq!(mysword_book_code(64), "3JN");
    }

    // ---- default_book_name ----

    #[test]
    fn default_book_name_genesis() {
        assert_eq!(default_book_name("GEN"), Some("Genesis"));
    }

    #[test]
    fn default_book_name_psalms() {
        assert_eq!(default_book_name("PSA"), Some("Psalm"));
    }

    #[test]
    fn default_book_name_revelation() {
        assert_eq!(default_book_name("REV"), Some("Revelation"));
    }

    #[test]
    fn default_book_name_unknown_code() {
        assert_eq!(default_book_name("XYZ"), None);
        assert_eq!(default_book_name(""), None);
    }

    #[test]
    fn default_book_name_numbered_books() {
        assert_eq!(default_book_name("1SA"), Some("1 Samuel"));
        assert_eq!(default_book_name("2KI"), Some("2 Kings"));
        assert_eq!(default_book_name("1TH"), Some("1 Thessalonians"));
    }

    // ---- sanitize_text ----

    #[test]
    fn sanitize_text_preserves_plain_text() {
        assert_eq!(sanitize_text("Hello world"), "Hello world");
    }

    #[test]
    fn sanitize_text_strips_usfm_markers() {
        assert_eq!(sanitize_text("\\v 1 In the beginning"), "1 In the beginning");
    }

    #[test]
    fn sanitize_text_strips_word_level_attributes() {
        assert_eq!(
            sanitize_text("\\w God|strong=\"H430\"\\w*"),
            "God"
        );
    }

    #[test]
    fn sanitize_text_normalises_whitespace() {
        assert_eq!(sanitize_text("  hello   world  "), "hello world");
    }

    #[test]
    fn sanitize_text_handles_empty_input() {
        assert_eq!(sanitize_text(""), "");
    }

    // ---- sanitize_mysword_text ----

    #[test]
    fn sanitize_mysword_strips_html_tags() {
        assert_eq!(
            sanitize_mysword_text("<b>Bold</b> text"),
            "Bold text"
        );
    }

    #[test]
    fn sanitize_mysword_strips_footnotes() {
        assert_eq!(
            sanitize_mysword_text("Word<f>footnote content</f> rest"),
            "Word rest"
        );
    }

    #[test]
    fn sanitize_mysword_strips_bracketed_variants() {
        assert_eq!(
            sanitize_mysword_text("text [ Var.: + extra] more"),
            "text more"
        );
    }

    // ---- starts_with_ci ----

    #[test]
    fn starts_with_ci_matches_case_insensitive() {
        assert!(starts_with_ci("<Font", "<f"));
        assert!(starts_with_ci("<FONT", "<f"));
    }

    #[test]
    fn starts_with_ci_rejects_shorter_haystack() {
        assert!(!starts_with_ci("a", "ab"));
    }

    #[test]
    fn starts_with_ci_empty_needle() {
        assert!(starts_with_ci("anything", ""));
    }
}
```

- [ ] **Step 2: Also add `sanitise_url` test**

The `sanitise_url` function is in `crates/presenter-bible/src/provider.rs`. Add at the end of that file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitise_url_preserves_alphanumeric_and_dots() {
        assert_eq!(sanitise_url("file.txt"), "file.txt");
        assert_eq!(sanitise_url("abc-123.zip"), "abc-123.zip");
    }

    #[test]
    fn sanitise_url_replaces_special_chars() {
        assert_eq!(sanitise_url("https://example.com/path"), "https___example.com_path");
    }

    #[test]
    fn sanitise_url_replaces_spaces() {
        assert_eq!(sanitise_url("my file"), "my_file");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p presenter-bible --lib`
Expected: All tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-bible/src/parsers.rs crates/presenter-bible/src/provider.rs
git commit -m "test(bible): add unit tests for book codes, names, text sanitization

mysword_book_code: 10 representative mappings + boundary + unknown.
default_book_name: 8 mappings + unknown codes.
sanitize_text: USFM markers, word attributes, whitespace, empty.
sanitize_mysword_text: HTML tags, footnotes, bracketed variants.
sanitise_url: alphanumeric passthrough, special char replacement."
```

---

## Task 7: presenter-importer — Strengthen Tests

**Files:**
- Modify: `crates/presenter-importer/src/lib.rs` (add to existing `mod tests`)

- [ ] **Step 1: Add classify_text_role tests for all aliases**

In `crates/presenter-importer/src/lib.rs`, add after the existing `classify_text_role_matches_aliases` test:

```rust
    #[test]
    fn classify_text_role_case_insensitive() {
        assert_eq!(classify_text_role("MAIN"), TextRole::Main);
        assert_eq!(classify_text_role("main"), TextRole::Main);
        assert_eq!(classify_text_role("  Main  "), TextRole::Main);
    }

    #[test]
    fn classify_text_role_all_main_aliases() {
        for alias in &["lyrics", "text", "hlavny", "hlavný"] {
            assert_eq!(
                classify_text_role(alias),
                TextRole::Main,
                "expected Main for alias '{alias}'"
            );
        }
    }

    #[test]
    fn classify_text_role_all_translation_aliases() {
        for alias in &["translation", "preklad", "english", "eng"] {
            assert_eq!(
                classify_text_role(alias),
                TextRole::Translation,
                "expected Translation for alias '{alias}'"
            );
        }
    }

    #[test]
    fn classify_text_role_all_stage_aliases() {
        for alias in &["stage", "stage display", "stage text"] {
            assert_eq!(
                classify_text_role(alias),
                TextRole::Stage,
                "expected Stage for alias '{alias}'"
            );
        }
    }

    #[test]
    fn classify_text_role_empty_is_unknown() {
        assert_eq!(classify_text_role(""), TextRole::Unknown);
        assert_eq!(classify_text_role("  "), TextRole::Unknown);
    }
```

- [ ] **Step 2: Strengthen slide_content_from_proto to verify actual text values**

In `crates/presenter-importer/src/lib.rs`, add after the existing `slide_content_preserves_real_text_when_stage_placeholder_removed` test:

```rust
    #[test]
    fn slide_content_assigns_translation_and_stage_roles() {
        let main_text = proto::graphics::Text {
            rtf_data: b"Main lyrics".to_vec(),
            ..Default::default()
        };
        let translation_text = proto::graphics::Text {
            rtf_data: b"Translation text".to_vec(),
            ..Default::default()
        };
        let stage_text = proto::graphics::Text {
            rtf_data: b"Stage text".to_vec(),
            ..Default::default()
        };

        let slide = proto::Slide {
            elements: vec![
                proto::slide::Element {
                    element: Some(proto::graphics::Element {
                        name: "Main".into(),
                        text: Some(main_text),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                proto::slide::Element {
                    element: Some(proto::graphics::Element {
                        name: "Translation".into(),
                        text: Some(translation_text),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                proto::slide::Element {
                    element: Some(proto::graphics::Element {
                        name: "Stage Display".into(),
                        text: Some(stage_text),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let content = slide_content_from_proto(&slide, None).expect("content");
        assert_eq!(content.main.value(), "Main lyrics");
        assert_eq!(content.translation.value(), "Translation text");
        assert_eq!(content.stage.value(), "Stage text");
    }
```

- [ ] **Step 3: Add build_group_lookup test**

In `crates/presenter-importer/src/lib.rs`, add to the test module. The proto types are generated from protobuf at build time (`proto::Presentation`, `proto::CueGroup`, etc.). Check the generated types by looking at how `build_group_lookup` and `resolve_cue_sequence` use them in the source (lines 362-506). Construct test data matching those field accesses:

```rust
    #[test]
    fn build_group_lookup_assigns_anchor_to_first_cue() {
        // NOTE: Proto types are generated. If this doesn't compile,
        // check the generated types in target/debug/build/presenter-importer-*/out/rv.data.rs
        // and adjust field names to match.
        let mut presentation = proto::Presentation::default();
        let mut cue_group = proto::CueGroup::default();
        let mut group = proto::cue_group::Group::default();
        group.name = "Verse 1".to_string();
        group.uuid = Some(proto::Uuid { string: "group-1".to_string() });
        cue_group.group = Some(group);
        cue_group.cue_identifiers = vec![
            proto::Uuid { string: "cue-a".to_string() },
            proto::Uuid { string: "cue-b".to_string() },
            proto::Uuid { string: "cue-c".to_string() },
        ];
        presentation.cue_groups = vec![cue_group];

        let lookup = build_group_lookup(&presentation);
        assert_eq!(lookup.len(), 3);

        let cue_a = &lookup["cue-a"];
        assert_eq!(cue_a.name, "Verse 1");
        assert!(cue_a.anchor, "first cue should be anchor");

        let cue_b = &lookup["cue-b"];
        assert_eq!(cue_b.name, "Verse 1");
        assert!(!cue_b.anchor, "second cue should not be anchor");

        let cue_c = &lookup["cue-c"];
        assert!(!cue_c.anchor, "third cue should not be anchor");
    }

    #[test]
    fn build_group_lookup_skips_empty_uuids() {
        let mut presentation = proto::Presentation::default();
        let mut cue_group = proto::CueGroup::default();
        let mut group = proto::cue_group::Group::default();
        group.name = "Chorus".to_string();
        group.uuid = Some(proto::Uuid { string: "group-1".to_string() });
        cue_group.group = Some(group);
        cue_group.cue_identifiers = vec![
            proto::Uuid { string: "cue-a".to_string() },
            proto::Uuid { string: String::new() },
        ];
        presentation.cue_groups = vec![cue_group];

        let lookup = build_group_lookup(&presentation);
        assert_eq!(lookup.len(), 1);
        assert!(lookup.contains_key("cue-a"));
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p presenter-importer --lib`
Expected: All tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-importer/src/lib.rs
git commit -m "test(importer): strengthen classify_text_role, slide_content, and build_group_lookup

classify_text_role: case insensitivity, all aliases for each role,
empty/whitespace input. slide_content: verify actual text values
for all three roles. build_group_lookup: anchor assignment logic,
empty UUID skipping."
```

---

## Task 8: Format, Version Bump, Push, and Monitor CI

**Files:**
- Modify: `Cargo.toml` (version bump if needed)

- [ ] **Step 1: Run cargo fmt**

Run: `cargo fmt --all --check`
Fix if needed: `cargo fmt --all`

- [ ] **Step 2: Check and bump version**

```bash
git fetch origin
DEV_VER=$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
MAIN_VER=$(git show origin/main:Cargo.toml | grep -m1 '^version = ' | sed 's/version = "\(.*\)"/\1/')
echo "dev=$DEV_VER main=$MAIN_VER"
```

If dev version is not higher than main, bump patch in `Cargo.toml` workspace `[workspace.package].version`.

- [ ] **Step 3: Commit version bump if needed**

```bash
git add Cargo.toml
git commit -m "chore: bump version to X.Y.Z"
```

- [ ] **Step 4: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 5: Monitor CI**

```bash
gh run list --branch dev --limit 3
gh run watch <run-id> --exit-status
```

Wait for ALL jobs to reach terminal state. If any fail, investigate with `gh run view <run-id> --log-failed`, fix ALL issues in ONE commit, push, and monitor again.

---

## Task 9: Create PR and Close Issues

- [ ] **Step 1: Create PR**

```bash
gh pr create --title "ci: fix mutation summary parsing and improve mutation score" --body "$(cat <<'EOF'
## Summary
- Fix mutation testing summary parsing to use `outcomes.json` instead of fragile stdout grep (#208)
- Add comprehensive unit tests across presenter-core, presenter-bible, and presenter-importer to improve mutation score from 0% baseline (#207)

## Test Coverage Added
| Crate | Functions Tested | New Tests |
|-------|-----------------|-----------|
| presenter-core | extract_song_prefix, Resolume validate, OSC validate, normalise_for_search, query_tokens, CountdownTimer boundary, BibleReference edges, MidiBinding value, slide text preservation | ~30 |
| presenter-bible | mysword_book_code, default_book_name, sanitize_text, sanitize_mysword_text, sanitise_url, starts_with_ci | ~20 |
| presenter-importer | classify_text_role aliases, slide_content roles, build_group_lookup anchor logic | ~10 |

Closes #207
Closes #208

## Test plan
- [ ] All new unit tests pass (`cargo test --workspace`)
- [ ] Mutation testing CI job produces valid summary with CAUGHT/MISSED/TIMEOUT counts
- [ ] Mutation score improved from 0% baseline

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 2: Verify PR is mergeable**

```bash
gh api repos/zbynekdrlik/presenter/pulls/<NUMBER> --jq '{mergeable: .mergeable, mergeable_state: .mergeable_state}'
```

Wait until `mergeable: true` and `mergeable_state: "clean"`.

- [ ] **Step 3: Report PR URL to user**

Provide the green PR URL and wait for user's explicit merge instruction.

---

## Verification Summary

| Fix | Test Method | Evidence |
|-----|-------------|----------|
| Summary parsing (#208) | CI artifact check | outcomes.json parsed, CAUGHT/MISSED/TIMEOUT reported correctly |
| extract_song_prefix | 7 unit tests | Digits, truncation, boundaries, edge cases |
| Resolume validate | 4 unit tests | All 3 error paths + valid case |
| OSC validate | 3 unit tests | Both error paths + valid default |
| Search normalisation | 11 unit tests | Diacritics, case, tokens, empty |
| Timer boundaries | 4 unit tests | Exact target, past target, accumulation, reset |
| BibleReference edges | 5 unit tests | Zero chapter, zero verses, single verse, with_code |
| MidiBinding value | 1 strengthened | Verify note() not just is_ok() |
| Slide text preservation | 1 strengthened | Verify main/translation/stage values |
| Book code mappings | 7 unit tests | OT/NT boundary, numbered, unknown |
| Book name mappings | 5 unit tests | Representative + unknown |
| Text sanitization | 8 unit tests | USFM, MySword, whitespace, attributes |
| classify_text_role | 5 unit tests | All aliases, case, empty |
| slide_content roles | 1 unit test | All 3 roles with actual text values |
| build_group_lookup | 2 unit tests | Anchor logic, empty UUID skip |
