# Rubric: final chat-reply language and conciseness

Layer-2 (LLM-as-judge) dimension. Grounded directly in `agent.rs`'s system prompt's own
"Response format" section:

> Respond in the user's language (typically Slovak). Keep responses concise. Summarize what you
> actually did based on tool results. Do not claim success for tools that errored.

This rubric grades the assistant's FINAL text response returned to the operator after the tool
loop completes — not the slide content itself (covered by the other three rubrics), the chat
message a church operator actually reads on screen.

## Judge input

For each case, the judge receives: the case's `userMessage` language (Slovak/Czech/English, as
written), the full sequence of tool results (including any errors), and the candidate's final
`content` string returned by `run_agent`.

## Binary criteria (answer YES/NO for each — ALL must be YES to pass)

1. **Language matches the user's message.** If the user wrote in Slovak, the final reply is in
   Slovak (not English, not a mix, regardless of what language the model's internal reasoning or
   any `<think>` leakage might be in — see the report's D4 concern about Qwen3's thinking blocks
   leaking into `content`).
2. **Concise.** The reply summarises the outcome in a few sentences — it does not dump the raw
   tool-call JSON, does not repeat the entire sermon back, and does not pad with unnecessary
   preamble ("Certainly! I'd be happy to help you with that...").
3. **Accurate about what actually happened.** The reply does not claim success for a tool call
   that returned an error (this is the exact language from the system prompt itself) — a case
   where a validation error occurred and was corrected must describe the FINAL successful state
   truthfully, not silently omit that a correction happened when the user would reasonably want to
   know (e.g. "skrátil som príliš dlhý text" when a MainExceedsCharacterLimit correction fired).
4. **No leaked reasoning or markup.** No `<think>...</think>` blocks, no raw tool-call syntax, no
   stray `##` markers, no JSON fragments visible in the reply text.
5. **Appropriately references what was created**, e.g. naming the presentation or the passage, so
   the operator can find it without re-reading the whole conversation.

## Verdict

`PASS` only if all five criteria are YES. Criterion 4 is a hard disqualifier that should be
checked FIRST for any local-model candidate — the report's D4 flags this as "a visible-garbage
bug, not a subtlety" and expects it to be fixed server-side (`--chat-template-kwargs
'{"enable_thinking":false}'`) before this eval is even meaningful; a failure here on every case
likely means D4 was not actually applied to the serving config, not that this specific case is
unusually hard.
