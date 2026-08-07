# Case schema

Every fixture in `corpus/{worship-crud,bible-authoring,adversarial}/*.case.json` is one JSON
object matching the shape below. The schema is designed so every Layer-1 structural check listed
in the research report (§6.4) is directly expressible from `expected` fields — see the mapping
table at the bottom.

This document is the schema **contract** that the future `ai_eval.rs` driver (report §8 step 8,
out of scope for this PR) will parse into real Rust structs. Nothing here executes yet; it is the
shape both the fixture author and the driver author agree to.

## Top-level fields

```jsonc
{
  // Unique id, kebab-case, prefixed by slice shorthand: wc- / ba- / adv-
  "id": "ba-01-verbatim-single-verse-seb",

  // Must match the directory it lives in.
  "slice": "worship-crud" | "bible-authoring" | "adversarial",

  // The chat message sent to `run_agent()` as the user's turn. Slovak where a real
  // church operator would write Slovak (most cases); mix in Czech/English only where the
  // case is specifically probing that language variant (e.g. a delete-gate probe).
  "userMessage": "...",

  // OPTIONAL. Pre-existing state / conversation history the driver must establish
  // BEFORE sending userMessage. Omit entirely for a fresh, empty-state case.
  "setup": {
    // Plain-language description for a human reading the fixture — always fill this in
    // even when `seed`/`priorTurns` are also present, so a reviewer doesn't have to
    // reverse-engineer intent from raw JSON.
    "description": "...",

    // OPTIONAL. Structured state the driver seeds via AppState calls before the turn
    // (worship libraries/presentations/slides, or an existing bible presentation).
    "seed": {
      "libraries": [
        {
          "name": "SOYER",
          "presentations": [
            {
              "name": "WONDER",
              "slides": [
                { "main": "...", "translation": "", "stage": "", "group": "Verse 1" }
              ]
            }
          ]
        }
      ],
      "biblePresentations": [
        {
          "name": "...",
          "slides": [ { "main": "16. ...", "mainReference": "Ján 3:16 (SEB)" } ]
        }
      ]
    },

    // OPTIONAL. Conversation turns BEFORE userMessage — used for cross-turn probes
    // (the delete-intent deferred-affirmation gate; #310).
    "priorTurns": [
      { "role": "user", "content": "vymaž prezentáciu Ranné piesne" },
      { "role": "assistant", "content": "Potvrdzuješ vymazanie prezentácie Ranné piesne?" }
    ]
  },

  "expected": {
    // OPTIONAL. Ordered SUBSEQUENCE the actual tool-call names must contain (in this
    // relative order; other tool calls may appear in between — this is a subsequence
    // match, not an exact/contiguous match). Captures sequencing sanity, e.g.
    // "load_bible_verses must happen before create_bible_presentation for the same ref".
    "toolSequence": ["load_bible_verses", "create_bible_presentation"],

    // OPTIONAL. Rule/error keys that MUST appear at least once in a tool-result during
    // the run. Covers both the bible_validator "rule" field (main_exceeds_character_limit,
    // unprocessed_bold_markers, missing_verse_number_prefix, empty_main_on_emphasis_slide,
    // reference_format_requires_parens) and the item-parse "error" field
    // (missing_items, invalid_verse_item, invalid_emphasis_item, invalid_item_kind).
    // Absent or [] means NONE of these are expected to fire on this case (a well-formed
    // case regressing to 0 is the point of the per-PR golden-trace lane).
    "validationErrors": ["main_exceeds_character_limit"],

    // OPTIONAL. How many self-correction attempts are acceptable after the FIRST
    // validation error before the case is scored a Layer-1 failure. Mirrors the
    // pass/fail bar's "successful self-correction ... within 3 retries" (§6.5 #2).
    // Only meaningful when validationErrors is non-empty.
    "selfCorrectWithinRetries": 3,

    // OPTIONAL. Expected outcome of the delete-intent gate for any delete_* tool call
    // made during this turn. "n/a" (or omit) when the case has no delete_* call at all.
    "deleteGate": "blocked" | "allowed" | "n/a",

    // OPTIONAL. Exact-string fidelity checks: wherever the sermon quoted a verse
    // VERBATIM, the corresponding slide's main text (the verse portion, i.e. everything
    // after the "N. " prefix) must equal `text` byte-for-byte. This is the church-critical
    // check — it catches silent verse corruption/hallucination without a judge.
    "verbatimVerses": [
      { "ref": "Ján 3:16", "translation": "SEB", "text": "Veď Boh tak miloval svet, ..." }
    ],

    // OPTIONAL. The complementary check for paraphrase/mismatch cases: the final slide
    // text MUST equal the SERMON's wording (expectedText) and must NOT silently revert
    // to the unedited DB text (dbText). Exercises "the sermon is authoritative for text
    // content" (agent.rs system prompt step 3).
    "overriddenVerses": [
      {
        "ref": "Ján 1:1",
        "translation": "SEB",
        "expectedText": "...(sermon's own wording)...",
        "dbText": "...(the DB text before override)..."
      }
    ],

    // OPTIONAL. Sane iteration-count ceiling for this specific case (a confused-looping
    // model retrying the same mistake past this bound is itself a regression signal).
    // Falls back to a global default in the driver when absent.
    "maxIterations": 8,

    // Free text. ALWAYS fill this in: which hard-case category this probes (for
    // bible-authoring: verbatim quote / paraphrase / overlapping ranges / bold spanning
    // verse boundary / title interleaved / multi-translation), which server validation
    // rule it targets (for adversarial), or which CRUD/delete-gate behavior it targets
    // (for worship-crud) — plus any rationale a future reader needs.
    "notes": "..."
  }
}
```

## Layer-1 check → schema field mapping (report §6.4)

| Layer-1 check (§6.4) | Expressed via |
|---|---|
| Every emitted tool-argument JSON deserializes into the real tool structs | Implicit — the driver validates EVERY tool call in EVERY case against the real `execute_tool` argument structs, regardless of `expected` |
| Correct rule-keyed error on adversarial cases; zero unexpected errors otherwise | `expected.validationErrors` |
| Self-correction after ≥1 validation error within N retries | `expected.selfCorrectWithinRetries` (paired with non-empty `validationErrors`) |
| Verse-text fidelity (verbatim quotes) | `expected.verbatimVerses` |
| Verse-text fidelity (sermon-authoritative overrides / paraphrase) | `expected.overriddenVerses` |
| Delete-gate behaviour matches expectation | `expected.deleteGate`, with `setup.priorTurns` for the deferred-affirmation cross-turn form |
| Sequencing sanity (no `create_bible_presentation` before `load_bible_verses` for the same ref; confused-loop bound) | `expected.toolSequence`, `expected.maxIterations` |

## Naming convention

- `wc-NN-<short-slug>.case.json` — worship-crud
- `ba-NN-<short-slug>.case.json` — bible-authoring
- `adv-NN-<short-slug>.case.json` — adversarial

`NN` is a zero-padded 2-digit sequence number, unique within its slice directory.

## Adding a new case

1. Pick the slice directory and the next `NN`.
2. Write the fixture against REAL data: a real verse from `data/bibles/` (see the README for how
   to read the mybible/USFM sources), or a real library/song name from `data/libraries/` for
   worship-crud. Never invent verse text — the whole point of `verbatimVerses` is catching
   corruption against a REAL canonical string.
3. Fill in `expected.notes` with the hard-case category or rule this targets.
4. Once the `ai_eval.rs` driver exists (report §8 step 8) and a fresh CLIProxyAPI login is
   available, re-run the golden capture for just this case and commit its trace under `golden/`
   (see the top-level README's "How golden traces get captured" section) — do NOT let a new case
   sit uncaptured, or the per-PR Layer-1 lane has nothing to regress-check it against.
