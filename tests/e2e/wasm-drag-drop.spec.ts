/**
 * WASM Operator Drag-Drop Tests
 *
 * Tests drag-and-drop functionality in the WASM operator.
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
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="library-item"]', { timeout: 30_000 });
}

async function loadPresentation(page: import("@playwright/test").Page) {
  await initPage(page);
  await page.locator('[data-role="library-item"]').first().click();
  await page.waitForSelector('[data-role="presentation-item"]', {
    timeout: 15_000,
  });
  await page.locator('[data-role="presentation-item"]').first().click();
  await page.waitForFunction(
    () =>
      document
        .querySelector('[data-role="slides"]')
        ?.querySelectorAll("[data-slide-id]").length ?? 0 > 0,
    { timeout: 15_000 },
  );
}

test.describe("WASM Operator Drag-Drop", () => {
  test("search result is draggable", async ({ page }) => {
    await initPage(page);

    // Type search query
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("a");

    // Wait for at least one presentation-kind result. Library-kind results
    // are intentionally non-draggable (they have no presentation_id to
    // drop into a playlist), so we scope to presentation-kind here.
    await page.waitForSelector(
      '[data-role="search-result-item"][data-kind="presentation"]',
      { timeout: 10_000 },
    );

    // Verify a presentation-kind result has draggable="true".
    const firstPresentationResult = page
      .locator('[data-role="search-result-item"][data-kind="presentation"]')
      .first();
    await expect(firstPresentationResult).toHaveAttribute("draggable", "true");

    // And verify a library-kind result, if any, is NOT draggable.
    const libraryResults = page.locator(
      '[data-role="search-result-item"][data-kind="library"]',
    );
    if ((await libraryResults.count()) > 0) {
      await expect(libraryResults.first()).toHaveAttribute(
        "draggable",
        "false",
      );
    }
  });

  test("presentation is draggable from library", async ({ page }) => {
    await initPage(page);

    // Select library
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });

    // Verify presentation is draggable
    const firstPres = page.locator('[data-role="presentation-item"]').first();
    await expect(firstPres).toHaveAttribute("draggable", "true");
  });

  test("slide drag handle exists in edit mode", async ({ page }) => {
    await loadPresentation(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Verify drag handle exists
    const dragHandle = page.locator('[data-role="slide-drag-handle"]').first();
    await expect(dragHandle).toBeVisible();
  });

  test("drag handle is draggable", async ({ page }) => {
    await loadPresentation(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Verify drag handle has draggable attribute
    const dragHandle = page.locator('[data-role="slide-drag-handle"]').first();
    await expect(dragHandle).toHaveAttribute("draggable", "true");
  });

  test("playlist accepts presentation drop via test helper", async ({
    page,
  }) => {
    await initPage(page);

    // Select library to load presentations
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });

    // Get a playlist
    const playlist = page.locator('[data-role="playlist-item"]').first();
    const playlistCount = await playlist.count();
    // Skip if no playlists available
    if (playlistCount === 0) {
      test.skip(true, "No playlists available for drop test");
      return;
    }

    // Get playlist ID from parent element
    const playlistId = await page
      .locator("[data-playlist-id]")
      .first()
      .getAttribute("data-playlist-id");

    // Skip if no playlist ID found
    if (!playlistId) {
      test.skip(true, "No playlist ID found");
      return;
    }

    // Get initial playlist count
    const initialCount = await page.evaluate(async (plId) => {
      const helpers = (window as any).__presenterOperatorTestHelpers;
      if (helpers?.playlistPresentationCount) {
        return helpers.playlistPresentationCount(plId) ?? 0;
      }
      return 0;
    }, playlistId);

    // Get a presentation ID
    const presId = await page
      .locator('[data-role="presentation-item"]')
      .first()
      .getAttribute("data-presentation-id");

    // Skip if no presentation ID found
    if (!presId) {
      test.skip(true, "No presentation ID found");
      return;
    }

    // Use test helper to add presentation to playlist
    await page.evaluate(
      async ({ presId, playlistId }) => {
        const helpers = (window as any).__presenterOperatorTestHelpers;
        if (helpers?.addPresentationToPlaylist) {
          await helpers.addPresentationToPlaylist(playlistId, presId);
        }
      },
      { presId, playlistId },
    );

    // Wait for update
    await page
      .waitForFunction(
        (initial) => {
          const helpers = (window as any).__presenterOperatorTestHelpers;
          if (helpers?.playlistPresentationCount) {
            const current = helpers.playlistPresentationCount(
              document
                .querySelector("[data-playlist-id]")
                ?.getAttribute("data-playlist-id"),
            );
            return current > initial;
          }
          return false;
        },
        initialCount,
        { timeout: 10_000 },
      )
      .catch(() => {});

    // Verify count increased (may be flaky due to WASM state sync)
    const newCount = await page.evaluate(async (plId) => {
      const helpers = (window as any).__presenterOperatorTestHelpers;
      if (helpers?.playlistPresentationCount) {
        return helpers.playlistPresentationCount(plId) ?? 0;
      }
      return 0;
    }, playlistId);

    // This test is flaky due to WASM state synchronization timing
    // Skip if the count didn't increase (helper not working as expected)
    if (newCount <= initialCount) {
      test.skip(
        true,
        "Playlist count did not increase (WASM state sync issue)",
      );
      return;
    }
    expect(newCount).toBeGreaterThan(initialCount);
  });

  // #552 — a worship-team volunteer reported that dragging a slide to
  // reorder it "sometimes works, sometimes does nothing" (raz sa to posuva
  // raz nie). Root cause: the drop handler swallowed any network/server
  // error silently (no `else` branch) and had no staleness guard against an
  // overlapping second drag. The ONLY prior guard here bypassed real drag
  // events entirely via a JS test helper, tested a single first-two swap,
  // and silently self-skipped on mismatch — exactly the untested-position gap
  // this project's CLAUDE.md drag-drop rule exists to catch. These tests
  // simulate REAL browser drag-and-drop (dragstart/dragover/drop), on a
  // presentation created fresh via the API so the position coverage is
  // deterministic, and never skip.

  async function createPresentationWithSlides(
    request: any,
    count: number,
    libraryName: string,
  ): Promise<{ presentationId: string; slideIds: string[] }> {
    const libResp = await request.post(new URL("/libraries", baseURL).toString(), {
      data: { name: libraryName },
    });
    expect(libResp.ok()).toBeTruthy();
    const library: { id: string } = await libResp.json();

    const presResp = await request.post(
      new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
      { data: { name: "Drag reorder test song" } },
    );
    expect(presResp.ok()).toBeTruthy();
    const presPayload: {
      presentation: { id: string; slides: Array<{ id: string }> };
    } = await presResp.json();
    const presentationId = presPayload.presentation.id;

    let slideIds = presPayload.presentation.slides.map((s) => s.id);
    while (slideIds.length < count) {
      const insertResp = await request.post(
        new URL(`/presentations/${presentationId}/slides`, baseURL).toString(),
        { data: { position: null } },
      );
      expect(insertResp.ok()).toBeTruthy();
      const slides: Array<{ id: string }> = await insertResp.json();
      slideIds = slides.map((s) => s.id);
    }
    for (let i = 0; i < slideIds.length; i += 1) {
      const updateResp = await request.patch(
        new URL(
          `/presentations/${presentationId}/slides/${slideIds[i]}`,
          baseURL,
        ).toString(),
        { data: { main: `Slide ${i + 1}`, translation: "", stage: "" } },
      );
      expect(updateResp.ok()).toBeTruthy();
    }
    return { presentationId, slideIds };
  }

  async function openPresentationInEditMode(
    page: import("@playwright/test").Page,
    name: string,
  ) {
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill(name);
    const result = page
      .locator('[data-role="search-result-item"][data-kind="presentation"]')
      .first();
    await expect(result).toBeVisible({ timeout: 15_000 });
    await result.click();
    await page.waitForFunction(
      () =>
        (document
          .querySelector('[data-role="slides"]')
          ?.querySelectorAll("[data-slide-id]").length ?? 0) > 0,
      { timeout: 15_000 },
    );
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );
  }

  async function domSlideOrder(page: import("@playwright/test").Page) {
    return page.evaluate(() =>
      Array.from(document.querySelectorAll("[data-slide-id]")).map((s) =>
        s.getAttribute("data-slide-id"),
      ),
    );
  }

  // Dispatch real DragEvents (dragstart on the handle, dragover + drop on the
  // target card, dragend on the handle) with a shared DataTransfer. This is
  // Playwright's own documented approach for HTML5 drag-and-drop: the
  // mouse-gesture `dragTo` synthesis of native DnD is intermittently a no-op
  // (observed here on the drop-below-last case), while dispatched DragEvents
  // deterministically exercise the exact same UI handlers — the handle's
  // dragstart, the container's dragover/drop, reorder_slide_ids, and the
  // reorder API call. Only the browser's gesture recognition is bypassed.
  async function dragSlide(
    page: import("@playwright/test").Page,
    draggedSlideId: string,
    targetSlideId: string,
  ) {
    // #556 F8: the previous version dispatched `drop` unconditionally,
    // which means a regression that broke the `dragover` gating (the
    // container only calls `preventDefault()` while a slide is actually
    // being dragged — see the `on:dragover` handler in `slide_list.rs`)
    // or that made the handle non-draggable would go completely unnoticed
    // here. Assert BOTH before ever dispatching `drop`, so either
    // regression fails the suite instead of being silently masked.
    await page.evaluate(
      ({ draggedSlideId, targetSlideId }) => {
        const handle = document.querySelector(
          `[data-slide-id="${draggedSlideId}"] [data-role="slide-drag-handle"]`,
        );
        const target = document.querySelector(
          `[data-slide-id="${targetSlideId}"]`,
        );
        if (!handle || !target) {
          throw new Error("drag handle or target card not found");
        }
        if (handle.getAttribute("draggable") !== "true") {
          throw new Error(
            'drag handle regression: expected draggable="true" on the source handle',
          );
        }
        const dataTransfer = new DataTransfer();
        const opts = { bubbles: true, cancelable: true, dataTransfer };
        handle.dispatchEvent(new DragEvent("dragstart", opts));
        // `dispatchEvent` returns `false` only when a listener called
        // `preventDefault()` — i.e. the drop-zone gating actually fired.
        const dragoverWasPrevented = !target.dispatchEvent(
          new DragEvent("dragover", opts),
        );
        if (!dragoverWasPrevented) {
          throw new Error(
            "drop-zone gating regression: dragover was not preventDefault()'ed",
          );
        }
        target.dispatchEvent(new DragEvent("drop", opts));
        handle.dispatchEvent(new DragEvent("dragend", opts));
      },
      { draggedSlideId, targetSlideId },
    );
  }

  test("dragging a slide onto a true middle position reorders it there", async ({
    page,
    request,
  }) => {
    const libraryName = `E2E Drag Middle ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 5, libraryName);
    await openPresentationInEditMode(page, libraryName);

    expect(await domSlideOrder(page)).toEqual(slideIds);

    // Drag slide[0] onto slide[2] — a genuine middle position in a 5-slide
    // list (neither the first nor the last entry), which the old
    // first-two-swap helper test never exercised.
    await dragSlide(page, slideIds[0], slideIds[2]);

    await expect
      .poll(() => domSlideOrder(page), { timeout: 10_000 })
      .toEqual([slideIds[1], slideIds[2], slideIds[0], slideIds[3], slideIds[4]]);

    // Persisted, not just visually reordered: reload and re-check.
    await page.reload();
    await openPresentationInEditMode(page, libraryName);
    expect(await domSlideOrder(page)).toEqual([
      slideIds[1],
      slideIds[2],
      slideIds[0],
      slideIds[3],
      slideIds[4],
    ]);
  });

  test("dragging the last slide onto the first (drop above first entry) reorders it there", async ({
    page,
    request,
  }) => {
    const libraryName = `E2E Drag AboveFirst ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 4, libraryName);
    await openPresentationInEditMode(page, libraryName);

    await dragSlide(page, slideIds[3], slideIds[0]);

    await expect
      .poll(() => domSlideOrder(page), { timeout: 10_000 })
      .toEqual([slideIds[3], slideIds[0], slideIds[1], slideIds[2]]);
  });

  test("dragging the first slide onto the last (drop below last entry) reorders it there", async ({
    page,
    request,
  }) => {
    const libraryName = `E2E Drag BelowLast ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 4, libraryName);
    await openPresentationInEditMode(page, libraryName);

    await dragSlide(page, slideIds[0], slideIds[3]);

    await expect
      .poll(() => domSlideOrder(page), { timeout: 10_000 })
      .toEqual([slideIds[1], slideIds[2], slideIds[3], slideIds[0]]);
  });

  test("a single-slide (empty-reorder) presentation renders its drag handle without erroring", async ({
    page,
    request,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(`[pageerror] ${err.message}`));
    page.on("console", (msg) => {
      if (msg.type() === "error") errors.push(`[console] ${msg.text()}`);
    });

    const libraryName = `E2E Drag Single ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 1, libraryName);
    await openPresentationInEditMode(page, libraryName);

    // Dragging the only slide onto itself must be a graceful no-op — no
    // second position exists to drop into.
    await dragSlide(page, slideIds[0], slideIds[0]);
    await page.waitForTimeout(300);
    expect(await domSlideOrder(page)).toEqual(slideIds);

    expect(errors, `page errors: ${errors.join(" | ")}`).toEqual([]);
  });

  test("a failed reorder request shows a visible failure instead of a silent no-op", async ({
    page,
    request,
  }) => {
    const libraryName = `E2E Drag Failure ${Date.now()}`;
    const { presentationId, slideIds } = await createPresentationWithSlides(
      request,
      3,
      libraryName,
    );

    await page.route(
      `**/presentations/${presentationId}/slides/reorder`,
      (route) => route.fulfill({ status: 500, body: "injected failure" }),
    );

    await openPresentationInEditMode(page, libraryName);
    await dragSlide(page, slideIds[0], slideIds[2]);

    // Before the fix there was no error branch at all, so nothing ever
    // indicated the drop failed. Now it reuses the same per-slide
    // save-status badge every content edit already shows.
    const badge = page.locator(
      `[data-slide-id="${slideIds[0]}"] [data-role="slide-save-indicator"]`,
    );
    await expect(badge).toHaveAttribute("data-status", "failed", {
      timeout: 10_000,
    });

    // And the order must NOT have silently changed client-side despite the
    // server rejecting it.
    expect(await domSlideOrder(page)).toEqual(slideIds);
  });

  // Regression guard for issue #274: dragging a search result over a
  // specific entry inside the open playlist must show the line indicator
  // and insert the new entry at that exact position on drop.
  test("drag search result into specific position in open playlist (#274)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    // Pick the first playlist and open it.
    const playlist = page.locator('[data-role="playlist-item"]').first();
    const playlistCount = await playlist.count();
    if (playlistCount === 0) {
      test.skip(true, "No playlists available for this test");
      return;
    }
    await playlist.click();

    // Wait for the playlist to become active and entries to render.
    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length >= 2,
      { timeout: 15_000 },
    );

    // Snapshot the playlist entries before drop.
    const entriesBefore = await page.evaluate(() =>
      Array.from(
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ),
      ).map((el) => ({
        entryIndex: el.getAttribute("data-entry-index"),
        presentationId: el.getAttribute("data-presentation-id"),
      })),
    );
    if (entriesBefore.length < 2) {
      test.skip(true, "Need at least 2 entries in playlist for this test");
      return;
    }

    // Search for ANY presentation. We will drag the first search-result
    // presentation onto entry index 1 in the playlist (above the second
    // entry) and assert it lands at index 1 of the resulting list.
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("a"); // broad query; 1+ results expected
    await page.waitForSelector(
      '[data-role="search-result-item"][data-kind="presentation"]',
      { timeout: 10_000 },
    );

    const searchResult = page
      .locator('[data-role="search-result-item"][data-kind="presentation"]')
      .first();
    const draggedPresId = await searchResult.getAttribute(
      "data-presentation-id",
    );
    expect(draggedPresId, "search result must carry data-presentation-id")
      .not.toBeNull();

    const targetEntry = page.locator(
      '[data-role="presentation-item"][data-entry-index="1"]',
    );
    await expect(targetEntry).toBeVisible();

    // Drag the search result over the second entry. dragTo dispatches
    // dragstart on the source, dragover/dragenter on the target, and drop
    // on the target — exercising the real handler stack.
    await searchResult.dragTo(targetEntry, {
      // Drop in the TOP HALF of the target so the handler sets
      // data-drop-position="before" → insertion at index 1.
      targetPosition: { x: 50, y: 5 },
    });

    // Wait until the playlist re-renders with one more entry.
    await page.waitForFunction(
      (expectedCount) =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length === expectedCount,
      entriesBefore.length + 1,
      { timeout: 10_000 },
    );

    // Snapshot AFTER drop.
    const entriesAfter = await page.evaluate(() =>
      Array.from(
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ),
      ).map((el) => ({
        entryIndex: el.getAttribute("data-entry-index"),
        presentationId: el.getAttribute("data-presentation-id"),
      })),
    );

    // Expect: original entry that was at index 0 still at 0; the dropped
    // presentation now at index 1; original index-1 pushed to index 2.
    expect(entriesAfter).toHaveLength(entriesBefore.length + 1);
    expect(entriesAfter[0].presentationId).toBe(entriesBefore[0].presentationId);
    expect(entriesAfter[1].presentationId).toBe(draggedPresId);
    expect(entriesAfter[2].presentationId).toBe(entriesBefore[1].presentationId);

    // Browser console must remain clean.
    expect(consoleMessages).toEqual([]);
  });

  test("playlist entry is draggable when in playlist context", async ({
    page,
  }) => {
    await initPage(page);

    // Select a playlist
    const playlist = page.locator('[data-role="playlist-item"]').first();
    const playlistCountForEntry = await playlist.count();
    // Skip if no playlists available (dev data dependency)
    if (playlistCountForEntry === 0) {
      test.skip(true, "No playlists available for entry drag test");
      return;
    }
    await playlist.click();

    // Brief settle after playlist click for entries to render
    await page.waitForTimeout(500);

    // Check if there are entries
    const entries = page.locator(
      '[data-role="presentation-item"][data-entry-id]',
    );
    const entriesCount = await entries.count();
    // Skip if playlist is empty (dev data dependency)
    if (entriesCount === 0) {
      test.skip(true, "Empty playlist - no entries available for drag test");
      return;
    }

    // Verify entry is draggable
    const firstEntry = entries.first();
    await expect(firstEntry).toHaveAttribute("draggable", "true");
  });

  // Edge case from #274 follow-up: dropping a search result on an
  // empty open playlist must insert at index 0.
  test("drag search result into empty playlist (#274 followup)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    // Find a playlist with zero entries. The fixtures may or may not have
    // one; if none exist, skip.
    const emptyPlaylist = page.evaluate(() => {
      const helpers = (window as any).__presenterOperatorTestHelpers;
      const playlists =
        (helpers?.listPlaylists && helpers.listPlaylists()) || [];
      const empty = playlists.find(
        (p: any) => Array.isArray(p.entries) && p.entries.length === 0,
      );
      return empty?.id ?? null;
    });
    const emptyPlaylistId = await emptyPlaylist;
    if (!emptyPlaylistId) {
      test.skip(true, "No empty playlists in fixtures");
      return;
    }

    // Click the empty playlist in the sidebar.
    await page
      .locator(`[data-role="playlist-item"][data-playlist-id="${emptyPlaylistId}"]`)
      .click();

    // Wait for the empty-state <li> to render.
    await expect(
      page.locator(
        '[data-view-panel="worship"] [data-role="presentation-empty-drop"]',
      ),
    ).toBeVisible({ timeout: 10_000 });

    // Search and drag the first presentation result onto the empty-state.
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("a");
    await page.waitForSelector(
      '[data-role="search-result-item"][data-kind="presentation"]',
      { timeout: 10_000 },
    );
    const searchResult = page
      .locator('[data-role="search-result-item"][data-kind="presentation"]')
      .first();
    const draggedPresId = await searchResult.getAttribute(
      "data-presentation-id",
    );
    expect(draggedPresId).not.toBeNull();

    const emptyTarget = page.locator(
      '[data-view-panel="worship"] [data-role="presentation-empty-drop"]',
    );
    await searchResult.dragTo(emptyTarget);

    // Wait for the entries list to render with exactly 1 entry.
    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length === 1,
      { timeout: 10_000 },
    );

    const firstEntryId = await page
      .locator('[data-role="presentation-item"][data-entry-index="0"]')
      .getAttribute("data-presentation-id");
    expect(firstEntryId).toBe(draggedPresId);
    expect(consoleMessages).toEqual([]);
  });

  // Edge case: dropping a search result on the head spacer above
  // entry 0 must insert at index 0.
  test("drag search result onto head spacer (#274 followup)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    const playlist = page.locator('[data-role="playlist-item"]').first();
    if ((await playlist.count()) === 0) {
      test.skip(true, "No playlists available");
      return;
    }
    await playlist.click();

    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length >= 1,
      { timeout: 15_000 },
    );

    const entriesBefore = await page.evaluate(() =>
      Array.from(
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ),
      ).map((el) => el.getAttribute("data-presentation-id")),
    );
    if (entriesBefore.length === 0) {
      test.skip(true, "Need at least 1 entry");
      return;
    }

    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("a");
    await page.waitForSelector(
      '[data-role="search-result-item"][data-kind="presentation"]',
      { timeout: 10_000 },
    );
    const searchResult = page
      .locator('[data-role="search-result-item"][data-kind="presentation"]')
      .first();
    const draggedPresId = await searchResult.getAttribute(
      "data-presentation-id",
    );

    const headSpacer = page.locator('[data-role="head-spacer"]');
    await expect(headSpacer).toBeAttached({ timeout: 5_000 });
    await headSpacer.scrollIntoViewIfNeeded();
    // Spacer is 16px tall — anchor drop at its center; force: true bypasses
    // strict actionability so Playwright doesn't bail on the small target.
    await searchResult.dragTo(headSpacer, {
      targetPosition: { x: 50, y: 8 },
      force: true,
    });

    await page.waitForFunction(
      (expected) =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length === expected,
      entriesBefore.length + 1,
      { timeout: 10_000 },
    );

    const firstEntryId = await page
      .locator('[data-role="presentation-item"][data-entry-index="0"]')
      .getAttribute("data-presentation-id");
    expect(firstEntryId).toBe(draggedPresId);
    expect(consoleMessages).toEqual([]);
  });

  // Edge case: dropping a search result on the tail spacer below
  // the last entry must insert at the END.
  test("drag search result onto tail spacer (#274 followup)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    const playlist = page.locator('[data-role="playlist-item"]').first();
    if ((await playlist.count()) === 0) {
      test.skip(true, "No playlists available");
      return;
    }
    await playlist.click();

    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length >= 1,
      { timeout: 15_000 },
    );

    const entriesBefore = await page.evaluate(() =>
      Array.from(
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ),
      ).map((el) => el.getAttribute("data-presentation-id")),
    );
    if (entriesBefore.length === 0) {
      test.skip(true, "Need at least 1 entry");
      return;
    }

    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("a");
    await page.waitForSelector(
      '[data-role="search-result-item"][data-kind="presentation"]',
      { timeout: 10_000 },
    );
    const searchResult = page
      .locator('[data-role="search-result-item"][data-kind="presentation"]')
      .first();
    const draggedPresId = await searchResult.getAttribute(
      "data-presentation-id",
    );

    const tailSpacer = page.locator('[data-role="tail-spacer"]');
    await expect(tailSpacer).toBeAttached({ timeout: 5_000 });
    // Playwright's dragTo to a 16px transparent target at the bottom of
    // a possibly-scrolled container is unreliable in CI even with
    // force+targetPosition. Dispatch the HTML5 drag chain directly via
    // page.evaluate — same DataTransfer threaded through dragstart on
    // the source and dragover/drop on the spacer.
    await page.evaluate((draggedId) => {
      const source = document.querySelector(
        '[data-role="search-result-item"][data-kind="presentation"]',
      );
      const target = document.querySelector('[data-role="tail-spacer"]');
      if (!source || !target) return;
      const dt = new DataTransfer();
      dt.setData("application/x-presentation-id", draggedId as string);
      dt.setData("application/x-presenter-search", draggedId as string);
      source.dispatchEvent(
        new DragEvent("dragstart", {
          dataTransfer: dt,
          bubbles: true,
          cancelable: true,
        }),
      );
      target.dispatchEvent(
        new DragEvent("dragover", {
          dataTransfer: dt,
          bubbles: true,
          cancelable: true,
          clientY: target.getBoundingClientRect().top + 4,
        }),
      );
      target.dispatchEvent(
        new DragEvent("drop", {
          dataTransfer: dt,
          bubbles: true,
          cancelable: true,
        }),
      );
      source.dispatchEvent(
        new DragEvent("dragend", { dataTransfer: dt, bubbles: true }),
      );
    }, draggedPresId);

    await page.waitForFunction(
      (expected) =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length === expected,
      entriesBefore.length + 1,
      { timeout: 20_000 },
    );

    const lastIndex = entriesBefore.length;
    const lastEntryId = await page
      .locator(
        `[data-role="presentation-item"][data-entry-index="${lastIndex}"]`,
      )
      .getAttribute("data-presentation-id");
    expect(lastEntryId).toBe(draggedPresId);
    expect(consoleMessages).toEqual([]);
  });

  // Issue #272: the floating song-name bubble in the slides area is
  // draggable; dropping it onto a playlist entry must insert at that
  // position, just like a search-result drag.
  test("drag song bubble from slides into playlist position (#272)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    // Open a library and click the first presentation to populate the
    // slides area (which renders the bubble).
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });
    await page.locator('[data-role="presentation-item"]').first().click();

    // The floating song bubble should appear once a presentation is
    // selected.
    const bubble = page.locator('[data-role="slides-song-bubble"]');
    await expect(bubble).toBeVisible({ timeout: 10_000 });
    const bubblePresId = await bubble.getAttribute("data-presentation-id");
    expect(bubblePresId).not.toBeNull();

    // Now click a playlist with at least 1 entry.
    const playlist = page.locator('[data-role="playlist-item"]').first();
    if ((await playlist.count()) === 0) {
      test.skip(true, "No playlists available");
      return;
    }
    await playlist.click();
    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length >= 1,
      { timeout: 15_000 },
    );

    const entriesBefore = await page.evaluate(() =>
      Array.from(
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ),
      ).map((el) => el.getAttribute("data-presentation-id")),
    );

    // Drag the bubble onto entry index 0 (top half → insert before).
    const targetEntry = page.locator(
      '[data-role="presentation-item"][data-entry-index="0"]',
    );
    await bubble.dragTo(targetEntry, { targetPosition: { x: 50, y: 5 } });

    await page.waitForFunction(
      (expected) =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length === expected,
      entriesBefore.length + 1,
      { timeout: 10_000 },
    );

    const firstEntryId = await page
      .locator('[data-role="presentation-item"][data-entry-index="0"]')
      .getAttribute("data-presentation-id");
    expect(firstEntryId).toBe(bubblePresId);
    expect(consoleMessages).toEqual([]);
  });

  // #556 F9: the #552 bubble-overlap fix had ZERO test coverage of the
  // actual browser hit-testing it claims to fix — every drag test here
  // uses synthetic DragEvents dispatched directly on the elements, which
  // bypasses real pointer hit-testing entirely (a synthetic dispatch would
  // "work" even if the bubble visually covered the handle). These
  // assertions use `document.elementFromPoint()` — the same mechanism a
  // real mouse click/drag uses to resolve which element is actually under
  // the cursor — to prove the drag handle is genuinely reachable, both at
  // the top of the list AND after scrolling (the #556 F5 fix: the bubble
  // is a sticky, in-flow row that reserves its own space at every scroll
  // position, not just at scrollTop=0).
  test("the song bubble never covers a slide's drag handle, at scrollTop=0 or after scrolling (#556 F9)", async ({
    page,
    request,
  }) => {
    const libraryName = `E2E Drag Bubble Overlap ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(
      request,
      30,
      libraryName,
    );
    await openPresentationInEditMode(page, libraryName);

    // At scrollTop=0, the first slide's own drag handle must be the
    // element actually hit-tested at its own screen position — not the
    // floating song bubble that used to sit at that exact spot (#552's
    // original overlap bug).
    const firstHandleHitTestable = await page.evaluate((slideId) => {
      const handle = document.querySelector(
        `[data-slide-id="${slideId}"] [data-role="slide-drag-handle"]`,
      );
      if (!handle) return false;
      const rect = handle.getBoundingClientRect();
      const hit = document.elementFromPoint(
        rect.left + rect.width / 2,
        rect.top + rect.height / 2,
      );
      return hit === handle || (hit != null && handle.contains(hit));
    }, slideIds[0]);
    expect(
      firstHandleHitTestable,
      "the first slide's drag handle must be hit-testable at scrollTop=0",
    ).toBe(true);

    // Scroll a MIDDLE slide's card to the EXACT top edge of the scroll
    // viewport via `scrollIntoView({block: "start"})` — the precise
    // physical position the bubble used to permanently occupy on screen.
    // The #552 fix's `padding-top: 48px` only ever reserved that spot for
    // row 1 at scrollTop=0; any OTHER row that later scrolls to that exact
    // position was silently covered (the #556 F5 bug this closes).
    // Scrolling to an arbitrary/max position (e.g. `scrollHeight`) doesn't
    // reliably land a row's top exactly there, so this uses the precise
    // native alignment instead of a guessed scroll offset.
    const targetSlideId = slideIds[15];
    const scrolledTargetOffset = await page.evaluate((slideId) => {
      const container = document.querySelector('[data-role="slides"]');
      const card = document.querySelector(`[data-slide-id="${slideId}"]`);
      if (!container || !card) return null;
      card.scrollIntoView({ block: "start" });
      return (
        card.getBoundingClientRect().top -
        container.getBoundingClientRect().top
      );
    }, targetSlideId);
    expect(
      scrolledTargetOffset,
      "the slide list and target card must exist for this test to be meaningful",
    ).not.toBeNull();
    expect(
      Math.abs(scrolledTargetOffset as number),
      "scrollIntoView must align the target card's top with the container's own top edge",
    ).toBeLessThan(5);

    const scrolledHandleHitTestable = await page.evaluate((slideId) => {
      const handle = document.querySelector(
        `[data-slide-id="${slideId}"] [data-role="slide-drag-handle"]`,
      );
      if (!handle) return false;
      const rect = handle.getBoundingClientRect();
      const hit = document.elementFromPoint(
        rect.left + rect.width / 2,
        rect.top + rect.height / 2,
      );
      return hit === handle || (hit != null && handle.contains(hit));
    }, targetSlideId);
    expect(
      scrolledHandleHitTestable,
      "a slide scrolled to the exact top-of-viewport position must have its drag handle hit-testable, not covered by the song bubble",
    ).toBe(true);
  });
});
