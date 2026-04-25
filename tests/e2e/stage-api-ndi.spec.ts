import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

const ALLOWED_CONSOLE_NOISE = [
  /integrity.*ignored.*preload/i,
  /ResizeObserver loop/i,
];

function collectConsoleErrors(page: import("@playwright/test").Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!ALLOWED_CONSOLE_NOISE.some((pattern) => pattern.test(text))) {
        messages.push(`[${msg.type()}] ${text}`);
      }
    }
  });
  return messages;
}

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

test("api layout renders ApiStage wrapper with no NDI source active", async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page);

  // Ensure no video source is active
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  // Switch stage to api layout
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "api" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="api"]', {
    timeout: 10_000,
  });

  // ApiStage wrapper must be in the DOM
  const wrapper = page.locator("div.stage-api");
  await expect(wrapper).toBeAttached();

  // No NDI image when no source is active
  const img = page.locator("img.stage-api__ndi");
  await expect(img).toHaveCount(0);

  // WorshipSnv content is nested inside the wrapper
  const slide = page.locator("div.stage-api .stage__current-slide");
  await expect(slide).toBeAttached();

  // Wrapper should be absolutely sized to viewport
  const wrapperStyle = await wrapper.evaluate((el) => {
    const cs = window.getComputedStyle(el);
    return {
      position: cs.position,
      width: cs.width,
      height: cs.height,
    };
  });
  expect(wrapperStyle.position).toBe("relative");

  // Slide text inside .stage-api must have a non-empty text-shadow
  const slideShadow = await page
    .locator("div.stage-api .stage__current-slide .stage__slide-text")
    .evaluate((el) => window.getComputedStyle(el).textShadow);
  expect(slideShadow).not.toBe("none");
  expect(slideShadow).not.toBe("");

  expect(consoleMessages).toEqual([]);
});

test("worship-snv layout is not affected by api stage changes", async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page);

  // Switch back to worship-snv
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "worship-snv" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="worship-snv"]', {
    timeout: 10_000,
  });

  // No api wrapper
  await expect(page.locator("div.stage-api")).toHaveCount(0);
  await expect(page.locator("img.stage-api__ndi")).toHaveCount(0);

  // Worship-snv slide text must NOT have a text-shadow (only api layout gets it)
  const slideShadow = await page
    .locator('div.stage-container[data-layout="worship-snv"] .stage__current-slide .stage__slide-text')
    .evaluate((el) => window.getComputedStyle(el).textShadow);
  expect(slideShadow).toBe("none");

  expect(consoleMessages).toEqual([]);
});

test("api layout shows connection-status overlay when NDI source activates", async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page);

  // Start clean
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  // Navigate FIRST so the WS is open before we activate
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "api" } },
  );
  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="api"]', {
    timeout: 10_000,
  });

  // Sanity: img not present yet
  await expect(page.locator("img.stage-api__ndi")).toHaveCount(0);

  // Create a bogus video source
  const createResp = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "E2E Stage API NDI Test", ndiName: "BOGUS_DOES_NOT_EXIST" } },
  );
  const source = await createResp.json();

  // Activate. The handler publishes NdiSourceActivated to the live hub
  // BEFORE attempting to start the stream, so even when start_stream fails
  // for a bogus name, the frontend still receives the event. We ignore
  // the HTTP status here and observe DOM effects instead.
  await page.request.post(
    new URL(
      `/integrations/video-sources/${source.id}/activate`,
      baseURL,
    ).toString(),
    { failOnStatusCode: false },
  );

  try {
    // The overlay should appear (status = "connecting") and the img should mount
    await expect(page.locator("img.stage-api__ndi")).toHaveCount(1, {
      timeout: 10_000,
    });
    await expect(page.locator("div.stage-api__overlay")).toBeVisible({
      timeout: 5_000,
    });
    await expect(page.locator("div.stage-api__overlay")).toContainText(
      /Connecting/i,
    );
  } finally {
    // Cleanup so subsequent test runs are clean
    await page.request.post(
      new URL("/integrations/video-sources/deactivate", baseURL).toString(),
      { failOnStatusCode: false },
    );
    await page.request.delete(
      new URL(
        `/integrations/video-sources/${source.id}`,
        baseURL,
      ).toString(),
      { failOnStatusCode: false },
    );
  }

  expect(consoleMessages).toEqual([]);
});
