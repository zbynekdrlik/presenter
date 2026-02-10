import { test, expect, Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 420_000 });

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let dbUrl: string;
let port: number;

async function waitForOperatorReady(page: Page) {
  await page.goto(new URL("/ui/operator", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForLoadState("networkidle");
  await page.waitForFunction(() => window.__presenterLiveConnected === true, {
    timeout: 30_000,
  });
}

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  dbUrl = config.dbUrl;
  port = config.port;
  await refreshDevData(dbUrl);
  serverHandle = await startTestServer(port, dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("drag search result to first position in playlist", async ({ page }) => {
  await waitForOperatorReady(page);

  // Create a playlist with one entry first
  const playlistName = `E2E Drag First ${Date.now()}`;
  await page.locator('[data-role="playlist-create"]').click();
  const playlistModal = page.locator('[data-role="playlist-edit-modal"]');
  await expect(playlistModal).toHaveAttribute("data-open", "true");
  await page.locator('[data-role="playlist-edit-name"]').fill(playlistName);
  await page.locator('[data-role="playlist-edit-save"]').click();
  await expect(playlistModal).toHaveAttribute("data-open", "false");

  const playlistButton = page.locator(
    '[data-role="playlist-item"][data-active="true"]',
  );
  await expect(playlistButton).toContainText(playlistName);
  const playlistId = await playlistButton.getAttribute("data-playlist-id");
  expect(playlistId).toBeTruthy();

  // Get library presentations to add to playlist
  const librariesResponse = await page.request.get(
    new URL("/libraries", baseURL).toString(),
    { timeout: 60_000 },
  );
  expect(librariesResponse.ok()).toBeTruthy();
  const libraries: Array<{
    id: string;
    name: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await librariesResponse.json();
  const source = libraries.find(
    (lib) => Array.isArray(lib.presentations) && lib.presentations.length >= 2,
  );
  if (!source) {
    throw new Error("Expected at least one library with 2+ presentations");
  }

  // Add two presentations to the playlist via test helpers
  for (const presentation of source.presentations.slice(0, 2)) {
    await page.evaluate(
      ({ playlistId: pid, presentationId }) => {
        const helpers = (window as any).__presenterOperatorTestHelpers;
        if (!helpers) throw new Error("test helpers unavailable");
        return helpers.addPresentationToPlaylist(presentationId, pid);
      },
      { playlistId: playlistId!, presentationId: presentation.id },
    );
  }

  // Wait for playlist to show 2 entries
  await expect
    .poll(async () =>
      page.evaluate((pid) => {
        const helpers = (window as any).__presenterOperatorTestHelpers;
        if (!helpers) return -1;
        return helpers.playlistPresentationCount(pid);
      }, playlistId!),
    )
    .toBe(2);

  // Switch to the playlist view to see items
  await playlistButton.click();
  const playlistItems = page.locator(
    '[data-role="presentation-list"] [data-role="presentation-item"][data-type="presentation"]',
  );
  await expect(playlistItems).toHaveCount(2, { timeout: 10_000 });

  // Record the names/IDs before the drag
  const firstItemIdBefore = await playlistItems
    .nth(0)
    .getAttribute("data-presentation-id");
  const secondItemIdBefore = await playlistItems
    .nth(1)
    .getAttribute("data-presentation-id");
  expect(firstItemIdBefore).toBeTruthy();
  expect(secondItemIdBefore).toBeTruthy();

  // Search for a third presentation to drag to position 0
  const thirdPresentation = source.presentations.find(
    (p) => p.id !== firstItemIdBefore && p.id !== secondItemIdBefore,
  );
  if (!thirdPresentation) {
    // If no third presentation available, use the first one (will duplicate in playlist)
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill(
      source.presentations[0].name.slice(
        0,
        Math.min(10, source.presentations[0].name.length),
      ),
    );
    const searchResult = page
      .locator('[data-role="search-result-item"][data-kind="presentation"]')
      .first();
    await expect(searchResult).toBeVisible({ timeout: 20_000 });

    // Drag to above the first item (to the top of the list)
    const firstItem = playlistItems.nth(0);
    await searchResult.dragTo(firstItem, {
      targetPosition: { x: 10, y: 2 },
    });
  } else {
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill(
      thirdPresentation.name.slice(
        0,
        Math.min(10, thirdPresentation.name.length),
      ),
    );
    const searchResult = page
      .locator('[data-role="search-result-item"][data-kind="presentation"]')
      .first();
    await expect(searchResult).toBeVisible({ timeout: 20_000 });

    // Drag to above the first item (target position near the top edge)
    const firstItem = playlistItems.nth(0);
    await searchResult.dragTo(firstItem, {
      targetPosition: { x: 10, y: 2 },
    });
  }

  // Verify 3 items now exist in the playlist
  await expect(playlistItems).toHaveCount(3, { timeout: 15_000 });

  // The previously first item should no longer be at position 0
  const firstItemIdAfter = await playlistItems
    .nth(0)
    .getAttribute("data-presentation-id");

  // The new item should be at position 0 (or at least the original first item
  // should have moved down)
  expect(firstItemIdAfter).not.toBe(firstItemIdBefore);
});

test("playlist name input receives focus when slide editor is active", async ({
  page,
}) => {
  await waitForOperatorReady(page);

  // Find a library with presentations
  const librariesResponse = await page.request.get(
    new URL("/libraries", baseURL).toString(),
    { timeout: 60_000 },
  );
  expect(librariesResponse.ok()).toBeTruthy();
  const libraries: Array<{
    id: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await librariesResponse.json();
  const source = libraries.find(
    (lib) => Array.isArray(lib.presentations) && lib.presentations.length > 0,
  );
  if (!source) {
    throw new Error("Expected at least one library with presentations");
  }

  // Select the library via the dashboard or modal
  const libraryButton = page.locator(
    `[data-role="library-list"] [data-role="library-item"][data-library-id="${source.id}"]`,
  );
  if (await libraryButton.count()) {
    await libraryButton.click();
  } else {
    await page.locator('[data-role="library-more"]').click();
    const modalButton = page.locator(
      `[data-role="library-modal-list"] [data-role="library-item"][data-library-id="${source.id}"]`,
    );
    await modalButton.click();
  }

  // Click on a presentation to load its slides
  const presentationItem = page
    .locator('[data-role="presentation-item"][data-type="presentation"]')
    .first();
  await presentationItem.click();

  // Switch to edit mode to get editable slide fields
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();

  // Wait for slides to render
  const slideContainer = page.locator('[data-role="slides"]');
  await expect(async () => {
    const count = await slideContainer.locator("[data-slide-id]").count();
    if (count === 0) throw new Error("no slides loaded");
  }).toPass({ timeout: 15_000, intervals: [200] });

  // Focus a slide text field (textarea)
  const slideTextarea = slideContainer
    .locator('textarea[data-field="main"]')
    .first();
  await slideTextarea.click();
  await expect(slideTextarea).toBeFocused();

  // Now click "+" to create a playlist — this should open the modal
  await page.locator('[data-role="playlist-create"]').click();
  const playlistModal = page.locator('[data-role="playlist-edit-modal"]');
  await expect(playlistModal).toHaveAttribute("data-open", "true");

  // The playlist name input should be focused and ready for typing
  const playlistNameInput = page.locator('[data-role="playlist-edit-name"]');
  await expect(playlistNameInput).toBeFocused({ timeout: 5_000 });

  // Type a name — this should go into the playlist input, NOT the slide field
  const testName = `Focus Test ${Date.now()}`;
  await playlistNameInput.fill(testName);
  await expect(playlistNameInput).toHaveValue(testName);

  // Verify the slide textarea did NOT receive the text
  const slideText = await slideTextarea.inputValue();
  expect(slideText).not.toContain("Focus Test");

  // Close the modal
  await page.locator('[data-role="playlist-edit-cancel"]').click();
  await expect(playlistModal).toHaveAttribute("data-open", "false");
});

test("delete presentation via edit modal with confirmation", async ({
  page,
}) => {
  await waitForOperatorReady(page);

  // Create a library and presentation specifically for deletion
  const libResp = await page.request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `E2E Delete Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string; name: string } = await libResp.json();

  const presResp = await page.request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Delete Me Presentation" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; name: string };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;

  // Reload the operator to pick up new data
  await waitForOperatorReady(page);

  // Navigate to the library containing our presentation
  const libraryButton = page.locator(
    `[data-role="library-list"] [data-role="library-item"][data-library-id="${library.id}"]`,
  );
  if (!(await libraryButton.count())) {
    // Library might not be favorited; open the library modal to find it
    await page.locator('[data-role="library-more"]').click();
    const modalButton = page.locator(
      `[data-role="library-modal-list"] [data-role="library-item"][data-library-id="${library.id}"]`,
    );
    await modalButton.click();
  } else {
    await libraryButton.click();
  }

  // Switch to edit mode so rename buttons are visible
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();

  // Find the presentation item and click the rename (pen) icon
  const presentationItem = page.locator(
    `[data-role="presentation-item"][data-presentation-id="${presentationId}"]`,
  );
  await expect(presentationItem).toBeVisible({ timeout: 10_000 });

  const renameButton = presentationItem.locator(
    '[data-action="presentation-rename"]',
  );
  await renameButton.click();

  // Verify the edit modal opens
  const editModal = page.locator('[data-role="presentation-edit-modal"]');
  await expect(editModal).toHaveAttribute("data-open", "true");

  // Verify the delete button is visible (not hidden for separator mode)
  const deleteButton = page.locator('[data-role="presentation-edit-delete"]');
  await expect(deleteButton).toBeVisible();

  // Click delete and accept the confirmation dialog
  page.once("dialog", async (dialog) => {
    expect(dialog.type()).toBe("confirm");
    expect(dialog.message()).toContain("Delete presentation");
    expect(dialog.message()).toContain("Delete Me Presentation");
    await dialog.accept();
  });
  await deleteButton.click();

  // Modal should close
  await expect(editModal).toHaveAttribute("data-open", "false");

  // Presentation should no longer be in the list
  await expect(presentationItem).toHaveCount(0, { timeout: 10_000 });

  // Verify via API that presentation is actually deleted
  const detailResp = await page.request.get(
    new URL(`/presentations/${presentationId}`, baseURL).toString(),
  );
  expect(detailResp.status()).toBe(404);

  // Verify a success toast appeared
  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toHaveAttribute("data-visible", "true");
  await expect(toast).toContainText(/deleted/i);
});

test("delete presentation dismiss confirmation keeps presentation", async ({
  page,
}) => {
  await waitForOperatorReady(page);

  // Create a library and presentation
  const libResp = await page.request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `E2E Keep Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string; name: string } = await libResp.json();

  const presResp = await page.request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Keep Me Presentation" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; name: string };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;

  await waitForOperatorReady(page);

  // Navigate to library
  const libraryButton = page.locator(
    `[data-role="library-list"] [data-role="library-item"][data-library-id="${library.id}"]`,
  );
  if (!(await libraryButton.count())) {
    await page.locator('[data-role="library-more"]').click();
    const modalButton = page.locator(
      `[data-role="library-modal-list"] [data-role="library-item"][data-library-id="${library.id}"]`,
    );
    await modalButton.click();
  } else {
    await libraryButton.click();
  }

  // Switch to edit mode so rename buttons are visible
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();

  const presentationItem = page.locator(
    `[data-role="presentation-item"][data-presentation-id="${presentationId}"]`,
  );
  await expect(presentationItem).toBeVisible({ timeout: 10_000 });

  const renameButton = presentationItem.locator(
    '[data-action="presentation-rename"]',
  );
  await renameButton.click();

  const editModal = page.locator('[data-role="presentation-edit-modal"]');
  await expect(editModal).toHaveAttribute("data-open", "true");

  // Click delete but DISMISS the confirmation
  page.once("dialog", async (dialog) => {
    expect(dialog.type()).toBe("confirm");
    await dialog.dismiss();
  });
  await page.locator('[data-role="presentation-edit-delete"]').click();

  // Modal should still be open (dismiss doesn't close it)
  await expect(editModal).toHaveAttribute("data-open", "true");

  // Close modal manually
  await page.locator('[data-role="presentation-edit-cancel"]').click();
  await expect(editModal).toHaveAttribute("data-open", "false");

  // Presentation should still exist
  await expect(presentationItem).toBeVisible();

  // Verify via API that presentation still exists
  const detailResp = await page.request.get(
    new URL(`/presentations/${presentationId}`, baseURL).toString(),
  );
  expect(detailResp.ok()).toBeTruthy();
});
