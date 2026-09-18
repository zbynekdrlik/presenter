/**
 * Stream-graphics countdown NO-FLICKER E2E (/stream/{slug}) — #776, epic #718.
 *
 * The owner reported the countdown "divne blika ked sa prepne kazde cislo" — a
 * per-tick fade because the countdown element rendered its digits through the
 * content-crossfade primitive (`CrossfadeText`), whose default transition is
 * `Fade{300}`, so every second the old digits faded out while the new faded in.
 *
 * A countdown must be a HARD CUT per tick: no crossfade layers, opacity always
 * 1, and the text node updated IN PLACE (never a re-mount). This spec seeds a
 * countdown with the DEFAULT (omitted) content_transition — i.e. the flickering
 * `Fade{300}` default — and proves the output page renders it with no fade.
 *
 * The scene/element seeding lanes (#706/#707) and the output page (#709+) are
 * all merged on `dev`, so this runs in the integrated pipeline.
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

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

async function createScene(
  request: APIRequestContext,
  name: string,
  kind: "base" | "overlay",
): Promise<number> {
  const resp = await request.post(`${baseURL}/stream/api/outputs/${SLUG}/scenes`, {
    data: { name, kind },
  });
  expect(resp.ok(), `create scene ${name} -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

async function addElement(
  request: APIRequestContext,
  sceneId: number,
  props: Record<string, unknown>,
): Promise<number> {
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

/** Set + auto-start the countdown ~2 min out so it renders MM:SS and ticks. */
async function startCountdown(request: APIRequestContext): Promise<void> {
  const target = new Date(Date.now() + 125_000).toISOString();
  const resp = await request.post(`${baseURL}/timers/command`, {
    data: { command: "set_countdown_target", target },
  });
  expect(resp.ok(), `start countdown -> ${resp.status()}`).toBeTruthy();
}

async function gotoStream(page: Page): Promise<void> {
  await page.goto(`${baseURL}/stream/${SLUG}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="stream-canvas"]', { timeout: 10_000 });
}

type FlickerRec = {
  minOpacity: number;
  sawLeaving: boolean;
  maxLayers: number;
  texts: string[];
};

test("countdown swaps digits with NO transition — no crossfade, opacity 1 across ticks (#776)", async ({
  page,
  request,
}) => {
  const consoleErrors: string[] = [];
  attachConsoleErrorCollector(page, consoleErrors);

  const sceneId = await createScene(request, "CD_Flicker776", "base");
  // content_transition OMITTED => deserialises to Fade{300} (the flickering
  // default the owner hit). The fix must render a hard cut REGARDLESS.
  const cdId = await addElement(request, sceneId, {
    kind: "countdown",
    timer_id: 1,
    style: {
      fontFamily: "Arial",
      sizePct: 12,
      color: "#ffffff",
      weight: 700,
      align: "center",
      lineHeight: 1.2,
    },
    frame: { xPct: 10, yPct: 20, wPct: 80, hPct: 20 },
  });
  await startCountdown(request);
  await activateBase(request, sceneId);

  await gotoStream(page);

  const countdown = page.locator(
    `[data-role="stream-element-countdown"][data-element-id="${cdId}"]`,
  );
  await expect(countdown).toHaveCount(1, { timeout: 10_000 });
  const content = countdown.locator('[data-role="stream-countdown-content"]');
  await expect(content).toHaveText(/\d/, { timeout: 10_000 });

  // The countdown must NOT use the content-crossfade primitive at all — this is
  // the discriminator: the flickering code kept a Fade `CrossfadeText`
  // (>=1 `stream-crossfade-layer`); the fix renders a plain stable text node.
  await expect(
    countdown.locator('[data-role="stream-crossfade-layer"]'),
  ).toHaveCount(0);

  // Tag the content node to prove it is the SAME DOM node after ticks (an
  // in-place text update, never a re-mount).
  await content.evaluate((el) => {
    (el as unknown as { __cdProbe?: boolean }).__cdProbe = true;
  });

  // Sample opacity + crossfade presence + distinct digit text across >=3 ticks.
  await page.evaluate((elementId) => {
    const root = document.querySelector(
      `[data-role="stream-element-countdown"][data-element-id="${elementId}"]`,
    );
    const rec: FlickerRec = { minOpacity: 1, sawLeaving: false, maxLayers: 0, texts: [] };
    (window as unknown as { __cd: FlickerRec }).__cd = rec;
    const start = performance.now();
    const tick = () => {
      if (!root) return;
      root.querySelectorAll("*").forEach((el) => {
        const o = parseFloat(getComputedStyle(el as Element).opacity);
        if (!Number.isNaN(o) && o < rec.minOpacity) rec.minOpacity = o;
      });
      const layers = root.querySelectorAll('[data-role="stream-crossfade-layer"]');
      if (layers.length > rec.maxLayers) rec.maxLayers = layers.length;
      if (root.querySelector('[class*="leaving"]')) rec.sawLeaving = true;
      const c = root.querySelector('[data-role="stream-countdown-content"]');
      const t = (c?.textContent ?? "").trim();
      if (t && rec.texts[rec.texts.length - 1] !== t) rec.texts.push(t);
      if (performance.now() - start < 3500) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, cdId);

  await page.waitForTimeout(3600);

  const rec = await page.evaluate(
    () => (window as unknown as { __cd: FlickerRec }).__cd,
  );

  // >=3 ticks actually advanced during the sample window.
  expect(
    rec.texts.length,
    `countdown advanced across ticks: ${JSON.stringify(rec.texts)}`,
  ).toBeGreaterThanOrEqual(3);
  // No content-crossfade layer ever appeared; opacity never dipped — a hard cut.
  expect(rec.maxLayers, "no content-crossfade layer on the countdown").toBe(0);
  expect(rec.sawLeaving, "no --leaving fade layer on the countdown").toBe(false);
  expect(rec.minOpacity, "countdown content opacity stays 1 (no fade)").toBeGreaterThan(0.99);

  // Same DOM node after all the ticks (stable text-node update, no re-mount).
  const stillSame = await content.evaluate(
    (el) => (el as unknown as { __cdProbe?: boolean }).__cdProbe === true,
  );
  expect(stillSame, "countdown content node is stable across ticks").toBe(true);

  expect(consoleErrors).toEqual([]);
});
