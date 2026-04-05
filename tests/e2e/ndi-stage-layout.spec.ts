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

test("uses canvas element for NDI rendering", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

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

  // Verify canvas element exists (not img)
  const canvas = page.locator("canvas.stage-ndi__video");
  await expect(canvas).toBeAttached();

  // Verify no img element for video
  const img = page.locator("img.stage-ndi__video");
  await expect(img).not.toBeAttached();

  expect(
    consoleMessages.filter((m) => !m.includes("favicon")),
  ).toEqual([]);
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

  // Measure frame delivery in the browser via canvas pixel changes
  const metrics = await page.evaluate(() => {
    return new Promise<{
      frames: number;
      fps: string;
      maxIntervalMs: string;
      stutters: number;
    }>((resolve) => {
      const canvas = document.querySelector(
        "canvas.stage-ndi__video",
      ) as HTMLCanvasElement | null;
      if (!canvas)
        return resolve({
          frames: 0,
          fps: "0",
          maxIntervalMs: "0",
          stutters: 0,
        });

      let frameCount = 0;
      let firstTime: number | null = null;
      let lastTime: number | null = null;
      const intervals: number[] = [];

      // Poll canvas pixel data changes to detect new frames
      const checkInterval = 10;
      let lastPixelHash = "";
      const timer = setInterval(() => {
        try {
          const c = canvas.getContext("2d");
          if (!c || canvas.width === 0) return;
          const pixel = c.getImageData(
            Math.floor(canvas.width / 2),
            Math.floor(canvas.height / 2),
            1,
            1,
          ).data;
          const hash = `${pixel[0]},${pixel[1]},${pixel[2]}`;
          if (hash !== lastPixelHash && hash !== "0,0,0") {
            lastPixelHash = hash;
            const now = performance.now();
            frameCount++;
            if (firstTime === null) {
              firstTime = now;
            } else {
              intervals.push(now - lastTime!);
            }
            lastTime = now;
          }
        } catch {
          /* ignore */
        }
      }, checkInterval);

      setTimeout(() => {
        clearInterval(timer);
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
  });

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
