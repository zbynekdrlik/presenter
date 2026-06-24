import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #462: the operator embedded its Settings view via `<iframe src="/ui/settings">`.
// That iframe is NOT a SOTA approach and was the direct cause of both reported
// defects:
//   1. a DOUBLED header — the iframe is a separate document with its own
//      `settings__header`, stacked under the operator's own header chrome, and
//      the CSS meant to hide it (`body.in-iframe`) never matched because the
//      settings page overwrote `body.class` to "settings".
//   2. NO scroll — the iframe is a fixed-height box (~412px) clipping the
//      ~7900px settings page.
// Every OTHER operator view (Worship, Bible, Timers, AI) is a native Leptos
// panel. The fix makes Settings the same: a native `<SettingsPage embedded=true/>`
// panel — no iframe, single header, content scrollable within the panel.
//
// These tests guard the architecture decision: the iframe must never return,
// the settings content must live in the operator document, and it must scroll.

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(
    config.port,
    config.dbUrl,
    config.oscPort,
  );
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

async function gotoOperatorSettings(page: Page): Promise<void> {
  await page.goto(new URL("/ui/operator/settings", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await expect(page.locator(".operator__header")).toHaveCount(1);
}

test("operator Settings is a native panel, not an iframe, with a single header", async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);

  await gotoOperatorSettings(page);

  // No iframe anywhere — Settings is rendered natively like every other view.
  await expect(page.locator("iframe.operator__settings-frame")).toHaveCount(0);
  await expect(page.locator("iframe")).toHaveCount(0);

  // The settings content lives in the operator document (same DOM, reachable
  // by the top-level page), inside the settings panel.
  const settingsMain = page.locator(
    '[data-view-panel="settings"] .settings__main',
  );
  await expect(settingsMain).toBeVisible();
  await expect(
    page.locator('[data-view-panel="settings"] .settings__card').first(),
  ).toBeVisible();

  // Only the operator header chrome — the embedded settings panel must NOT
  // render its own "Presenter Settings / Back to hub" header.
  await expect(page.locator(".settings__header")).toHaveCount(0);

  // The embedded settings toast must NOT collide with the operator's own
  // [data-role="toast"] — when embedded it uses data-role="settings-toast".
  // Otherwise operator E2E selectors match two elements (strict-mode
  // violation), which is the regression this guards (#462).
  await expect(page.locator('[data-role="toast"]')).toHaveCount(1);

  // In create mode (nothing being edited) the Resolume "Cancel" button must not
  // render — guards the embedded regression where it relied on a body[data-mode]
  // CSS rule that never matched under body.operator, leaving Cancel always
  // visible and polluting the operator's data-mode (#462).
  await expect(
    page.locator('[data-view-panel="settings"] [data-role="host-reset"]'),
  ).toHaveCount(0);

  expect(errors).toEqual([]);
});

test("operator Settings content scrolls to the last card within the panel", async ({
  page,
}) => {
  await gotoOperatorSettings(page);

  const cards = page.locator('[data-view-panel="settings"] .settings__card');
  const count = await cards.count();
  expect(count).toBeGreaterThan(0);

  const lastCard = cards.nth(count - 1);
  // If the panel clips scrolling (the iframe bug), this never brings the last
  // card into view.
  await lastCard.scrollIntoViewIfNeeded();
  await expect(lastCard).toBeInViewport();
});

test("standalone /ui/settings keeps its own header and scrolls", async ({
  page,
}) => {
  await page.goto(new URL("/ui/settings", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Standalone page DOES show its own header (no operator chrome around it).
  await expect(page.locator(".settings__header")).toHaveCount(1);

  const cards = page.locator(".settings__main .settings__card");
  const count = await cards.count();
  expect(count).toBeGreaterThan(0);
  const lastCard = cards.nth(count - 1);
  await lastCard.scrollIntoViewIfNeeded();
  await expect(lastCard).toBeInViewport();
});
