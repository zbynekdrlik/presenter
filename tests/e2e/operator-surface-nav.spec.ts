/**
 * Operator surface links E2E (#326, #875 H1).
 *
 * Asserts the links to the other surfaces are the items of the desktop
 * header's Obrazovky menu (including on the bible internal view), with Slovak
 * labels, are absent on tablet and camera, and open in a new tab
 * (target=_blank rel=noopener).
 *
 * Also asserts zero browser console errors/warnings per
 * ci/browser-console-zero-errors.md.
 */

import { test, expect, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  headerMenu,
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

// Known-benign Chromium warnings to ignore. These are browser-side
// informational notes about W3C-spec attributes Chromium does not yet act on;
// they fire on every WASM-loaded page across the codebase and are not project
// bugs. Filter narrowly so real warnings still surface.
const BENIGN_WARNING_PATTERNS: RegExp[] = [
  // crbug.com/981419: Chromium logs this every time it sees `integrity` on a
  // `<link rel=preload>`. Trunk emits SRI on the WASM preload tag (valid per
  // the SRI spec) but Chromium ignores it and warns. Anchor on the crbug URL
  // (more stable than the human-readable wording) to match the filter
  // pattern already used in `tests/e2e/api-stage.spec.ts`.
  /crbug\.com\/981419/i,
];

function isBenignWarning(text: string): boolean {
  return BENIGN_WARNING_PATTERNS.some((re) => re.test(text));
}

function collectConsole(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    const type = msg.type();
    if (type === "error" || type === "warning") {
      const text = msg.text();
      if (type === "warning" && isBenignWarning(text)) {
        return;
      }
      messages.push(`[${type}] ${text}`);
    }
  });
  page.on("pageerror", (err) => {
    messages.push(`[pageerror] ${err.message}`);
  });
  return messages;
}

const EXPECTED_TARGETS: ReadonlyArray<{ name: string; label: string; href: string }> = [
  { name: "Stage", label: "Pódium", href: "/stage" },
  { name: "Camera", label: "Kamera", href: "/ui/camera" },
  { name: "Tablet", label: "Tablet", href: "/ui/tablet" },
  { name: "Timer", label: "Časovač", href: "/stream/timer" },
  { name: "Stream", label: "Stream", href: "/ui/stream" },
];

/** Open the Obrazovky menu; its panel holds the surface links. */
async function openScreensMenu(page: Page) {
  const menu = headerMenu(page, "screens");
  await expect(menu.panel).toBeHidden();
  await menu.toggle.click();
  await expect(menu.toggle).toHaveAttribute("aria-expanded", "true");
  return menu.panel;
}

async function waitForOperatorReady(page: Page): Promise<void> {
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
}

test("the Obrazovky menu on /ui/operator lists the 5 surfaces with Slovak labels", async ({ page }) => {
  const consoleMessages = collectConsole(page);

  await page.goto(`${baseURL}/ui/operator`);
  await waitForOperatorReady(page);

  const panel = await openScreensMenu(page);
  const nav = panel.locator('[data-role="surface-nav"]');
  await expect(nav).toBeVisible();
  await expect(nav.locator('[data-role="surface-nav-link"]')).toHaveCount(EXPECTED_TARGETS.length);

  for (const target of EXPECTED_TARGETS) {
    const link = nav.locator(`[data-role="surface-nav-link"][data-target="${target.name}"]`);
    await expect(link, `link for ${target.name} should exist`).toHaveCount(1);
    await expect(link).toBeVisible();
    await expect(link).toContainText(target.label);
    await expect(link).toHaveAttribute("href", target.href);
    await expect(link).toHaveAttribute("target", "_blank");
    const rel = await link.getAttribute("rel");
    expect(rel ?? "", `rel for ${target.name} should contain noopener`).toContain("noopener");
  }

  // Picking a surface opens it in a new tab and closes the menu.
  const [tab] = await Promise.all([
    page.context().waitForEvent("page"),
    nav.locator('[data-role="surface-nav-link"][data-target="Tablet"]').click(),
  ]);
  await tab.waitForLoadState("domcontentloaded");
  expect(new URL(tab.url()).pathname).toBe("/ui/tablet");
  await tab.close();
  await expect(panel).toBeHidden();
  await expect(headerMenu(page, "screens").toggle).toHaveAttribute("aria-expanded", "false");
  expect(new URL(page.url()).pathname).toBe("/ui/operator");

  expect(consoleMessages, "browser console must be clean").toEqual([]);
});

test("the Obrazovky menu works on the bible internal view", async ({ page }) => {
  const consoleMessages = collectConsole(page);

  await page.goto(`${baseURL}/ui/operator/bible`);
  await waitForOperatorReady(page);

  const panel = await openScreensMenu(page);
  const nav = panel.locator('[data-role="surface-nav"]');
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
