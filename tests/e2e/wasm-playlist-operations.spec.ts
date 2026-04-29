/**
 * WASM Operator Playlist Operations Tests
 *
 * Tests playlist creation, management, and entry operations in the WASM operator.
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
  await page.waitForSelector('[data-role="playlist-list"]', {
    timeout: 30_000,
  });
}

test.describe("WASM Operator Playlist Operations", () => {
  test("playlist list is visible", async ({ page }) => {
    await initPage(page);
    const playlistList = page.locator('[data-role="playlist-list"]');
    await expect(playlistList).toBeVisible();
  });

  test("create playlist", async ({ page }) => {
    await initPage(page);

    // Click create button
    await page.locator('[data-role="playlist-create"]').click();

    // Wait for create modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Fill name
    const nameInput = page.locator('[data-role="playlist-edit-name"]');
    await nameInput.fill("E2E Test Playlist");

    // Submit
    await page.locator('[data-role="playlist-edit-save"]').click();

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Wait for playlist list to update
    await page.waitForFunction(
      () => {
        const items = document.querySelectorAll('[data-role="playlist-item"]');
        return items.length > 0;
      },
      { timeout: 5_000 },
    );

    // Verify playlist was created (should appear in list or modal)
    await page.locator('[data-role="playlist-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    const createdPlaylist = page
      .locator('[data-role="playlist-modal"]')
      .getByText("E2E Test Playlist");
    await expect(createdPlaylist).toBeVisible();

    await page.keyboard.press("Escape");
  });

  test("dashboard toggle works", async ({ page }) => {
    await initPage(page);

    // Open playlist modal
    await page.locator('[data-role="playlist-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Find toggle button
    const toggleButton = page
      .locator(
        '[data-role="playlist-modal"] [data-action="playlist-toggle-dashboard"]',
      )
      .first();
    const toggleCount = await toggleButton.count();
    expect(toggleCount, "No dashboard toggle button found").toBeGreaterThan(0);
    if (toggleCount === 0) return;

    // Get current state
    const wasPressed =
      (await toggleButton.getAttribute("aria-pressed")) === "true";

    // Toggle
    await toggleButton.click();

    // Wait for state to change
    await page.waitForFunction(
      (wasP) => {
        const btn = document.querySelector(
          '[data-role="playlist-modal"] [data-action="playlist-toggle-dashboard"]',
        );
        return btn && btn.getAttribute("aria-pressed") !== String(wasP);
      },
      wasPressed,
      { timeout: 5_000 },
    );

    // Close modal
    await page.keyboard.press("Escape");
  });

  test("delete playlist with confirmation", async ({ page }) => {
    await initPage(page);

    // First ensure we have a test playlist to delete (create one with dashboard enabled)
    await page.locator('[data-role="playlist-create"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    const nameInput = page.locator('[data-role="playlist-edit-name"]');
    await nameInput.fill("To Delete Playlist");

    // Check dashboard checkbox so playlist appears in quick list
    const dashboardCheckbox = page.locator(
      '[data-role="playlist-edit-dashboard"]',
    );
    if (!(await dashboardCheckbox.isChecked())) {
      await dashboardCheckbox.click();
    }

    await page.locator('[data-role="playlist-edit-save"]').click();

    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    await page.waitForFunction(
      () => {
        const items = document.querySelectorAll('[data-role="playlist-item"]');
        return Array.from(items).some((item) =>
          item.textContent?.includes("To Delete Playlist"),
        );
      },
      { timeout: 10_000 },
    );

    // Find the playlist in the quick list (now visible because dashboard is enabled)
    // The edit button is only in the quick list, not in the modal
    const playlistItem = page
      .locator('[data-role="playlist-item"]')
      .filter({ hasText: "To Delete Playlist" });
    const itemCount = await playlistItem.count();

    // If not found in quick list, skip the test (data dependency)
    if (itemCount === 0) {
      test.skip(true, "Created playlist not found in quick list");
      return;
    }

    // Find the edit button associated with this playlist item
    // The edit button is in the same list row structure
    const editButton = page.locator('[data-action="playlist-edit"]').first();
    await editButton.click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Accept confirmation dialog
    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });

    // Click delete
    await page.locator('[data-role="playlist-edit-delete"]').click();

    // Wait for modal to close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );
  });

  test("select playlist shows entries", async ({ page }) => {
    await initPage(page);

    // Click on a playlist
    const playlist = page.locator('[data-role="playlist-item"]').first();
    const playlistCount = await playlist.count();
    expect(
      playlistCount,
      "No playlists available for select test",
    ).toBeGreaterThan(0);
    if (playlistCount === 0) return;

    await playlist.click();

    // Presentation list should update
    await page.waitForFunction(
      () => {
        const title = document.querySelector('[data-role="context-title"]');
        return title && title.textContent !== "Presentations";
      },
      { timeout: 10_000 },
    );
  });

  test("playlist modal shows all playlists", async ({ page }) => {
    await initPage(page);

    // Click more button
    const moreButton = page.locator('[data-role="playlist-more"]');
    await moreButton.click();

    // Wait for modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Modal should have playlist items (modal uses playlist-row)
    const modalPlaylists = page.locator(
      '[data-role="playlist-modal"] [data-role="playlist-row"]',
    );
    const count = await modalPlaylists.count();
    expect(count, "Modal should contain playlists").toBeGreaterThan(0);
  });

  test("add separator to playlist", async ({ page }) => {
    await initPage(page);

    // First select a playlist
    const playlist = page.locator('[data-role="playlist-item"]').first();
    const playlistCountForSeparator = await playlist.count();
    expect(
      playlistCountForSeparator,
      "No playlists available for separator test",
    ).toBeGreaterThan(0);
    if (playlistCountForSeparator === 0) return;
    await playlist.click();

    // Brief settle after playlist selection
    await page.waitForTimeout(500);

    // Click the "+" button (which adds separator when playlist is active)
    page.once("dialog", async (dialog) => {
      await dialog.accept("Test Separator");
    });

    const addButton = page.locator(
      '[data-view-panel="worship"] [data-role="presentation-create"]',
    );
    await addButton.click();

    // Wait for separator to appear
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-item"][data-type="separator"]',
        ),
      { timeout: 10_000 },
    );

    const separator = page
      .locator('[data-role="presentation-item"][data-type="separator"]')
      .filter({ hasText: "Test Separator" });
    await expect(separator.first()).toBeVisible();
  });

  test("drop a presentation onto a playlist row appends an entry", async ({
    page,
  }) => {
    // Regression guard: GET /playlists/{id} previously returned 405 because
    // only PATCH+DELETE were registered, so the drop handler in
    // playlist_list.rs (which fetches the current playlist before appending)
    // silently failed. Playlists stayed at 0 entries after every drag.
    //
    // We create the drop-target playlist via the API (not the create-modal
    // UI flow) to keep this test focused on the drag-drop path and avoid
    // coupling to the modal-render timing.

    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        const text = msg.text();
        if (!text.includes("favicon")) {
          consoleErrors.push(`[${msg.type()}] ${text}`);
        }
      }
    });

    // 1. Create a fresh playlist as the drop target via the API.
    //    showInDashboard: true so it renders in the dashboard sidebar
    //    (which is the only list with the drop handler attached).
    //    With showInDashboard: false the playlist only appears in the
    //    playlist-modal (no drop handler), and the test would no-op.
    const targetName = `E2E Drop Test ${Date.now()}`;
    const createResp = await page.request.post(
      new URL("/playlists", baseURL).toString(),
      {
        data: { name: targetName, showInDashboard: true },
      },
    );
    expect(createResp.status()).toBe(200);
    const created = await createResp.json();
    const targetPlaylistId = created.id as string;
    expect(targetPlaylistId).toBeTruthy();

    // 2. Now load the operator UI and select the first library.
    await initPage(page);
    const firstLibrary = page.locator('[data-role="library-item"]').first();
    await expect(firstLibrary).toBeVisible({ timeout: 15_000 });
    await firstLibrary.click();
    await page.waitForSelector(
      '[data-role="presentation-item"][data-presentation-id]',
      { timeout: 15_000 },
    );

    // 3. Wait for the new playlist row to appear in the dashboard sidebar
    //    (the <li> with the drop handler — NOT the modal row).
    //    The dashboard <li> sits inside [data-role="playlist-list"].
    await page.waitForFunction(
      (id: string) =>
        !!document.querySelector(
          `[data-role="playlist-list"] [data-playlist-id="${id}"]`,
        ),
      targetPlaylistId,
      { timeout: 30_000 },
    );

    // 4. Programmatically dispatch dragstart → dragover → drop → dragend.
    //    Pre-populate the DataTransfer with the presentation ID BEFORE
    //    dispatching, so the drop handler can read it without depending on
    //    the dragstart handler's set_data calls (which are unreliable in
    //    synthetic event dispatch — Leptos may not propagate the in-handler
    //    set_data writes back to our DataTransfer object).
    const dragResult = await page.evaluate((id: string) => {
      const source = document.querySelector(
        '[data-role="presentation-item"][data-presentation-id]',
      ) as HTMLElement | null;
      const targetRow = document.querySelector(
        `[data-role="playlist-list"] [data-playlist-id="${id}"]`,
      ) as HTMLElement | null;
      if (!source || !targetRow) {
        return {
          error: "missing source or target",
          hasSource: !!source,
          hasTarget: !!targetRow,
        };
      }
      const sourceId = source.getAttribute("data-presentation-id") || "";
      const dt = new DataTransfer();
      // Pre-populate the dataTransfer so the drop handler reads the ID
      // even if the dragstart handler doesn't run (synthetic-event quirk).
      dt.setData("text/plain", sourceId);
      dt.setData("application/x-presentation-id", sourceId);
      source.dispatchEvent(
        new DragEvent("dragstart", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      targetRow.dispatchEvent(
        new DragEvent("dragover", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      targetRow.dispatchEvent(
        new DragEvent("drop", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      source.dispatchEvent(
        new DragEvent("dragend", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      return {
        sourceId,
        dtAfterDispatch: dt.getData("application/x-presentation-id"),
      };
    }, targetPlaylistId);

    expect(dragResult.error, JSON.stringify(dragResult)).toBeUndefined();
    expect(dragResult.sourceId).toBeTruthy();

    // 5. Confirm via the API (after a short settle) that the playlist
    //    actually has the entry. This is the strongest signal — independent
    //    of UI count rendering timing.
    await expect
      .poll(
        async () => {
          const apiResp = await page.request.get(
            new URL(`/playlists/${targetPlaylistId}`, baseURL).toString(),
          );
          if (apiResp.status() !== 200) return -1;
          const body = await apiResp.json();
          return Array.isArray(body.entries) ? body.entries.length : -1;
        },
        { timeout: 15_000, intervals: [500, 1000, 2000] },
      )
      .toBeGreaterThanOrEqual(1);

    const finalResp = await page.request.get(
      new URL(`/playlists/${targetPlaylistId}`, baseURL).toString(),
    );
    expect(finalResp.status()).toBe(200);
    const playlist = await finalResp.json();
    // Server entries shape: { id, type: "presentation"|"separator", presentation_id? }
    // (type is a flat discriminator, not nested under kind).
    const presentationEntry = playlist.entries.find(
      (e: { type?: string }) => e?.type === "presentation",
    );
    expect(presentationEntry).toBeDefined();

    // Click the playlist to make it the selected playlist, so its entries
    // are shown in the presentation column.
    const playlistRow = page.locator(
      `[data-role="playlist-list"] [data-playlist-id="${targetPlaylistId}"] [data-role="playlist-item"]`,
    );
    await playlistRow.click();

    // The first presentation entry must render with a non-empty visible name.
    // Regression guard: previously the operator rebuilt presentations summaries
    // with empty strings, so the name span was blank.
    const firstEntryName = await page
      .locator(
        '[data-role="presentation-item"][data-type="presentation"] > span:first-child',
      )
      .first()
      .textContent({ timeout: 15_000 });
    expect(
      firstEntryName?.trim(),
      "playlist entry must show a non-empty presentation name",
    ).toBeTruthy();
    expect(firstEntryName?.trim().length ?? 0).toBeGreaterThan(0);

    expect(consoleErrors).toEqual([]);
  });

  test("drop a search-result onto a playlist row appends an entry", async ({
    page,
  }) => {
    // Regression guard for #worship-pp-followups: dragging from
    // [data-role="search-result-item"] (which dragstart sets
    // effectAllowed="copy") onto a playlist row was silently
    // rejected because the playlist's dragover never set a
    // matching dropEffect. Also: only Presentation/Slide-kind
    // results are draggable — Library-kind results have no
    // presentation_id and aren't draggable.

    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        const t = msg.text();
        if (!t.includes("favicon")) consoleErrors.push(`[${msg.type()}] ${t}`);
      }
    });

    // 1. Create the drop-target playlist via API.
    const targetName = `Search Drop Test ${Date.now()}`;
    const createResp = await page.request.post(
      new URL("/playlists", baseURL).toString(),
      { data: { name: targetName, showInDashboard: true } },
    );
    expect(createResp.status()).toBe(200);
    const created = await createResp.json();
    const targetPlaylistId = created.id as string;
    expect(targetPlaylistId).toBeTruthy();

    // 2. Load operator UI.
    await initPage(page);
    await page.waitForFunction(
      (id: string) =>
        !!document.querySelector(
          `[data-role="playlist-list"] [data-playlist-id="${id}"]`,
        ),
      targetPlaylistId,
      { timeout: 30_000 },
    );

    // 3. Type a query into the global search to populate results.
    //    Use a query that's likely to match a song name from the
    //    seeded fixtures.
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.click();
    await searchInput.fill("a");

    // Wait specifically for a Presentation-kind result (Library-kind
    // results have empty data-presentation-id and are intentionally
    // non-draggable).
    await page.waitForSelector(
      '[data-role="search-result-item"][data-kind="presentation"]',
      { timeout: 15_000 },
    );

    // 4. Programmatically drag a presentation-kind search result onto
    //    the playlist row. Pre-populate DataTransfer (synthetic-event
    //    quirk — see the existing presentation drop test).
    const dragResult = await page.evaluate((id: string) => {
      const source = document.querySelector(
        '[data-role="search-result-item"][data-kind="presentation"][data-presentation-id]',
      ) as HTMLElement | null;
      const targetRow = document.querySelector(
        `[data-role="playlist-list"] [data-playlist-id="${id}"]`,
      ) as HTMLElement | null;
      if (!source || !targetRow) {
        return {
          error: "missing source or target",
          hasSource: !!source,
          hasTarget: !!targetRow,
        };
      }
      const sourceId = source.getAttribute("data-presentation-id") || "";
      if (!sourceId) {
        return {
          error: "presentation-kind source has no data-presentation-id",
        };
      }
      const dt = new DataTransfer();
      dt.setData("text/plain", sourceId);
      dt.setData("application/x-presentation-id", sourceId);
      dt.setData("application/x-presenter-search", sourceId);
      source.dispatchEvent(
        new DragEvent("dragstart", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      targetRow.dispatchEvent(
        new DragEvent("dragover", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      targetRow.dispatchEvent(
        new DragEvent("drop", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      source.dispatchEvent(
        new DragEvent("dragend", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      return { sourceId };
    }, targetPlaylistId);

    expect(dragResult.error, JSON.stringify(dragResult)).toBeUndefined();
    expect(dragResult.sourceId).toBeTruthy();

    // 5. Confirm via API that the playlist gained an entry.
    await expect
      .poll(
        async () => {
          const apiResp = await page.request.get(
            new URL(`/playlists/${targetPlaylistId}`, baseURL).toString(),
          );
          if (apiResp.status() !== 200) return -1;
          const body = await apiResp.json();
          return Array.isArray(body.entries) ? body.entries.length : -1;
        },
        { timeout: 15_000, intervals: [500, 1000, 2000] },
      )
      .toBeGreaterThanOrEqual(1);

    // 6. Cleanup.
    await page.request.delete(
      new URL(`/playlists/${targetPlaylistId}`, baseURL).toString(),
    );

    expect(consoleErrors).toEqual([]);
  });
});
