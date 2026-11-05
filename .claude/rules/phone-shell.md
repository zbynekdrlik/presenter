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
- **Header search:** stays on Biblia until the verse console replaces it (spec B2c); it is the
  only full-text verse search.
- **Local CSS check without a WASM build:** on a deployed build delete every rule containing
  `data-shell` through the CSSOM, then `addStyleTag` the new `phone_shell.css`. Injecting on top
  alone leaves the old `display: none` rules winning.
- **E2E:** profiles `PHONE_360` (360×740), `PHONE_390`, `PHONE_LANDSCAPE` in `support.ts`;
  `(pointer: coarse)` needs `hasTouch` (ui skill #569); tap with `.tap()`.

- **Toasts (#875):** the `Toast` component owns the lifetime (one restarting 3.5 s timer), callers only set the message. On the phone's Bible Live tab the toast sits above `.console-bar` (`bible_console.css`), never over its buttons.

## Desktop header (#875)

`body.operator:not([data-shell="phone"]) .operator__header` is ONE 40 px flex row (H1, mockup v3, `styles/header.css`): Piesne, Biblia (`DIRECT_VIEWS`), the Stránky / Obrazovky / Pripojenia menus (`components/header_menus.rs`), the search (`order: 3`, `flex: 1`, ≥ 150 px), Stage Output + Stage Text (selects only, 32 px, label kept for screen readers), Live/Edit, ⚙ (= `settings`). The brand row is the phone's, hidden on a desktop; the version stays in the corner footer and at the bottom of the Pripojenia list (`operator__version-badge` still in the DOM).
- **Menus** are mounted only on a desktop (`<Show when=!phone>`), panels hidden with `hidden`, never unmounted, so the chips poll once and the Pripojenia button reads them through the chips' optional `report` signals (`summarize`: a host in `error` or AI `unavailable` = "Odpojené: <names>", orange; a first poll or a failed last call is not an outage). One open at a time (`RwSignal<Option<&str>>`); Escape (focus back on the button), a `pointerdown` outside every `[data-role="header-menu"]` and a click on any link or button in a panel close it. Panels are absolute under the button: the header never grows.
- **Every section button keeps `data-role="view-toggle"` + `data-view`**, also the Stránky items and ⚙ (its text is the label in an `sr-only` span, the gear is CSS), so the DOM order stays `VIEWS`' order. Specs open a section with `openView(page, id)` from `support.ts` (opens Stránky when needed), never by clicking a menu item directly; `headerMenu(page, id)` gives a menu's toggle and panel.
- **Stage preview (H2, `components/stage_preview.rs` `PreviewPlace`, `styles/stage_preview.css`):** on a desktop it lives in the catalog column, never over the page: on Piesne the LAST child of `.operator__catalog` (`Worship`: the live mirror, the column's full inner width at 16:9 = 360 × 202.5 px, 300 × 168.75 under 1025 px, flush with its bottom; Ableton + Follow top-left, broom top-right, counter bottom-right on the picture; the lists above shrink, `catalog-top` ≥ 120 px, `catalog-bottom` ≥ 140 px), on Biblia the last child of `.operator__catalog--bible` (`Bible`: the verse on the wall, 318 × 179 px, the tabs scroll above it), on every other section the counter + broom in `header-right` (`Header`). A phone keeps its row in `header-right` (`Phone`, no mirror). Each mount point is a `<Show>` on shell + view, so exactly ONE copy is mounted (specs locate `clear-slide` / `stage-monitor` / `ableset-*` strictly) and at most one mirror socket. Heights are explicit (no `aspect-ratio`: an iPad Air 2 gets the desktop shell); the iframe scale is fixed per column width (0.1875 / 0.15625). `operator-stage-preview-layout.spec.ts` pins the placement, the corners and the hit targets at 1280×720 and 1920×1080.
- `operator-header-layout.spec.ts`: header ≤ 44 px, no overlap, nothing past the right edge at 1024–2560 px on Piesne, Biblia and Časovače (counter + broom in the row); menu open/close/keyboard; a dead Resolume host (port 1) turns the closed Pripojenia button orange with its name.
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
