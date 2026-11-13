/**
 * #944 (owner, 2026-11-13, SNV desktop operator): the Piesne left column gave
 * Knižnice the room and Playlisty ONE row — one resizer sized both together
 * and the library list, having more rows, kept the space. Each section now has
 * its own size, remembered per browser:
 *
 * - by default the libraries (the least used part) are small and the playlists
 *   show at least three rows, with SNV-like data (6 libraries, 4 playlists);
 * - a handle between Knižnice and Playlisty sizes the libraries, the handle
 *   under Playlisty sizes the playlists; both survive a reload;
 * - a size stored by an older build (`catalogTopHeight`) never leaves the
 *   playlists at one row again.
 *
 * Also asserts zero browser console errors/warnings per
 * ci/browser-console-zero-errors.md.
 */

import { test, expect, type Page } from "@playwright/test";
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

async function send(method: string, path: string, body?: unknown): Promise<any> {
  const resp = await fetch(new URL(path, baseURL).toString(), {
    method,
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await resp.text();
  expect(resp.ok, `${method} ${path} → ${resp.status} ${text.slice(0, 200)}`).toBeTruthy();
  return text ? JSON.parse(text) : null;
}

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);

  // SNV-like: six favourite libraries (the list shows the favourites), four
  // playlists on the dashboard.
  const favorites: { ids: string[] } = await send("GET", "/libraries/favorites");
  for (let i = favorites.ids.length; i < 6; i++) {
    const library = await send("POST", "/libraries", { name: `_E2E Knižnica ${i + 1}` });
    await send("POST", `/libraries/${library.id}/favorite`, { favorite: true });
  }
  const playlists: { showInDashboard?: boolean }[] = await send("GET", "/playlists");
  for (let i = playlists.filter((p) => p.showInDashboard).length; i < 4; i++) {
    await send("POST", "/playlists", { name: `_E2E Playlist ${i + 1}`, showInDashboard: true });
  }
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

const LIBRARY_LIST = '[data-view-panel="worship"] [data-role="library-list"]';
const PLAYLIST_LIST = '[data-view-panel="worship"] [data-role="playlist-list"]';

async function open(page: Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
  await expect(page.locator("body")).toHaveAttribute("data-shell", "desktop");
  await expect(page.locator(`${LIBRARY_LIST} .operator__list-row`)).toHaveCount(6);
  await expect(page.locator(`${PLAYLIST_LIST} .operator__list-row`)).toHaveCount(4);
}

/** The list's height and how many of its rows are WHOLLY visible in it. */
async function section(page: Page, selector: string): Promise<{ height: number; rows: number }> {
  return page.evaluate((selector) => {
    const list = document.querySelector(selector);
    if (!list) return { height: -1, rows: -1 };
    const box = list.getBoundingClientRect();
    const rows = Array.from(list.querySelectorAll(".operator__list-row")).filter((row) => {
      const r = row.getBoundingClientRect();
      return r.height > 0 && r.top >= box.top - 1 && r.bottom <= box.bottom + 1;
    }).length;
    return { height: Math.round(box.height), rows };
  }, selector);
}

/** Drags a resize handle vertically by `dy` px with the mouse. */
async function drag(page: Page, role: string, dy: number) {
  const handle = page.locator(`[data-view-panel="worship"] [data-role="${role}"]`);
  await expect(handle).toBeVisible();
  const box = await handle.boundingBox();
  if (!box) throw new Error(`${role} has no box`);
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y + dy / 2, { steps: 4 });
  await page.mouse.move(x, y + dy, { steps: 4 });
  await page.mouse.up();
}

for (const viewport of [
  { width: 1280, height: 720 },
  { width: 1920, height: 1080 },
]) {
  test(`default: playlists show at least three rows, libraries stay small at ${viewport.width}×${viewport.height}`, async ({
    page,
  }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    await page.setViewportSize(viewport);
    await open(page);

    const libraries = await section(page, LIBRARY_LIST);
    const playlists = await section(page, PLAYLIST_LIST);
    expect(playlists.rows, "playlist rows wholly visible").toBeGreaterThanOrEqual(3);
    expect(libraries.rows, "library rows: at least one").toBeGreaterThanOrEqual(1);
    expect(libraries.rows, "libraries are the small section (6 exist)").toBeLessThanOrEqual(3);
    // The song list keeps its rows under them.
    const songs = await page.locator('[data-view-panel="worship"] [data-role="catalog-bottom"]').boundingBox();
    expect(songs?.height ?? 0, "song list").toBeGreaterThanOrEqual(100);

    expect(errors).toEqual([]);
  });
}

test("each section resizes on its own and keeps its size after a reload", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.setViewportSize({ width: 1920, height: 1080 });
  await open(page);

  const libsBefore = await section(page, LIBRARY_LIST);
  const plsBefore = await section(page, PLAYLIST_LIST);

  // Libraries: the handle between Knižnice and Playlisty, 34 px = one row down.
  await drag(page, "library-resizer", 34);
  const libsAfter = await section(page, LIBRARY_LIST);
  expect(Math.abs(libsAfter.height - (libsBefore.height + 34)), "libraries grew by the drag").toBeLessThanOrEqual(2);
  expect(libsAfter.rows).toBe(libsBefore.rows + 1);
  const plsUnchanged = await section(page, PLAYLIST_LIST);
  expect(plsUnchanged.height, "the playlists keep their size").toBe(plsBefore.height);

  // Playlists: the handle under Playlisty, one row up.
  await drag(page, "catalog-resizer", -34);
  const plsAfter = await section(page, PLAYLIST_LIST);
  expect(Math.abs(plsAfter.height - (plsBefore.height - 34)), "playlists shrank by the drag").toBeLessThanOrEqual(2);
  expect((await section(page, LIBRARY_LIST)).height, "the libraries keep their size").toBe(libsAfter.height);

  // Remembered by this browser.
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
  await expect(page.locator(`${PLAYLIST_LIST} .operator__list-row`)).toHaveCount(4);
  await expect(page.locator(`${LIBRARY_LIST} .operator__list-row`)).toHaveCount(6);
  expect(Math.abs((await section(page, LIBRARY_LIST)).height - libsAfter.height), "libraries after reload").toBeLessThanOrEqual(1);
  expect(Math.abs((await section(page, PLAYLIST_LIST)).height - plsAfter.height), "playlists after reload").toBeLessThanOrEqual(1);

  expect(errors).toEqual([]);
});

test("a size stored by an older build does not leave the playlists at one row", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.setViewportSize({ width: 1280, height: 720 });
  // The value the old single resizer stored (its minimum, 200 px, for both lists).
  await page.addInitScript(() => {
    localStorage.setItem("presenter:catalogTopHeight", JSON.stringify("200"));
  });
  await open(page);

  expect((await section(page, PLAYLIST_LIST)).rows, "playlist rows wholly visible").toBeGreaterThanOrEqual(3);

  expect(errors).toEqual([]);
});
