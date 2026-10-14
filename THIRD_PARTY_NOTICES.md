# Third-party notices

## Bible-Passage-Reference-Parser-Languages (MIT)

`data/bible-book-names/bprp-slk.yaml`, `bprp-ces.yaml`, `bprp-eng.yaml` and
`bprp-eng-preferred.yaml` are copies (plus one source comment line) of
`book_names/all/{slk,ces,eng}.yaml` and `book_names/preferred/eng.yaml` from
<https://github.com/openbibleinfo/Bible-Passage-Reference-Parser-Languages> at commit
`5735787d970a87483eac580e5c1d1b5192c3ed48` (2026-02-08). License: MIT,
Copyright (c) 2026 Stephen Smith — full text in `data/bible-book-names/LICENSE-BPRP.md`.
`crates/presenter-core/src/bible/resolver/names_table.rs` is generated from them by
`scripts/dev/gen_bible_book_names.py`.

## OpenBible.info cross-references (CC BY)

`frequency_rank` in `data/bible-book-names/curated.yaml` ranks the 66 books by the
cross-reference votes in <https://a.openbible.info/data/cross-references.zip>
(OpenBible.info, Creative Commons Attribution license), downloaded 2026-10-12.

## biblia.sk book menus

`data/bible-book-names/biblia-sk.json` lists the book abbreviations and titles of the SEB,
SEP, SSV and ROH reader menus of <https://www.biblia.sk> (Slovenská biblická spoločnosť):
names and abbreviations only, no Bible text.
