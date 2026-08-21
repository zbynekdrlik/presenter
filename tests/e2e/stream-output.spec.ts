/**
 * Stream-graphics OUTPUT page E2E (/stream/{slug}) — #709, epic #718.
 *
 * Verifies the transparent OBS browser source: it cold-loads the output def over
 * REST, subscribes to /live/ws, and renders an IMAGE element (a PNG) plus a
 * ticking COUNTDOWN element driven by Presenter's timer; a clear removes them;
 * the body/html are transparent; and a WS drop triggers a reconnect + def
 * refetch. Zero console errors AND warnings asserted last.
 *
 * ── WIRE CONTRACT (verified against the merged #706/#707/#708 handlers) ───────
 * The REST seeding + activation + asset upload below drive endpoints owned by
 * the (now merged) lanes #706 (StreamManager + stream_state/timers LiveEvents),
 * #707 (`/stream/api/*` REST) and #708 (`/stream/assets` upload).
 * `POST /stream/api/scenes/{scene_id}/elements` deserializes its body directly
 * as `StreamElementProps` (`crates/presenter-core/src/stream.rs`) — there is NO
 * `{kind, z_order, props}` wrapper: `z_order` is always server-assigned
 * (`next_element_z_order`) and is never accepted from the client. The 9 wire
 * DTO structs in `presenter-core::stream` are `#[serde(rename_all = "camelCase")]`
 * (`Frame`, `Shadow`, `TextStyle`, ...), so a props payload's `frame`/`style`
 * VALUES use camelCase keys (`xPct`, `fontFamily`, ...) while the
 * `StreamElementProps` enum's OWN tag/fields stay snake_case by design
 * (`#[serde(tag = "kind", rename_all = "snake_case")]` — `asset_id`, `timer_id`).
 * The multipart upload field name is `file`, matching #708's handler exactly.
 */

import { readFileSync } from "fs";
import path from "path";
import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  REPO_ROOT,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let dbUrl: string;
let port: number;
let oscPort: number | undefined;

const SLUG = "stream"; // the migration-seeded default output.

// 1x1 transparent PNG fixture — valid magic bytes for #708's upload validation.
const PNG_FIXTURE = path.join(REPO_ROOT, "tests", "e2e", "fixtures", "stream-1x1.png");

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  dbUrl = config.dbUrl;
  port = config.port;
  oscPort = config.oscPort;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

// ── REST helpers (see the WIRE CONTRACT note above) ────────────────────────────

async function postJson(
  request: APIRequestContext,
  apiPath: string,
  body: unknown,
): Promise<void> {
  const resp = await request.post(`${baseURL}${apiPath}`, { data: body });
  expect(resp.ok(), `${apiPath} -> ${resp.status()}`).toBeTruthy();
}

/** Upload the 1x1 PNG as an asset; returns its id. (#708 `POST /stream/assets`) */
async function uploadAsset(request: APIRequestContext): Promise<number> {
  const resp = await request.post(`${baseURL}/stream/assets`, {
    multipart: {
      file: {
        name: "dot.png",
        mimeType: "image/png",
        buffer: readFileSync(PNG_FIXTURE),
      },
    },
  });
  expect(resp.ok(), `upload asset -> ${resp.status()}`).toBeTruthy();
  const body = await resp.json();
  return body.id as number;
}

/** Create a base scene, add an image + a countdown element, return the scene id. */
async function seedBaseScene(
  request: APIRequestContext,
  assetId: number,
): Promise<number> {
  const sceneResp = await request.post(
    `${baseURL}/stream/api/outputs/${SLUG}/scenes`,
    { data: { name: "BaseA", kind: "base"} },
  );
  expect(sceneResp.ok(), `create scene -> ${sceneResp.status()}`).toBeTruthy();
  const sceneId = (await sceneResp.json()).id as number;

  // Image element — Frame 10/20/50/30 %, contain, full opacity. The request
  // body IS the `StreamElementProps` JSON directly (no `{kind, z_order, props}`
  // wrapper — see the WIRE CONTRACT note above); `z_order` is always
  // server-assigned. `asset_id`/`fit`/`opacity` stay snake_case (the enum's own
  // tag/fields); `frame`'s VALUE is a camelCase `Frame`.
  await postJson(request, `/stream/api/scenes/${sceneId}/elements`, {
    kind: "image",
    asset_id: assetId,
    fit: "contain",
    frame: { xPct: 10, yPct: 20, wPct: 50, hPct: 30 },
    opacity: 1.0,
  });

  // Countdown element — Frame 10/60/80/20 %, big centered white text w/ shadow.
  // `timer_id` stays snake_case (enum field); `style`'s VALUE is a camelCase
  // `TextStyle` (nested `shadow` likewise a camelCase `Shadow`).
  await postJson(request, `/stream/api/scenes/${sceneId}/elements`, {
    kind: "countdown",
    timer_id: 1,
    style: {
      fontFamily: "Arial",
      sizePct: 12,
      color: "#ffffff",
      weight: 700,
      align: "center",
      lineHeight: 1.2,
      shadow: { xPx: 2, yPx: 2, blurPx: 4, color: "#000000" },
    },
    frame: { xPct: 10, yPct: 60, wPct: 80, hPct: 20 },
  });

  return sceneId;
}

/** Set + auto-start the countdown ~2 min out so it renders in MM:SS and ticks. */
async function startCountdown(request: APIRequestContext): Promise<void> {
  const target = new Date(Date.now() + 125_000).toISOString();
  await postJson(request, `/timers/command`, {
    command: "set_countdown_target",
    target,
  });
}

async function activateBase(
  request: APIRequestContext,
  sceneId: number,
): Promise<void> {
  await request.put(`${baseURL}/stream/api/outputs/${SLUG}/active-scene`, {
    data: { sceneId },
  });
}

async function clearOutput(request: APIRequestContext): Promise<void> {
  await postJson(request, `/stream/api/outputs/${SLUG}/clear`, {});
}

async function gotoStream(page: Page): Promise<void> {
  await page.goto(`${baseURL}/stream/${SLUG}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="stream-canvas"]', { timeout: 10_000 });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test.describe("Stream output page", () => {
  test("renders image + ticking countdown, transparent, clears, reconnects", async ({
    page,
    request,
  }) => {
    const consoleErrors: string[] = [];
    attachConsoleErrorCollector(page, consoleErrors);

    // Count def refetches to prove the reconnect resync.
    let defRequests = 0;
    page.on("request", (req) => {
      if (req.url().includes(`/stream/api/outputs/${SLUG}/def`)) defRequests++;
    });

    // Seed: asset -> base scene (image + countdown) -> countdown running -> activate.
    const assetId = await uploadAsset(request);
    const sceneId = await seedBaseScene(request, assetId);
    await startCountdown(request);
    await activateBase(request, sceneId);

    await gotoStream(page);

    // ── Transparency (OBS alpha) ──
    const bodyBg = await page.evaluate(
      () => getComputedStyle(document.body).backgroundColor,
    );
    expect(bodyBg).toBe("rgba(0, 0, 0, 0)");
    const htmlBg = await page.evaluate(
      () => getComputedStyle(document.documentElement).backgroundColor,
    );
    expect(htmlBg).toBe("rgba(0, 0, 0, 0)");

    // ── Image element ──
    const image = page.locator('[data-role="stream-element-image"]');
    await expect(image).toHaveCount(1);
    const img = image.locator("img");
    await expect(img).toHaveAttribute("src", `/stream/assets/${assetId}`);
    // Wait for the PNG to actually decode (guards the zero-console assert: a 404
    // image would both fail this and log a console error).
    await expect
      .poll(async () => img.evaluate((el: HTMLImageElement) => el.naturalWidth))
      .toBeGreaterThan(0);
    // Frame -> computed geometry (% of the viewport). left 10% of width.
    const imageGeom = await image.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return { left: r.left, top: r.top, vw: window.innerWidth, vh: window.innerHeight };
    });
    expect(imageGeom.left).toBeCloseTo(imageGeom.vw * 0.1, 0);
    expect(imageGeom.top).toBeCloseTo(imageGeom.vh * 0.2, 0);
    const objectFit = await img.evaluate((el) => getComputedStyle(el).objectFit);
    expect(objectFit).toBe("contain");

    // ── Countdown element: TextStyle mapping + ticking ──
    const countdown = page.locator('[data-role="stream-element-countdown"]');
    await expect(countdown).toHaveCount(1);
    // Non-empty MM:SS text within a couple of ticks (cold-load timers + WS).
    await expect(countdown).toHaveText(/\d/, { timeout: 5_000 });
    // font-size is size_pct(12) vh -> ~12% of viewport height in px.
    const fontPx = await countdown.evaluate((el) =>
      parseFloat(getComputedStyle(el).fontSize),
    );
    expect(fontPx).toBeCloseTo((await page.evaluate(() => window.innerHeight)) * 0.12, 0);
    // text-shadow present (not "none").
    const textShadow = await countdown.evaluate(
      (el) => getComputedStyle(el).textShadow,
    );
    expect(textShadow).not.toBe("none");
    expect(textShadow.length).toBeGreaterThan(0);
    // Ticks: the text changes as the local tick advances between server
    // pushes. Poll for a changed value instead of an arbitrary sleep.
    const t0 = (await countdown.textContent())?.trim() ?? "";
    expect(t0).not.toBe("");
    await expect
      .poll(async () => (await countdown.textContent())?.trim() ?? "", {
        timeout: 5_000,
      })
      .not.toBe(t0);

    // ── Clear ⇒ elements gone ──
    await clearOutput(request);
    await expect(image).toHaveCount(0, { timeout: 5_000 });
    await expect(countdown).toHaveCount(0, { timeout: 5_000 });

    // Re-activate for the reconnect test.
    await activateBase(request, sceneId);
    await expect(image).toHaveCount(1, { timeout: 5_000 });

    // ── WS drop ⇒ reconnect + def refetch ──
    const defsBefore = defRequests;
    await stopServer(serverHandle);
    serverHandle = await startTestServer(port, dbUrl, oscPort);
    // The page must reconnect (backoff) and refetch the def, re-rendering state.
    await page.waitForFunction(
      () =>
        document
          .querySelector('[data-role="stream-canvas"]')
          ?.getAttribute("data-ws-state") === "connected",
      undefined,
      { timeout: 45_000 },
    );
    await expect
      .poll(() => defRequests, { timeout: 15_000 })
      .toBeGreaterThan(defsBefore);
    await expect(image).toHaveCount(1, { timeout: 10_000 });

    // Zero console errors AND warnings — asserted last.
    expect(consoleErrors).toEqual([]);
  });
});
