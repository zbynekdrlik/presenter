/**
 * E2E for #599: the AI panel's PRIMARY logged-out state.
 *
 * Before this, a dead Claude login only surfaced as a small line buried
 * inside the AI panel's collapsed settings drawer — nothing told the
 * operator to use the existing login flow at `/ui/operator/ai` (real
 * incident 2026-07-26: AI verses died right before an event, fixed via SSH
 * instead of the GUI).
 *
 * Mirrors `ai-status-chip.spec.ts`'s technique: the real login state depends
 * on this box's CLIProxyAPI session, so every scenario is driven by
 * intercepting `/ai/status` via `page.route`, never the live proxy.
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
  proxy: {
    claudeAuthenticated: boolean;
    tokenExpiresAt?: string | null;
  },
) {
  await page.route("**/ai/status", async (route) => {
    await route.fulfill({
      json: {
        connected: proxy.claudeAuthenticated,
        error: proxy.claudeAuthenticated
          ? null
          : "Claude not authenticated — run /ai/proxy/login to re-authorize",
        proxy: {
          running: true,
          port: 18787,
          apiUrl: "http://127.0.0.1:18787/v1",
          binaryFound: true,
          claudeAuthenticated: proxy.claudeAuthenticated,
          tokenExpiresAt: proxy.tokenExpiresAt ?? null,
        },
      },
    });
  });
}

async function gotoAiPanel(page: Page) {
  await page.goto(new URL("/ui/operator/ai", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForFunction(
    () => document.body.getAttribute("data-view") === "ai",
    { timeout: 5_000 },
  );
}

test("logged out shows the primary login banner with a visible CTA", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { claudeAuthenticated: false });

  await gotoAiPanel(page);

  const banner = page.locator('[data-role="ai-login-banner"]');
  await expect(banner).toHaveAttribute("data-visible", "true", { timeout: 30_000 });
  await expect(banner).toBeVisible();

  const cta = page.locator('[data-role="ai-login-cta"]');
  await expect(cta).toBeVisible();
  await expect(cta).toHaveText("Prihlásiť sa");

  expect(consoleMessages).toEqual([]);
});

test("logged in shows no login banner", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, {
    claudeAuthenticated: true,
    tokenExpiresAt: "2099-01-01T00:00:00Z",
  });

  await gotoAiPanel(page);

  const banner = page.locator('[data-role="ai-login-banner"]');
  // Always mounted (never conditionally rendered) — assert the attribute,
  // never element count/visibility, per the toast-component discipline.
  await expect(banner).toHaveAttribute("data-visible", "false", { timeout: 30_000 });
  await expect(banner).toBeHidden();

  expect(consoleMessages).toEqual([]);
});

test("a known token expiry shows how long the login stays valid while authenticated", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, {
    claudeAuthenticated: true,
    tokenExpiresAt: "2099-06-15T12:00:00Z",
  });

  await gotoAiPanel(page);

  const validity = page.locator('[data-role="ai-token-validity"]');
  await expect(validity).toHaveAttribute("data-visible", "true", { timeout: 30_000 });
  await expect(validity).toBeVisible();
  await expect(validity).toHaveText(/plat[íi] do/);
  await expect(validity).toHaveText(/2099/);

  expect(consoleMessages).toEqual([]);
});

test("an expired token is named in the logged-out banner's subtext", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, {
    claudeAuthenticated: false,
    tokenExpiresAt: "2020-03-10T08:00:00Z",
  });

  await gotoAiPanel(page);

  const banner = page.locator('[data-role="ai-login-banner"]');
  await expect(banner).toHaveAttribute("data-visible", "true", { timeout: 30_000 });

  const subtext = banner.locator(".ai-chat__login-banner-subtext");
  await expect(subtext).toHaveText(/vypr[šs]alo/);
  await expect(subtext).toHaveText(/2020/);

  // The validity note is the "still valid" state — must stay hidden while
  // logged out, even though we DO know an expiry.
  const validity = page.locator('[data-role="ai-token-validity"]');
  await expect(validity).toHaveAttribute("data-visible", "false");

  expect(consoleMessages).toEqual([]);
});
