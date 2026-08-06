# Rubric: verse-wording reconciliation fidelity

Layer-2 (LLM-as-judge) dimension. Layer-1 already does the EXACT-STRING checks
(`expected.verbatimVerses` / `expected.overriddenVerses` in the case JSON) — this rubric is for
what an exact-string diff cannot capture: whether the candidate's reconciliation between the DB
verse text and the sermon's own wording was *faithful and natural*, not just byte-identical to one
or the other.

Grounded in `agent.rs`'s `format_system_prompt` steps 2-3:

> For each loaded verse, compare its text to the sermon's wording. The sermon is authoritative for
> text content. If they differ, REPLACE the text field with the sermon's wording. If the pastor
> quotes a verse number that does not match the DB (e.g. says Ján 3:16 but quotes Ján 3:17 text),
> keep the sermon's text and the sermon's verse number.

## Judge input

For each case, the judge receives: the case's `userMessage` (the sermon excerpt), the candidate's
captured `toolCalls` (in particular every `load_bible_verses` call and its DB result), and the
final composed slide text from `create_bible_presentation`.

## Binary criteria (answer YES/NO for each — ALL must be YES to pass)

1. **No silent DB reversion.** Where the sermon's wording differs from the DB text the model
   loaded, the final slide text matches the SERMON's wording, not the unedited DB text.
2. **No invented wording.** Where the sermon quotes a verse verbatim (matches the DB text), the
   final slide text is not paraphrased, shortened, or embellished beyond the sermon's own words.
3. **Verse-number handling matches the mismatch rule.** When the sermon's cited verse number and
   its quoted wording point to two different DB verses, the final slide keeps the SERMON's cited
   number paired with the SERMON's quoted text — it does not silently "fix" the number to match
   the wording, or vice versa.
4. **No cross-verse bleed.** Text belonging to one verse does not leak into an adjacent verse's
   slide item (relevant for bold-spanning-verse-boundary and overlapping-range cases).
5. **Reference labels stay consistent with the verse set actually shown**, per the multi-slide
   rule in `style_guide.md` (all slides from one passage carry the SAME full-range reference).

## Verdict

`PASS` only if all five criteria are YES. Any single `NO` is `FAIL` — report which criterion
failed and quote the specific slide/tool-call that violates it. Do not average; a binary
checklist, not a 1-10 quality score (verbosity/format bias — futureagi.com 2026, cited in the
report §6.4).
