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
          entries: [
            { type: "presentation", presentationId: presentation.id },
          ],
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
        inactive: inactive
          ? getComputedStyle(inactive).backgroundColor
          : null,
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
});
