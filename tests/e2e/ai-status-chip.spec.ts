/**
 * E2E for #598: the operator-header AI connection indicator, mirroring the
 * Resolume chip (#564, `resolume-status-chip.spec.ts`).
 *
 * Unlike Resolume (many hosts, each proven by a real mock TCP server), there
 * is exactly one AI proxy on this server, and its real login state depends
 * on this box's CLIProxyAPI session — not something a test should depend
 * on. So the four required states are driven by intercepting `/ai/status`
 * itself via `page.route`, the same technique `operator-version-recovery.spec.ts`
 * already uses for `/healthz`.
 */

import { test, expect, type Page } from "@playwright/test";
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

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

async function mockAiStatus(
  page: Page,
  proxy: { running: boolean; binaryFound: boolean; claudeAuthenticated: boolean },
) {
  await page.route("**/ai/status", async (route) => {
    await route.fulfill({
      json: {
        connected: proxy.running && proxy.binaryFound && proxy.claudeAuthenticated,
        error: null,
        proxy: {
          running: proxy.running,
          port: 18787,
          apiUrl: "http://127.0.0.1:18787/v1",
          binaryFound: proxy.binaryFound,
          claudeAuthenticated: proxy.claudeAuthenticated,
        },
      },
    });
  });
}

test("mounted in the top brand row, never next to Stage Output", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { running: true, binaryFound: true, claudeAuthenticated: true });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  await expect(page.locator('[data-role="ai-status-chip"]')).toHaveCount(1);
  await expect(
    page.locator('.operator__header-brand [data-role="ai-status-chip"]'),
  ).toHaveCount(1);
  await expect(
    page.locator('.operator__header-right [data-role="ai-status-chip"]'),
  ).toHaveCount(0);

  expect(consoleMessages).toEqual([]);
});

test("all three signals healthy shows the connected state and links to the AI panel", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { running: true, binaryFound: true, claudeAuthenticated: true });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "ok", { timeout: 30_000 });
  await expect(chip).toHaveText("AI: pripojené");
  await expect(chip).toHaveAttribute("href", "/ui/operator/ai");
  await expect(chip).toHaveAttribute("title", /prihlásená/);

  expect(consoleMessages).toEqual([]);
});

test("not authenticated is reported distinctly from a down proxy or a missing binary", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { running: true, binaryFound: true, claudeAuthenticated: false });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "logged-out", { timeout: 30_000 });
  await expect(chip).toHaveText("AI: odhlásené");
  await expect(chip).toHaveAttribute("title", /nie je prihlásená/);

  expect(consoleMessages).toEqual([]);
});

test("proxy not running is reported distinctly", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { running: false, binaryFound: true, claudeAuthenticated: false });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "proxy-down", { timeout: 30_000 });
  await expect(chip).toHaveText("AI: proxy nebeží");

  expect(consoleMessages).toEqual([]);
});

test("missing binary is reported distinctly", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { running: false, binaryFound: false, claudeAuthenticated: false });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "missing-binary", { timeout: 30_000 });
  await expect(chip).toHaveText("AI: chýba binárka");

  expect(consoleMessages).toEqual([]);
});

test("clicking the chip navigates straight to the AI panel", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { running: true, binaryFound: true, claudeAuthenticated: false });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "logged-out", { timeout: 30_000 });
  await chip.click();
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  await expect(page).toHaveURL(/\/ui\/operator\/ai$/);
  const aiButton = page.locator('[data-role="view-toggle"][data-view="ai"]');
  await expect(aiButton).toHaveAttribute("data-active", "true");

  expect(consoleMessages).toEqual([]);
});

test("a failed poll shows the neutral checking state, never a false failure claim, with a clean console", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);

  await page.route("**/ai/status", async (route) => {
    await route.fulfill({ status: 500, body: "boom" });
  });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "checking", { timeout: 30_000 });
  await expect(chip).toHaveText("AI: kontrolujem…");

  expect(consoleMessages).toEqual([]);
});
