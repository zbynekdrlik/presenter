/**
 * Stream-graphics OUTPUT page TRANSITIONS E2E (/stream/{slug}) — #716, epic #718.
 *
 * Verifies the CSS-crossfade transitions the #716 lane adds to the transparent
 * OBS output page:
 *   - SCENE-SWITCH CROSSFADE: switching the active base scene keeps the OUTGOING
 *     scene mounted (opacity animating to 0) while the incoming fades in — a
 *     mid-switch sample sees BOTH base scenes present and the outgoing opacity in
 *     (0,1); after the fade only the new scene remains in the DOM.
 *   - NO NODE LEAKS: after 5 rapid base switches exactly one base-scene container
 *     remains.
 *   - CONTENT FADE vs CUT: on a slide change a `Fade` lyrics element shows a
 *     transient TWO-layer overlap (old + new text) then settles to one; a `Cut`
 *     element never shows two layers (instant swap).
 *   - Zero console errors AND warnings (asserted last).
 *
 * Timing is sampled via in-browser requestAnimationFrame recorders (robust vs
 * frame-exact Playwright polling) — see startRecorder/readRecorder.
 *
 * ── INTEGRATION NOTE (same as #709/#710's stream-output specs) ────────────────
 * The `/stream/api/*` scene/element seeding + activation below is owned by the
 * PARALLEL lanes #706 (StreamManager + stream_state LiveEvent) and #707 (REST).
 * Those land on other branches, so this spec PASSES only after #706/#707 are
 * integrated with #709+#710+#716. The worship-stage (POST /stage/state) endpoints
 * it triggers DO exist today. Every `/stream/api/*` shape below is the epic-#718
 * §5/§8 + #707 contract; the create-body field names (incl. the element
 * `content_transition` prop, serde tag `mode`) are CONTRACT ASSUMPTIONS centralized
 * in the helpers here for one-place reconcile. See the ticket hand-back's
 * CONTRACT-ASSUMPTIONS list.
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

// Whitelisted, web-safe font (avoids a missing-woff2 404 in the zero-console gate).
const FONT = "Arial";

function textStyle(): Record<string, unknown> {
  return {
    font_family: FONT,
    size_pct: 6,
    color: "#ffffff",
    weight: 700,
    align: "center",
    line_height: 1.2,
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

async function addLyrics(
  request: APIRequestContext,
  sceneId: number,
  zOrder: number,
  contentTransition: Record<string, unknown>,
): Promise<number> {
  const props = {
    kind: "lyrics",
    show_main: true,
    show_translation: false,
    main_style: textStyle(),
    translation_style: textStyle(),
    frame: { x_pct: 5, y_pct: 5 + zOrder * 30, w_pct: 60, h_pct: 25 },
    content_transition: contentTransition,
  };
  const resp = await request.post(`${baseURL}/stream/api/scenes/${sceneId}/elements`, {
    data: { kind: "lyrics", z_order: zOrder, props },
  });
  expect(resp.ok(), `add lyrics -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

async function activateBase(request: APIRequestContext, sceneId: number): Promise<void> {
  const resp = await request.put(`${baseURL}/stream/api/outputs/${SLUG}/active-scene`, {
    data: { scene_id: sceneId },
  });
  expect(resp.ok(), `activate base -> ${resp.status()}`).toBeTruthy();
}

async function seedSong(
  request: APIRequestContext,
  main: string,
): Promise<{ presentationId: string; slideId: string }> {
  const libResp = await request.post(`${baseURL}/libraries`, {
    data: { name: `Transitions Lib ${Date.now()}-${Math.random()}` },
  });
  expect(libResp.ok()).toBeTruthy();
  const library = (await libResp.json()) as { id: string };

  const presResp = await request.post(
    `${baseURL}/libraries/${library.id}/presentations`,
    { data: { name: "Transitions Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const pres = (await presResp.json()) as {
    presentation: { id: string; slides: Array<{ id: string }> };
  };
  const presentationId = pres.presentation.id;
  const slideId = pres.presentation.slides[0].id;

  const patchResp = await request.patch(
    `${baseURL}/presentations/${presentationId}/slides/${slideId}`,
    { data: { main, translation: "", stage: "", group: "Verse 1" } },
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

async function gotoStream(page: Page): Promise<void> {
  await page.goto(`${baseURL}/stream/${SLUG}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="stream-canvas"]', { timeout: 10_000 });
}

const baseScenes = (page: Page) =>
  page.locator('[data-role="stream-scene"][data-scene-kind="base"]');

type Frame = Record<string, number>;

/** Read the frames an in-test rAF recorder collected into `window.__rec`. */
async function readFrames(page: Page): Promise<Frame[]> {
  return page.evaluate(
    () => (window as unknown as { __rec: { frames: Frame[] } }).__rec.frames,
  );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test.describe("Stream output transitions", () => {
  test("scene switch crossfades both scenes then settles to one (no leak)", async ({
    page,
    request,
  }) => {
    const consoleErrors: string[] = [];
    attachConsoleErrorCollector(page, consoleErrors);

    const sceneA = await createScene(request, "CrossfadeA", "base", 0);
    const sceneB = await createScene(request, "CrossfadeB", "base", 1);

    await activateBase(request, sceneA);
    await gotoStream(page);

    // Wait for A present and its cold-load fade-in to settle (opacity ~1), so we
    // isolate the fade-OUT during the switch.
    await page.waitForFunction(
      (id) => {
        const el = document.querySelector(`[data-role="stream-scene"][data-scene-id="${id}"]`);
        return !!el && parseFloat(getComputedStyle(el).opacity) > 0.98;
      },
      sceneA,
      { timeout: 10_000 },
    );

    // Record A's opacity + the base-scene count across the switch window (an
    // in-browser rAF loop, robust vs frame-exact Playwright polling).
    await page.evaluate((idA) => {
      const rec = { frames: [] as Array<Record<string, number>> };
      (window as unknown as { __rec: typeof rec }).__rec = rec;
      const start = performance.now();
      const tick = () => {
        const a = document.querySelector(
          `[data-role="stream-scene"][data-scene-id="${idA}"]`,
        );
        rec.frames.push({
          aOpacity: a ? parseFloat(getComputedStyle(a).opacity) : -1,
          baseCount: document.querySelectorAll(
            '[data-role="stream-scene"][data-scene-kind="base"]',
          ).length,
        });
        if (performance.now() - start < 1600) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    }, sceneA);

    await activateBase(request, sceneB);
    await page.waitForTimeout(1600);

    const frames = await readFrames(page);
    // Both base scenes co-existed at some sampled frame (outgoing kept mounted).
    const maxBase = Math.max(...frames.map((f) => f.baseCount));
    expect(maxBase, "both base scenes present mid-switch").toBe(2);
    // The outgoing scene animated its opacity through the (0,1) range.
    const midFade = frames.some((f) => f.aOpacity > 0.02 && f.aOpacity < 0.98);
    expect(midFade, "outgoing scene opacity sampled between 0 and 1").toBeTruthy();

    // After the fade only the new base scene remains in the DOM.
    await expect(baseScenes(page)).toHaveCount(1, { timeout: 5_000 });
    await expect(baseScenes(page)).toHaveAttribute("data-scene-id", String(sceneB));

    // ── No node leaks: 5 rapid switches settle to exactly one base container. ──
    for (const id of [sceneA, sceneB, sceneA, sceneB, sceneA]) {
      await activateBase(request, id);
    }
    // Allow every leaving layer's removal timeout to fire.
    await page.waitForTimeout(2_000);
    await expect(baseScenes(page)).toHaveCount(1);
    await expect(baseScenes(page)).toHaveAttribute("data-scene-id", String(sceneA));

    expect(consoleErrors).toEqual([]);
  });

  test("content fade overlaps two layers; cut swaps with no overlap", async ({
    page,
    request,
  }) => {
    const consoleErrors: string[] = [];
    attachConsoleErrorCollector(page, consoleErrors);

    // One base scene, two lyrics elements bound to the same worship content: one
    // Fade (600 ms, to widen the overlap window), one Cut.
    const scene = await createScene(request, "ContentScene", "base", 0);
    const lFade = await addLyrics(request, scene, 0, { mode: "fade", duration_ms: 600 });
    const lCut = await addLyrics(request, scene, 1, { mode: "cut" });
    await activateBase(request, scene);

    await gotoStream(page);

    // First slide: both elements show it (a single layer each).
    const song1 = await seedSong(request, "Amazing Grace");
    await triggerSong(request, song1.presentationId, song1.slideId);
    const fadeMain = page
      .locator(`[data-role="stream-element-lyrics"][data-element-id="${lFade}"]`)
      .locator('[data-role="stream-lyrics-main"]');
    await expect(fadeMain).toHaveText("Amazing Grace", { timeout: 5_000 });

    // Record per-element crossfade-layer counts across the slide CHANGE.
    await page.evaluate(
      ({ fadeId, cutId }) => {
        const rec = { frames: [] as Array<Record<string, number>> };
        (window as unknown as { __rec: typeof rec }).__rec = rec;
        const start = performance.now();
        const layersOf = (id: number) =>
          document.querySelectorAll(
            `[data-role="stream-element-lyrics"][data-element-id="${id}"] [data-role="stream-crossfade-layer"]`,
          );
        const tick = () => {
          const fade = layersOf(fadeId);
          let midOpacity = 0;
          fade.forEach((el) => {
            const o = parseFloat(getComputedStyle(el).opacity);
            if (o > 0.02 && o < 0.98) midOpacity = 1;
          });
          rec.frames.push({
            fadeLayers: fade.length,
            cutLayers: layersOf(cutId).length,
            fadeMidOpacity: midOpacity,
          });
          if (performance.now() - start < 1600) requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      },
      { fadeId: lFade, cutId: lCut },
    );

    const song2 = await seedSong(request, "How Great Thou Art");
    await triggerSong(request, song2.presentationId, song2.slideId);
    await page.waitForTimeout(1600);

    const frames = await readFrames(page);
    // FADE: the old + new text overlapped (two layers) at some frame, with an
    // opacity mid-fade; CUT: never more than one layer (instant swap).
    expect(Math.max(...frames.map((f) => f.fadeLayers)), "fade overlaps two layers").toBe(2);
    expect(frames.some((f) => f.fadeMidOpacity === 1), "fade layer opacity in (0,1)").toBeTruthy();
    expect(Math.max(...frames.map((f) => f.cutLayers)), "cut never overlaps").toBe(1);

    // After the fade both settle to the new text, single layer.
    await expect(fadeMain).toHaveText("How Great Thou Art", { timeout: 5_000 });
    await expect(
      page
        .locator(`[data-role="stream-element-lyrics"][data-element-id="${lFade}"]`)
        .locator('[data-role="stream-crossfade-layer"]'),
    ).toHaveCount(1, { timeout: 5_000 });

    expect(consoleErrors).toEqual([]);
  });
});
