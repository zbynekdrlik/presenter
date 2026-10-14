/**
 * Stream output TEXT BLOCK fade E2E (/stream/{slug}) — #863, verse + lyrics.
 *
 * Owner report: the verse reference does not fade with the verse, and when the
 * next verse is shorter it jumps. The verse lines used to crossfade on their
 * own, so an unchanged reference never faded; the text's grid cell kept the
 * taller outgoing layer until it was removed, then shrank, and the fully
 * visible reference jumped to its new position. The lyrics element had the
 * same shape (main + translation as separate crossfades in a centred column).
 *
 * Both elements now crossfade their lines as ONE block. Each test shows a long
 * text and then a SHORT one with the IDENTICAL second line (verse reference /
 * lyrics translation) under `Fade` and records, every animation frame, each
 * line node's top and its EFFECTIVE opacity (its own times every ancestor's).
 * It asserts:
 *   - no line node is ever seen fully visible (opacity >= 0.99) at two tops
 *     more than 2 px apart;
 *   - the unchanged second line fades with the block (sampled mid-fade);
 *   - the final state (one copy of each line, the short text);
 *   - zero console errors AND warnings (asserted last).
 *
 * Line nodes are recorded by CLASS, not `data-role`: the role names only the
 * current copy, while the class is on both layers' lines (and, before #863, on
 * the per-line crossfade wrapper), so the outgoing copy is tracked too.
 *
 * The long text is triggered BEFORE the page loads. The connect-time cold-load
 * (`GET /bible/active-slide`, `GET /stage/snapshot`) then shows it, so a late
 * cold-load response can never overwrite a newer live event.
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

// Whitelisted, web-safe font (no woff2 request in the zero-console gate).
const FONT = "Arial";

const FADE_MS = 400;
// Long enough to wrap over several lines in the 80 %-wide frame, so the second
// line's position under the long and the short text differs by far more than
// the 2 px bound.
const LONG_TEXT =
  "Lebo tak Boh miloval svet, že svojho jednorodeného Syna dal, aby nezahynul nik, " +
  "kto v neho verí, ale aby mal večný život. Veď Boh neposlal svojho Syna na svet, " +
  "aby svet odsúdil, ale aby bol svet skrze neho spasený.";
const SHORT_TEXT = "Ježiš zaplakal.";
const REFERENCE = "Ján 3:16-17 (SEB)";
const TRANSLATION = "For God so loved the world";
const RECORD_MS = 1_500; // > FADE_MS + the 80 ms removal buffer, with margin.
const FRAME = { xPct: 10, yPct: 10, wPct: 80, hPct: 80 };
const FADE = { mode: "fade", duration_ms: FADE_MS };

function textStyle(sizePct: number, color: string): Record<string, unknown> {
  return {
    fontFamily: FONT,
    sizePct,
    color,
    weight: 700,
    align: "center",
    lineHeight: 1.2,
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

// ── REST helpers (same wire shapes as stream-output-content.spec.ts) ──────────

async function createScene(request: APIRequestContext, name: string): Promise<number> {
  const resp = await request.post(`${baseURL}/stream/api/outputs/${SLUG}/scenes`, {
    data: { name, kind: "base" },
  });
  expect(resp.ok(), `create scene ${name} -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

async function addElement(
  request: APIRequestContext,
  sceneId: number,
  props: Record<string, unknown>,
): Promise<number> {
  // The body IS the bare StreamElementProps JSON (flattened, `kind` tag).
  const resp = await request.post(`${baseURL}/stream/api/scenes/${sceneId}/elements`, {
    data: props,
  });
  expect(resp.ok(), `add element -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

async function activateBase(request: APIRequestContext, sceneId: number): Promise<void> {
  const resp = await request.put(`${baseURL}/stream/api/outputs/${SLUG}/active-scene`, {
    data: { sceneId },
  });
  expect(resp.ok(), `activate base -> ${resp.status()}`).toBeTruthy();
}

async function triggerVerse(
  request: APIRequestContext,
  mainText: string,
  mainReference: string,
): Promise<void> {
  const resp = await request.post(`${baseURL}/bible/trigger-slide`, {
    data: { mainText, mainReference, secondaryText: "", secondaryReference: "" },
  });
  expect(resp.ok(), `trigger verse -> ${resp.status()}`).toBeTruthy();
}

/** Create a library + presentation, set the first slide's text, return ids. */
async function seedSong(
  request: APIRequestContext,
  main: string,
  translation: string,
): Promise<{ presentationId: string; slideId: string }> {
  const libResp = await request.post(`${baseURL}/libraries`, {
    data: { name: `Block Fade Lib ${Date.now()}-${Math.random()}` },
  });
  expect(libResp.ok()).toBeTruthy();
  const library = (await libResp.json()) as { id: string };

  const presResp = await request.post(`${baseURL}/libraries/${library.id}/presentations`, {
    data: { name: "Block Fade Song" },
  });
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
  song: { presentationId: string; slideId: string },
): Promise<void> {
  const resp = await request.post(`${baseURL}/stage/state`, {
    data: { presentationId: song.presentationId, currentSlideId: song.slideId },
  });
  expect(resp.status(), "trigger stage state").toBe(204);
}

async function gotoStream(page: Page): Promise<void> {
  await page.goto(`${baseURL}/stream/${SLUG}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="stream-canvas"]', { timeout: 10_000 });
}

// ── The in-page line recorder ─────────────────────────────────────────────────

type LineSample = { id: number; top: number; opacity: number; text: string };
type LineFrame = { lines: LineSample[] };
type LineRecorder = { on: boolean; frames: LineFrame[] };

/**
 * Init script: a requestAnimationFrame loop that, while `__lineRec.on`, records
 * every node matching `selector`. Each node gets a stable id from a WeakMap
 * (node identity), so a node that MOVES is told apart from a new node at
 * another position. Opacity is the EFFECTIVE one: the node's own times every
 * ancestor's, since the crossfade animates a wrapping layer.
 */
function installLineRecorder(selector: string): void {
  const ids = new WeakMap<Element, number>();
  let nextId = 1;
  const rec: LineRecorder = { on: false, frames: [] };
  (window as unknown as { __lineRec: LineRecorder }).__lineRec = rec;
  const effectiveOpacity = (el: Element): number => {
    let opacity = 1;
    for (let node: Element | null = el; node; node = node.parentElement) {
      opacity *= parseFloat(getComputedStyle(node).opacity);
    }
    return opacity;
  };
  const tick = () => {
    if (rec.on) {
      const lines: LineSample[] = [];
      document.querySelectorAll(selector).forEach((el) => {
        let id = ids.get(el);
        if (id === undefined) {
          id = nextId++;
          ids.set(el, id);
        }
        lines.push({
          id,
          top: el.getBoundingClientRect().top,
          opacity: effectiveOpacity(el),
          text: (el.textContent ?? "").trim(),
        });
      });
      rec.frames.push({ lines });
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

/** Wait until exactly `count` recorded-selector nodes exist, all fully visible. */
async function waitForSettled(page: Page, selector: string, count: number): Promise<void> {
  await page.waitForFunction(
    ({ selector, count }) => {
      const nodes = Array.from(document.querySelectorAll(selector));
      if (nodes.length !== count) return false;
      return nodes.every((el) => {
        let opacity = 1;
        for (let node: Element | null = el; node; node = node.parentElement) {
          opacity *= parseFloat(getComputedStyle(node).opacity);
        }
        return opacity > 0.99;
      });
    },
    { selector, count },
    { timeout: 10_000 },
  );
}

async function startRecording(page: Page): Promise<void> {
  await page.evaluate(() => {
    const rec = (window as unknown as { __lineRec: LineRecorder }).__lineRec;
    rec.frames = [];
    rec.on = true;
  });
}

async function stopRecording(page: Page): Promise<LineSample[]> {
  const frames = await page.evaluate(() => {
    const rec = (window as unknown as { __lineRec: LineRecorder }).__lineRec;
    rec.on = false;
    return rec.frames;
  });
  expect(frames.length, "the recorder sampled the change").toBeGreaterThan(20);
  return frames.flatMap((f) => f.lines);
}

/** No node is seen fully visible at two tops more than 2 px apart. */
function expectNoVisibleJump(samples: LineSample[]): void {
  const visibleTops = new Map<number, { text: string; tops: number[] }>();
  for (const s of samples) {
    if (s.opacity < 0.99) continue;
    const entry = visibleTops.get(s.id) ?? { text: s.text, tops: [] };
    entry.tops.push(s.top);
    visibleTops.set(s.id, entry);
  }
  for (const [id, { text, tops }] of visibleTops) {
    const jumpPx = Math.max(...tops) - Math.min(...tops);
    expect(
      jumpPx,
      `line node #${id} ("${text}") was fully visible at tops ${jumpPx.toFixed(1)} px apart`,
    ).toBeLessThanOrEqual(2);
  }
}

/** The unchanged line was on screen before the change and faded with the block. */
function expectFadedWithTheBlock(samples: LineSample[], text: string): void {
  expect(
    samples.some((s) => s.text === text && s.opacity >= 0.99),
    `"${text}" was sampled settled before the change`,
  ).toBeTruthy();
  expect(
    samples.some((s) => s.text === text && s.opacity > 0.02 && s.opacity < 0.98),
    `"${text}" was sampled mid-fade`,
  ).toBeTruthy();
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test.describe("Stream output text block fade (#863)", () => {
  test("a shorter verse with the same reference fades the block, the reference never jumps", async ({
    page,
    request,
  }) => {
    const consoleErrors: string[] = [];
    attachConsoleErrorCollector(page, consoleErrors);

    const scene = await createScene(request, "VerseBlockScene");
    const verseId = await addElement(request, scene, {
      kind: "verse",
      show_secondary: false,
      text_style: textStyle(7, "#ffffff"),
      secondary_style: textStyle(5, "#dddddd"),
      reference_style: textStyle(4, "#ffcc00"),
      frame: FRAME,
      content_transition: FADE,
    });
    await activateBase(request, scene);
    await triggerVerse(request, LONG_TEXT, REFERENCE);

    const elementSelector = `[data-role="stream-element-verse"][data-element-id="${verseId}"]`;
    const referenceNodes = `${elementSelector} .stream-verse__reference`;
    await page.addInitScript(installLineRecorder, referenceNodes);
    await gotoStream(page);

    const element = page.locator(elementSelector);
    const text = element.locator('[data-role="stream-verse-text"]');
    const reference = element.locator('[data-role="stream-verse-reference"]');

    // ── The long verse (cold-loaded), settled: one reference, fully visible. ──
    await expect(text).toHaveText(LONG_TEXT, { timeout: 10_000 });
    await expect(reference).toHaveText(REFERENCE);
    await waitForSettled(page, referenceNodes, 1);

    // ── The short verse, SAME reference, recorded every frame. ──
    await startRecording(page);
    await page.waitForTimeout(150); // a few frames of the settled long verse
    await triggerVerse(request, SHORT_TEXT, REFERENCE);
    await page.waitForTimeout(RECORD_MS);
    const samples = await stopRecording(page);

    expectNoVisibleJump(samples);
    expectFadedWithTheBlock(samples, REFERENCE);

    // ── Final state: one reference, the short verse. ──
    await expect(reference).toHaveCount(1, { timeout: 5_000 });
    await expect(text).toHaveCount(1, { timeout: 5_000 });
    await expect(text).toHaveText(SHORT_TEXT);
    await expect(reference).toHaveText(REFERENCE);

    // Zero console errors AND warnings — asserted last.
    expect(consoleErrors).toEqual([]);
  });

  test("a shorter lyric line with the same translation fades the block, no line jumps", async ({
    page,
    request,
  }) => {
    const consoleErrors: string[] = [];
    attachConsoleErrorCollector(page, consoleErrors);

    const scene = await createScene(request, "LyricsBlockScene");
    const lyricsId = await addElement(request, scene, {
      kind: "lyrics",
      show_main: true,
      show_translation: true,
      main_style: textStyle(7, "#ffffff"),
      translation_style: textStyle(5, "#cccccc"),
      frame: FRAME,
      content_transition: FADE,
    });
    await activateBase(request, scene);
    const longSlide = await seedSong(request, LONG_TEXT, TRANSLATION);
    const shortSlide = await seedSong(request, SHORT_TEXT, TRANSLATION);
    await triggerSong(request, longSlide);

    const elementSelector = `[data-role="stream-element-lyrics"][data-element-id="${lyricsId}"]`;
    const lineNodes = `${elementSelector} .stream-lyrics__line`;
    await page.addInitScript(installLineRecorder, lineNodes);
    await gotoStream(page);

    const element = page.locator(elementSelector);
    const main = element.locator('[data-role="stream-lyrics-main"]');
    const translation = element.locator('[data-role="stream-lyrics-translation"]');

    // ── The long slide (cold-loaded), settled: main + translation, visible. ──
    await expect(main).toHaveText(LONG_TEXT, { timeout: 10_000 });
    await expect(translation).toHaveText(TRANSLATION);
    await waitForSettled(page, lineNodes, 2);

    // ── The short slide, SAME translation, recorded every frame. ──
    await startRecording(page);
    await page.waitForTimeout(150); // a few frames of the settled long slide
    await triggerSong(request, shortSlide);
    await page.waitForTimeout(RECORD_MS);
    const samples = await stopRecording(page);

    expectNoVisibleJump(samples);
    expectFadedWithTheBlock(samples, TRANSLATION);

    // ── Final state: one main line (the short one), one translation. ──
    await expect(main).toHaveCount(1, { timeout: 5_000 });
    await expect(translation).toHaveCount(1, { timeout: 5_000 });
    await expect(main).toHaveText(SHORT_TEXT);
    await expect(translation).toHaveText(TRANSLATION);
    await waitForSettled(page, lineNodes, 2);

    // Zero console errors AND warnings — asserted last.
    expect(consoleErrors).toEqual([]);
  });
});
