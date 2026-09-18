/**
 * #779 lower-third nameplates ("menovky") — end-to-end.
 *
 * A lower_third element carries the LOOK (bar/accent/styles/animation); the
 * runtime "active nameplate" state carries the TEXT. This spec drives the REST
 * surface + the transparent output page like a user's OBS source:
 *  - create a person plate + a lower_third element, show the plate via API →
 *    the plate animates in with both texts + a settled identity transform;
 *  - A→B swap settles to a single plate showing B's text;
 *  - hide removes the plate node;
 *  - auto-hide (element auto_hide_s>0) removes the node server-side after the delay;
 *  - the song plate shows the live song title + library;
 *  - zero console errors throughout.
 *
 * Contract-shaped REST helpers mirror stream-output-content.spec.ts.
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

const SLUG = "stream";
const FONT = "Arial"; // web-safe, avoids a missing-woff2 404 in the zero-console gate.

function textStyle(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    fontFamily: FONT,
    sizePct: 6,
    color: "#ffffff",
    weight: 700,
    align: "left",
    lineHeight: 1.2,
    ...overrides,
  };
}

function lowerThirdProps(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    kind: "lower_third",
    frame: { xPct: 6, yPct: 74, wPct: 46, hPct: 14 },
    bar_color: "#0f172a",
    bar_opacity: 0.85,
    accent_color: "#38bdf8",
    accent_width_pct: 2.5,
    primary_style: textStyle({ sizePct: 5, weight: 700 }),
    secondary_style: textStyle({ sizePct: 3.5, weight: 400 }),
    padding_pct: 3,
    animation: "slide_left",
    in_ms: 200,
    out_ms: 150,
    auto_hide_s: 0,
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

// ── REST helpers ──────────────────────────────────────────────────────────────

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

async function patchElement(
  request: APIRequestContext,
  elementId: number,
  props: Record<string, unknown>,
): Promise<void> {
  const resp = await request.patch(`${baseURL}/stream/api/elements/${elementId}`, {
    data: props,
  });
  expect(resp.ok(), `patch element -> ${resp.status()}`).toBeTruthy();
}

async function activateBase(request: APIRequestContext, sceneId: number): Promise<void> {
  const resp = await request.put(`${baseURL}/stream/api/outputs/${SLUG}/active-scene`, {
    data: { sceneId },
  });
  expect(resp.ok(), `activate base -> ${resp.status()}`).toBeTruthy();
}

async function createNameplate(
  request: APIRequestContext,
  primaryText: string,
  secondaryText: string,
): Promise<number> {
  const resp = await request.post(`${baseURL}/stream/api/outputs/${SLUG}/nameplates`, {
    data: { primaryText, secondaryText },
  });
  expect(resp.ok(), `create nameplate -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

async function showNameplate(
  request: APIRequestContext,
  body: Record<string, unknown>,
): Promise<number> {
  const resp = await request.put(`${baseURL}/stream/api/outputs/${SLUG}/nameplates/active`, {
    data: body,
  });
  return resp.status();
}

/**
 * Seed a song: a library named `libraryName` + a presentation named `songTitle`.
 * The song plate resolves primary = song title (from the presentation NAME),
 * secondary = library name — NOT the slide lyric — so the names matter here.
 */
async function seedSong(
  request: APIRequestContext,
  songTitle: string,
  libraryName: string,
): Promise<{ presentationId: string; slideId: string }> {
  const libResp = await request.post(`${baseURL}/libraries`, {
    data: { name: libraryName },
  });
  expect(libResp.ok()).toBeTruthy();
  const library = (await libResp.json()) as { id: string };
  const presResp = await request.post(
    `${baseURL}/libraries/${library.id}/presentations`,
    { data: { name: songTitle } },
  );
  expect(presResp.ok()).toBeTruthy();
  const pres = (await presResp.json()) as {
    presentation: { id: string; slides: Array<{ id: string }> };
  };
  const presentationId = pres.presentation.id;
  const slideId = pres.presentation.slides[0].id;
  const patchResp = await request.patch(
    `${baseURL}/presentations/${presentationId}/slides/${slideId}`,
    { data: { main: "verš", translation: "", stage: "", group: "Verse 1" } },
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

const plates = (page: Page) => page.locator('[data-role="stream-lower-third-plate"]');
const primary = (page: Page) =>
  page.locator('[data-role="stream-lower-third-primary"]');
const secondary = (page: Page) =>
  page.locator('[data-role="stream-lower-third-secondary"]');

// ── Test ──────────────────────────────────────────────────────────────────────

test.describe("Stream lower-third nameplates (#779)", () => {
  test("show / swap / hide / auto-hide / song, clean console", async ({ page, request }) => {
    const consoleErrors: string[] = [];
    attachConsoleErrorCollector(page, consoleErrors);

    const baseId = await createScene(request, "NameplateScene", "base");
    const elementId = await addElement(request, baseId, lowerThirdProps());
    await activateBase(request, baseId);
    await gotoStream(page);

    // Transparency preserved (OBS alpha).
    expect(
      await page.evaluate(() => getComputedStyle(document.body).backgroundColor),
    ).toBe("rgba(0, 0, 0, 0)");

    // Idle: no plate on air.
    await expect(plates(page)).toHaveCount(0);

    const idA = await createNameplate(request, "Ján Novák", "pastor");
    const idB = await createNameplate(request, "Eva Malá", "vedúca chvál");

    // ── SHOW A: plate animates in with both texts + a settled identity transform.
    expect(await showNameplate(request, { source: "person", id: idA })).toBe(200);
    await expect(plates(page)).toHaveCount(1, { timeout: 5_000 });
    await expect(primary(page)).toHaveText("Ján Novák");
    await expect(secondary(page)).toHaveText("pastor");
    // After the enter animation completes the plate rests at transform: none.
    await expect
      .poll(async () => plates(page).first().evaluate((el) => getComputedStyle(el).transform), {
        timeout: 3_000,
      })
      .toBe("none");

    // ── SWAP A→B: old fades out, new fades in, settles to a single plate = B.
    expect(await showNameplate(request, { source: "person", id: idB })).toBe(200);
    await expect(primary(page)).toHaveText("Eva Malá", { timeout: 5_000 });
    await expect
      .poll(async () => plates(page).count(), { timeout: 3_000 })
      .toBe(1);

    // ── HIDE: the plate node leaves the DOM after the out animation.
    expect(await showNameplate(request, { source: null })).toBe(200);
    await expect(plates(page)).toHaveCount(0, { timeout: 3_000 });

    // ── AUTO-HIDE: patch the element to auto_hide_s=1, show A, node auto-removes.
    await patchElement(request, elementId, lowerThirdProps({ auto_hide_s: 1 }));
    expect(await showNameplate(request, { source: "person", id: idA })).toBe(200);
    await expect(plates(page)).toHaveCount(1, { timeout: 5_000 });
    // Server-side auto-hide fires after ~1 s and broadcasts a hide.
    await expect(plates(page)).toHaveCount(0, { timeout: 4_000 });

    // ── SONG plate: shows the live song title + library.
    const song = await seedSong(request, "Ako Ťa mám rád", "Kapela XY");
    await triggerSong(request, song.presentationId, song.slideId);
    expect(await showNameplate(request, { source: "song" })).toBe(200);
    await expect(plates(page)).toHaveCount(1, { timeout: 5_000 });
    await expect(primary(page)).toHaveText("Ako Ťa mám rád");
    await expect(secondary(page)).toHaveText("Kapela XY");

    // Clean console (last assertion).
    expect(consoleErrors).toEqual([]);
  });
});
