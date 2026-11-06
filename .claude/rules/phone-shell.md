---
paths:
  - "crates/presenter-ui/src/state/phone_shell.rs"
  - "crates/presenter-ui/src/state/view_nav.rs"
  - "crates/presenter-ui/src/components/phone_tab_bar.rs"
  - "crates/presenter-ui/src/components/header.rs"
  - "crates/presenter-ui/src/components/header_menus.rs"
  - "crates/presenter-ui/src/components/surface_nav.rs"
  - "crates/presenter-ui/styles/header.css"
  - "crates/presenter-ui/styles/stage_preview.css"
  - "tests/e2e/operator-stage-preview-layout.spec.ts"
  - "tests/e2e/operator-header-layout.spec.ts"
  - "crates/presenter-ui/src/components/stage_preview.rs"
  - "crates/presenter-ui/src/components/more_sheet.rs"
  - "crates/presenter-ui/src/components/stage_layout_select.rs"
  - "crates/presenter-ui/src/components/ai_status.rs"
  - "crates/presenter-ui/src/components/resolume_status.rs"
  - "crates/presenter-ui/src/components/slide_list.rs"
  - "crates/presenter-ui/styles/phone_shell.css"
  - "crates/presenter-ui/index.html"
  - "tests/e2e/phone-shell.spec.ts"
  - "tests/e2e/worship-touch.spec.ts"
  - "tests/e2e/operator-labels-h3.spec.ts"
  - "crates/presenter-ui/styles/operator.css"
---

# The operator on a phone (#850 B1)

The 2026-10-12 service failed from a phone: the burger never closed, the header was 1,112 px wide,
fields were 13.6 px (iOS zooms) and the Bible page was a 153 px nested scroll box.

- **One condition.** `state::phone_shell::PHONE_SHELL_QUERY` (`(max-width: 768px), (pointer: coarse)
  and (max-height: 500px)`) → `body[data-shell="phone"|"desktop"]` + `AppContext::phone_shell`.
  CSS keys on `[data-shell]` only; never add another "phone" media query. A landscape phone
  (844×390, coarse) is a phone; a short desktop window (fine pointer) is not — the spec pins both.
  Safari < 14 has no `MediaQueryList.addEventListener`: keep the `addListener` fallback.
- **Navigation** = `PhoneTabBar` + `state::view_nav` (`VIEWS` is the ONE Slovak label source for
  both navs, Menovky included, `set_view` adds no history entry for the shown section, popstate reads
  `view_from_path`). The burger is gone; never bring back an overlay menu.
- **16 px fields** come from `@media (pointer: coarse) { input, select, textarea { font-size:
  max(16px, 1em) !important } }` in `phone_shell.css` — every surface, every new field.
  `!important` because the fields' own class rules are more specific.
- **No sideways scroll.** The 2026-10-15 offenders were the absolutely positioned brand-nav and
  the unwrapped timers row. A new flex row gets `flex-wrap`, a long token `overflow-wrap:
  anywhere`; `phone-shell.spec.ts` checks every section at 360×740, 390×844 and 844×390.
- **Touch never sends on press.** `slide_list.rs`: `pointerdown` triggers only for a mouse; touch
  and pen use `click`, cancelled when the pointer moved > 10 px (`moved_past_tap`). A scroll that
  starts on a card must send nothing (`worship-touch.spec.ts`, CDP `touchDrag`).
- **No second app on a phone:** the stage-preview iframe mounts only on a desktop. Its sibling
  controls stay: Clear, Follow, Ableton and the stage counter are the only way to reach them on
  Piesne, so a phone keeps `.operator__stage-preview` there as a row in `header-right`.
- **Height:** `operator.css` sets `min-height: 100vh`, which beats `100dvh` under a browser toolbar;
  the phone shell resets it to 0. Playwright has no dynamic toolbar, so only the computed value is
  testable.
- **Rail = landscape AND ≥ 560 px.** `interactive-widget=resizes-content` makes a small portrait
  phone wider than tall while the keyboard is open.
- **Header search:** not on Biblia since #850 B2c (`<Show>` on the view, phone and desktop);
  the verse console's field is the reference AND the full-text verse search there. The other
  sections keep it (songs, libraries).
- **Local CSS check without a WASM build:** on a deployed build delete every rule containing
  `data-shell` through the CSSOM, then `addStyleTag` the new `phone_shell.css`. Injecting on top
  alone leaves the old `display: none` rules winning.
- **E2E:** profiles `PHONE_360` (360×740), `PHONE_390`, `PHONE_LANDSCAPE` in `support.ts`;
  `(pointer: coarse)` needs `hasTouch` (ui skill #569); tap with `.tap()`.

- **Toasts (#875):** the `Toast` component owns the lifetime (one restarting 3.5 s timer), callers only set the message. On the phone's Bible Live tab the toast sits above `.console-bar` (`bible_console.css`), never over its buttons.

## Desktop header (#875)

`body.operator:not([data-shell="phone"]) .operator__header` is ONE 40 px flex row (H1, mockup v3, `styles/header.css`): Piesne, Biblia (`DIRECT_VIEWS`), the Stránky / Obrazovky / Pripojenia menus (`components/header_menus.rs`), the search (`order: 3`, `flex: 1`, ≥ 150 px; not on Biblia since #850 B2c), Stage Output + Stage Text (selects only, 32 px, label kept for screen readers), Live/Edit, ⚙ (= `settings`). The brand row is the phone's, hidden on a desktop; the version stays in the corner footer and at the bottom of the Pripojenia list (`operator__version-badge` still in the DOM).
- **Menus** are mounted only on a desktop (`<Show when=!phone>`), panels hidden with `hidden`, never unmounted, so the chips poll once and the Pripojenia button reads them through the chips' optional `report` signals (`summarize`: a host in `error` or AI `unavailable` = "Odpojené: <names>", orange; a first poll or a failed last call is not an outage). One open at a time (`RwSignal<Option<&str>>`); Escape (focus back on the button), a `pointerdown` outside every `[data-role="header-menu"]` and a click on any link or button in a panel close it. Panels are absolute under the button: the header never grows.
- **Every section button keeps `data-role="view-toggle"` + `data-view`**, also the Stránky items and ⚙ (its text is the label in an `sr-only` span, the gear is CSS), so the DOM order stays `VIEWS`' order. Specs open a section with `openView(page, id)` from `support.ts` (opens Stránky when needed), never by clicking a menu item directly; `headerMenu(page, id)` gives a menu's toggle and panel.
- **Stage preview (H2, `components/stage_preview.rs` `PreviewPlace`, `styles/stage_preview.css`):** on a desktop it lives in the catalog column, never over the page: on Piesne the LAST child of `.operator__catalog` (`Worship`: the live mirror, the column's full inner width at 16:9 = 360 × 202.5 px, 300 × 168.75 under 1025 px, flush with its bottom; Ableton + Follow top-left, broom top-right, counter bottom-right on the picture; the lists above shrink, `catalog-top` ≥ 120 px, `catalog-bottom` ≥ 140 px), on Biblia the last child of `.operator__catalog--bible` (`Bible`: the verse on the wall, the same 360 × 202.5 px / 300 × 168.75 under 1025 px since the Biblia column took the Piesne mockup-v3 frame (361 px at x = 0, 1 px `border-right`, no card, work area flush; `bible.css` end), the tabs scroll above it), on every other section the counter + broom in `header-right` (`Header`). A phone keeps its row in `header-right` (`Phone`, no mirror). Each mount point is a `<Show>` on shell + view, so exactly ONE copy is mounted (specs locate `clear-slide` / `stage-monitor` / `ableset-*` strictly) and at most one mirror socket. Heights are explicit (no `aspect-ratio`: an iPad Air 2 gets the desktop shell); the iframe scale is fixed per column width (0.1875 / 0.15625). `operator-stage-preview-layout.spec.ts` pins the placement, the corners and the hit targets at 1280×720 and 1920×1080. Desktop left columns drop operator.css's old `max-height: 100vh - 5.5rem` + `position: sticky; top: 4.75rem` (sized for the 173 px header; under 40 px sticky pushed the Biblia column 41 px below the slides, `wasm-bible` two-column test); the Biblia panel gets ONE `minmax(0, 1fr)` row and its column `height: auto` (bible.css's content-box `height: 100%` ran 33 px past the panel).
- **Row fit at 1024 px:** search `flex: 1 1 0` (it gives way first, ≥ 150 px) with `input { min-width: 0 }` (its intrinsic width ran out of the form), the header stage copy `flex: none` (a shrunk box let the broom spill over Live), menu buttons `min-width: 0` + ellipsis. Verify CSS against a CI binary locally: serve the new joined stylesheet for `/ui-pkg/app-*.css` via `page.route` in temp spec copies (netns recipe in `nameplates-ui.md`).
- `operator-header-layout.spec.ts`: header ≤ 44 px, no overlap, nothing past the right edge at 1024–2560 px on Piesne, Biblia and Časovače (counter + broom in the row); menu open/close/keyboard; a dead Resolume host (port 1) turns the closed Pripojenia button orange with its name.
- **Labels + control sizes (H3, mockup v1 table "Preklad nápisov"):** every operator label is Slovak, sentence case; no `text-transform: uppercase` anywhere in `.operator__header` / `.operator__main` (Nastavenia since H4 too) and no ALL-CAPS literals (the console's Zobraziť / Ďalší / Koniec knihy). Data stays as stored (library, song, book and layout names). Ableton / Sledovať pódium have ONE label each; the state is `data-state` + `aria-pressed` and the look (on = filled accent, `operator.css` end; on the desktop preview mockup v3's pill: on = green border + `#bbf7d0` text, `operator_frame.css` — `operator-controls.spec.ts` compares background + border + text colour). Desktop controls: ONE rule at the end of `controls.css` (moved there and extended by H4, below), `:where(body.operator:not([data-shell="phone"]) .operator__main > section) button|select|input { min-height: 40px }` — `:where()` keeps it at a bare `button`'s specificity so a component asking for MORE (the console's 44 / 48 px) still wins; never a per-page height. Exempt: the header row (32 px inside the 40 px row), the controls ON the stage preview picture (H2 sizes, reset by `.operator__stage-preview button { min-height: 0 }`) and, since the mockup-v3 frame below, the Piesne list rows (30 px), their "+" / ⋯ (26 px) and ⋮ menus and the work-area top row (32 px: "+ Snímka", the "Snímok v riadku" stepper, on Biblia too) — and Biblia's 34 px sub-tabs (`bible-tab`) and 36 px translation buttons (`console-translation*`, mockup v3 section 3, #850 B2c) — the owner-approved mockup wins over the 40 px rule there (`MOCKUP_DENSE` in the spec). 48 px = show on the wall / clear the wall (`console-show`, `console-preview-show`, `console-clear-live`, `clear-broadcast`, `slide-trigger`). A button with no look of its own takes `operator__list-action`. `operator-labels-h3.spec.ts` pins it on Piesne (Live + Edit) and Biblia (all three tabs) at 1280×720 and 1920×1080: no uppercase, the Slovak labels, no English word of the table outside the data containers, every control ≥ 40 px.
- **Mockup-v3 frame (owner 2026-11-06, "nezhoduje sa s tým, čo sme si schválili"):** `styles/operator_frame.css` (linked after stage_preview.css, every rule `body.operator:not([data-shell="phone"])`, so the phone is untouched) draws docs/mockups/operator-redesign-v3.html section 1 with its exact sizes. Piesne: the lists column 361 px (360 + the 1 px `border-right`) at x = 0, the work area flush, no card border / radius / gap; `catalog-top` is as tall as its rows up to the resizer's height (`--catalog-top-max` inline next to the phone's `height`, `height: auto !important`), Knižnice and Playlisty scroll their own rows; rows 30 px / 4 px apart / 13 px, the count muted 12 px inside the row, selected `#1d3a5f` + inset 3 px blue; the ⋮ menu absolute inside the row's right end (opacity 0 until hover / focus / the selected row — Playwright still clicks it); headings 13 px / 600; "+" and the "all" ⋯ 26 px squares (the button keeps the count for the phone: `operator__group-count-value` vs `operator__group-more-icon`); the song heading = "Piesne v knižnici" / "Piesne v playliste" + the open name muted (`context-title`, `hidden` while nothing is open; the phone shows the name alone through DOM order + sibling CSS). Work area: the song title = the draggable `slides-song-bubble`, rendered in `.operator__worktop` on a desktop and in the grid on a phone (`render_song_bubble(.., phone)`: exactly one copy), "+ Snímka" there too (the phone keeps the floating "+"); live cards ≥ 16:9 (`aspect-ratio` with `min-height: auto`, so a long slide grows), text top left 16 px, the header moved under it by `order` as the muted 11 px caption, a group without its own colour = "· Sloha 1" text, a coloured group a small pill; the slide on the wall 2 px red on `#3b1418` + `slide-live-tag` "NA STENE" (static span, shown by `.is-active`, so a stage change re-renders nothing). Naživo selected red, Úprava blue. Preview pills 26 px / 11 px on `rgba(15,23,42,.8)`, on = green border + `#bbf7d0`; broom 32 px circle; counter `#86efac` 12 px. Časovače / AI / Menovky: flush panes split by 1 px lines, titles 17 px bold. `operator-mockup-v3.spec.ts` pins it at 1280×720 and 1920×1080 + the phone's own look; compare visually by rendering the mockup's first `.app` at 100vw × 100vh next to a build screenshot.
- **Shared controls (H4, mockup v1 "Jeden vzhľad prvkov"):** `styles/controls.css` (linked right after operator.css) is the ONE look of a button (`.ui-button` + `--primary` / `--danger` / `--quiet` / `--icon`), a state toggle (`.ui-toggle`: one label, `data-state="on|off"` + `aria-pressed`, on = filled accent), the fields (text / number / select inside `.settings-layout`, `.stream-editor`, `.nameplates-view`) and a top row for a page without the operator header (`.ui-topbar` + `__title` / `__tab` / `__button` / `__panel`, the header's 32-px-in-40-px look; a form a row button opens is a `.ui-topbar__panel` under the row, the row never grows). Nastavenia, the Stream editor and the plate list (editor + operator Menovky) use only these; a page never restyles them, it places them (`settings__add-source`: `align-self`). The 40 px rule there covers the operator sections (Nastavenia included now), the standalone `body.settings .settings-layout` and `.stream-editor` minus its `.ui-topbar` (exempt like the operator header; its `__panel` is 40 px again). Exempt on purpose: the stream canvas overlay handles and the plate drag handle are not buttons / are 40 px buttons respectively, checkboxes / radios / colour / file inputs keep their native size. `operator-labels-h4.spec.ts` pins Nastavenia (operator panel + standalone, seeded Resolume / Android / NDI rows, the inline editor open) and `/ui/stream` (all three tabs, a scene with a countdown element, the output name panel) at 1280×720 and 1920×1080: no uppercase, the Slovak labels, no English word of its table outside the data containers, every control ≥ 40 px, buttons only `.ui-button` / `.ui-toggle` (+ the top-row classes), the stream top row ≤ 44 px with nothing outside it.
- **Verifying Rust UI changes locally without building the server:** `trunk build --release` (nightly) in `crates/presenter-ui`, then strip `integrity`/`crossorigin`/preload from `dist/index.html` and run `scripts/ci/bundle_ui_css.py dist` (it refuses links that still carry `integrity`). Run the specs against the CI `build-artifacts` binary with a temp module that `context.route`s `**/ui-pkg/**` to the local `dist/` and replaces every document response containing `/ui-pkg/` with the local `index.html` (also patch `browser.newContext` for specs that open their own contexts). Copy the main checkout's `data/bibles-private/` into the worktree so `refreshDevData` works.
- **"Viac" sheet** (`components/more_sheet.rs`): mounted once in the phone shell, toggled by
  `data-open`. Opening pushes a same-URL history entry; ✕, the backdrop and Escape close at once
  and go back once (a double tap must not leave the page), then a 350 ms transparent shield takes
  the double tap's second tap (the sheet is gone by then, the slide under it would go to the wall);
  a real Back closes through `operator.rs`'s popstate; a reload with it open drops its entry. Stage controls mount per open: a closed sheet keeps no half-answered
  confirm.
- **The chips move** between the desktop header and the sheet when a window or an iPad crosses
  768 px, so `<Show>` disposes a mounted copy. A timer that outlives its component reads disposed
  signals and PANICS (`get_untracked`), and a stacked one doubles the polls. A poller in a
  component that can unmount holds its timer in `StoredValue::new_local(interval)` (dropped with
  the owner = cancelled, works on the host build, `on_cleanup` does not), and reads after an
  `await` with `try_get_untracked`. Never `interval.forget()` there.
- **Stage Output from a phone** = `StageLayoutSelect confirm=true`: the change waits for
  "Prepnúť"; the select keeps showing the layout on air. The desktop header switches at once. A
  refused POST puts the select back.
