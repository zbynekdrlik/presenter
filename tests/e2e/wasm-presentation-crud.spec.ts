/**
 * WASM Operator Presentation CRUD Tests
 *
 * Tests creating, editing, and deleting presentations in the WASM operator.
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

async function selectLibrary(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui-next/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="library-item"]', { timeout: 30_000 });
  await page.locator('[data-role="library-item"]').first().click();
  await page.waitForSelector('[data-role="presentation-list"]', {
    timeout: 15_000,
  });
}

test.describe("WASM Operator Presentation CRUD", () => {
  test("create blank presentation", async ({ page }) => {
    await selectLibrary(page);

    const initialCount = await page
      .locator('[data-role="presentation-item"]')
      .count();

    // Click the create button
    const createButton = page.locator('[data-role="presentation-create"]');
    await createButton.click();

    // Wait for create modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Enter a name
    const nameInput = page.locator('[data-role="presentation-create-name"]');
    await nameInput.fill("E2E Test Presentation");

    // Click blank option
    const blankButton = page.locator('[data-role="presentation-create-blank"]');
    await blankButton.click();

    // Wait for modal to close and presentation to appear
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // New presentation should be selected
    await page.waitForTimeout(1000);
    const selectedPres = page.locator(
      '[data-role="presentation-item"][data-active="true"]',
    );
    await expect(selectedPres).toContainText("E2E Test Presentation");
  });

  test("create from paste with verse parsing", async ({ page }) => {
    await selectLibrary(page);

    // Click the create button
    const createButton = page.locator('[data-role="presentation-create"]');
    await createButton.click();

    // Wait for create modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Enter a name
    const nameInput = page.locator('[data-role="presentation-create-name"]');
    await nameInput.fill("E2E Paste Test");

    // Click paste option
    const pasteButton = page.locator('[data-role="presentation-create-paste"]');
    await pasteButton.click();

    // Wait for paste area
    await page.waitForSelector('[data-role="presentation-create-paste-area"]', {
      state: "visible",
    });

    // Enter song text with verse markers
    const pasteTextarea = page.locator(
      '[data-role="presentation-create-paste-text"]',
    );
    await pasteTextarea.fill(`Verse 1
This is the first verse
With multiple lines

Chorus
This is the chorus
With more lines

Verse 2
This is the second verse`);

    // Click create/confirm
    const confirmButton = page.locator(
      '[data-role="presentation-create-paste-confirm"]',
    );
    await confirmButton.click();

    // Wait for modal to close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Should have 3 slides (Verse 1, Chorus, Verse 2)
    await page.waitForFunction(
      () => document.querySelectorAll("[data-slide-id]").length === 3,
      { timeout: 10_000 },
    );

    const slideCount = await page.locator("[data-slide-id]").count();
    expect(slideCount).toBe(3);
  });

  test("rename presentation", async ({ page }) => {
    await selectLibrary(page);

    // Switch to edit mode to see edit buttons
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Click edit button on first presentation
    const editButton = page
      .locator(
        '[data-role="presentation-item"] [data-action="presentation-rename"]',
      )
      .first();
    await editButton.click();

    // Wait for edit modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Get current name and modify it
    const nameInput = page.locator('[data-role="presentation-edit-name"]');
    const originalName = await nameInput.inputValue();
    const newName = originalName + " RENAMED";

    await nameInput.fill(newName);

    // Click save
    const saveButton = page.locator('[data-role="presentation-edit-save"]');
    await saveButton.click();

    // Wait for modal to close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Verify name changed
    await page.waitForTimeout(500);
    const renamedPres = page
      .locator('[data-role="presentation-item"]')
      .filter({ hasText: newName });
    await expect(renamedPres.first()).toBeVisible();

    // Restore original name
    await page
      .locator(
        '[data-role="presentation-item"] [data-action="presentation-rename"]',
      )
      .first()
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
    await page
      .locator('[data-role="presentation-edit-name"]')
      .fill(originalName);
    await page.locator('[data-role="presentation-edit-save"]').click();
  });

  test("delete presentation with confirmation", async ({ page }) => {
    await selectLibrary(page);

    // First create a presentation to delete
    const createButton = page.locator('[data-role="presentation-create"]');
    await createButton.click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
    await page
      .locator('[data-role="presentation-create-name"]')
      .fill("TO_BE_DELETED");
    await page.locator('[data-role="presentation-create-blank"]').click();
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    const initialCount = await page
      .locator('[data-role="presentation-item"]')
      .count();

    // Click edit on the to-be-deleted presentation
    const toDelete = page
      .locator('[data-role="presentation-item"]')
      .filter({ hasText: "TO_BE_DELETED" });
    await toDelete.locator('[data-action="presentation-rename"]').click();

    // Wait for edit modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Set up dialog handler
    page.once("dialog", async (dialog) => {
      expect(dialog.type()).toBe("confirm");
      await dialog.accept();
    });

    // Click delete
    const deleteButton = page.locator('[data-role="presentation-edit-delete"]');
    await deleteButton.click();

    // Wait for modal to close and count to decrease
    await page.waitForFunction(
      (initial) =>
        document.querySelectorAll('[data-role="presentation-item"]').length <
        initial,
      initialCount,
      { timeout: 10_000 },
    );

    // Verify deleted
    await expect(toDelete).not.toBeVisible({ timeout: 2_000 });
  });

  test("delete cancellation preserves presentation", async ({ page }) => {
    await selectLibrary(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    const initialCount = await page
      .locator('[data-role="presentation-item"]')
      .count();

    // Click edit on first presentation
    const editButton = page
      .locator(
        '[data-role="presentation-item"] [data-action="presentation-rename"]',
      )
      .first();
    await editButton.click();

    // Wait for edit modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Set up dialog handler to cancel
    page.once("dialog", async (dialog) => {
      await dialog.dismiss();
    });

    // Click delete
    const deleteButton = page.locator('[data-role="presentation-edit-delete"]');
    await deleteButton.click();

    // Wait a bit and verify count unchanged
    await page.waitForTimeout(500);
    const newCount = await page
      .locator('[data-role="presentation-item"]')
      .count();
    expect(newCount).toBe(initialCount);
  });
});
