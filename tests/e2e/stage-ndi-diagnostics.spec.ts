import { test, expect, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// #732 — stage-display diagnostics telemetry.
//
// The grey native play-arrow never reproduced on the emulated Android System
// WebViews, and the real Vestel/TCL TVs are dark between events — so the
// product SELF-REPORTS each stage TV's WebView userAgent + NDI <video> runtime
// state over the presence/heartbeat socket. This asserts that end-to-end path:
// a real stage connection reports its user_agent AND an ndi_video snapshot
// (paused:false, videoWidth>0) on the existing GET /stage/connections monitor
// surface the operator already reads for "N stage displays connected".
//
// Runs on the standard GitHub-hosted `e2e` lane (no NDI SDK/GPU): WHEP is
// bypassed by assigning a synthetic canvas.captureStream() MediaStream onto
// the mounted <video> — same technique as stage-ndi-playback-guard.spec.ts.
// ─────────────────────────────────────────────────────────────────────────

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";
let dbUrl = "";
let port = 0;

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

/** Console noise expected on a host with no live NDI source/SDK: the WHEP
 * POST is answered 503 (no SDK) / 204 (configured-but-not-producing) and the
 * client backs off quietly with a WARN — same allow-list the sibling NDI
 * specs use. */
const ALLOWED_CONSOLE_NOISE = [
  /Failed to load resource.*\b(503|204)\b/i,
  /WHEP (POST|connect)[^\n]*\b(503|204)\b/i,
  /reconnect_loop.*connect_whep failed/i,
];

function collectConsoleNoise(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    const text = msg.text();
    if (!ALLOWED_CONSOLE_NOISE.some((re) => re.test(text))) {
      messages.push(`[${msg.type()}] ${text}`);
    }
  });
  page.on("pageerror", (err) => messages.push(`[pageerror] ${err.message}`));
  return messages;
}

/** Create + activate a not-producing NDI source, select the ndi-fullscreen
 * layout, and open `/stage` (a REAL stage connection, not ?preview=1) with the
 * NDI <video> mounted. Returns the created source id (caller cleans it up). */
async function openStageWithNdiVideo(page: Page): Promise<string> {
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
    { failOnStatusCode: false },
  );
  await page.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });

  const created = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "Diagnostics", ndiName: "BOGUS-Diagnostics" } },
  );
  const source = await created.json();
  await page.request.post(
    new URL(
      `/integrations/video-sources/${source.id}/activate`,
      baseURL,
    ).toString(),
    { failOnStatusCode: false },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
    timeout: 10_000,
  });
  await page.waitForSelector('[data-role="ndi-video"]', { timeout: 15_000 });
  return source.id as string;
}

async function deactivateAndDelete(page: Page, sourceId: string): Promise<void> {
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
    { failOnStatusCode: false },
  );
  await page.request.delete(
    new URL(`/integrations/video-sources/${sourceId}`, baseURL).toString(),
    { failOnStatusCode: false },
  );
}

type NdiVideoDiag = {
  paused?: boolean;
  videoWidth?: number;
  readyState?: number;
  hasSrcObject?: boolean;
  coverVisible?: boolean;
  layoutCode?: string;
};
type StageConnection = {
  userAgent?: string;
  ndiVideo?: NdiVideoDiag;
  lastDiagAt?: string;
};

async function fetchConnections(page: Page): Promise<StageConnection[]> {
  const res = await page.request.get(
    new URL("/stage/connections", baseURL).toString(),
  );
  return (await res.json()) as StageConnection[];
}

test("stage display reports userAgent + NDI <video> diagnostics on /stage/connections (#732)", async ({
  page,
}) => {
  const consoleMessages = collectConsoleNoise(page);
  const sourceId = await openStageWithNdiVideo(page);

  try {
    // Bypass WHEP: assign a synthetic canvas-captured MediaStream directly onto
    // the mounted <video> so it genuinely plays (paused:false, videoWidth>0)
    // without a live NDI source or GPU.
    await page.evaluate((sel) => {
      const video = document.querySelector(sel) as HTMLVideoElement;
      const canvas = document.createElement("canvas");
      canvas.width = 32;
      canvas.height = 32;
      const ctx = canvas.getContext("2d")!;
      let toggle = false;
      const interval = setInterval(() => {
        toggle = !toggle;
        ctx.fillStyle = toggle ? "blue" : "red";
        ctx.fillRect(0, 0, 32, 32);
      }, 100);
      (window as unknown as { __e2eDiagInterval?: number }).__e2eDiagInterval =
        interval as unknown as number;
      ctx.fillStyle = "red";
      ctx.fillRect(0, 0, 32, 32);
      const stream = canvas.captureStream(10);
      video.srcObject = stream;
      video.muted = true;
      return video.play();
    }, '[data-role="ndi-video"]');

    // The synthetic stream is genuinely playing before we assert telemetry.
    await expect
      .poll(
        () =>
          page.evaluate(
            (sel) => (document.querySelector(sel) as HTMLVideoElement).paused,
            '[data-role="ndi-video"]',
          ),
        { timeout: 10_000 },
      )
      .toBe(false);

    // The stage connection must appear on the monitor endpoint WITH a userAgent
    // and an ndi_video snapshot showing a playing, decoded video. The snapshot
    // arrives via the on-change StageDiag push (paused true→false) and/or the
    // heartbeat carrier — poll until the server has stored it.
    await expect
      .poll(
        async () => {
          const conns = await fetchConnections(page);
          const live = conns.find(
            (c) =>
              !!c.userAgent &&
              !!c.ndiVideo &&
              c.ndiVideo.paused === false &&
              (c.ndiVideo.videoWidth ?? 0) > 0,
          );
          return live ? "found" : JSON.stringify(conns);
        },
        { timeout: 20_000, intervals: [500, 1000, 2000] },
      )
      .toBe("found");

    // Read back concrete values and assert the fields are coherent (#732).
    const conns = await fetchConnections(page);
    const live = conns.find(
      (c) => !!c.ndiVideo && c.ndiVideo.paused === false,
    )!;
    expect(live.userAgent, "userAgent present").toBeTruthy();
    expect(
      live.userAgent!.length,
      "userAgent is a real UA string",
    ).toBeGreaterThan(10);
    expect(live.ndiVideo!.videoWidth, "videoWidth > 0").toBeGreaterThan(0);
    expect(live.ndiVideo!.hasSrcObject, "has a MediaStream").toBe(true);
    expect(live.ndiVideo!.layoutCode, "carries the layout code").toBe(
      "ndi-fullscreen",
    );
    expect(live.lastDiagAt, "records when the snapshot arrived").toBeTruthy();

    await page.evaluate(() => {
      const w = window as unknown as { __e2eDiagInterval?: number };
      if (w.__e2eDiagInterval) clearInterval(w.__e2eDiagInterval);
    });

    expect(consoleMessages).toEqual([]);
  } finally {
    await deactivateAndDelete(page, sourceId);
  }
});
