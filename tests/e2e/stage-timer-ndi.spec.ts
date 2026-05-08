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

function collectConsoleErrors(
  page: import("@playwright/test").Page,
  extraAllowed: RegExp[] = [],
): string[] {
  const messages: string[] = [];
  const allowed = [...ALLOWED_CONSOLE_NOISE, ...extraAllowed];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!allowed.some((pattern) => pattern.test(text))) {
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

test("timer layout renders without NDI image when no source is active", async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page);

  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "timer" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="timer"]', {
    timeout: 10_000,
  });

  const wrapper = page.locator('div.stage-container[data-layout="timer"]');
  await expect(wrapper).toBeAttached();

  await expect(wrapper.locator(".stage-timer__display")).toBeAttached();
  await expect(wrapper.locator(".stage-timer__text")).toBeVisible();

  await expect(wrapper.locator("img.stage-timer__ndi")).toHaveCount(0);

  const textShadow = await wrapper
    .locator(".stage-timer__text")
    .evaluate((el) => window.getComputedStyle(el).textShadow);
  expect(textShadow).not.toBe("none");
  expect(textShadow).not.toBe("");

  expect(consoleMessages).toEqual([]);
});

test("timer layout renders NDI image when an NDI source is active", async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page, [
    /Failed to load resource.*503/i,
  ]);

  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "timer" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="timer"]', {
    timeout: 10_000,
  });

  const createResp = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "E2E Stage Timer NDI Test", ndiName: "BOGUS-FOR-TIMER-TEST" } },
  );
  const source = await createResp.json();

  try {
    await page.request.post(
      new URL(
        `/integrations/video-sources/${source.id}/activate`,
        baseURL,
      ).toString(),
      { failOnStatusCode: false },
    );

    const wrapper = page.locator('div.stage-container[data-layout="timer"]');
    await expect(wrapper.locator("img.stage-timer__ndi")).toBeVisible({
      timeout: 10_000,
    });

    await expect(wrapper.locator(".stage-timer__text")).toBeVisible();

    const zIndex = await wrapper
      .locator(".stage-timer__display")
      .evaluate((el) => window.getComputedStyle(el).zIndex);
    expect(Number(zIndex)).toBeGreaterThanOrEqual(2);
  } finally {
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
