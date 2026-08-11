/**
 * #574: an operator tab left open across a deploy must recover on its own.
 *
 * Incident (2026-07-24, SNV prod): the morning deploy restarted
 * `presenter.service` under an operator tab open since before the deploy.
 * The stale in-tab WASM app rendered the worship left panel WITHOUT library
 * names until a manual reload. Fix: (1) the operator polls /healthz
 * periodically and shows a one-click "reload" banner on a version mismatch
 * against the version this tab actually loaded with, and (2) on WS
 * reconnect (a server restart always drops the socket) it re-fetches
 * libraries/playlists/presentations so a change that happened while
 * disconnected is never left stale/blank.
 *
 * Each test manages its OWN dedicated server instance (stopped/restarted
 * mid-test), separate from the shared-server pattern other spec files use.
 */

import { test, expect } from "@playwright/test";
import { spawnSync } from "child_process";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

test("shows a one-click reload banner when the server version changes under an open tab (#574)", async ({
  page,
}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  await refreshDevData(config.dbUrl);
  const server = await startTestServer(config.port, config.dbUrl);
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  try {
    // Pass the REAL /healthz response through until THIS tab's own baseline
    // is actually captured (`__presenterVersionBaselineKnown()`), then start
    // reporting a DIFFERENT version — simulating a deploy that happened
    // while this tab stayed open. Gated on the app's OWN state (#640), not a
    // fixed wall-clock delay: WASM init + the first fetch are each budgeted
    // up to 30s, so a hardcoded few-second grace window can capture the
    // ALREADY-simulated value as the baseline on a slow boot and never
    // observe a mismatch at all.
    let simulateNewVersion = false;
    await page.route("**/healthz", async (route) => {
      const response = await route.fetch();
      if (!simulateNewVersion) {
        await route.fulfill({ response });
        return;
      }
      const body = await response.json();
      body.version = `${body.version}-simulated-newer`;
      await route.fulfill({ response, json: body });
    });

    await page.goto(`${config.baseURL}/ui/operator`);
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });
    await page.waitForSelector('[data-role="library-item"]', {
      timeout: 30_000,
    });

    // No banner before the baseline is even known.
    await expect(
      page.locator('[data-role="version-update-banner"]'),
    ).toHaveCount(0);

    await page.waitForFunction(
      () => (window as any).__presenterVersionBaselineKnown?.() === true,
      { timeout: 30_000 },
    );
    simulateNewVersion = true;

    // The periodic poll must observe the mismatch and show the banner.
    const banner = page.locator('[data-role="version-update-banner"]');
    await expect(banner).toBeVisible({ timeout: 30_000 });

    // #640: the banner must PUSH the header down, never overlap it — it
    // used to sit `position: fixed` above everything with no offset applied
    // for the sticky header's own height.
    const header = page.locator(".operator__header");
    const bannerBox = await banner.boundingBox();
    const headerBox = await header.boundingBox();
    expect(bannerBox).not.toBeNull();
    expect(headerBox).not.toBeNull();
    if (bannerBox && headerBox) {
      expect(bannerBox.y + bannerBox.height).toBeLessThanOrEqual(headerBox.y);
    }

    // Clicking Reload actually reloads the page (one-click recovery).
    const navigated = page.waitForEvent("load");
    await page.locator('[data-role="version-update-reload"]').click();
    await navigated;

    expect(consoleMessages).toEqual([]);
  } finally {
    await stopServer(server);
  }
});

test("self-heals version-drift detection even when the tab's first /healthz calls fail (#640)", async ({
  page,
}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  await refreshDevData(config.dbUrl);
  const server = await startTestServer(config.port, config.dbUrl);
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  try {
    // The operator page fires exactly two independent /healthz calls at
    // mount (the header badge + the shared VersionLabel footer). Fail BOTH
    // unconditionally by REQUEST COUNT (never wall clock — that would just
    // reintroduce the #640 F3 race) to reproduce a transient failure during
    // boot. A malformed 200 (not an abort / non-2xx status) reproduces a
    // fetch that fails to PARSE without adding a browser-console
    // "Failed to load resource" entry — get_json() funnels both failure
    // shapes into the same Err path (see the ui skill's #598 note).
    let healthzCalls = 0;
    await page.route("**/healthz", async (route) => {
      healthzCalls += 1;
      if (healthzCalls <= 2) {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: "not-json",
        });
        return;
      }
      await route.continue();
    });

    await page.goto(`${config.baseURL}/ui/operator`);
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    // Before #640: `known_version` was set ONLY by the (now-failed) mount
    // fetch, so this would time out forever. The fix lets the periodic 15s
    // poll self-heal the baseline on its own next successful call.
    await page.waitForFunction(
      () => (window as any).__presenterVersionBaselineKnown?.() === true,
      { timeout: 30_000 },
    );

    // Prove the self-healed baseline actually WORKS for drift detection —
    // not just that some baseline exists.
    await page.route("**/healthz", async (route) => {
      const response = await route.fetch();
      const body = await response.json();
      body.version = `${body.version}-simulated-newer`;
      await route.fulfill({ response, json: body });
    });

    await expect(
      page.locator('[data-role="version-update-banner"]'),
    ).toBeVisible({ timeout: 30_000 });

    expect(consoleMessages).toEqual([]);
  } finally {
    await stopServer(server);
  }
});

test("WS reconnect after a server restart refreshes libraries without a page reload (#574)", async ({
  page,
}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  await refreshDevData(config.dbUrl);
  let server: ServerHandle | undefined = await startTestServer(
    config.port,
    config.dbUrl,
  );

  try {
    const createResp = await page.request.post(
      new URL("/libraries", config.baseURL).toString(),
      { data: { name: `WsReconnect${Date.now()}` } },
    );
    expect(createResp.ok()).toBeTruthy();
    const library: { id: string; name: string } = await createResp.json();

    await page.goto(`${config.baseURL}/ui/operator`);
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });
    await page.waitForFunction(
      () => (window as any).__presenterLiveConnected?.() === true,
      { timeout: 15_000 },
    );

    // Kill the server — the tab's WS drops for real (not simulated).
    await stopServer(server);
    server = undefined;
    await page.waitForFunction(
      () => (window as any).__presenterLiveConnected?.() === false,
      { timeout: 15_000 },
    );

    // While the server is down, rename the library DIRECTLY in the sqlite
    // file — this change can never reach the tab via a live broadcast
    // (nothing is running to send one). Only a post-reconnect REFETCH can
    // ever show it; stale locally-retained state would keep the OLD name.
    const dbPath = config.dbUrl.replace(/^sqlite:\/\//, "");
    const renamedTo = `WsReconnectRenamed${Date.now()}`;
    const sql = spawnSync("sqlite3", [
      dbPath,
      `UPDATE libraries SET name = '${renamedTo}', search_name = '${renamedTo.toLowerCase()}' WHERE id = '${library.id}';`,
    ]);
    expect(sql.status, sql.stderr.toString()).toBe(0);

    // Restart the SAME server on the SAME port + DB file — the client's
    // own reconnect loop (2s delay, see crates/presenter-ui/src/ws/mod.rs)
    // picks it back up with NO page reload.
    server = await startTestServer(config.port, config.dbUrl);
    await page.waitForFunction(
      () => (window as any).__presenterLiveConnected?.() === true,
      { timeout: 15_000 },
    );

    // The renamed library must show up WITHOUT a page reload.
    await page.locator('[data-role="library-more"]').click();
    await expect(
      page.locator(
        `[data-role="library-row"][data-library-id="${library.id}"] .operator__list-label`,
      ),
    ).toHaveText(renamedTo, { timeout: 15_000 });
  } finally {
    await stopServer(server);
  }
});
