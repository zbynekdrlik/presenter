/**
 * Stream-graphics OUTPUT page CONTENT E2E (/stream/{slug}) — #710, epic #718.
 *
 * Verifies the LYRICS and VERSE elements the #710 lane adds to the transparent
 * OBS output page. Lyrics bind to the EXISTING worship-stage pipeline (a slide
 * triggered via POST /stage/state), verse binds to the EXISTING Bible pipeline
 * (POST /bible/trigger-slide + /bible/clear) — no new content events. Asserts:
 *   - lyrics shows the current slide's main + translation with configured styles;
 *   - a slide change updates the lyrics text live;
 *   - translation is HIDDEN when show_translation=false (toggle);
 *   - verse shows main text + reference (+ secondary when show_secondary), and
 *     the secondary lines are HIDDEN when show_secondary=false (toggle);
 *   - BibleCleared removes the verse; a stage clear removes the lyrics;
 *   - a 4000-char SlideText is clipped to its Frame (overflow hidden, box does
 *     not grow past the Frame);
 *   - zero console errors AND warnings (asserted last).
 *
 * ── INTEGRATION NOTE (same as #709's stream-output.spec.ts) ───────────────────
 * The `/stream/api/*` scene/element seeding + activation below is owned by the
 * PARALLEL lanes #706 (StreamManager + stream_state LiveEvent) and #707 (REST).
 * Those land on other branches, so this spec PASSES only after #706/#707 are
 * integrated with #709+#710. The worship-stage (POST /stage/state) and Bible
 * (POST /bible/trigger-slide, /bible/clear) endpoints it triggers DO exist
 * today — only the scene/element seeding depends on the sibling lanes. Every
 * `/stream/api/*` shape below is the epic-#718 §5/§8 + #707 contract; the
 * create-body field names are CONTRACT ASSUMPTIONS centralized in the helpers
 * here for one-place reconcile. See the ticket hand-back's CONTRACT-ASSUMPTIONS.
 */

import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

const SLUG = "stream"; // the migration-seeded default output.

// Whitelisted, web-safe font (avoids a missing-woff2 404 in the zero-console
// gate — the OFL families' @font-face is a #709 follow-up).
const FONT = "Arial";

function textStyle(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    font_family: FONT,
    size_pct: 6,
    color: "#ffffff",
    weight: 700,
    align: "center",
    line_height: 1.2,
    ...overrides,
  };
}

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

// ── REST helpers (CONTRACT-shaped; see integration note) ──────────────────────

async function createScene(
  request: APIRequestContext,
  name: string,
  kind: "base" | "overlay",
  position: number,
): Promise<number> {
  const resp = await request.post(`${baseURL}/stream/api/outputs/${SLUG}/scenes`, {
    data: { name, kind, position },
  });
  expect(resp.ok(), `create scene ${name} -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

async function addElement(
  request: APIRequestContext,
  sceneId: number,
  zOrder: number,
  props: Record<string, unknown>,
): Promise<number> {
  const resp = await request.post(`${baseURL}/stream/api/scenes/${sceneId}/elements`, {
    data: { kind: props.kind, z_order: zOrder, props },
  });
  expect(resp.ok(), `add element -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

async function activateBase(request: APIRequestContext, sceneId: number): Promise<void> {
  const resp = await request.put(`${baseURL}/stream/api/outputs/${SLUG}/active-scene`, {
    data: { scene_id: sceneId },
  });
  expect(resp.ok(), `activate base -> ${resp.status()}`).toBeTruthy();
}

async function activateOverlay(request: APIRequestContext, sceneId: number): Promise<void> {
  const resp = await request.put(`${baseURL}/stream/api/outputs/${SLUG}/overlays/${sceneId}`, {
    data: { active: true },
  });
  expect(resp.ok(), `activate overlay -> ${resp.status()}`).toBeTruthy();
}

/** Create a library + presentation, set the first slide's text, return ids. */
async function seedSong(
  request: APIRequestContext,
  main: string,
  translation: string,
): Promise<{ presentationId: string; slideId: string }> {
  const libResp = await request.post(`${baseURL}/libraries`, {
    data: { name: `Lyrics Lib ${Date.now()}-${Math.random()}` },
  });
  expect(libResp.ok()).toBeTruthy();
  const library = (await libResp.json()) as { id: string };

  const presResp = await request.post(
    `${baseURL}/libraries/${library.id}/presentations`,
    { data: { name: "Lyrics Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const pres = (await presResp.json()) as {
    presentation: { id: string; slides: Array<{ id: string }> };
  };
  const presentationId = pres.presentation.id;
  const slideId = pres.presentation.slides[0].id;

  const patchResp = await request.patch(
    `${baseURL}/presentations/${presentationId}/slides/${slideId}`,
    { data: { main, translation, stage: "", group: "Verse 1" } },
  );
  expect(patchResp.ok()).toBeTruthy();
  return { presentationId, slideId };
}

async function triggerSong(
  request: APIRequestContext,
  presentationId: string,
  slideId: string,
): Promise<void> {
  const resp = await request.post(`${baseURL}/stage/state`, {
    data: { presentationId, currentSlideId: slideId },
  });
  expect(resp.status(), "trigger stage state").toBe(204);
}

async function triggerVerse(
  request: APIRequestContext,
  fields: Record<string, string>,
): Promise<void> {
  const resp = await request.post(`${baseURL}/bible/trigger-slide`, { data: fields });
  expect(resp.ok(), `trigger verse -> ${resp.status()}`).toBeTruthy();
}

async function clearVerse(request: APIRequestContext): Promise<void> {
  const resp = await request.post(`${baseURL}/bible/clear`);
  expect(resp.status(), "clear bible").toBe(204);
}

async function clearStage(request: APIRequestContext): Promise<void> {
  const resp = await request.post(`${baseURL}/stage/clear`);
  expect(resp.status(), "clear stage").toBe(204);
}

async function gotoStream(page: Page): Promise<void> {
  await page.goto(`${baseURL}/stream/${SLUG}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="stream-canvas"]', { timeout: 10_000 });
}

const lyricsEl = (page: Page, id: number) =>
  page.locator(`[data-role="stream-element-lyrics"][data-element-id="${id}"]`);
const verseEl = (page: Page, id: number) =>
  page.locator(`[data-role="stream-element-verse"][data-element-id="${id}"]`);

// ── Test ──────────────────────────────────────────────────────────────────────

test.describe("Stream output content (lyrics + verse)", () => {
  test("lyrics + verse render, toggle, clear, clip, transparent", async ({
    page,
    request,
  }) => {
    const consoleErrors: string[] = [];
    attachConsoleErrorCollector(page, consoleErrors);

    // Base scene: two lyrics elements — L_BOTH (main+translation), L_MAIN
    // (translation OFF, the toggle case).
    const baseId = await createScene(request, "LyricsScene", "base", 0);
    const lBoth = await addElement(request, baseId, 0, {
      kind: "lyrics",
      show_main: true,
      show_translation: true,
      main_style: textStyle({ size_pct: 8 }),
      translation_style: textStyle({ size_pct: 5, color: "#cccccc" }),
      frame: { x_pct: 5, y_pct: 10, w_pct: 60, h_pct: 30 },
    });
    const lMain = await addElement(request, baseId, 1, {
      kind: "lyrics",
      show_main: true,
      show_translation: false,
      main_style: textStyle({ size_pct: 8 }),
      translation_style: textStyle(),
      frame: { x_pct: 5, y_pct: 45, w_pct: 60, h_pct: 20 },
    });

    // Overlay scene: two verse elements — V_BOTH (secondary ON), V_MAIN
    // (secondary OFF, the toggle case).
    const overlayId = await createScene(request, "VerseScene", "overlay", 0);
    const vBoth = await addElement(request, overlayId, 0, {
      kind: "verse",
      show_secondary: true,
      text_style: textStyle({ size_pct: 7 }),
      secondary_style: textStyle({ size_pct: 5, color: "#dddddd" }),
      reference_style: textStyle({ size_pct: 3, color: "#aaaaaa" }),
      frame: { x_pct: 20, y_pct: 60, w_pct: 60, h_pct: 35 },
    });
    const vMain = await addElement(request, overlayId, 1, {
      kind: "verse",
      show_secondary: false,
      text_style: textStyle({ size_pct: 7 }),
      secondary_style: textStyle(),
      reference_style: textStyle({ size_pct: 3 }),
      frame: { x_pct: 20, y_pct: 5, w_pct: 60, h_pct: 20 },
    });

    await activateBase(request, baseId);
    await activateOverlay(request, overlayId);

    await gotoStream(page);

    // ── Transparency (OBS alpha) ──
    expect(
      await page.evaluate(() => getComputedStyle(document.body).backgroundColor),
    ).toBe("rgba(0, 0, 0, 0)");
    expect(
      await page.evaluate(() => getComputedStyle(document.documentElement).backgroundColor),
    ).toBe("rgba(0, 0, 0, 0)");

    // ── LYRICS: trigger a worship slide ⇒ main + translation shown ──
    const song = await seedSong(request, "Amazing Grace", "Úžasná Milosť");
    await triggerSong(request, song.presentationId, song.slideId);

    const bothMain = lyricsEl(page, lBoth).locator('[data-role="stream-lyrics-main"]');
    const bothTrans = lyricsEl(page, lBoth).locator('[data-role="stream-lyrics-translation"]');
    await expect(bothMain).toHaveText("Amazing Grace", { timeout: 5_000 });
    await expect(bothTrans).toHaveText("Úžasná Milosť");
    // main text styled at 8vh.
    const mainFontPx = await bothMain.evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
    expect(mainFontPx).toBeCloseTo((await page.evaluate(() => window.innerHeight)) * 0.08, 0);

    // ── LYRICS toggle: show_translation=false ⇒ translation line ABSENT ──
    await expect(lyricsEl(page, lMain).locator('[data-role="stream-lyrics-main"]')).toHaveText(
      "Amazing Grace",
    );
    await expect(
      lyricsEl(page, lMain).locator('[data-role="stream-lyrics-translation"]'),
    ).toHaveCount(0);

    // ── LYRICS live update: a different slide changes the text ──
    const song2 = await seedSong(request, "How Great Thou Art", "Aká si veľký");
    await triggerSong(request, song2.presentationId, song2.slideId);
    await expect(bothMain).toHaveText("How Great Thou Art", { timeout: 5_000 });
    await expect(bothTrans).toHaveText("Aká si veľký");

    // ── VERSE: trigger a bible slide ⇒ text + reference (+ secondary) ──
    await triggerVerse(request, {
      mainText: "For God so loved the world",
      mainReference: "John 3:16 (NIV)",
      secondaryText: "Lebo tak Boh miloval svet",
      secondaryReference: "Ján 3:16 (SEB)",
    });
    await expect(
      verseEl(page, vBoth).locator('[data-role="stream-verse-text"]'),
    ).toHaveText("For God so loved the world", { timeout: 5_000 });
    await expect(
      verseEl(page, vBoth).locator('[data-role="stream-verse-reference"]'),
    ).toHaveText("John 3:16 (NIV)");
    await expect(
      verseEl(page, vBoth).locator('[data-role="stream-verse-secondary-text"]'),
    ).toHaveText("Lebo tak Boh miloval svet");
    await expect(
      verseEl(page, vBoth).locator('[data-role="stream-verse-secondary-reference"]'),
    ).toHaveText("Ján 3:16 (SEB)");

    // ── VERSE toggle: show_secondary=false ⇒ secondary lines ABSENT ──
    await expect(
      verseEl(page, vMain).locator('[data-role="stream-verse-text"]'),
    ).toHaveText("For God so loved the world");
    await expect(
      verseEl(page, vMain).locator('[data-role="stream-verse-secondary-text"]'),
    ).toHaveCount(0);
    await expect(
      verseEl(page, vMain).locator('[data-role="stream-verse-secondary-reference"]'),
    ).toHaveCount(0);

    // ── BibleCleared ⇒ verse gone ──
    await clearVerse(request);
    await expect(
      verseEl(page, vBoth).locator('[data-role="stream-verse-text"]'),
    ).toHaveCount(0, { timeout: 5_000 });
    await expect(
      verseEl(page, vMain).locator('[data-role="stream-verse-text"]'),
    ).toHaveCount(0);

    // ── 4000-char SlideText clipped to its Frame (overflow hidden, no grow) ──
    const bigText = "A".repeat(4000);
    const bigSong = await seedSong(request, bigText, "");
    await triggerSong(request, bigSong.presentationId, bigSong.slideId);
    await expect(bothMain).toHaveText(bigText, { timeout: 5_000 });
    const clip = await lyricsEl(page, lBoth).evaluate((el) => ({
        height: el.getBoundingClientRect().height,
        overflow: getComputedStyle(el).overflow,
        vh: window.innerHeight,
      }));
    // Container stays at its Frame height (30% vh) despite 4000 chars, and clips.
    expect(clip.overflow).toBe("hidden");
    expect(clip.height).toBeCloseTo(clip.vh * 0.3, 0);

    // ── Stage clear ⇒ lyrics gone (no active content ⇒ transparent) ──
    await clearStage(request);
    await expect(bothMain).toHaveCount(0, { timeout: 5_000 });
    await expect(bothTrans).toHaveCount(0);

    // Zero console errors AND warnings — asserted last.
    expect(consoleErrors).toEqual([]);
  });
});
