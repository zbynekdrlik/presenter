# Rubric: Slovak wording naturalness

Layer-2 (LLM-as-judge) dimension. Applies specifically where the model EDITS or GENERATES Slovak
text itself, rather than passing through an unedited DB verse or the sermon's own literal words:
uppercase-emphasis transforms, any wording the model had to normalise (abbreviation resolution,
book-name canonicalisation), and — most importantly for a smaller multilingual model — whether the
Slovak it produces reads as natural Slovak rather than a stiff or slightly-off machine translation
artifact (the report §3's cited concern for any non-Slovak-native model, including the
recommended Qwen3-8B).

## Judge input

For each case, the judge receives: the sermon excerpt (`userMessage`), and the final composed
slide text and any assistant-authored text the model produced along the way.

## Binary criteria (answer YES/NO for each — ALL must be YES to pass)

1. **Grammatically correct Slovak** — correct case endings, verb agreement, diacritics preserved
   (a model quietly dropping diacritics, e.g. "verili" losing "í", is a fail here even though it
   might still pass a byte-level fidelity check if the sermon itself lacked diacritics).
2. **No literal-translation artifacts** — no word-for-word calques from English phrasing that a
   native Slovak speaker would never produce, and no code-switched English words dropped into an
   otherwise-Slovak sentence unless the sermon's own source did that intentionally (e.g. the
   multi-translation case, where English IS the point).
3. **Uppercase emphasis transforms preserve natural reading** — `##word##` -> `WORD` keeps the
   surrounding sentence grammatically intact; the model doesn't restructure the sentence to "make
   the uppercase work" in a way the sermon never asked for.
4. **Book-name and abbreviation resolution uses the correct Slovak canonical form** from
   `style_guide.md`'s mapping table (e.g. "Žid" → "Židom", not a literal transliteration or a
   guessed alternative).
5. **The overall register matches a church sermon/liturgical context** — not overly casual, not
   robotic boilerplate.

## Verdict

`PASS` only if all five criteria are YES. This is the dimension most likely to separate a
genuinely Slovak-capable multilingual model (Qwen3, EuroLLM per the report's shortlist) from one
that merely gets the mechanics right but produces stilted output — treat it as a real
disqualifying signal, not a nice-to-have.
