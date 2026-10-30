---
paths:
  - "crates/presenter-ui/src/state/phone_shell.rs"
  - "crates/presenter-ui/src/state/view_nav.rs"
  - "crates/presenter-ui/src/components/phone_tab_bar.rs"
  - "crates/presenter-ui/src/components/header.rs"
  - "crates/presenter-ui/src/components/stage_preview.rs"
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
