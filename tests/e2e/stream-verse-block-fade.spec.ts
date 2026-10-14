/**
 * Stream output VERSE BLOCK fade E2E (/stream/{slug}) — #863.
 *
 * Owner report: the verse reference does not fade with the verse, and when the
 * next verse is shorter it jumps. The four verse lines used to crossfade on
 * their own, so an unchanged reference never faded; the text's grid cell kept
 * the taller outgoing layer until it was removed, then shrank, and the fully
 * visible reference jumped to its new position.
 *
 * The verse block now crossfades as ONE unit. This spec shows a long verse and
 * then a SHORT verse with the IDENTICAL reference under `Fade` and records,
 * every animation frame, each reference node's top and its EFFECTIVE opacity
 * (its own times every ancestor's). It asserts:
 *   - no reference node is ever seen fully visible (opacity >= 0.99) at two tops
 *     more than 2 px apart;
 *   - the reference fades with the verse (sampled mid-fade);
 *   - the final state is one reference + the short text;
 *   - zero console errors AND warnings (asserted last).
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
const REFERENCE = "Ján 3:16-17 (SEB)";
// Long enough to wrap over several lines in the 80 %-wide frame, so the
// reference position of the long and the short verse differ by far more than
// the 2 px bound.
const LONG_VERSE =
  "Lebo tak Boh miloval svet, že svojho jednorodeného Syna dal, aby nezahynul nik, " +
  "kto v neho verí, ale aby mal večný život. Veď Boh neposlal svojho Syna na svet, " +
  "aby svet odsúdil, ale aby bol svet skrze neho spasený.";
const SHORT_VERSE = "Ježiš zaplakal.";
const RECORD_MS = 1_500; // > FADE_MS + the 80 ms removal buffer, with margin.

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

async function addVerse(request: APIRequestContext, sceneId: number): Promise<number> {
  // The body IS the bare StreamElementProps JSON (flattened, `kind` tag).
  const resp = await request.post(`${baseURL}/stream/api/scenes/${sceneId}/elements`, {
    data: {
      kind: "verse",
      show_secondary: false,
      text_style: textStyle(7, "#ffffff"),
      secondary_style: textStyle(5, "#dddddd"),
      reference_style: textStyle(4, "#ffcc00"),
      frame: { xPct: 10, yPct: 10, wPct: 80, hPct: 80 },
      content_transition: { mode: "fade", duration_ms: FADE_MS },
    },
  });
  expect(resp.ok(), `add verse -> ${resp.status()}`).toBeTruthy();
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

async function gotoStream(page: Page): Promise<void> {
  await page.goto(`${baseURL}/stream/${SLUG}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="stream-canvas"]', { timeout: 10_000 });
}

// ── The in-page reference recorder ────────────────────────────────────────────

type RefSample = { id: number; top: number; opacity: number; text: string };
type RefFrame = { refs: RefSample[] };
type RefRecorder = { on: boolean; frames: RefFrame[] };

/**
 * Init script: a requestAnimationFrame loop that, while `__refRec.on`, records
 * every `[data-role="stream-verse-reference"]` node. Each node gets a stable id
 * from a WeakMap (node identity), so a node that MOVES is told apart from a new
 * node at another position. Opacity is the EFFECTIVE one: the node's own times
 * every ancestor's, since the crossfade animates a wrapping layer.
 */
function installReferenceRecorder(): void {
  const ids = new WeakMap<Element, number>();
  let nextId = 1;
  const rec: RefRecorder = { on: false, frames: [] };
  (window as unknown as { __refRec: RefRecorder }).__refRec = rec;
  const effectiveOpacity = (el: Element): number => {
    let opacity = 1;
    for (let node: Element | null = el; node; node = node.parentElement) {
      opacity *= parseFloat(getComputedStyle(node).opacity);
    }
    return opacity;
  };
  const tick = () => {
    if (rec.on) {
      const refs: RefSample[] = [];
      document.querySelectorAll('[data-role="stream-verse-reference"]').forEach((el) => {
        let id = ids.get(el);
        if (id === undefined) {
          id = nextId++;
          ids.set(el, id);
        }
        refs.push({
          id,
          top: el.getBoundingClientRect().top,
          opacity: effectiveOpacity(el),
          text: (el.textContent ?? "").trim(),
        });
      });
      rec.frames.push({ refs });
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

async function startRecording(page: Page): Promise<void> {
  await page.evaluate(() => {
    const rec = (window as unknown as { __refRec: RefRecorder }).__refRec;
    rec.frames = [];
    rec.on = true;
  });
}

async function stopRecording(page: Page): Promise<RefFrame[]> {
  return page.evaluate(() => {
    const rec = (window as unknown as { __refRec: RefRecorder }).__refRec;
    rec.on = false;
    return rec.frames;
  });
}

// ── Test ──────────────────────────────────────────────────────────────────────

test.describe("Stream output verse block fade (#863)", () => {
  test("a shorter verse with the same reference fades the block, the reference never jumps", async ({
    page,
    request,
  }) => {
    const consoleErrors: string[] = [];
    attachConsoleErrorCollector(page, consoleErrors);

    const scene = await createScene(request, "VerseBlockScene");
    const verseId = await addVerse(request, scene);
    await activateBase(request, scene);

    await page.addInitScript(installReferenceRecorder);
    await gotoStream(page);

    const element = page.locator(
      `[data-role="stream-element-verse"][data-element-id="${verseId}"]`,
    );
    const text = element.locator('[data-role="stream-verse-text"]');
    const reference = element.locator('[data-role="stream-verse-reference"]');

    // ── The long verse, settled: one reference, fully visible. ──
    await triggerVerse(request, LONG_VERSE, REFERENCE);
    await expect(text).toHaveText(LONG_VERSE, { timeout: 10_000 });
    await expect(reference).toHaveCount(1);
    await page.waitForFunction(
      (id) => {
        const refs = document.querySelectorAll(
          `[data-role="stream-element-verse"][data-element-id="${id}"] [data-role="stream-verse-reference"]`,
        );
        if (refs.length !== 1) return false;
        let opacity = 1;
        for (let node: Element | null = refs[0]; node; node = node.parentElement) {
          opacity *= parseFloat(getComputedStyle(node).opacity);
        }
        return opacity > 0.99;
      },
      verseId,
      { timeout: 10_000 },
    );

    // ── The short verse, SAME reference, recorded every frame. ──
    await startRecording(page);
    await page.waitForTimeout(150); // a few frames of the settled long verse
    await triggerVerse(request, SHORT_VERSE, REFERENCE);
    await page.waitForTimeout(RECORD_MS);
    const frames = await stopRecording(page);

    expect(frames.length, "the recorder sampled the change").toBeGreaterThan(20);
    const samples = frames.flatMap((f) => f.refs);
    expect(
      samples.some((s) => s.text === REFERENCE && s.opacity >= 0.99),
      "the settled reference was sampled before the change",
    ).toBeTruthy();

    // No reference node is seen fully visible at two positions.
    const visibleTops = new Map<number, number[]>();
    for (const s of samples) {
      if (s.opacity < 0.99) continue;
      const tops = visibleTops.get(s.id) ?? [];
      tops.push(s.top);
      visibleTops.set(s.id, tops);
    }
    const jumps = [...visibleTops.entries()].map(([id, tops]) => ({
      id,
      jumpPx: Math.max(...tops) - Math.min(...tops),
    }));
    for (const { id, jumpPx } of jumps) {
      expect(
        jumpPx,
        `reference node #${id} was fully visible at tops ${jumpPx.toFixed(1)} px apart`,
      ).toBeLessThanOrEqual(2);
    }

    // The reference fades together with the verse.
    expect(
      samples.some((s) => s.opacity > 0.02 && s.opacity < 0.98),
      "the reference was sampled mid-fade",
    ).toBeTruthy();

    // ── Final state: one reference, the short verse. ──
    await expect(reference).toHaveCount(1, { timeout: 5_000 });
    await expect(text).toHaveCount(1, { timeout: 5_000 });
    await expect(text).toHaveText(SHORT_VERSE);
    await expect(reference).toHaveText(REFERENCE);

    // Zero console errors AND warnings — asserted last.
    expect(consoleErrors).toEqual([]);
  });
});
