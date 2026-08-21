/**
 * Stream-graphics COMPANION full-journey E2E (/stream/{slug}) — #717, epic #718.
 *
 * The Resolume-replacement "in production" test: an operator's Bitfocus
 * Companion buttons drive the whole show over the REAL companion WebSocket,
 * and a transparent OBS output page follows every step. It combines the two
 * proven idioms already in this suite:
 *   - the real companion socket (hello → welcome → command → ack + variables)
 *     from `companion-session.spec.ts`, and
 *   - the REST scene/element seeding + output-page data-role asserts from
 *     `stream-output.spec.ts` / `stream-output-content.spec.ts`.
 *
 * The user's setup is built once via REST in `beforeAll`:
 *   base scenes    — `ytfast` (PNG + countdown), `5min` (bigger countdown),
 *                    `1min` (biggest countdown), `chvaly` (lyrics+translation,
 *                    fade content-transition);
 *   overlay scenes — `Logo` (PNG), `Verse` (verse text + translation, fade).
 *
 * Companion wire contract (verified against `companion/stream.rs` +
 * `ops/companion/presenter/lib/stream.js`):
 *   stream_scene_set     { scene, output? }  — exclusive base activate
 *   stream_scene_clear   { output? }         — base → none
 *   stream_overlay_on/_off/_toggle { scene, output? }
 *   stream_clear         { output? }         — base + all overlays off
 * `output` defaults to "stream"; scene addressed by NAME, case-insensitive.
 * Every activation broadcasts `LiveEvent::StreamState`, which the live-loop
 * resolves into the `stream_scene` / `stream_overlays` companion variables.
 *
 * Asserts, across three tests, each ending with zero console errors AND
 * warnings as its LAST assertion:
 *   A — companion switches the exclusive base (ytfast→5min→1min→chvaly, three
 *       distinct countdown font sizes prove the "3 sizes"), toggles the two
 *       overlays INDEPENDENTLY, shows lyrics+translation and verse+translation,
 *       then `stream_clear` returns the output to transparent; the companion
 *       `stream_scene` / `stream_overlays` variables track each step.
 *   B — active show-state survives a server restart mid-show (the DB-persisted
 *       active scene + overlays re-render after the OBS source reconnects).
 *   C — two simultaneous output-page contexts (multi-OBS) converge on the same
 *       companion command.
 *
 * The bundled OFL font families (Inter / Oswald / Bebas Neue, #717) are used
 * deliberately in the element styles: if a bundled `@font-face` path is wrong
 * the woff2 404s and the zero-console assertion catches it.
 *
 * Tier-0 note: this spec's actual green run happens in the integrated CI e2e
 * lane (the runner builds the server + trunk dist); it cannot run on the
 * Tier-0 dev box (no local `npx playwright test`, no local build).
 */

import { readFileSync } from "fs";
import path from "path";
import {
  test,
  expect,
  request as playwrightRequest,
  type APIRequestContext,
  type Browser,
  type Page,
} from "@playwright/test";
import WebSocket from "ws";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  REPO_ROOT,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

const SLUG = "stream"; // the migration-seeded default output.
const PNG_FIXTURE = path.join(REPO_ROOT, "tests", "e2e", "fixtures", "stream-1x1.png");

// ── companion socket helper (mirrors companion-session.spec.ts) ────────────────

type VarEntry = { name?: unknown; value?: unknown };
type WsMsg = Record<string, unknown>;

const HELLO_PAYLOAD = {
  type: "hello",
  client: "Playwright",
  instanceName: "stream-companion-spec",
};

/** Open a companion WebSocket, expose handshake + send + variable helpers. */
function createCompanionSocket(wsURL: string) {
  const socket = new WebSocket(wsURL);
  const errors: Error[] = [];
  // Running, merged companion variable state — updated on EVERY `variables`
  // frame (delta or full). `stream_scene` / `stream_overlays` land here after
  // the async live-loop resolves them, so tests poll this map.
  const vars = new Map<string, string>();

  socket.on("message", (raw: WebSocket.RawData) => {
    try {
      const parsed = JSON.parse(raw.toString()) as WsMsg;
      if (parsed.type === "variables" && Array.isArray(parsed.values)) {
        for (const e of parsed.values as VarEntry[]) {
          vars.set(String(e.name ?? ""), String(e.value ?? ""));
        }
      }
    } catch (error) {
      errors.push(error as Error);
    }
  });

  const waitForMessage = (
    predicate: (msg: WsMsg) => boolean,
    timeoutMs = 5_000,
  ) =>
    new Promise<WsMsg>((resolve, reject) => {
      const timeout = setTimeout(() => {
        cleanup();
        reject(new Error("Timed out waiting for expected Companion message"));
      }, timeoutMs);
      const cleanup = () => {
        clearTimeout(timeout);
        socket.off("message", handleMessage);
      };
      const handleMessage = (raw: WebSocket.RawData) => {
        try {
          const parsed = JSON.parse(raw.toString()) as WsMsg;
          if (predicate(parsed)) {
            cleanup();
            resolve(parsed);
          }
        } catch (error) {
          cleanup();
          reject(error as Error);
        }
      };
      socket.on("message", handleMessage);
    });

  /** Connect, send hello, receive welcome + the initial variables frame. */
  async function handshake() {
    await new Promise<void>((resolve, reject) => {
      socket.once("error", (err) => reject(err));
      socket.once("open", () => {
        socket.send(JSON.stringify(HELLO_PAYLOAD));
        resolve();
      });
    });
    await waitForMessage((msg) => msg.type === "welcome");
    await waitForMessage((msg) => msg.type === "variables");
  }

  /** Send a stream command; resolve with its ack/error reply. */
  async function sendCommand(
    command: string,
    payload: Record<string, unknown> = {},
  ): Promise<WsMsg> {
    socket.send(JSON.stringify({ type: "command", command, payload }));
    return waitForMessage(
      (msg) =>
        (msg.type === "ack" && msg.command === command) || msg.type === "error",
    );
  }

  /** Send a command and assert the server ACKed it (no error reply). */
  async function expectAck(
    command: string,
    payload: Record<string, unknown> = {},
  ): Promise<void> {
    const reply = await sendCommand(command, payload);
    expect(
      reply.type,
      `${command} ${JSON.stringify(payload)} -> ${JSON.stringify(reply)}`,
    ).toBe("ack");
  }

  return { socket, errors, vars, handshake, waitForMessage, sendCommand, expectAck };
}

// ── module state ───────────────────────────────────────────────────────────────

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let dbUrl: string;
let port: number;
let oscPort: number | undefined;
let wsURL: string;

type Ids = {
  assetId: number;
  ytfast: number;
  ytfastImg: number;
  ytfastCd: number;
  fiveMinCd: number;
  oneMinCd: number;
  chvaly: number;
  chvalyLyrics: number;
  logo: number;
  logoImg: number;
  verseEl: number;
};
let ids: Ids;

// ── REST helpers (verified stream wire contract) ───────────────────────────────

function textStyle(
  fontFamily: string,
  sizePct: number,
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    fontFamily,
    sizePct,
    color: "#ffffff",
    weight: 700,
    align: "center",
    lineHeight: 1.2,
    ...overrides,
  };
}

async function uploadAsset(api: APIRequestContext): Promise<number> {
  const resp = await api.post(`${baseURL}/stream/assets`, {
    multipart: {
      file: {
        name: "dot.png",
        mimeType: "image/png",
        buffer: readFileSync(PNG_FIXTURE),
      },
    },
  });
  expect(resp.ok(), `upload asset -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

async function createScene(
  api: APIRequestContext,
  name: string,
  kind: "base" | "overlay",
): Promise<number> {
  const resp = await api.post(`${baseURL}/stream/api/outputs/${SLUG}/scenes`, {
    data: { name, kind },
  });
  expect(resp.ok(), `create scene ${name} -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

// The body IS the bare `StreamElementProps` JSON (no {kind,z_order,props}
// wrapper); z_order is server-assigned. Enum tag/fields are snake_case
// (`kind`, `asset_id`, `show_main`, ...); nested `frame`/`style` VALUES are
// camelCase (`xPct`, `fontFamily`, ...).
async function addElement(
  api: APIRequestContext,
  sceneId: number,
  props: Record<string, unknown>,
): Promise<number> {
  const resp = await api.post(`${baseURL}/stream/api/scenes/${sceneId}/elements`, {
    data: props,
  });
  expect(resp.ok(), `add element -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

/** Build the user's full scene/element setup once. Returns all ids. */
async function buildSetup(api: APIRequestContext): Promise<Ids> {
  const assetId = await uploadAsset(api);

  // Base `ytfast`: PNG + countdown (smallest, 10vh).
  const ytfast = await createScene(api, "ytfast", "base");
  const ytfastImg = await addElement(api, ytfast, {
    kind: "image",
    asset_id: assetId,
    fit: "contain",
    frame: { xPct: 10, yPct: 20, wPct: 50, hPct: 30 },
    opacity: 1.0,
  });
  const ytfastCd = await addElement(api, ytfast, {
    kind: "countdown",
    timer_id: 1,
    style: textStyle("Oswald", 10, {
      shadow: { xPx: 2, yPx: 2, blurPx: 4, color: "#000000" },
    }),
    frame: { xPct: 10, yPct: 60, wPct: 80, hPct: 20 },
    content_transition: { mode: "cut" },
  });

  // Base `5min`: bigger countdown (14vh), no image.
  const fiveMin = await createScene(api, "5min", "base");
  const fiveMinCd = await addElement(api, fiveMin, {
    kind: "countdown",
    timer_id: 1,
    style: textStyle("Oswald", 14),
    frame: { xPct: 10, yPct: 40, wPct: 80, hPct: 25 },
    content_transition: { mode: "cut" },
  });

  // Base `1min`: biggest countdown (18vh), no image.
  const oneMin = await createScene(api, "1min", "base");
  const oneMinCd = await addElement(api, oneMin, {
    kind: "countdown",
    timer_id: 1,
    style: textStyle("Oswald", 18),
    frame: { xPct: 10, yPct: 40, wPct: 80, hPct: 30 },
    content_transition: { mode: "cut" },
  });

  // Base `chvaly`: lyrics main + translation, fade content-transition.
  const chvaly = await createScene(api, "chvaly", "base");
  const chvalyLyrics = await addElement(api, chvaly, {
    kind: "lyrics",
    show_main: true,
    show_translation: true,
    main_style: textStyle("Inter", 8),
    translation_style: textStyle("Inter", 5, { color: "#cccccc" }),
    frame: { xPct: 5, yPct: 10, wPct: 60, hPct: 40 },
    content_transition: { mode: "fade", duration_ms: 400 },
  });

  // Overlay `Logo`: PNG.
  const logo = await createScene(api, "Logo", "overlay");
  const logoImg = await addElement(api, logo, {
    kind: "image",
    asset_id: assetId,
    fit: "contain",
    frame: { xPct: 70, yPct: 5, wPct: 25, hPct: 20 },
    opacity: 1.0,
  });

  // Overlay `Verse`: verse text + translation (secondary) + reference, fade.
  const verse = await createScene(api, "Verse", "overlay");
  const verseEl = await addElement(api, verse, {
    kind: "verse",
    show_secondary: true,
    text_style: textStyle("Inter", 6),
    secondary_style: textStyle("Inter", 4, { color: "#dddddd" }),
    // Bebas Neue is a single-weight (400) display face — request 400 so the
    // browser uses the real glyphs instead of synth-bolding a 700.
    reference_style: textStyle("Bebas Neue", 3, { color: "#aaaaaa", weight: 400 }),
    frame: { xPct: 15, yPct: 60, wPct: 70, hPct: 35 },
    content_transition: { mode: "fade", duration_ms: 400 },
  });

  return {
    assetId,
    ytfast,
    ytfastImg,
    ytfastCd,
    fiveMinCd,
    oneMinCd,
    chvaly,
    chvalyLyrics,
    logo,
    logoImg,
    verseEl,
  };
}

/** Set + auto-start the countdown ~2 min out so it renders MM:SS and ticks. */
async function startCountdown(api: APIRequestContext): Promise<void> {
  const target = new Date(Date.now() + 125_000).toISOString();
  const resp = await api.post(`${baseURL}/timers/command`, {
    data: { command: "set_countdown_target", target },
  });
  expect(resp.ok(), `set countdown target -> ${resp.status()}`).toBeTruthy();
}

/** Create a library + presentation, set the first slide's text; return ids. */
async function seedSong(
  api: APIRequestContext,
  main: string,
  translation: string,
): Promise<{ presentationId: string; slideId: string }> {
  const libResp = await api.post(`${baseURL}/libraries`, {
    data: { name: `Companion Lib ${Date.now()}-${Math.random()}` },
  });
  expect(libResp.ok()).toBeTruthy();
  const library = (await libResp.json()) as { id: string };

  const presResp = await api.post(
    `${baseURL}/libraries/${library.id}/presentations`,
    { data: { name: "Companion Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const pres = (await presResp.json()) as {
    presentation: { id: string; slides: Array<{ id: string }> };
  };
  const presentationId = pres.presentation.id;
  const slideId = pres.presentation.slides[0].id;

  const patchResp = await api.patch(
    `${baseURL}/presentations/${presentationId}/slides/${slideId}`,
    { data: { main, translation, stage: "", group: "Verse 1" } },
  );
  expect(patchResp.ok()).toBeTruthy();
  return { presentationId, slideId };
}

async function triggerSong(
  api: APIRequestContext,
  presentationId: string,
  slideId: string,
): Promise<void> {
  const resp = await api.post(`${baseURL}/stage/state`, {
    data: { presentationId, currentSlideId: slideId },
  });
  expect(resp.status(), "trigger stage state").toBe(204);
}

async function triggerVerse(
  api: APIRequestContext,
  fields: Record<string, string>,
): Promise<void> {
  const resp = await api.post(`${baseURL}/bible/trigger-slide`, { data: fields });
  expect(resp.ok(), `trigger verse -> ${resp.status()}`).toBeTruthy();
}

async function gotoStream(page: Page): Promise<void> {
  await page.goto(`${baseURL}/stream/${SLUG}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="stream-canvas"]', { timeout: 10_000 });
}

async function assertTransparent(page: Page): Promise<void> {
  expect(
    await page.evaluate(() => getComputedStyle(document.body).backgroundColor),
  ).toBe("rgba(0, 0, 0, 0)");
  expect(
    await page.evaluate(
      () => getComputedStyle(document.documentElement).backgroundColor,
    ),
  ).toBe("rgba(0, 0, 0, 0)");
}

// data-role element locators (targeted by element id — unambiguous even during
// a scene crossfade, when the outgoing scene briefly still renders its own).
const imageEl = (page: Page, id: number) =>
  page.locator(`[data-role="stream-element-image"][data-element-id="${id}"]`);
const countdownEl = (page: Page, id: number) =>
  page.locator(`[data-role="stream-element-countdown"][data-element-id="${id}"]`);
const lyricsMain = (page: Page, id: number) =>
  page.locator(
    `[data-role="stream-element-lyrics"][data-element-id="${id}"] [data-role="stream-lyrics-main"]`,
  );
const lyricsTranslation = (page: Page, id: number) =>
  page.locator(
    `[data-role="stream-element-lyrics"][data-element-id="${id}"] [data-role="stream-lyrics-translation"]`,
  );
const verseText = (page: Page, id: number) =>
  page.locator(
    `[data-role="stream-element-verse"][data-element-id="${id}"] [data-role="stream-verse-text"]`,
  );
const verseSecondary = (page: Page, id: number) =>
  page.locator(
    `[data-role="stream-element-verse"][data-element-id="${id}"] [data-role="stream-verse-secondary-text"]`,
  );

/** Wait for the output page's WS to report connected + a fresh def refetch. */
async function waitForReconnect(
  page: Page,
  defsBefore: number,
  counter: () => number,
) {
  await page.waitForFunction(
    () =>
      document
        .querySelector('[data-role="stream-canvas"]')
        ?.getAttribute("data-ws-state") === "connected",
    undefined,
    { timeout: 45_000 },
  );
  await expect.poll(counter, { timeout: 15_000 }).toBeGreaterThan(defsBefore);
}

// ── setup ──────────────────────────────────────────────────────────────────────

test.describe.configure({ timeout: 240_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  dbUrl = config.dbUrl;
  port = config.port;
  oscPort = config.oscPort;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);

  // Enable the companion socket and discover its bound port (mirrors
  // companion-session.spec.ts).
  const desiredPort = config.port + 100;
  const enable = await fetch(new URL("/settings/features", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ companionEnabled: true, companionPort: desiredPort }),
  });
  if (!enable.ok) {
    throw new Error(`Failed to enable companion websocket (${enable.status})`);
  }
  const features = await fetch(new URL("/settings/features", baseURL).toString(), {
    headers: { Accept: "application/json" },
  });
  if (!features.ok) {
    throw new Error(`Failed to fetch feature flags (${features.status})`);
  }
  const payload = (await features.json()) as {
    companionPort?: number;
    companion_port?: number;
  };
  const base = new URL(baseURL);
  const rawPortValue =
    payload.companionPort ?? payload.companion_port ?? desiredPort;
  const parsedPort = Number.parseInt(String(rawPortValue), 10);
  const companionPort =
    Number.isFinite(parsedPort) && parsedPort >= 1 ? parsedPort : desiredPort;
  wsURL = `${base.protocol.replace("http", "ws")}//${base.hostname}:${companionPort}/companion/ws`;

  // Build the user's scene/element setup once.
  const api = await playwrightRequest.newContext();
  try {
    ids = await buildSetup(api);
  } finally {
    await api.dispose();
  }
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

// ── tests ──────────────────────────────────────────────────────────────────────

test.describe("Stream graphics — companion full journey", () => {
  test("companion drives base switch, overlays, content and clear", async ({
    page,
    request,
  }) => {
    const consoleErrors: string[] = [];
    attachConsoleErrorCollector(page, consoleErrors);

    const sock = createCompanionSocket(wsURL);
    await sock.handshake();

    // Baseline: nothing active.
    await sock.expectAck("stream_clear");

    // Content available before we switch to the scenes that show it.
    await startCountdown(request);
    const song = await seedSong(request, "Amazing Grace", "Úžasná Milosť");
    await triggerSong(request, song.presentationId, song.slideId);
    await triggerVerse(request, {
      mainText: "For God so loved the world",
      mainReference: "John 3:16",
      secondaryText: "Lebo tak Boh miloval svet",
      secondaryReference: "Ján 3:16",
    });

    await gotoStream(page);

    // Cold, nothing active → transparent, no elements.
    await assertTransparent(page);
    await expect(imageEl(page, ids.ytfastImg)).toHaveCount(0);
    await expect(countdownEl(page, ids.ytfastCd)).toHaveCount(0);

    // ── ytfast: PNG + smallest countdown ──
    await sock.expectAck("stream_scene_set", { scene: "ytfast" });
    await expect(imageEl(page, ids.ytfastImg)).toHaveCount(1, { timeout: 5_000 });
    const img = imageEl(page, ids.ytfastImg).locator("img");
    await expect(img).toHaveAttribute("src", `/stream/assets/${ids.assetId}`);
    await expect
      .poll(() => img.evaluate((el: HTMLImageElement) => el.naturalWidth))
      .toBeGreaterThan(0);
    await expect(countdownEl(page, ids.ytfastCd)).toHaveText(/\d/, { timeout: 5_000 });
    const vh = await page.evaluate(() => window.innerHeight);
    const ytFont = await countdownEl(page, ids.ytfastCd).evaluate((el) =>
      parseFloat(getComputedStyle(el).fontSize),
    );
    expect(ytFont).toBeCloseTo(vh * 0.1, 0);
    // Companion variables reflect the active base.
    await expect
      .poll(() => sock.vars.get("stream_scene"), { timeout: 5_000 })
      .toBe("ytfast");

    // ── 5min: exclusive switch → image gone, bigger countdown (14vh) ──
    await sock.expectAck("stream_scene_set", { scene: "5min" });
    await expect(imageEl(page, ids.ytfastImg)).toHaveCount(0, { timeout: 5_000 });
    await expect(countdownEl(page, ids.ytfastCd)).toHaveCount(0, { timeout: 5_000 });
    await expect(countdownEl(page, ids.fiveMinCd)).toHaveText(/\d/, { timeout: 5_000 });
    const fiveFont = await countdownEl(page, ids.fiveMinCd).evaluate((el) =>
      parseFloat(getComputedStyle(el).fontSize),
    );
    expect(fiveFont).toBeCloseTo(vh * 0.14, 0);
    expect(fiveFont).toBeGreaterThan(ytFont);

    // ── 1min: biggest countdown (18vh) ──
    await sock.expectAck("stream_scene_set", { scene: "1min" });
    await expect(countdownEl(page, ids.fiveMinCd)).toHaveCount(0, { timeout: 5_000 });
    await expect(countdownEl(page, ids.oneMinCd)).toHaveText(/\d/, { timeout: 5_000 });
    const oneFont = await countdownEl(page, ids.oneMinCd).evaluate((el) =>
      parseFloat(getComputedStyle(el).fontSize),
    );
    expect(oneFont).toBeCloseTo(vh * 0.18, 0);
    expect(oneFont).toBeGreaterThan(fiveFont);

    // ── chvaly: lyrics main + translation ──
    await sock.expectAck("stream_scene_set", { scene: "chvaly" });
    await expect(countdownEl(page, ids.oneMinCd)).toHaveCount(0, { timeout: 5_000 });
    await expect(lyricsMain(page, ids.chvalyLyrics)).toHaveText("Amazing Grace", {
      timeout: 5_000,
    });
    await expect(lyricsTranslation(page, ids.chvalyLyrics)).toHaveText("Úžasná Milosť");
    await expect
      .poll(() => sock.vars.get("stream_scene"), { timeout: 5_000 })
      .toBe("chvaly");

    // ── Verse overlay ON (independent of base) → verse + translation ──
    await sock.expectAck("stream_overlay_toggle", { scene: "Verse" });
    await expect(verseText(page, ids.verseEl)).toHaveText(
      "For God so loved the world",
      { timeout: 5_000 },
    );
    await expect(verseSecondary(page, ids.verseEl)).toHaveText(
      "Lebo tak Boh miloval svet",
    );
    // Base lyrics still there — overlay is independent.
    await expect(lyricsMain(page, ids.chvalyLyrics)).toHaveText("Amazing Grace");
    await expect
      .poll(() => sock.vars.get("stream_overlays"), { timeout: 5_000 })
      .toContain("Verse");

    // ── Logo overlay ON as well → both overlays active, both render ──
    await sock.expectAck("stream_overlay_toggle", { scene: "Logo" });
    await expect(imageEl(page, ids.logoImg)).toHaveCount(1, { timeout: 5_000 });
    await expect(verseText(page, ids.verseEl)).toHaveText("For God so loved the world");
    await expect
      .poll(() => sock.vars.get("stream_overlays") ?? "", { timeout: 5_000 })
      .toContain("Logo");
    expect(sock.vars.get("stream_overlays") ?? "").toContain("Verse");

    // ── Verse overlay OFF → verse gone, Logo + lyrics stay ──
    await sock.expectAck("stream_overlay_toggle", { scene: "Verse" });
    await expect(verseText(page, ids.verseEl)).toHaveCount(0, { timeout: 5_000 });
    await expect(imageEl(page, ids.logoImg)).toHaveCount(1);
    await expect(lyricsMain(page, ids.chvalyLyrics)).toHaveText("Amazing Grace");
    await expect
      .poll(() => sock.vars.get("stream_overlays") ?? "", { timeout: 5_000 })
      .not.toContain("Verse");

    // ── stream_clear → base + all overlays gone → transparent ──
    await sock.expectAck("stream_clear");
    await expect(lyricsMain(page, ids.chvalyLyrics)).toHaveCount(0, { timeout: 5_000 });
    await expect(imageEl(page, ids.logoImg)).toHaveCount(0, { timeout: 5_000 });
    await assertTransparent(page);
    await expect.poll(() => sock.vars.get("stream_scene"), { timeout: 5_000 }).toBe("-");
    await expect
      .poll(() => sock.vars.get("stream_overlays"), { timeout: 5_000 })
      .toBe("-");

    expect(sock.errors).toHaveLength(0);
    // Zero console errors AND warnings — asserted last.
    expect(consoleErrors).toEqual([]);
    sock.socket.close();
  });

  test("active show-state survives a server restart mid-show", async ({
    page,
    request,
  }) => {
    const consoleErrors: string[] = [];
    attachConsoleErrorCollector(page, consoleErrors);

    let defRequests = 0;
    page.on("request", (req) => {
      if (req.url().includes(`/stream/api/outputs/${SLUG}/def`)) defRequests++;
    });

    const sock = createCompanionSocket(wsURL);
    await sock.handshake();

    // Set an image-based base + overlay (static, DB-persisted — no in-memory
    // content pipeline needed to prove the active state persists).
    await sock.expectAck("stream_clear");
    await sock.expectAck("stream_scene_set", { scene: "ytfast" });
    await sock.expectAck("stream_overlay_toggle", { scene: "Logo" });
    sock.socket.close();

    await gotoStream(page);
    await expect(imageEl(page, ids.ytfastImg)).toHaveCount(1, { timeout: 5_000 });
    await expect(imageEl(page, ids.logoImg)).toHaveCount(1, { timeout: 5_000 });

    // ── Restart the server mid-show ──
    const defsBefore = defRequests;
    await stopServer(serverHandle);
    serverHandle = await startTestServer(port, dbUrl, oscPort);

    // The OBS source reconnects (backoff) and refetches the def.
    await waitForReconnect(page, defsBefore, () => defRequests);

    // The DB-persisted active state is intact after restart.
    const def = (await request
      .get(`${baseURL}/stream/api/outputs/${SLUG}/def`)
      .then((r) => r.json())) as {
      activeSceneId: number | null;
      scenes: Array<{ id: number; kind: string; isActive: boolean }>;
    };
    expect(def.activeSceneId).toBe(ids.ytfast);
    const activeOverlay = def.scenes.find((s) => s.isActive && s.kind === "overlay");
    expect(
      activeOverlay?.id,
      "the Logo overlay stayed active across restart",
    ).toBe(ids.logo);

    // …and re-renders identically once the OBS source reconnects.
    await expect(imageEl(page, ids.ytfastImg)).toHaveCount(1, { timeout: 10_000 });
    await expect(imageEl(page, ids.logoImg)).toHaveCount(1, { timeout: 10_000 });

    // Zero console errors AND warnings — asserted last.
    expect(consoleErrors).toEqual([]);
  });

  test("two output-page contexts converge under companion control", async ({
    browser,
  }: {
    browser: Browser;
  }) => {
    const ctxA = await browser.newContext();
    const ctxB = await browser.newContext();
    const pageA = await ctxA.newPage();
    const pageB = await ctxB.newPage();
    const errorsA: string[] = [];
    const errorsB: string[] = [];
    attachConsoleErrorCollector(pageA, errorsA);
    attachConsoleErrorCollector(pageB, errorsB);

    // Content for the chvaly lyrics (each context cold-loads it on connect).
    const song = await seedSong(pageA.request, "How Great Thou Art", "Aká si veľký");
    await triggerSong(pageA.request, song.presentationId, song.slideId);

    const sock = createCompanionSocket(wsURL);
    await sock.handshake();
    await sock.expectAck("stream_clear");

    await gotoStream(pageA);
    await gotoStream(pageB);

    // ── Activate chvaly → BOTH contexts show the same lyrics ──
    await sock.expectAck("stream_scene_set", { scene: "chvaly" });
    for (const p of [pageA, pageB]) {
      await expect(lyricsMain(p, ids.chvalyLyrics)).toHaveText("How Great Thou Art", {
        timeout: 8_000,
      });
    }

    // ── Logo overlay ON → BOTH show the logo image ──
    await sock.expectAck("stream_overlay_toggle", { scene: "Logo" });
    for (const p of [pageA, pageB]) {
      await expect(imageEl(p, ids.logoImg)).toHaveCount(1, { timeout: 8_000 });
    }

    // ── Clear → BOTH transparent ──
    await sock.expectAck("stream_clear");
    for (const p of [pageA, pageB]) {
      await expect(lyricsMain(p, ids.chvalyLyrics)).toHaveCount(0, { timeout: 8_000 });
      await expect(imageEl(p, ids.logoImg)).toHaveCount(0);
      await assertTransparent(p);
    }
    sock.socket.close();

    // Zero console errors AND warnings — asserted last.
    expect(errorsA).toEqual([]);
    expect(errorsB).toEqual([]);

    await ctxA.close();
    await ctxB.close();
  });
});
