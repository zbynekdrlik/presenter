---
paths:
  - "crates/presenter-server/src/ai/agent.rs"
  - "crates/presenter-server/src/ai/agent_guard.rs"
  - "crates/presenter-server/src/ai/bible_validator.rs"
  - "crates/presenter-server/src/ai/tools/bible_presentation.rs"
  - "crates/presenter-server/src/ai/agent_budget_tests.rs"
  - "crates/presenter-ui/src/components/ai_status.rs"
---

# AI agent loop + bible validator (#784)

## The bible reference the validator sees is COMPOSED server-side, not typed by the model

`create_bible_presentation` takes verse ITEMS (number/text/book/chapter/translation) and calls
`state/slides/compose.rs::compose_bible_items_into_slides` → `format_verse_range`, which renders a
NON-contiguous verse set as a comma-list (`Numeri 13:1, 3, 5`) and a contiguous one as a range
(`13:1-5`). It then runs `validate_bible_slide` on every COMPOSED slide. So a validator that
rejects a legitimate composed form loops the agent on the composer's OWN output — the PP incident
(#784): a non-contiguous passage → composed `Daniel 10:2, 3, 12, 13, 14 (ROH)` → rejected 8× →
24 agent iterations → 120 s client timeout. **Before changing `bible_validator`, check what
`format_verse_range` can emit** — the validator must ACCEPT every well-formed composer output.
`normalize_reference` canonicalises multi-range/comma-list/lowercase-code/duplicated-chapter and
rejects only genuine garbage; `validate_bible_slide` returns the canonical string and
`create_bible_presentation` writes it back into the slide so Resolume gets one form.

## Test the agent turn budget via `run_agent_with_guard`, NEVER env (Tier-0, parallel tests race)

`PRESENTER_AI_TURN_BUDGET_SECS` drives the production budget, but `std::env::set_var` is
process-global and the `#[tokio::test]` suite runs in parallel — a stray small budget makes OTHER
`run_agent` tests give up early and false-fail. `run_agent` is a thin env wrapper over
`run_agent_with_guard(.., guard: AgentGuard)`; tests pass an explicit
`AgentGuard::new(Duration, max_consecutive)` for a deterministic, race-free budget/threshold. The
pure budget/streak logic is also unit-tested in `agent_guard.rs` with an explicit `Duration::ZERO`
/ generous budget — no real 90 s wait, mirroring `context_budget::parse_context_budget_bytes` and
`last_error::AiCallHealth::active_failure(window)`.

## Forcing a deterministic validation rejection in an agent test uses `##` bold markers

A well-formed non-contiguous reference now NORMALISES, so it can no longer drive the
consecutive-rejection guard. Use raw `##` markers in the verse `text`
(`{kind:"verse", text:"##bad## text", ...}`) → the composed slide `main` contains `##` → the
`unprocessed_bold_markers` rule fires on EVERY round. That is the stable, always-rejected tool
result the wiremock/E2E give-up tests rely on (`agent_budget_tests.rs`
`AlwaysRejectedBibleToolCall`).

## A GaveUp is NOT an outage — it must not flip the "AI down" badge

The turn-budget / consecutive-rejection give-up returns a normal Slovak text response via
`conclude_turn` (Ok, not Err) and records NOTHING into `AiCallHealth` — the provider calls
themselves succeeded, the backend is healthy. Only a real transport failure records a failure
(`record_completion_health`) that `evaluate_ai_status` folds into `connected:false`.

## Header chip fold-vs-probe wording depends on a cross-crate string prefix

`router/ai_health.rs::apply_last_completion_failure` prefixes a fold-window last-completion
failure with exactly `"posledné AI volanie zlyhalo"`. `presenter-ui/components/ai_status.rs`'s
`LAST_CALL_FAILED_PREFIX` must match that literal byte-for-byte to render the softer
"AI: posledné volanie zlyhalo" (yellow) state instead of "AI nedostupná" (red, a genuine probe
outage). Change one, change both — they are in different crates with no shared constant.
