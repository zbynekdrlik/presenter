/**
 * The desktop Piesne page = the approved mockup v3 (#875, owner 2026-11-06:
 * "aktuálny vizuál Presentera sa nezhoduje s tým, čo sme si schválili",
 * docs/mockups/operator-redesign-v3.html section "1 · Piesne").
 *
 * At 1280×720 and 1920×1080:
 *   - the lists column is 360 px wide at x = 0, flush, split from the work area
 *     by one 1 px line: no card border, no radius, no gap between the columns;
 *   - list rows are 30 px tall, 4 px apart, 13 px text, the count muted 12 px
 *     inside the row, the selected row #1d3a5f with a 3 px blue bar, the ⋮ menu
 *     inside its row;
 *   - section headings 13 px / 600 / no letter-spacing, "+" a 26 px square,
 *     no round count bubble;
 *   - the work area's top row: the song title bold 17 px, "Snímok v riadku"
 *     with 32 px stepper buttons;
 *   - the slide on the wall: 2 px red border and "NA STENE" in its caption;
 *     the cards are 16:9 (at least);
 *   - Naživo selected = red, Úprava selected = blue;
 *   - the preview pills are 26 px / 11 px (on = green border), the broom a
 *     32 px circle, the counter green 12 px.
 * Biblia's work area (section "3 · Biblia v rovnakom rámci"): the title row
 * "Ján 3:16-18 · SEB + KJV" bold 17 px with the 32 px stepper on the right and
 * the selection controls in the "+ Snímka" look; 16:9 verse cards (growing,
 * never cut) with the text top left and the caption "1 · Ján 3:16" grey 11 px
 * at the bottom; after Zobraziť the card on the wall is red with "NA STENE",
 * and a card's caption still puts it on the wall.
 * The phone shell keeps its own look (count bubble, no caption tag).
 *
 * Also asserts zero browser console errors/warnings per
 * ci/browser-console-zero-errors.md.
 */

import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  openView,
  PHONE_390,
  refreshDevData,
  selectPassage,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

const VIEWPORTS = [
  { width: 1280, height: 720 },
  { width: 1920, height: 1080 },
];

const RED = "rgb(220, 38, 38)";
const BLUE = "rgb(59, 130, 246)";

type Look = {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
  fontSize: string;
  fontWeight: string;
  letterSpacing: string;
  color: string;
  background: string;
  boxShadow: string;
  borderRadius: string;
  borderTop: string;
  borderRight: string;
  borderLeft: string;
  marginLeft: string;
  visible: boolean;
};

/** Box + the computed style the mockup pins, of the first match. */
async function look(page: Page, selector: string): Promise<Look> {
  const result = await page.evaluate((sel) => {
    const el = document.querySelector<HTMLElement>(sel);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return {
      left: r.left,
      top: r.top,
      right: r.right,
      bottom: r.bottom,
      width: r.width,
      height: r.height,
      fontSize: s.fontSize,
      fontWeight: s.fontWeight,
      letterSpacing: s.letterSpacing,
      color: s.color,
      background: s.backgroundColor,
      boxShadow: s.boxShadow,
      borderRadius: s.borderTopLeftRadius,
      borderTop: `${s.borderTopWidth} ${s.borderTopStyle}`,
      borderRight: `${s.borderRightWidth} ${s.borderRightStyle}`,
      borderLeft: `${s.borderLeftWidth} ${s.borderLeftStyle}`,
      marginLeft: s.marginLeft,
      visible: el.getClientRects().length > 0 && s.visibility !== "hidden" && s.display !== "none",
    };
  }, selector);
  if (!result) throw new Error(`${selector}: not in the page`);
  return result;
}

function near(actual: number, expected: number, what: string, tolerance = 1) {
  expect(Math.abs(actual - expected), `${what}: ${actual} ≈ ${expected}`).toBeLessThanOrEqual(tolerance);
}

/** Piesne with the first library, its first song, and its second slide sent to the wall. */
async function openSongLive(page: Page, viewport: { width: number; height: number }) {
  await page.setViewportSize(viewport);
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
  await expect(page.locator("body")).toHaveAttribute("data-shell", "desktop");
  await openView(page, "worship");
  await page.locator('[data-role="library-item"]').first().click();
  await page.locator('[data-role="presentation-item"]').first().click();
  const slides = page.locator('[data-role="slides"] article[data-slide-id]');
  await expect(slides.nth(1)).toBeVisible();
  await slides.nth(1).click();
  await expect(slides.nth(1)).toHaveClass(/is-active/);
}

for (const viewport of VIEWPORTS) {
  const size = `${viewport.width}×${viewport.height}`;

  test(`Piesne at ${size} is mockup v3`, async ({ page }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    await openSongLive(page, viewport);
    const worship = '[data-view-panel="worship"]';

    // The frame: a 360 px column at x = 0, one 1 px line, no card, no gap.
    const column = await look(page, `${worship} [data-role="catalog"]`);
    near(column.left, 0, "column left");
    near(column.width, 361, "column width (360 + the line)");
    expect(column.borderRight).toBe("1px solid");
    expect(column.borderTop).toMatch(/^0px/);
    expect(column.borderLeft).toMatch(/^0px/);
    expect(column.borderRadius).toBe("0px");
    const work = await look(page, `${worship} .operator__slides-column`);
    near(work.left, column.right, "work area starts at the line");
    expect(work.borderTop).toMatch(/^0px/);
    expect(work.borderLeft).toMatch(/^0px/);
    expect(work.borderRadius).toBe("0px");
    near(work.right, viewport.width, "work area reaches the right edge");

    // Section headings, "+" squares, no count bubble.
    const heading = await look(page, `${worship} .operator__group-header h2`);
    expect(heading.fontSize).toBe("13px");
    expect(heading.fontWeight).toBe("600");
    expect(heading.letterSpacing).toBe("normal");
    await expect(page.locator(`${worship} .operator__group h2`).first()).toHaveText("Knižnice");
    await expect(page.locator(`${worship} .operator__group h2`).nth(1)).toHaveText("Playlisty");
    await expect(page.locator(".operator__presentations-kind")).toHaveText("Piesne v knižnici");
    for (const role of ["library-create", "playlist-create", "presentation-create", "library-more"]) {
      const button = await look(page, `[data-role="${role}"]`);
      near(button.width, 26, `${role} width`);
      near(button.height, 26, `${role} height`);
    }
    await expect(page.locator(`${worship} .operator__group-count-value`).first()).toBeHidden();
    await expect(page.locator('[data-role="presentation-count"]')).toBeHidden();

    // Dense rows: 30 px, 4 px apart, 13 px, count muted 12 px, selected = blue bar.
    const libraryRow = await look(page, `${worship} .operator__list-row`);
    near(libraryRow.height, 30, "library row height");
    const libraryButton = await look(page, '[data-role="library-item"][data-active="true"]');
    expect(libraryButton.fontSize).toBe("13px");
    // Polled: rows fade their background (operator.css transition).
    await expect
      .poll(async () => (await look(page, '[data-role="library-item"][data-active="true"]')).background)
      .toBe("rgb(29, 58, 95)");
    expect(libraryButton.boxShadow).toContain("inset");
    expect(libraryButton.boxShadow).toContain("3px 0px 0px");
    const count = await look(page, '[data-role="library-item"] [data-role="library-count"]');
    expect(count.fontSize).toBe("12px");
    expect(count.color).toBe("rgb(148, 163, 184)");
    expect(count.right).toBeLessThanOrEqual(libraryRow.right);
    const menu = await look(page, '[data-action="library-edit"]');
    expect(menu.left, "⋮ inside its row").toBeGreaterThanOrEqual(libraryRow.left);
    expect(menu.right, "⋮ inside its row").toBeLessThanOrEqual(libraryRow.right + 0.5);
    expect(menu.top).toBeGreaterThanOrEqual(libraryRow.top - 0.5);
    expect(menu.bottom).toBeLessThanOrEqual(libraryRow.bottom + 0.5);
    const songs = page.locator('[data-role="presentation-item"]');
    const first = await songs.nth(0).boundingBox();
    const second = await songs.nth(1).boundingBox();
    near(first!.height, 30, "song row height");
    near(second!.y - first!.y, 34, "30 px rows, 4 px apart");
    const song = await look(page, '[data-role="presentation-item"].is-active');
    expect(song.fontSize).toBe("13px");
    await expect
      .poll(async () => (await look(page, '[data-role="presentation-item"].is-active')).background)
      .toBe("rgb(29, 58, 95)");

    // The work area's top row: the song title and "Snímok v riadku − N +".
    const title = await look(page, '.operator__worktop [data-role="slides-song-bubble"]');
    expect(title.fontSize).toBe("17px");
    expect(title.fontWeight).toBe("700");
    const songName = (await songs.nth(0).locator("span").first().innerText()).trim();
    await expect(page.locator('.operator__worktop [data-role="slides-song-bubble"]')).toHaveText(songName);
    const worktopControl = page.locator(`${worship} .operator__worktop [data-role="slide-columns-control"]`);
    await expect(worktopControl.locator(".operator__slide-columns-label")).toHaveText("Snímok v riadku");
    const label = await look(page, `${worship} .operator__worktop .operator__slide-columns`);
    expect(label.fontSize).toBe("13px");
    expect(label.color).toBe("rgb(148, 163, 184)");
    for (const role of ["slide-columns-decrease", "slide-columns-increase"]) {
      const button = await look(page, `${worship} .operator__worktop [data-role="${role}"]`);
      near(button.width, 32, `${role} width`);
      near(button.height, 32, `${role} height`);
    }
    expect(title.top, "title in the top row").toBeLessThan(label.bottom);

    // Slides: 16:9 cards; the one on the wall red with "NA STENE".
    const live = await look(page, '[data-role="slides"] article.is-active');
    expect(live.borderTop).toBe("2px solid");
    await expect
      .poll(() => page.locator('[data-role="slides"] article.is-active').evaluate((el) => getComputedStyle(el).borderTopColor))
      .toBe(RED);
    await expect(page.locator('[data-role="slides"] article.is-active [data-role="slide-live-tag"]')).toBeVisible();
    await expect(page.locator('[data-role="slides"] article.is-active [data-role="slide-live-tag"]')).toHaveText("NA STENE");
    await expect(page.locator('[data-role="slides"] article:not(.is-active) [data-role="slide-live-tag"]').first()).toBeHidden();
    const card = await look(page, '[data-role="slides"] article:not(.is-active)');
    expect(card.borderTop).toBe("1px solid");
    expect(card.borderRadius).toBe("10px");
    expect(card.height, "16:9 at least").toBeGreaterThanOrEqual((card.width * 9) / 16 - 1);
    const caption = await look(page, '[data-role="slides"] article:not(.is-active) .operator__slide-index');
    expect(caption.fontSize).toBe("11px");
    expect(caption.top, "caption at the bottom").toBeGreaterThan(card.top + card.height / 2);

    // Naživo red, Úprava blue.
    const liveToggle = '[data-role="mode-toggle"][data-mode="live"]';
    const editToggle = '[data-role="mode-toggle"][data-mode="edit"]';
    // Polled: the toggle fades its background (operator.css transition).
    const background = (selector: string) => async () => (await look(page, selector)).background;
    await expect.poll(background(liveToggle)).toBe(RED);
    await page.locator(editToggle).click();
    await expect(page.locator("body")).toHaveAttribute("data-mode", "edit");
    await expect.poll(background(editToggle)).toBe(BLUE);
    await expect.poll(background(liveToggle)).not.toBe(RED);
    await page.locator(liveToggle).click();
    await expect(page.locator("body")).toHaveAttribute("data-mode", "live");

    // The preview: 26 px pills (on = green border), a 32 px broom circle, the green counter.
    for (const role of ["ableset-enable", "ableset-follow"]) {
      const pill = await look(page, `[data-role="${role}"]`);
      near(pill.height, 26, `${role} height`);
      expect(pill.fontSize).toBe("11px");
      expect(pill.background).toBe("rgba(15, 23, 42, 0.8)");
    }
    await page.locator('[data-role="ableset-enable"]').evaluate((el) => el.setAttribute("data-state", "on"));
    // Only the look is checked here (no AbleSet in CI); polled past the transition.
    await expect
      .poll(() =>
        page.locator('[data-role="ableset-enable"]').evaluate((el) => {
          const s = getComputedStyle(el);
          return { border: s.borderTopColor, color: s.color };
        }),
      )
      .toEqual({ border: "rgb(34, 197, 94)", color: "rgb(187, 247, 208)" });
    const broom = await look(page, '[data-role="clear-slide"]');
    near(broom.width, 32, "broom width");
    near(broom.height, 32, "broom height");
    expect(broom.borderRadius).toBe("50%");
    const counter = await look(page, '[data-role="stage-monitor"]');
    expect(counter.fontSize).toBe("12px");
    expect(counter.color).toBe("rgb(134, 239, 172)");

    expect(errors).toEqual([]);
  });

  // Mockup section "3 · Biblia v rovnakom rámci": the same title row and
  // cards as Piesne, the passage named with its translations.
  test(`Biblia work area at ${size} is mockup v3`, async ({ page }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    await page.setViewportSize(viewport);
    await page.goto(`${baseURL}/ui/operator/bible`);
    await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
    await expect(page.locator("body")).toHaveAttribute("data-shell", "desktop");
    const root = page.locator('[data-bible-panel="live"] [data-role="bible-console"]');
    await expect(root).toHaveAttribute("data-books-translation", /.+/, { timeout: 30_000 });
    await page.locator('[data-role="console-translation"][data-kind="main"][data-translation="slk-seb"]').click();
    await expect(root).toHaveAttribute("data-books-translation", "slk-seb");
    await page.locator('[data-role="console-translation"][data-kind="secondary"][data-translation="eng-kjv"]').click();
    const bible = '[data-view-panel="bible"]';

    // Empty: no title text, the empty line in the grid, no selection controls.
    await expect(page.locator(`${bible} [data-role="bible-worktop-title"]`)).toHaveText("");
    await expect(page.locator(`${bible} [data-role="slides"] .operator__slides-empty`)).toHaveText("Načítaj verše a zobrazia sa tu snímky.");
    await expect(page.locator('[data-role="bible-selection"]')).toHaveCount(0);

    await selectPassage(page, "jn 3:16-18");

    // The title row: the passage + both translations bold 17 px, the stepper right.
    const titleSel = `${bible} .operator__worktop [data-role="bible-worktop-title"]`;
    await expect(page.locator(titleSel)).toHaveText("Ján 3:16-18 · SEB + KJV");
    const title = await look(page, titleSel);
    expect(title.fontSize).toBe("17px");
    expect(title.fontWeight).toBe("700");
    const control = page.locator(`${bible} .operator__worktop [data-role="slide-columns-control"]`);
    await expect(control.locator(".operator__slide-columns-label")).toHaveText("Snímok v riadku");
    const stepper = await look(page, `${bible} .operator__worktop .operator__slide-columns`);
    expect(stepper.fontSize).toBe("13px");
    expect(stepper.color).toBe("rgb(148, 163, 184)");
    near(stepper.right, viewport.width - 12, "stepper at the row's right end");
    for (const role of ["slide-columns-decrease", "slide-columns-increase"]) {
      const button = await look(page, `${bible} .operator__worktop [data-role="${role}"]`);
      near(button.width, 32, `${role} width`);
      near(button.height, 32, `${role} height`);
    }
    expect(title.top, "title in the top row").toBeLessThan(stepper.bottom);
    // The selection controls stay in the row, in the "+ Snímka" look.
    const selectAll = await look(page, `${bible} .operator__worktop [data-role="select-all-slides"]`);
    near(selectAll.height, 32, "Vybrať všetko height");
    expect(selectAll.fontSize).toBe("13px");
    await expect(page.locator(`${bible} .operator__worktop [data-role="presentation-add"]`)).toBeVisible();
    // "+ Snímka" fills a prepared presentation: not offered on the Live tab.
    await expect(page.locator(`${bible} [data-role="add-empty-slide"]`)).toHaveCount(0);

    // The cards: 16:9 at least, 10 px radius, 1 px border, text top left,
    // the caption "1 · Ján 3:16…" grey 11 px at the bottom.
    const cards = page.locator(`${bible} [data-role="slide-card"]`);
    const card = await look(page, `${bible} [data-role="slide-card"]`);
    expect(card.borderTop).toBe("1px solid");
    expect(card.borderRadius).toBe("10px");
    expect(card.background).toBe("rgb(30, 41, 59)");
    expect(card.height, "16:9 at least").toBeGreaterThanOrEqual((card.width * 9) / 16 - 1);
    // A long slide grows instead of being cut: nothing spills out of a card,
    // and its caption sits below its text.
    for (let i = 0; i < (await cards.count()); i++) {
      const fits = await cards.nth(i).evaluate((el) => {
        const box = el.getBoundingClientRect();
        const text = el.querySelector(".operator__slide-bodies")!.getBoundingClientRect();
        const caption = el.querySelector('[data-role="slide-caption"]')!.getBoundingClientRect();
        return {
          inside: text.bottom <= box.bottom + 0.5 && caption.bottom <= box.bottom + 0.5,
          below: caption.top >= text.bottom - 0.5,
        };
      });
      expect(fits, `card ${i + 1} holds its text and caption`).toEqual({ inside: true, below: true });
    }
    const text = await look(page, `${bible} [data-role="slide-card"] .operator__slide-text--main`);
    near(text.left, card.left + 12, "text inset 12 px from the left");
    expect(text.top, "text at the top").toBeLessThan(card.top + 40);
    expect(
      await page.locator(`${bible} [data-role="slide-card"] .operator__slide-text--main`).first().evaluate((el) => getComputedStyle(el).textAlign),
    ).toBe("left");
    await expect(cards.first().locator('[data-role="slide-caption"]')).toHaveText(/^1 · Ján 3:16/);
    const caption = await look(page, `${bible} [data-role="slide-card"] [data-role="slide-caption"]`);
    expect(caption.fontSize).toBe("11px");
    expect(caption.color).toBe("rgb(148, 163, 184)");
    expect(caption.top, "caption at the bottom").toBeGreaterThan(card.top + card.height / 2);
    // #828 counts stay on the card.
    await expect(cards.first().locator('[data-role="slide-char-count"][data-field="main"]')).toBeVisible();
    await expect(cards.first().locator('[data-role="slide-char-count"][data-field="translation"]')).toBeVisible();
    await expect(cards.first().locator('[data-role="slide-live-tag"]')).toBeHidden();

    // Zobraziť: the first card goes red with "NA STENE" in its caption.
    await page.locator('[data-role="console-show"]').click();
    await expect(cards.first()).toHaveAttribute("data-on-wall", "true");
    await expect
      .poll(() => cards.first().evaluate((el) => getComputedStyle(el).borderTopColor))
      .toBe(RED);
    const live = await look(page, `${bible} [data-role="slide-card"][data-on-wall="true"]`);
    expect(live.borderTop).toBe("2px solid");
    expect(live.background).toBe("rgb(59, 20, 24)");
    await expect(cards.first().locator('[data-role="slide-live-tag"]')).toBeVisible();
    await expect(cards.first().locator('[data-role="slide-live-tag"]')).toHaveText("NA STENE");
    // The card's caption is still its trigger: the second card takes the wall.
    if ((await cards.count()) > 1) {
      await cards.nth(1).locator('[data-role="slide-trigger-zone"]').click();
      await expect(cards.nth(1)).toHaveAttribute("data-on-wall", "true");
      await expect(cards.first()).toHaveAttribute("data-on-wall", "false");
      await expect(cards.first().locator('[data-role="slide-live-tag"]')).toBeHidden();
    }

    expect(errors).toEqual([]);
  });
}

test("the phone keeps its own lists and slides", async ({ browser }) => {
  const context = await browser.newContext({ ...PHONE_390, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.goto("/ui/operator");
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
  await expect(page.locator("body")).toHaveAttribute("data-shell", "phone");
  await expect(page.locator(".operator__group-count-value").first()).toBeVisible();
  await expect(page.locator(".operator__group-more-icon").first()).toBeHidden();
  await expect(page.locator(".operator__presentations-kind")).toHaveText("Piesne v knižnici");
  await page.locator('[data-role="library-item"]').first().tap();
  // The heading is the library's name alone, with the count beside it.
  await expect(page.locator('[data-role="context-title"]')).toBeVisible();
  await expect(page.locator(".operator__presentations-kind")).toBeHidden();
  await expect(page.locator('[data-role="presentation-count"]')).toBeVisible();
  // Open a song the way a session restore does (on a phone the song list
  // lies under the slides, worship-touch.spec.ts).
  const songId = await page.locator('[data-role="presentation-item"]').first().getAttribute("data-presentation-id");
  await page.evaluate((id) => {
    sessionStorage.setItem("presenter:currentPresentationId", JSON.stringify(id));
    sessionStorage.setItem("presenter:view", JSON.stringify("worship"));
    sessionStorage.setItem("presenter:mode", JSON.stringify("live"));
  }, songId);
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
  await expect(page.locator('[data-role="slides"] article[data-slide-id]').first()).toBeVisible();
  // The sticky song bubble stays in the slide grid, the floating "+" over it.
  await expect(page.locator('[data-role="slides"] [data-role="slides-song-bubble"]')).toBeVisible();
  await expect(page.locator(".operator__worktop-add")).toHaveCount(0);
  await expect(page.locator(".operator__slides-add-floating")).toBeVisible();
  await expect(page.locator(".operator__slide-columns-label:visible")).toHaveCount(0);
  await expect(page.locator('[data-role="slide-live-tag"]:visible')).toHaveCount(0);
  expect(errors).toEqual([]);
  await context.close();
});

test("the phone's Bible card shows its reference alone", async ({ browser }) => {
  const context = await browser.newContext({ ...PHONE_390, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.goto("/ui/operator/bible");
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
  await selectPassage(page, "Ján 3:16-18");
  const card = page.locator('[data-view-panel="bible"] [data-role="slide-card"]').first();
  await expect(card.locator(".operator__slide-trigger-ref")).toBeVisible();
  await expect(card.locator('[data-role="slide-caption"]')).toBeHidden();
  expect(errors).toEqual([]);
  await context.close();
});
