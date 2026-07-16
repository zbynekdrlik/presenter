/**
 * Operator startup restore with a STALE `activePlaylistId` — clean console
 * (issue #561).
 *
 * Regression guard for the bug where the operator's startup session-restore
 * blindly fetched `GET /playlists/<id>` for whatever id sessionStorage had
 * stored, even when that playlist no longer exists (deleted since the last
 * visit, or the dev DB was swapped/refreshed). The browser's OWN network
 * layer auto-logs the failed request as a console error ("Failed to load
 * resource: the server responded with a status of 404"), violating the
 * browser-console-zero-errors rule — on EVERY reload, since the stale key is
 * never cleared. The fix checks the already-fetched playlist LIST for
 * membership before ever issuing a per-id fetch; when the id is absent it
 * removes the stale `presenter:activePlaylistId` sessionStorage key and
 * starts with no active playlist instead of firing the doomed request.
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
let baseURL = "";

/** A REAL playlist that exists in the DB — distinct from the bogus stale id. */
let realPlaylistId: string;
let realPlaylistName: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);

  realPlaylistName = "_E2E Stale Restore Playlist";
  // showInDashboard: true — the dashboard panel only lists playlists that
  // are either dashboard-favorited OR currently the active selection
  // (playlist_list.rs). Our stale scenario's active id never matches a real
  // playlist, so a non-favorited playlist would stay hidden behind the
  // "Show all playlists" modal; favoriting it keeps this test's assertions
  // against the always-visible dashboard list.
  const resp = await fetch(new URL("/playlists", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: realPlaylistName, showInDashboard: true }),
  });
  const playlist = await resp.json();
  realPlaylistId = playlist.id;
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("stale activePlaylistId is cleared silently — zero console errors, app stays functional", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    const text = msg.text();
    // crbug.com/981419: a Chromium-version-specific `integrity` preload
    // warning for the trunk-generated wasm preload tag — unrelated artifact,
    // same narrow allowance used by stage-empty-db-console.spec.ts (#383).
    if (
      msg.type() === "warning" &&
      text.includes("crbug.com/981419") &&
      text.includes("integrity")
    ) {
      return;
    }
    consoleMessages.push(`[${msg.type()}] ${text}`);
  });
  page.on("pageerror", (err) => {
    consoleMessages.push(`[pageerror] ${err.message}`);
  });

  // Bogus playlist id that does NOT exist in the DB — simulates a deleted
  // playlist / DB swap leaving a dead pointer in sessionStorage.
  const staleId = "7fb85548-f24e-4c90-837f-0af85c6203c1";

  await page.goto(new URL("/ui/operator", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  // Seed sessionStorage with gloo-storage-compatible format (JSON-encoded
  // string value, "presenter:" key prefix) so WASM reads it on init, exactly
  // as it would after a real deleted-playlist / DB-swap scenario.
  await page.evaluate((id) => {
    sessionStorage.setItem("presenter:activePlaylistId", JSON.stringify(id));
  }, staleId);
  await page.reload({ waitUntil: "domcontentloaded" });

  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('[data-role="playlist-list"]', {
    state: "visible",
    timeout: 30_000,
  });

  // Let any async fetches (the startup session-restore) settle so a 404
  // network error would have time to surface in the console.
  await page.waitForTimeout(3_000);

  // App stays fully functional: the real playlist list loaded and renders,
  // proving startup wasn't derailed by the stale id.
  await expect(
    page.locator('[data-role="playlist-item"]', { hasText: realPlaylistName }),
  ).toBeVisible();

  // Primary regression guard: the stale id must never surface a browser
  // "Failed to load resource: ... 404" console entry.
  expect(consoleMessages).toEqual([]);

  // The stale key is cleared, not left to repeat the failure on every reload.
  const remainingKey = await page.evaluate(() =>
    sessionStorage.getItem("presenter:activePlaylistId"),
  );
  expect(remainingKey).toBeNull();

  // Functional check: clicking the REAL playlist still works normally after
  // a stale-id startup — selecting it updates the context title.
  await page
    .locator('[data-role="playlist-item"]', { hasText: realPlaylistName })
    .click();
  await expect(page.locator('[data-role="context-title"]')).toHaveText(
    realPlaylistName,
    { timeout: 10_000 },
  );
});
