/**
 * WASM Operator Modals Tests
 *
 * Tests modal interactions in the WASM operator.
 */

import { test, expect } from "@playwright/test";
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
});

async function initPage(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="library-item"]', { timeout: 30_000 });
}

test.describe("WASM Operator Modals", () => {
  test("library modal opens and closes", async ({ page }) => {
    await initPage(page);

    // Click more button
    const moreButton = page.locator('[data-role="library-more"]');
    await moreButton.click();

    // Modal should open
    await page.waitForFunction(
      () =>
        document.querySelector('[data-role="library-modal"][data-open="true"]'),
      { timeout: 5_000 },
    );

    // Press Escape to close
    await page.keyboard.press("Escape");

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="library-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
  });

  test("library modal: select library", async ({ page }) => {
    await initPage(page);

    // Open modal
    await page.locator('[data-role="library-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector('[data-role="library-modal"][data-open="true"]'),
      { timeout: 5_000 },
    );

    // Click a library in the modal (modal items are library-row)
    const modalLibrary = page
      .locator(
        '[data-role="library-modal"] [data-role="library-row"] .operator__list-button',
      )
      .first();
    const libCount = await modalLibrary.count();
    expect(libCount, "No libraries in modal").toBeGreaterThan(0);
    if (libCount > 0) {
      await modalLibrary.click();

      // Modal should close and library should be selected
      await page.waitForFunction(
        () =>
          !document.querySelector(
            '[data-role="library-modal"][data-open="true"]',
          ),
        { timeout: 5_000 },
      );
    }
  });

  test("library modal: favorite toggle", async ({ page }) => {
    await initPage(page);

    // Open library modal
    await page.locator('[data-role="library-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector('[data-role="library-modal"][data-open="true"]'),
      { timeout: 5_000 },
    );

    // Find a star button (uses data-action for toggle buttons)
    const starButton = page
      .locator(
        '[data-role="library-modal"] [data-action="library-toggle-favorite"]',
      )
      .first();
    const starCount = await starButton.count();
    expect(
      starCount,
      "No library favorite toggle button found",
    ).toBeGreaterThan(0);
    if (starCount > 0) {
      // Get current state via aria-pressed
      const wasFavorite =
        (await starButton.getAttribute("aria-pressed")) === "true";

      // Click to toggle
      await starButton.click();

      // Wait for state to change
      await page.waitForFunction(
        (wasFav) => {
          const star = document.querySelector(
            '[data-role="library-modal"] [data-action="library-toggle-favorite"]',
          );
          return star && star.getAttribute("aria-pressed") !== String(wasFav);
        },
        wasFavorite,
        { timeout: 5_000 },
      );
    }

    // Close modal
    await page.keyboard.press("Escape");
  });

  test("library edit modal: rename", async ({ page }) => {
    await initPage(page);

    // Click library edit button (uses data-action)
    const editButton = page.locator('[data-action="library-edit"]').first();
    const editButtonCount = await editButton.count();
    expect(
      editButtonCount,
      "No library edit button found for rename test",
    ).toBeGreaterThan(0);
    if (editButtonCount === 0) return;
    await editButton.click();

    // Wait for edit modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="library-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Verify modal has name input
    const nameInput = page.locator('[data-role="library-edit-name"]');
    await expect(nameInput).toBeVisible();

    // Close without saving
    await page.keyboard.press("Escape");
  });

  test("library create modal: create library end-to-end", async ({
    page,
  }) => {
    await initPage(page);

    const libraryName = `E2E Created Library ${Date.now()}`;

    // Click the sidebar "+ Create library" button (regression guard for #560:
    // this must open the CREATE modal, not the edit modal reused for renames)
    await page.locator('[data-role="library-create"]').click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="library-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // The modal must be in "create" mode: title says "Create Library" and
    // there is no "Delete library" button (nothing exists yet to delete).
    await expect(
      page.locator('[data-role="library-edit-modal"]'),
    ).toHaveAttribute("data-mode", "create");
    await expect(page.locator('[data-role="library-edit-title"]')).toHaveText(
      "Create Library",
    );
    await expect(
      page.locator('[data-role="library-edit-delete"]'),
    ).toBeHidden();

    // Fill in a name and save.
    await page.locator('[data-role="library-edit-name"]').fill(libraryName);
    await page.locator('[data-role="library-edit-save"]').click();

    // Modal should close on success (it never closes on the reported 404).
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="library-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Toast must show success, not "Error: HTTP 404: Not Found".
    await expect(page.locator('[data-role="toast"]')).toContainText(
      /saved|success/i,
      { timeout: 5_000 },
    );

    // Verify the library actually persisted server-side (not just a UI toast).
    const response = await fetch(`${baseURL}/libraries`);
    expect(response.ok).toBe(true);
    const libraries = (await response.json()) as Array<{
      id: string;
      name: string;
    }>;
    const created = libraries.find((lib) => lib.name === libraryName);
    expect(
      created,
      `Library "${libraryName}" not found via GET /libraries`,
    ).toBeTruthy();

    // Cleanup so later tests in this file see a stable library set.
    if (created) {
      await fetch(`${baseURL}/libraries/${created.id}`, { method: "DELETE" });
    }
  });

  // #641: library create is NOT idempotent (#571 added the re-entry guard
  // to the shared save handler, but only the presentation "create blank"
  // path ever got an E2E covering it). A double-click on "Save changes"
  // must be swallowed by the guard and create exactly ONE library — never
  // two.
  test("library create modal: double-click submits exactly once (#641)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    attachConsoleErrorCollector(page, consoleMessages);

    await initPage(page);

    const libraryName = `DblSubmitLib${Date.now()}`;

    await page.locator('[data-role="library-create"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="library-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    await page.locator('[data-role="library-edit-name"]').fill(libraryName);

    // The second click must be swallowed by the re-entry guard AND the
    // disabled button — never create a second library.
    await page.locator('[data-role="library-edit-save"]').dblclick();

    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="library-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    await expect(page.locator('[data-role="toast"]')).toContainText(
      /saved|success/i,
      { timeout: 5_000 },
    );

    // A double-submit race (guard removed) would create TWO libraries —
    // assert exactly one exists server-side, not just that the toast fired.
    const response = await fetch(`${baseURL}/libraries`);
    expect(response.ok).toBe(true);
    const libraries = (await response.json()) as Array<{
      id: string;
      name: string;
    }>;
    const created = libraries.filter((lib) => lib.name === libraryName);
    expect(created).toHaveLength(1);

    // Cleanup so later tests in this file see a stable library set.
    for (const lib of created) {
      await fetch(`${baseURL}/libraries/${lib.id}`, { method: "DELETE" });
    }

    expect(consoleMessages).toEqual([]);
  });

  test("playlist modal opens and closes", async ({ page }) => {
    await initPage(page);

    // Click playlist more button
    const moreButton = page.locator('[data-role="playlist-more"]');
    await moreButton.click();

    // Modal should open
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Press Escape to close
    await page.keyboard.press("Escape");

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
  });

  test("playlist edit modal: rename", async ({ page }) => {
    await initPage(page);

    // Find a playlist edit button (uses data-action)
    const editButton = page.locator('[data-action="playlist-edit"]').first();
    const playlistEditCount = await editButton.count();
    expect(
      playlistEditCount,
      "No playlist edit button found for rename test",
    ).toBeGreaterThan(0);
    if (playlistEditCount === 0) return;
    await editButton.click();

    // Wait for edit modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Get current name
    const nameInput = page.locator('[data-role="playlist-edit-name"]');
    const originalName = await nameInput.inputValue();

    // Modify name
    await nameInput.fill(originalName + "_TEST");

    // Save
    await page.locator('[data-role="playlist-edit-save"]').click();

    // Wait for modal to close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Restore original name
    await editButton.click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
    await nameInput.fill(originalName);
    await page.locator('[data-role="playlist-edit-save"]').click();
  });

  // #641: playlist create is NOT idempotent (same shared save handler /
  // #571 guard as the library modal above). A double-click on "Save
  // changes" must be swallowed by the guard and create exactly ONE
  // playlist — never two.
  test("playlist create modal: double-click submits exactly once (#641)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    attachConsoleErrorCollector(page, consoleMessages);

    await initPage(page);

    const playlistName = `DblSubmitPlaylist${Date.now()}`;

    await page.locator('[data-role="playlist-create"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    await page.locator('[data-role="playlist-edit-name"]').fill(playlistName);

    // The second click must be swallowed by the re-entry guard AND the
    // disabled button — never create a second playlist.
    await page.locator('[data-role="playlist-edit-save"]').dblclick();

    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    await expect(page.locator('[data-role="toast"]')).toContainText(
      /saved|success/i,
      { timeout: 5_000 },
    );

    // A double-submit race (guard removed) would create TWO playlists —
    // assert exactly one exists server-side.
    const response = await fetch(`${baseURL}/playlists`);
    expect(response.ok).toBe(true);
    const playlists = (await response.json()) as Array<{
      id: string;
      name: string;
    }>;
    const created = playlists.filter((pl) => pl.name === playlistName);
    expect(created).toHaveLength(1);

    // Cleanup so later tests in this file see a stable playlist set.
    for (const pl of created) {
      await fetch(`${baseURL}/playlists/${pl.id}`, { method: "DELETE" });
    }

    expect(consoleMessages).toEqual([]);
  });

  test("presentation create modal: navigate steps", async ({ page }) => {
    await initPage(page);

    // Select a library first
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-list"]', {
      timeout: 15_000,
    });

    // Open create modal
    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Verify options panel is visible
    const optionsPanel = page.locator(
      '[data-role="presentation-create-options"]',
    );
    await expect(optionsPanel).toBeVisible();

    // Click paste option
    await page.locator('[data-role="presentation-create-paste"]').click();

    // Paste area should be visible
    const pasteArea = page.locator(
      '[data-role="presentation-create-paste-area"]',
    );
    await expect(pasteArea).toBeVisible();

    // Click back
    await page.locator('[data-role="presentation-create-paste-back"]').click();

    // Options should be visible again
    await expect(optionsPanel).toBeVisible();

    // Click import option
    await page.locator('[data-role="presentation-create-import"]').click();

    // Import area should be visible
    const importArea = page.locator(
      '[data-role="presentation-create-import-area"]',
    );
    await expect(importArea).toBeVisible();

    // Close modal
    await page.keyboard.press("Escape");
  });

  test("all modals: escape closes", async ({ page }) => {
    await initPage(page);

    // Test library modal
    await page.locator('[data-role="library-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector('[data-role="library-modal"][data-open="true"]'),
      { timeout: 5_000 },
    );
    await page.keyboard.press("Escape");
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="library-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Test playlist modal
    await page.locator('[data-role="playlist-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
    await page.keyboard.press("Escape");
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
  });
});
