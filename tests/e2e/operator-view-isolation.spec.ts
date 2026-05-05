import { test, expect, type Page } from "@playwright/test";
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

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

function collectConsoleMessages(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    if (msg.text().includes("crbug.com/981419")) return;
    messages.push(`[${msg.type()}] ${msg.text()}`);
  });
  return messages;
}

async function openOperator(page: Page, viewPath: string): Promise<void> {
  await page.goto(new URL(`/ui/operator${viewPath}`, baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
}

test("worship view shows worship UI", async ({ page }) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "");

  await expect(page.locator('[data-role="worship-preview"]')).toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("bible view hides worship UI, shows bible panel", async ({ page }) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "/bible");

  await expect(page.locator('[data-role="worship-preview"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="bible"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("timers view hides worship UI, shows timers panel", async ({ page }) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "/timers");

  await expect(page.locator('[data-role="worship-preview"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="timers"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("ai view hides worship UI, shows ai panel", async ({ page }) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "/ai");

  await expect(page.locator('[data-role="worship-preview"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="ai"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("settings view hides worship UI, shows settings panel", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "/settings");

  await expect(page.locator('[data-role="worship-preview"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="settings"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});
