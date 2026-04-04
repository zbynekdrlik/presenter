/**
 * Operator UI Control Buttons E2E Tests
 *
 * Tests that the Ableton ON/OFF and Follow ON/OFF buttons on the
 * operator UI actually work — clicking them toggles state, and the
 * UI reflects the server state correctly.
 *
 * These tests exist because the buttons were broken since the WASM
 * migration (camelCase serde mismatch) and no test caught it.
 */

import { test, expect } from "@playwright/test";
import {
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
});

/** Ensure Ableton settings exist in the DB so that enable/disable works. */
async function ensureAblesetSettings(
  request: import("@playwright/test").APIRequestContext,
  enabled: boolean,
) {
  const resp = await request.put(`${baseURL}/integrations/ableset/settings`, {
    data: {
      enabled,
      host: "test.lan",
      oscPort: 39051,
      httpPort: 80,
      libraryName: "TEST",
      songPrefixLength: 3,
    },
  });
  expect(resp.ok()).toBe(true);
}

/** Filter console messages, ignoring expected connection errors to fake host. */
function filterRealErrors(messages: string[]): string[] {
  return messages.filter(
    (m) =>
      !m.includes("test.lan") &&
      !m.includes("WebSocket") &&
      !m.includes("Failed to fetch") &&
      !m.includes("Failed to load resource") &&
      !m.includes("404"),
  );
}

test.describe("Operator Control Buttons", () => {
  test("Ableton ON/OFF button toggles state", async ({ page }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    // Enable Ableton via API so button starts as ON
    await ensureAblesetSettings(page.request, true);

    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    const abletonButton = page.locator('[data-role="ableset-enable"]');
    await expect(abletonButton).toBeVisible();

    // Button should show ON state (server has enabled=true, UI fetches on load)
    await expect(abletonButton).toHaveAttribute("data-state", "on", {
      timeout: 10_000,
    });
    await expect(abletonButton).toHaveText("Ableton ON");

    // Click to disable
    await abletonButton.click();
    await expect(abletonButton).toHaveAttribute("data-state", "off", {
      timeout: 5_000,
    });
    await expect(abletonButton).toHaveText("Ableton OFF");

    // Verify via API that server state changed
    const statusResp = await page.request.get(
      `${baseURL}/integrations/ableset/status`,
    );
    const status = await statusResp.json();
    expect(status.enabled).toBe(false);

    // Click to re-enable
    await abletonButton.click();
    await expect(abletonButton).toHaveAttribute("data-state", "on", {
      timeout: 5_000,
    });
    await expect(abletonButton).toHaveText("Ableton ON");

    // Verify via API
    const statusResp2 = await page.request.get(
      `${baseURL}/integrations/ableset/status`,
    );
    const status2 = await statusResp2.json();
    expect(status2.enabled).toBe(true);

    expect(filterRealErrors(consoleMessages)).toEqual([]);
  });

  test("Follow ON/OFF button toggles state", async ({ page }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    // Enable Ableton + Follow via API
    await ensureAblesetSettings(page.request, true);
    await page.request.post(`${baseURL}/integrations/ableset/follow`, {
      data: { enabled: true },
    });

    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    const followButton = page.locator('[data-role="ableset-follow"]');
    await expect(followButton).toBeVisible();

    // Button should show ON
    await expect(followButton).toHaveAttribute("data-state", "on", {
      timeout: 10_000,
    });
    await expect(followButton).toHaveText("Follow ON");

    // Click to disable
    await followButton.click();
    await expect(followButton).toHaveAttribute("data-state", "off", {
      timeout: 5_000,
    });
    await expect(followButton).toHaveText("Follow OFF");

    // Verify via API
    const statusResp = await page.request.get(
      `${baseURL}/integrations/ableset/status`,
    );
    const status = await statusResp.json();
    expect(status.followEnabled).toBe(false);

    // Click to re-enable
    await followButton.click();
    await expect(followButton).toHaveAttribute("data-state", "on", {
      timeout: 5_000,
    });
    await expect(followButton).toHaveText("Follow ON");

    expect(filterRealErrors(consoleMessages)).toEqual([]);
  });

  test("Follow resets when Ableton is disabled", async ({ page }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    // Enable both
    await ensureAblesetSettings(page.request, true);
    await page.request.post(`${baseURL}/integrations/ableset/follow`, {
      data: { enabled: true },
    });

    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    const abletonButton = page.locator('[data-role="ableset-enable"]');
    const followButton = page.locator('[data-role="ableset-follow"]');

    // Both should be ON
    await expect(abletonButton).toHaveAttribute("data-state", "on", {
      timeout: 10_000,
    });
    await expect(followButton).toHaveAttribute("data-state", "on", {
      timeout: 10_000,
    });

    // Click Ableton OFF — Follow should also reset to OFF
    await abletonButton.click();
    await expect(abletonButton).toHaveAttribute("data-state", "off", {
      timeout: 5_000,
    });
    await expect(followButton).toHaveAttribute("data-state", "off", {
      timeout: 5_000,
    });

    expect(filterRealErrors(consoleMessages)).toEqual([]);
  });

  test("button state persists after page reload", async ({ page }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    // Set known state via API
    await ensureAblesetSettings(page.request, true);
    await page.request.post(`${baseURL}/integrations/ableset/follow`, {
      data: { enabled: true },
    });

    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    const abletonButton = page.locator('[data-role="ableset-enable"]');
    const followButton = page.locator('[data-role="ableset-follow"]');

    await expect(abletonButton).toHaveAttribute("data-state", "on", {
      timeout: 10_000,
    });
    await expect(followButton).toHaveAttribute("data-state", "on", {
      timeout: 10_000,
    });

    // Reload
    await page.reload();
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // State should persist (loaded from server on init)
    await expect(abletonButton).toHaveAttribute("data-state", "on", {
      timeout: 10_000,
    });
    await expect(followButton).toHaveAttribute("data-state", "on", {
      timeout: 10_000,
    });

    expect(filterRealErrors(consoleMessages)).toEqual([]);
  });
});
