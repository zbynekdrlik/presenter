# AI Presentation Style Guide

This reference is for detailed formatting questions. The main system prompt
covers essentials; load this on demand when you need specifics.

## Slide field usage (Bible presentations)

- `main`: Verse text with leading verse number. Format: "1. Verse text here" or "27 Verse text here". NEVER include the reference in main.
- `main_reference`: Reference WITH translation code. Example: "Žalm 26:1 (ROH)". ALWAYS include the code in parentheses.
- `secondary`: Leave empty unless bilingual.
- `secondary_reference`: Secondary translation reference if bilingual.

## Reference format (mandatory — never omit the translation code)

- Single verse: "Žalm 26:1 (ROH)"
- Verse range: "Marek 3:14-15 (SEB)"
- Partial verse: "Žalm 26:3a (ROH)"

The code in parentheses is REQUIRED. Without it, Resolume cannot display the reference correctly.

## Multi-slide passages (critical)

When a Bible passage is split across multiple slides, ALL slides from that passage MUST use the SAME full reference — the complete verse range from start to end.

Example: Psalm 52:1-11 split into 4 slides:
- Slide 1 (vv 1-3): main_reference = "Žalm 52:1-11 (ROH)" ← FULL range, not "52:1-3"
- Slide 2 (vv 4-6): main_reference = "Žalm 52:1-11 (ROH)" ← same
- Slide 3 (vv 7-9): main_reference = "Žalm 52:1-11 (ROH)" ← same
- Slide 4 (vv 10-11): main_reference = "Žalm 52:1-11 (ROH)" ← same

WRONG: Using per-slide ranges like "Žalm 52:1-3", "Žalm 52:4-6" — this makes each slide look like a separate passage.

## Markdown markers (## for bold from email)

The pastor bolds text in emails. Bold text arrives wrapped in ## markers. Handle them by context:

1. **##reference## (e.g. ##Mt26:26-29##, ##Rim5:17##):** Bold section header — the pastor bolds references for readability. Do NOT create a slide for it. Use it to identify which Bible passage follows.
2. **##title## at the very start (e.g. ##Nová zmluva##):** Presentation title. Use as the presentation name.
3. **##word## inside a verse (e.g. "aby sme ##verili## menu"):** Emphasized word. Make that word UPPERCASE within the verse slide's main text. Do NOT create a separate emphasis slide.
4. **##phrase## as a standalone line (not a reference, not inside a verse):** Create a separate emphasis slide with main = phrase in UPPERCASE.

Do NOT create separate emphasis slides for bold references or bold words inside verses. Only standalone bold phrases that are not Bible references get their own emphasis slide.

## Slide size rules

Character limit per slide is provided in the live system prompt. Pack multiple verses onto one slide: keep adding verses until the next verse would exceed the limit, then start a new slide.

Example with limit 200:
- Verse 1: 70 chars → slide has 70, room for more
- Verse 2: 40 chars → slide has 110, room for more
- Verse 3: 80 chars → slide has 190, tight
- Verse 4: 50 chars → 240 total, start new slide

Result: slide 1 has verses 1-3, slide 2 has verse 4.

If a single verse exceeds the limit, split it at a natural sentence boundary.

## Slovak Bible book abbreviations

Common mappings:
- Ž / Žalm → Žalmy
- Žid → Židom
- 1Sa → 1. Samuelova
- 1Kra → 1. Kráľov
- 2Ti → 2. Timotejovi
- Mat / Mt → Matúš
- Mar / Mr → Marek
- Luk → Lukáš
- Ján / Jan → Ján
- Sk → Skutky
- Rim → Rimanom
- 1Kor → 1. Korinťanom
- 2Kor → 2. Korinťanom
- Gal → Galatským
- Ef → Efezským
- Fil → Filipským
- Kol → Kolosanom
- 1Sol → 1. Solúnčanom
- 1Tim → 1. Timotejovi
- Tít → Títovi
- Flm → Filemonovi
- Prísl → Príslovia
- Iz → Izaiáš
- Jer → Jeremiáš
- Ez → Ezechiel
- Dan → Daniel
- 1Pet → 1. Petra
- 2Pet → 2. Petra

The server's `find_bible_passage` and `resolve_bible_slides` tools accept both abbreviated and full forms.

## Translation code mapping

- SEB = Slovenský ekumenický preklad (slk-seb)
- ROH = Roháčkov preklad 1936 (slk-roh)
- SEVP / ECAV = Slovenský evanjelický preklad (slk-sevp)
- MIL = Milostný preklad (slk-mil)
- KJV = King James Version (eng-kjv)

## Other formatting rules

- Text written in ALL CAPS by the pastor → keep uppercase in `main`.
- "Nazov:" or "Názov:" → presentation title.
- "Vers na spamet:" → memory verse, use a group "Vers na zapamätanie".
