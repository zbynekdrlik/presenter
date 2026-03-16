/**
 * WASM Operator Presentation CRUD Tests
 *
 * Tests presentation creation, editing, and deletion in the WASM operator.
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

async function initPage(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="library-item"]', { timeout: 30_000 });
}

async function selectLibrary(page: import("@playwright/test").Page) {
  await initPage(page);
  await page.locator('[data-role="library-item"]').first().click();
  await page.waitForSelector('[data-role="presentation-list"]', {
    timeout: 15_000,
  });
}

test.describe("WASM Operator Presentation CRUD", () => {
  test("create blank presentation", async ({ page }) => {
    await selectLibrary(page);

    // Open create modal
    await page.locator('[data-role="presentation-create"]').click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Fill in name
    const nameInput = page.locator('[data-role="presentation-create-name"]');
    await nameInput.fill("E2E Test Presentation");

    // Click blank option
    await page.locator('[data-role="presentation-create-blank"]').click();

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Toast should show success
    await expect(page.locator('[data-role="toast"]')).toContainText(
      /created|success/i,
      { timeout: 5_000 },
    );
  });

  test("create presentation from paste", async ({ page }) => {
    await selectLibrary(page);

    // Open create modal
    await page.locator('[data-role="presentation-create"]').click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Fill in name
    await page
      .locator('[data-role="presentation-create-name"]')
      .fill("Pasted Presentation");

    // Click paste option
    await page.locator('[data-role="presentation-create-paste"]').click();

    // Wait for paste area
    const pasteArea = page.locator(
      '[data-role="presentation-create-paste-area"]',
    );
    await expect(pasteArea).toBeVisible();

    // Fill in paste text with verse markers
    const pasteText = `Verse 1
Line one of verse
Line two of verse

Chorus
This is the chorus
Multiple lines here`;

    await page
      .locator('[data-role="presentation-create-paste-text"]')
      .fill(pasteText);

    // Click confirm
    await page
      .locator('[data-role="presentation-create-paste-confirm"]')
      .click();

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Toast should show success
    await expect(page.locator('[data-role="toast"]')).toContainText(
      /created|paste|success/i,
      { timeout: 5_000 },
    );
  });

  test("rename presentation", async ({ page }) => {
    await selectLibrary(page);

    // Wait for presentations to load
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });

    // Switch to edit mode (rename buttons only visible in edit mode)
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Find rename button (uses data-action)
    const renameButton = page
      .locator('[data-action="presentation-rename"]')
      .first();
    const renameCount = await renameButton.count();
    expect(renameCount, "No presentation rename button found").toBeGreaterThan(
      0,
    );
    if (renameCount === 0) return;

    await renameButton.click();

    // Wait for edit modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Get current name
    const nameInput = page.locator('[data-role="presentation-edit-name"]');
    const originalName = await nameInput.inputValue();

    // Change name
    await nameInput.fill(originalName + "_RENAMED");

    // Save
    await page.locator('[data-role="presentation-edit-save"]').click();

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Toast should show success
    await expect(page.locator('[data-role="toast"]')).toContainText(
      /renamed|saved|success/i,
      { timeout: 5_000 },
    );

    // Restore original name
    await renameButton.click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
    await nameInput.fill(originalName);
    await page.locator('[data-role="presentation-edit-save"]').click();
  });

  test("delete presentation with confirmation", async ({ page }) => {
    await selectLibrary(page);

    // First create a test presentation to delete
    await page.locator('[data-role="presentation-create"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    await page
      .locator('[data-role="presentation-create-name"]')
      .fill("To Be Deleted");
    await page.locator('[data-role="presentation-create-blank"]').click();

    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    await page.waitForTimeout(500);

    // Switch to edit mode (rename buttons only visible in edit mode)
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Find the rename button for the newly created presentation (to open edit modal)
    const renameButtons = page.locator('[data-action="presentation-rename"]');
    const renameCount = await renameButtons.count();
    expect(
      renameCount,
      "No rename buttons available for delete test",
    ).toBeGreaterThan(0);
    if (renameCount === 0) return;

    // Click the first one (most recently created is usually first)
    await renameButtons.first().click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Set up dialog handler
    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });

    // Click delete
    await page.locator('[data-role="presentation-edit-delete"]').click();

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Toast should show success
    await expect(page.locator('[data-role="toast"]')).toContainText(
      /deleted|success/i,
      { timeout: 5_000 },
    );
  });

  test("delete cancellation preserves presentation", async ({ page }) => {
    await selectLibrary(page);

    // Wait for presentations
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });

    // Switch to edit mode (rename buttons only visible in edit mode)
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    const renameButton = page
      .locator('[data-action="presentation-rename"]')
      .first();
    const renameCount = await renameButton.count();
    expect(
      renameCount,
      "No presentation found for cancel test",
    ).toBeGreaterThan(0);
    if (renameCount === 0) return;

    await renameButton.click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Get the presentation name
    const nameInput = page.locator('[data-role="presentation-edit-name"]');
    const originalName = await nameInput.inputValue();

    // Set up dialog handler to DISMISS
    page.once("dialog", async (dialog) => {
      await dialog.dismiss();
    });

    // Click delete
    await page.locator('[data-role="presentation-edit-delete"]').click();

    // Modal should remain open (dialog was dismissed)
    await page.waitForTimeout(500);
    const modalStillOpen = await page.evaluate(
      () =>
        !!document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
    );
    expect(modalStillOpen).toBe(true);

    // Close modal
    await page.keyboard.press("Escape");
  });

  test("presentation create modal: back navigation", async ({ page }) => {
    await selectLibrary(page);

    // Open create modal
    await page.locator('[data-role="presentation-create"]').click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Click paste
    await page.locator('[data-role="presentation-create-paste"]').click();

    // Paste area should be visible
    const pasteArea = page.locator(
      '[data-role="presentation-create-paste-area"]',
    );
    await expect(pasteArea).toBeVisible();

    // Click back
    await page.locator('[data-role="presentation-create-paste-back"]').click();

    // Options should be visible again
    const options = page.locator('[data-role="presentation-create-options"]');
    await expect(options).toBeVisible();

    // Click import
    await page.locator('[data-role="presentation-create-import"]').click();

    // Import area should be visible
    const importArea = page.locator(
      '[data-role="presentation-create-import-area"]',
    );
    await expect(importArea).toBeVisible();

    // Click back
    await page.locator('[data-role="presentation-create-import-back"]').click();

    // Options should be visible again
    await expect(options).toBeVisible();

    // Close modal
    await page.keyboard.press("Escape");
  });
});
