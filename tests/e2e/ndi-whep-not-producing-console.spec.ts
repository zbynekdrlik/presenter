import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #431: the prod stage logged repeated browser console ERRORS —
//   [ERROR] Failed to load resource: ... 404 (Not Found) @ /ndi/whep/<uuid>
//   [WARNING] reconnect_loop: connect_whep failed: "WHEP POST returned 404"
// — when a configured NDI source was activated but not currently producing a
// pipeline. The server now returns 204 No Content for that
// configured-but-not-producing state (POST handler in ndi_whep.rs), and the
// stage client treats the 204 as a non-error "not producing yet" outcome that
// schedules a backed-off retry WITHOUT logging anything. Net effect: ZERO
// browser console errors for a configured-but-not-producing source.
//
// This test runs on hosts WITHOUT a live NDI sender (CI runners and the dev2
// box during a normal autopilot run). The source is configured + activated but
// no real NDI source feeds it, so the WHEP POST exercises exactly the
// not-producing path. The hard requirement asserted here is the #431
// regression guard: the stage console must NEVER show a /ndi/whep 404 (nor the
// "WHEP POST returned 404" warning). 503 (no libndi / pipeline can't start) is
// the only acceptable transient; 404 is the bug.

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

test("WHEP POST never returns 404 for a configured-but-not-producing source (#431)", async ({
  request,
}) => {
  // Create + activate a source that has no real NDI sender behind it.
  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "TEST-NOT-PRODUCING", ndiName: "NO-SUCH-NDI-SOURCE (none)" } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();
  await request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );

  const offer = "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n";
  const whep = await request.post(new URL(`/ndi/whep/${src.id}`, baseURL).toString(), {
    data: offer,
    headers: { "Content-Type": "application/sdp" },
  });
  // 204 (configured-but-not-producing, libndi present), 201 (a real sender
  // appeared and produced an answer), or 503 (no libndi / pipeline can't
  // start) are all acceptable. 404 is the #431 regression and is BANNED.
  expect(
    [201, 204, 503],
    `WHEP POST must never be 404 for a configured source (#431), got ${whep.status()}`,
  ).toContain(whep.status());
  expect(whep.status()).not.toBe(404);
});

test("stage with a configured-but-not-producing NDI source has zero console errors (#431)", async ({
  page,
}) => {
  // The ONLY console noise tolerated is a transient 503 (no libndi / pipeline
  // can't start on this host). A /ndi/whep 404 — the #431 bug — is NOT
  // tolerated, nor is any other error/warning.
  const ALLOWED = [
    /Failed to load resource.*503/i,
    /WHEP POST returned 503/i,
    /WHEP connect for.*failed/i,
  ];
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!ALLOWED.some((re) => re.test(text))) {
        consoleMessages.push(`[${msg.type()}] ${text}`);
      }
    }
  });
  page.on("pageerror", (err) => {
    consoleMessages.push(`[pageerror] ${err.message}`);
  });

  // Also fail loudly if a /ndi/whep request ever returns 404 on the wire —
  // independent of how the browser surfaces it to the console.
  const whep404s: string[] = [];
  page.on("response", (resp) => {
    if (resp.url().includes("/ndi/whep/") && resp.status() === 404) {
      whep404s.push(`${resp.request().method()} ${resp.url()} -> 404`);
    }
  });

  const created = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "TEST-NOT-PRODUCING", ndiName: "NO-SUCH-NDI-SOURCE (none)" } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();
  await page.request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );
  await page.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', { timeout: 10_000 });

  // The NdiVideo element renders for the configured source (placeholder, no
  // srcObject — nothing is producing).
  const videoEl = page.locator('[data-role="ndi-video"]');
  await expect(videoEl).toHaveCount(1);
  await expect(videoEl).toHaveAttribute("data-source-id", src.id);

  // Give the reconnect loop a couple of cycles to POST WHEP at least once
  // (and, pre-fix, to spam 404s).
  await page.waitForTimeout(4_000);

  // #431 hard requirement: not a single /ndi/whep 404, and a clean console.
  expect(whep404s, `/ndi/whep must never 404 for a configured source (#431)`).toEqual([]);
  expect(consoleMessages).toEqual([]);
});
