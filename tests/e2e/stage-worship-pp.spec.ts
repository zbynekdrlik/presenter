import { test, expect } from "@playwright/test";
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

test.describe("Stage worship-pp layout", () => {
  test("slides-area and playlist-sidebar do not overlap", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        const t = msg.text();
        if (!t.includes("favicon")) consoleErrors.push(`[${msg.type()}] ${t}`);
      }
    });

    // Set the stage layout to worship-pp via API.
    const layoutResp = await page.request.post(
      new URL("/stage/layout", baseURL).toString(),
      { data: { code: "worship-pp" } },
    );
    expect(layoutResp.ok()).toBeTruthy();

    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });
    await page.waitForSelector('body[data-layout-code="worship-pp"]', {
      timeout: 10_000,
    });

    // Both wrapper boxes exist and are visible.
    const slidesArea = page.locator(".stage-pp__slides-area");
    const sidebar = page.locator(".stage-pp__playlist-sidebar");
    await expect(slidesArea).toBeVisible();
    await expect(sidebar).toBeVisible();

    // Slides-area's right edge must be <= sidebar's left edge.
    const overlap = await page.evaluate(() => {
      const a = document
        .querySelector(".stage-pp__slides-area")
        ?.getBoundingClientRect();
      const b = document
        .querySelector(".stage-pp__playlist-sidebar")
        ?.getBoundingClientRect();
      if (!a || !b) return { error: "missing rect" } as const;
      return {
        aRight: a.right,
        bLeft: b.left,
        overlap: a.right > b.left,
      } as const;
    });
    expect(
      "overlap" in overlap ? overlap.overlap : true,
      `slides-area right=${"aRight" in overlap ? overlap.aRight : "?"} sidebar left=${"bLeft" in overlap ? overlap.bLeft : "?"}`,
    ).toBe(false);

    // The six slide regions live INSIDE slides-area.
    for (const cls of [
      ".stage__current-group",
      ".stage__current-song",
      ".stage__current-slide",
      ".stage__next-group",
      ".stage__next-song",
      ".stage__next-slide",
    ]) {
      const inside = await page.evaluate((selector) => {
        const el = document.querySelector(selector);
        return !!el?.closest(".stage-pp__slides-area");
      }, cls);
      expect(inside, `${cls} should be inside slides-area`).toBe(true);
    }

    expect(consoleErrors).toEqual([]);
  });

  test("active playlist entry has high-contrast background distinct from inactive", async ({
    page,
  }) => {
    // Set worship-pp layout
    const layoutResp = await page.request.post(
      new URL("/stage/layout", baseURL).toString(),
      { data: { code: "worship-pp" } },
    );
    expect(layoutResp.ok()).toBeTruthy();

    // Seed: pick a library and a presentation from the seeded data.
    const libsResp = await page.request.get(
      new URL("/libraries", baseURL).toString(),
    );
    expect(libsResp.ok()).toBeTruthy();
    const libs = (await libsResp.json()) as Array<{
      id: string;
      presentations?: Array<{
        id: string;
        slides?: Array<{ id: string }>;
      }>;
    }>;
    const presentation = libs
      .flatMap((lib) => lib.presentations ?? [])
      .find((p) => (p.slides?.length ?? 0) > 0);
    if (!presentation || !presentation.slides || !presentation.slides[0]) {
      test.skip(true, "test fixture has no presentation with slides");
      return;
    }
    const slideId = presentation.slides[0].id;

    // Create a playlist with that presentation as an entry.
    const playlistResp = await page.request.post(
      new URL("/playlists", baseURL).toString(),
      { data: { name: `Highlight Test ${Date.now()}`, showInDashboard: true } },
    );
    expect(playlistResp.ok()).toBeTruthy();
    const playlist = (await playlistResp.json()) as { id: string };
    const entriesResp = await page.request.put(
      new URL(`/playlists/${playlist.id}/entries`, baseURL).toString(),
      {
        data: {
          entries: [{ type: "presentation", presentationId: presentation.id }],
        },
      },
    );
    expect(entriesResp.ok()).toBeTruthy();

    // Trigger the presentation onto stage so it becomes active.
    const stageResp = await page.request.post(
      new URL("/stage/state", baseURL).toString(),
      {
        data: {
          presentationId: presentation.id,
          currentSlideId: slideId,
          playlistId: playlist.id,
        },
      },
    );
    expect(stageResp.status()).toBe(204);

    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });
    await page.waitForSelector('body[data-layout-code="worship-pp"]', {
      timeout: 10_000,
    });

    // Wait for the active row to appear with the active class.
    const active = page.locator(".stage-pp__playlist-entry--active").first();
    await expect(active).toBeVisible({ timeout: 15_000 });

    // Background of active row must NOT be transparent and (if present)
    // must differ from inactive rows.
    const colors = await page.evaluate(() => {
      const a = document.querySelector(
        ".stage-pp__playlist-entry--active",
      ) as HTMLElement | null;
      const inactive = Array.from(
        document.querySelectorAll(".stage-pp__playlist-entry"),
      ).find(
        (e) => !e.classList.contains("stage-pp__playlist-entry--active"),
      ) as HTMLElement | null;
      return {
        active: a ? getComputedStyle(a).backgroundColor : null,
        inactive: inactive ? getComputedStyle(inactive).backgroundColor : null,
      };
    });
    expect(colors.active).toBeTruthy();
    expect(colors.active).not.toBe("rgba(0, 0, 0, 0)");
    expect(colors.active).not.toBe("transparent");
    if (colors.inactive) {
      expect(colors.active).not.toBe(colors.inactive);
    }

    // Cleanup: delete the playlist
    await page.request.delete(
      new URL(`/playlists/${playlist.id}`, baseURL).toString(),
    );
  });

  test("sidebar is narrower (~22%) and entries have projector-readable font", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        const t = msg.text();
        if (t.includes("favicon")) return;
        if (t.includes("crbug.com/981419")) return;
        consoleErrors.push(`[${msg.type()}] ${t}`);
      }
    });

    // Set worship-pp layout
    const layoutResp = await page.request.post(
      new URL("/stage/layout", baseURL).toString(),
      { data: { code: "worship-pp" } },
    );
    expect(layoutResp.ok()).toBeTruthy();

    // Seed a playlist with one entry and trigger it so the sidebar has content.
    const libsResp = await page.request.get(
      new URL("/libraries", baseURL).toString(),
    );
    const libs = (await libsResp.json()) as Array<{
      id: string;
      presentations?: Array<{ id: string; slides?: Array<{ id: string }> }>;
    }>;
    const presentation = libs
      .flatMap((lib) => lib.presentations ?? [])
      .find((p) => (p.slides?.length ?? 0) > 0);
    if (!presentation || !presentation.slides || !presentation.slides[0]) {
      test.skip(true, "test fixture has no presentation with slides");
      return;
    }
    const slideId = presentation.slides[0].id;

    const playlistResp = await page.request.post(
      new URL("/playlists", baseURL).toString(),
      {
        data: {
          name: `Sidebar Width Test ${Date.now()}`,
          showInDashboard: true,
        },
      },
    );
    const playlist = (await playlistResp.json()) as { id: string };
    await page.request.put(
      new URL(`/playlists/${playlist.id}/entries`, baseURL).toString(),
      {
        data: {
          entries: [{ type: "presentation", presentationId: presentation.id }],
        },
      },
    );
    await page.request.post(new URL("/stage/state", baseURL).toString(), {
      data: {
        presentationId: presentation.id,
        currentSlideId: slideId,
        playlistId: playlist.id,
      },
    });

    await page.goto(new URL("/stage", baseURL).toString());
    await page.setViewportSize({ width: 1920, height: 1080 });
    await page.waitForFunction(
      () => document.body.dataset.wasmReady === "true",
      { timeout: 30_000 },
    );
    await page.waitForFunction(
      () => document.body.dataset.layoutCode === "worship-pp",
      { timeout: 30_000 },
    );

    // Wait for the playlist sidebar to render with at least one entry.
    await page.waitForSelector(".stage-pp__playlist-entry", {
      timeout: 15_000,
    });

    // Read sidebar width and entry font-size from computed styles.
    const measurements = await page.evaluate(() => {
      const sidebar = document.querySelector(
        ".stage-pp__playlist-sidebar",
      ) as HTMLElement | null;
      const entry = document.querySelector(
        ".stage-pp__playlist-entry",
      ) as HTMLElement | null;
      if (!sidebar || !entry) return { error: "missing element" } as const;
      const sidebarRect = sidebar.getBoundingClientRect();
      const viewportWidth = window.innerWidth;
      const entryStyle = getComputedStyle(entry);
      return {
        sidebarRatio: sidebarRect.width / viewportWidth,
        entryFontSizePx: parseFloat(entryStyle.fontSize),
      } as const;
    });

    expect("error" in measurements, JSON.stringify(measurements)).toBe(false);
    if ("sidebarRatio" in measurements) {
      // Sidebar must be ~22% (allow ±3% slack for borders/scrollbar).
      expect(measurements.sidebarRatio).toBeGreaterThan(0.19);
      expect(measurements.sidebarRatio).toBeLessThan(0.25);
      // Floor of 70px locks in the 7.5vh × 1080p = 81px font from
      // #worship-pp-10-row-fit; any accidental drop back to 5vh
      // (≈54px) or smaller will fail the test.
      expect(measurements.entryFontSizePx).toBeGreaterThanOrEqual(70);
    }

    // Cleanup
    await page.request.delete(
      new URL(`/playlists/${playlist.id}`, baseURL).toString(),
    );

    expect(consoleErrors).toEqual([]);
  });

  test("active highlight moves to the new song when the operator triggers a different presentation", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        const t = msg.text();
        if (!t.includes("favicon") && !t.includes("crbug.com/981419")) {
          consoleErrors.push(`[${msg.type()}] ${t}`);
        }
      }
    });

    // Set worship-pp layout
    const layoutResp = await page.request.post(
      new URL("/stage/layout", baseURL).toString(),
      { data: { code: "worship-pp" } },
    );
    expect(layoutResp.ok()).toBeTruthy();

    // Find TWO presentations with at least one slide each.
    const libsResp = await page.request.get(
      new URL("/libraries", baseURL).toString(),
    );
    const libs = (await libsResp.json()) as Array<{
      id: string;
      presentations?: Array<{
        id: string;
        name?: string;
        slides?: Array<{ id: string }>;
      }>;
    }>;
    const allPres = libs
      .flatMap((lib) => lib.presentations ?? [])
      .filter((p) => (p.slides?.length ?? 0) > 0 && !!p.name);
    if (allPres.length < 2) {
      test.skip(true, "fixture has fewer than 2 presentations with slides");
      return;
    }
    const p1 = allPres[0];
    const p2 = allPres[1];
    if (
      !p1.slides ||
      !p2.slides ||
      !p1.slides[0] ||
      !p2.slides[0] ||
      !p1.name ||
      !p2.name
    ) {
      test.skip(true, "presentations missing required fields");
      return;
    }
    const p1SlideId = p1.slides[0].id;
    const p2SlideId = p2.slides[0].id;

    // Create a playlist with both presentations.
    const playlistResp = await page.request.post(
      new URL("/playlists", baseURL).toString(),
      {
        data: {
          name: `Highlight Move Test ${Date.now()}`,
          showInDashboard: true,
        },
      },
    );
    const playlist = (await playlistResp.json()) as { id: string };
    await page.request.put(
      new URL(`/playlists/${playlist.id}/entries`, baseURL).toString(),
      {
        data: {
          entries: [
            { type: "presentation", presentationId: p1.id },
            { type: "presentation", presentationId: p2.id },
          ],
        },
      },
    );

    // Trigger P1.
    const trig1 = await page.request.post(
      new URL("/stage/state", baseURL).toString(),
      {
        data: {
          presentationId: p1.id,
          currentSlideId: p1SlideId,
          playlistId: playlist.id,
        },
      },
    );
    expect(trig1.status()).toBe(204);

    // Open the stage page.
    await page.setViewportSize({ width: 1920, height: 1080 });
    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForFunction(
      () => document.body.dataset.wasmReady === "true",
      { timeout: 30_000 },
    );
    await page.waitForFunction(
      () => document.body.dataset.layoutCode === "worship-pp",
      { timeout: 30_000 },
    );

    // Wait for both rows to render.
    await page.waitForFunction(
      () => document.querySelectorAll(".stage-pp__playlist-entry").length >= 2,
      { timeout: 15_000 },
    );

    // Helper: read which row index has the active class. Returns -1 if none.
    const activeIndex = async (): Promise<number> =>
      page.evaluate(() => {
        const rows = Array.from(
          document.querySelectorAll(".stage-pp__playlist-entry"),
        );
        return rows.findIndex((r) =>
          r.classList.contains("stage-pp__playlist-entry--active"),
        );
      });

    // After triggering P1, P1's row (index 0) should be active.
    await expect.poll(activeIndex, { timeout: 10_000 }).toBe(0);

    // Now trigger P2. The highlight MUST move to row index 1.
    const trig2 = await page.request.post(
      new URL("/stage/state", baseURL).toString(),
      {
        data: {
          presentationId: p2.id,
          currentSlideId: p2SlideId,
          playlistId: playlist.id,
        },
      },
    );
    expect(trig2.status()).toBe(204);

    // Regression guard: the active class must now be on row 1, not row 0.
    await expect.poll(activeIndex, { timeout: 10_000 }).toBe(1);

    // And ensure row 0 is no longer active.
    const row0Active = await page.evaluate(() => {
      const rows = Array.from(
        document.querySelectorAll(".stage-pp__playlist-entry"),
      );
      return (
        rows[0]?.classList.contains("stage-pp__playlist-entry--active") ?? false
      );
    });
    expect(row0Active).toBe(false);

    // Cleanup
    await page.request.delete(
      new URL(`/playlists/${playlist.id}`, baseURL).toString(),
    );

    expect(consoleErrors).toEqual([]);
  });

  test("current-song badge is sourced from the playlist's active entry, and the active row auto-scrolls into view as the song advances past the visible area (#461)", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        const t = msg.text();
        if (!t.includes("favicon") && !t.includes("crbug.com/981419")) {
          consoleErrors.push(`[${msg.type()}] ${t}`);
        }
      }
    });

    // Set worship-pp layout.
    const layoutResp = await page.request.post(
      new URL("/stage/layout", baseURL).toString(),
      { data: { code: "worship-pp" } },
    );
    expect(layoutResp.ok()).toBeTruthy();

    // Collect MANY presentations with slides, with DISTINCT sanitized names so
    // the keyed <For> doesn't collide. Skip names with a 3-digit prefix so the
    // sanitized name equals the trimmed raw name (clean assertion).
    const libsResp = await page.request.get(
      new URL("/libraries", baseURL).toString(),
    );
    expect(libsResp.ok()).toBeTruthy();
    const libs = (await libsResp.json()) as Array<{
      id: string;
      presentations?: Array<{
        id: string;
        name?: string;
        slides?: Array<{ id: string }>;
      }>;
    }>;
    const seen = new Set<string>();
    const picked: Array<{ id: string; name: string; slideId: string }> = [];
    for (const lib of libs) {
      for (const p of lib.presentations ?? []) {
        const rawName = (p.name ?? "").trim();
        const firstSlide = p.slides?.[0]?.id;
        if (!rawName || !firstSlide) continue;
        if (/^\d{3}\s/.test(rawName)) continue; // would be stripped server-side
        if (seen.has(rawName)) continue;
        seen.add(rawName);
        picked.push({ id: p.id, name: rawName, slideId: firstSlide });
        if (picked.length >= 16) break;
      }
      if (picked.length >= 16) break;
    }
    // The seeded library has 1000+ presentations, so this is a hard guard, not
    // a silent skip (test-strictness): a thin fixture must fail loudly.
    expect(
      picked.length,
      `fixture needs >=14 distinct slide-bearing presentations (got ${picked.length})`,
    ).toBeGreaterThanOrEqual(14);

    // Create a playlist holding all picked presentations, in order.
    const playlistResp = await page.request.post(
      new URL("/playlists", baseURL).toString(),
      { data: { name: `Autoscroll Test ${Date.now()}`, showInDashboard: true } },
    );
    expect(playlistResp.ok()).toBeTruthy();
    const playlist = (await playlistResp.json()) as { id: string };
    const entriesResp = await page.request.put(
      new URL(`/playlists/${playlist.id}/entries`, baseURL).toString(),
      {
        data: {
          entries: picked.map((p) => ({
            type: "presentation",
            presentationId: p.id,
          })),
        },
      },
    );
    expect(entriesResp.ok()).toBeTruthy();

    const lastIdx = picked.length - 1;

    // Trigger the FIRST presentation so the early row is active.
    const trigFirst = await page.request.post(
      new URL("/stage/state", baseURL).toString(),
      {
        data: {
          presentationId: picked[0].id,
          currentSlideId: picked[0].slideId,
          playlistId: playlist.id,
        },
      },
    );
    expect(trigFirst.status()).toBe(204);

    // Open the stage at 1080p (sidebar fits ~10 rows; 14+ entries overflow).
    await page.setViewportSize({ width: 1920, height: 1080 });
    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForFunction(() => document.body.dataset.wasmReady === "true", {
      timeout: 30_000,
    });
    await page.waitForFunction(
      () => document.body.dataset.layoutCode === "worship-pp",
      { timeout: 30_000 },
    );
    await page.waitForFunction(
      (n) =>
        document.querySelectorAll(".stage-pp__playlist-entry").length >= n,
      picked.length,
      { timeout: 15_000 },
    );

    const activeIndex = async (): Promise<number> =>
      page.evaluate(() => {
        const rows = Array.from(
          document.querySelectorAll(".stage-pp__playlist-entry"),
        );
        return rows.findIndex((r) =>
          r.classList.contains("stage-pp__playlist-entry--active"),
        );
      });

    const badgeText = async (): Promise<string> =>
      (
        (await page
          .locator(".stage__current-song .stage__song-name-text")
          .textContent()) ?? ""
      ).trim();

    const activeRowText = async (): Promise<string> =>
      (
        (await page
          .locator(".stage-pp__playlist-entry--active")
          .first()
          .textContent()) ?? ""
      ).trim();

    // Row 0 active; the badge must reflect the PLAYLIST's active entry — equal
    // to both the active row's text and the triggered presentation's name.
    await expect.poll(activeIndex, { timeout: 10_000 }).toBe(0);
    await expect.poll(badgeText, { timeout: 10_000 }).toBe(picked[0].name);
    expect(await badgeText()).toBe(await activeRowText());

    // Sanity: the LAST row must currently be OUT of view (below the fold), so
    // there is genuinely something to scroll. Proves overflow exists.
    const lastRowInitiallyHidden = await page.evaluate((idx) => {
      const sidebar = document
        .querySelector(".stage-pp__playlist-sidebar")
        ?.getBoundingClientRect();
      const rows = document.querySelectorAll(".stage-pp__playlist-entry");
      const last = rows[idx]?.getBoundingClientRect();
      if (!sidebar || !last) return false;
      // Below the fold: the row's top is at/under the sidebar's bottom edge.
      return last.top >= sidebar.bottom;
    }, lastIdx);
    expect(
      lastRowInitiallyHidden,
      "last playlist row should start below the fold",
    ).toBe(true);

    // Advance to the LAST presentation (off-screen). The active row must
    // auto-scroll into view AND the badge must update to the new active song.
    const trigLast = await page.request.post(
      new URL("/stage/state", baseURL).toString(),
      {
        data: {
          presentationId: picked[lastIdx].id,
          currentSlideId: picked[lastIdx].slideId,
          playlistId: playlist.id,
        },
      },
    );
    expect(trigLast.status()).toBe(204);

    await expect.poll(activeIndex, { timeout: 10_000 }).toBe(lastIdx);
    await expect
      .poll(badgeText, { timeout: 10_000 })
      .toBe(picked[lastIdx].name);
    expect(await badgeText()).toBe(await activeRowText());

    // The active row is now scrolled INTO the sidebar's visible viewport.
    await expect
      .poll(
        async () =>
          page.evaluate(() => {
            const sidebar = document
              .querySelector(".stage-pp__playlist-sidebar")
              ?.getBoundingClientRect();
            const active = document
              .querySelector(".stage-pp__playlist-entry--active")
              ?.getBoundingClientRect();
            if (!sidebar || !active) return false;
            // Fully within the sidebar's vertical viewport (2px slack).
            return (
              active.top >= sidebar.top - 2 &&
              active.bottom <= sidebar.bottom + 2
            );
          }),
        { timeout: 10_000 },
      )
      .toBe(true);

    // Cleanup.
    await page.request.delete(
      new URL(`/playlists/${playlist.id}`, baseURL).toString(),
    );

    expect(consoleErrors).toEqual([]);
  });
});
