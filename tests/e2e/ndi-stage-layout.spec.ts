import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

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

test("ndi-fullscreen appears in stage displays list", async ({ request }) => {
  const resp = await request.get(
    new URL("/stage-displays", baseURL).toString(),
  );
  expect(resp.status()).toBe(200);
  const layouts = await resp.json();
  const ndi = layouts.find((l: any) => l.code === "ndi-fullscreen");
  expect(ndi).toBeDefined();
  expect(ndi.name).toBe("NDI FULLSCREEN");
});

test("stage page renders ndi-fullscreen layout", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Set layout to ndi-fullscreen
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "ndi-fullscreen" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
    timeout: 10_000,
  });

  // Verify placeholder is shown (no active source in test env)
  const placeholder = page.locator(".stage-ndi__placeholder");
  await expect(placeholder).toBeVisible();
  await expect(placeholder).toContainText("No video source");

  expect(consoleMessages).toEqual([]);
});

test("uses native MJPEG img for NDI rendering", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Need an active source for the img to appear (it's inside Show when=ndi_active)
  const statusResp = await page.request.get(
    new URL("/ndi/status", baseURL).toString(),
  );
  const { available } = await statusResp.json();
  test.skip(!available, "NDI SDK not available");

  // Wait for finder
  await new Promise((r) => setTimeout(r, 6000));
  const sourcesResp = await page.request.get(
    new URL("/ndi/sources", baseURL).toString(),
  );
  const sources = await sourcesResp.json();
  test.skip(sources.length === 0, "No NDI sources on network");

  const createResp = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "E2E MJPEG", ndiName: sources[0].name } },
  );
  const source = await createResp.json();
  await page.request.post(
    new URL(
      `/integrations/video-sources/${source.id}/activate`,
      baseURL,
    ).toString(),
  );

  await page.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
    timeout: 10_000,
  });

  // Verify img element with /ndi/mjpeg src
  const img = page.locator("img.stage-ndi__video");
  await expect(img).toBeAttached();
  const src = await img.getAttribute("src");
  expect(src).toBe("/ndi/mjpeg");

  expect(
    consoleMessages.filter((m) => !m.includes("favicon")),
  ).toEqual([]);

  // Cleanup
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await page.request.delete(
    new URL(
      `/integrations/video-sources/${source.id}`,
      baseURL,
    ).toString(),
  );
});

test("frame delivery is smooth (requires NDI source)", async ({
  page,
  request,
}) => {
  const statusResp = await request.get(
    new URL("/ndi/status", baseURL).toString(),
  );
  const { available } = await statusResp.json();
  test.skip(!available, "NDI SDK not available");

  // Wait for finder to discover sources
  await new Promise((r) => setTimeout(r, 6000));
  const sourcesResp = await request.get(
    new URL("/ndi/sources", baseURL).toString(),
  );
  const sources = await sourcesResp.json();
  test.skip(sources.length === 0, "No NDI sources on network");

  // Create and activate a video source
  const createResp = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "E2E Test", ndiName: sources[0].name } },
  );
  const source = await createResp.json();
  await request.post(
    new URL(
      `/integrations/video-sources/${source.id}/activate`,
      baseURL,
    ).toString(),
  );

  // Set layout and navigate
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });
  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Measure frame delivery via WebSocket (server-side quality)
  const metrics = await page.evaluate((url: string) => {
    return new Promise<{
      frames: number;
      fps: string;
      maxIntervalMs: string;
      stutters: number;
    }>((resolve) => {
      const wsUrl = url.replace("http", "ws") + "/ndi/stream";
      const ws = new WebSocket(wsUrl);
      ws.binaryType = "arraybuffer";

      let frameCount = 0;
      let firstTime: number | null = null;
      let lastTime: number | null = null;
      const intervals: number[] = [];

      ws.onmessage = () => {
        const now = performance.now();
        frameCount++;
        if (firstTime === null) {
          firstTime = now;
        } else {
          intervals.push(now - lastTime!);
        }
        lastTime = now;
      };

      setTimeout(() => {
        ws.close();
        if (frameCount < 2) {
          return resolve({
            frames: frameCount,
            fps: "0",
            maxIntervalMs: "0",
            stutters: 0,
          });
        }
        const elapsed = (lastTime! - firstTime!) / 1000;
        const fps = frameCount / elapsed;
        const avgInterval =
          intervals.reduce((a, b) => a + b, 0) / intervals.length;
        const maxInterval = Math.max(...intervals);
        const stutters = intervals.filter((i) => i > avgInterval * 2).length;
        resolve({
          frames: frameCount,
          fps: fps.toFixed(1),
          maxIntervalMs: maxInterval.toFixed(1),
          stutters,
        });
      }, 5000);
    });
  }, baseURL);

  // Assertions
  expect(metrics.frames).toBeGreaterThan(50);
  expect(parseFloat(metrics.fps)).toBeGreaterThan(20);
  expect(metrics.stutters).toBeLessThanOrEqual(2);

  // Cleanup
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await request.delete(
    new URL(
      `/integrations/video-sources/${source.id}`,
      baseURL,
    ).toString(),
  );
});
