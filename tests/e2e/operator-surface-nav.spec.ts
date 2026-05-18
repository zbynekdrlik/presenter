/**
 * Operator Surface-Nav Strip E2E (#326).
 *
 * Asserts the 4-pill jump-link row appears on operator chrome
 * (including the bible internal view), is absent on tablet and camera,
 * and links open in a new tab (target=_blank rel=noopener).
 *
 * Also asserts zero browser console errors/warnings per
 * ci/browser-console-zero-errors.md.
 */

import { test, expect, type Page } from "@playwright/test";
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

function collectConsole(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    const type = msg.type();
    if (type === "error" || type === "warning") {
      messages.push(`[${type}] ${msg.text()}`);
    }
  });
  page.on("pageerror", (err) => {
    messages.push(`[pageerror] ${err.message}`);
  });
  return messages;
}

const EXPECTED_TARGETS: ReadonlyArray<{ name: string; href: string }> = [
  { name: "Stage", href: "/stage" },
  { name: "Camera", href: "/ui/camera" },
  { name: "Tablet", href: "/ui/tablet" },
  { name: "Timer", href: "/overlays/timer" },
];

async function waitForOperatorReady(page: Page): Promise<void> {
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
}

test("surface-nav strip is visible on /ui/operator with 4 correct anchors", async ({ page }) => {
  const consoleMessages = collectConsole(page);

  await page.goto(`${baseURL}/ui/operator`);
  await waitForOperatorReady(page);

  const nav = page.locator('[data-role="surface-nav"]');
  await expect(nav).toBeVisible();

  for (const target of EXPECTED_TARGETS) {
    const link = nav.locator(`[data-role="surface-nav-link"][data-target="${target.name}"]`);
    await expect(link, `link for ${target.name} should exist`).toHaveCount(1);
    await expect(link).toHaveAttribute("href", target.href);
    await expect(link).toHaveAttribute("target", "_blank");
    const rel = await link.getAttribute("rel");
    expect(rel ?? "", `rel for ${target.name} should contain noopener`).toContain("noopener");
  }

  expect(consoleMessages, "browser console must be clean").toEqual([]);
});

test("surface-nav strip is visible on the bible internal view", async ({ page }) => {
  const consoleMessages = collectConsole(page);

  await page.goto(`${baseURL}/ui/operator/bible`);
  await waitForOperatorReady(page);

  const nav = page.locator('[data-role="surface-nav"]');
  await expect(nav).toBeVisible();

  for (const target of EXPECTED_TARGETS) {
    await expect(
      nav.locator(`[data-role="surface-nav-link"][data-target="${target.name}"]`),
    ).toHaveCount(1);
  }

  expect(consoleMessages, "browser console must be clean").toEqual([]);
});

test("surface-nav strip is absent on /ui/tablet and /ui/camera", async ({ page }) => {
  const consoleMessages = collectConsole(page);

  await page.goto(`${baseURL}/ui/tablet`);
  await waitForOperatorReady(page);
  await expect(page.locator('[data-role="surface-nav"]')).toHaveCount(0);

  await page.goto(`${baseURL}/ui/camera`);
  await waitForOperatorReady(page);
  await expect(page.locator('[data-role="surface-nav"]')).toHaveCount(0);

  expect(consoleMessages, "browser console must be clean").toEqual([]);
});
