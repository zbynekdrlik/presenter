/**
 * #700 — Bible tab header preview.
 *
 * The live stage-display preview iframe (`/stage?preview=1`, embedded on every
 * operator view by #460) must be shown ONLY on the worship tab. On the Bible
 * tab the header must go back to the pre-#460 VERSE preview
 * (`[data-role="bible-preview"]`, reading the active bible broadcast).
 *
 * RED before the fix: current code renders the stage iframe unconditionally on
 * every view and has no bible verse-preview element, so the "iframe absent on
 * bible" + "verse preview present on bible" assertions fail.
 */

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

test("worship tab: stage preview iframe present, verse preview absent", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "");

  // Stage-display mirror iframe is shown on worship.
  await expect(page.locator('[data-role="stage-preview-frame"]')).toBeVisible();
  // The pre-#460 bible verse preview is NOT shown on worship.
  await expect(page.locator('[data-role="bible-preview"]')).not.toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("bible tab: stage preview iframe ABSENT, verse preview present", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "/bible");

  // #700: the stage-display mirror iframe must NOT be rendered on the bible tab.
  await expect(page.locator('[data-role="stage-preview-frame"]')).toHaveCount(0);
  // The pre-#460 verse preview is shown instead.
  await expect(page.locator('[data-role="bible-preview"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("switching worship <-> bible toggles the header preview reactively", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "");

  const iframe = page.locator('[data-role="stage-preview-frame"]');
  const versePreview = page.locator('[data-role="bible-preview"]');

  await expect(iframe).toBeVisible();
  // Gate on the embedded /stage app fully connecting BEFORE we unmount it: a
  // cleanly-connected stage WS closes silently on unmount, whereas tearing the
  // iframe down mid-connect can surface aborted-subresource / WS-close noise in
  // the parent console (this spec asserts the console is empty, and per project
  // rule the parent listener also captures the iframe's console). Mirrors the
  // readiness gate the sibling iframe specs (wasm-stage / wasm-bible / api-stage)
  // use before reading the embedded stage.
  await expect(
    page
      .frameLocator("iframe.operator__stage-iframe")
      .locator(".stage-container"),
  ).toBeVisible({ timeout: 30_000 });

  // Switch to Bible (the real-user action from the bug report).
  await page.locator('[data-role="view-toggle"][data-view="bible"]').click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-view") === "bible",
    { timeout: 5_000 },
  );
  await expect(iframe).toHaveCount(0);
  await expect(versePreview).toBeVisible();

  // Switch back to Worship — iframe returns, verse preview gone.
  await page.locator('[data-role="view-toggle"][data-view="worship"]').click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-view") === "worship",
    { timeout: 5_000 },
  );
  await expect(iframe).toBeVisible();
  await expect(versePreview).not.toBeVisible();

  expect(consoleMessages).toEqual([]);
});
