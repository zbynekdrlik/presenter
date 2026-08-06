# Rubric: slide-splitting reasonableness

Layer-2 (LLM-as-judge) dimension. The server's greedy packer
(`state/slides/compose.rs::compose_bible_items_into_slides`) decides the MECHANICAL slide breaks —
this rubric is not about re-deriving the packer's arithmetic (Layer-1 already replays the real
packer), it is about whether the candidate's INPUT to that packer (the shape and order of its
`items[]` stream) produces a reasonable result for a human reading the slides live during a
service.

## Judge input

For each case, the judge receives: the sermon excerpt (`userMessage`), the character limit in
effect (from the live-context section of the system prompt), and the ordered list of composed
slides (main text + main_reference) that `create_bible_presentation` actually persisted.

## Binary criteria (answer YES/NO for each — ALL must be YES to pass)

1. **No pointlessly fragmented slides.** Short verses that would comfortably fit together under
   the character limit are not each forced onto their own slide by artificial item-level breaks
   the model introduced (e.g. redundant single-verse `create_bible_presentation` groupings where
   one multi-verse item stream would have packed naturally).
2. **No orphaned single-word or single-line slides** caused by a bold marker or reference header
   being mishandled into its own near-empty item.
3. **Emphasis slides are placed at a sensible point** in the sermon's flow — not bunched
   incorrectly relative to the verses they're meant to accent, and not duplicated.
4. **Deliberate repeats are preserved, not collapsed.** When the sermon re-quotes the same verse
   for emphasis (the overlapping-range hard case), the repeat is not silently dropped as if it
   were a duplicate error — but it also is not needlessly split further than the sermon implies.
5. **Multi-translation content stays visually separated**, one translation per slide/group, never
   interleaved line-by-line in a way that would confuse an operator triggering slides live.

## Verdict

`PASS` only if all five criteria are YES. This dimension is inherently more subjective than
verse-fidelity — when a criterion is borderline, judge against "would a church operator triggering
this presentation live find it awkward or confusing", not against a single hypothetical ideal
slide count.
