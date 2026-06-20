/**
 * Stage display on an EMPTY database — clean browser console (issue #383).
 *
 * Regression guard for the bug where `GET /stage/snapshot` returned HTTP 404
 * ("Stage display unavailable") when the database had no presentations. The
 * browser's network layer auto-logs the failed request as a console error
 * ("Failed to load resource: the server responded with a status of 404"),
 * violating the browser-console-zero-errors rule — even though the WASM client
 * swallows the `Err` gracefully. The fix makes the no-presentation / no-default
 * case return a 200 with an empty snapshot, so the console stays clean.
 *
 * CRITICAL: this spec deliberately does NOT call refreshDevData — it boots a
 * server against a FRESH, EMPTY database (migrations run, but no libraries /
 * presentations are imported). That empty-DB state is exactly what triggered
 * the 404 in production on a freshly-provisioned instance.
 */

import { test, expect } from "@playwright/test";
import { promises as fsp } from "fs";
import path from "path";
import {
  deriveTestConfig,
  startTestServer,
  stopServer,
  REPO_ROOT,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;

  // Use a dedicated, guaranteed-empty DB for this spec. The shared per-worker
  // DB (deriveTestConfig) is reused across runs and may already contain seeded
  // data, which would mask the empty-DB bug. Delete any stale file + sidecars
  // first so the server boots against a truly empty database.
  const emptyDbPath = path.join(
    REPO_ROOT,
    "var",
    "tmp",
    `presenter_e2e_stage_empty_${config.workerIndex}.db`,
  );
  for (const suffix of ["", "-shm", "-wal"]) {
    await fsp.rm(`${emptyDbPath}${suffix}`, { force: true });
  }
  await fsp.mkdir(path.dirname(emptyDbPath), { recursive: true });
  // `?mode=rwc` so SQLite CREATES the file on first connect — unlike the
  // seeded specs, nothing (refreshDevData / importer) creates it beforehand.
  const emptyDbUrl = `sqlite://${emptyDbPath}?mode=rwc`;

  serverHandle = await startTestServer(config.port, emptyDbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

test("stage page on empty DB has a clean console (no 404 for /stage/snapshot)", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      // crbug.com/981419: some local Chromium builds emit an `integrity`
      // preload warning for the trunk-generated <link rel=preload as=fetch
      // type=wasm integrity=...> tag. It is purely a browser-version artifact
      // (absent on the CI runner's Chromium) and unrelated to issue #383's
      // server-side 404. Ignore ONLY that exact warning so this regression
      // guard stays reliable in both CI and local dev while still catching the
      // 404. The guard is narrow (type===warning AND both tokens) so it cannot
      // mask an unrelated real error — the #383 404 contains neither token.
      const isIntegrityPreloadWarning =
        msg.type() === "warning" &&
        text.includes("crbug.com/981419") &&
        text.includes("integrity");
      if (isIntegrityPreloadWarning) {
        return;
      }
      consoleMessages.push(`[${msg.type()}] ${text}`);
    }
  });
  page.on("pageerror", (err) => {
    consoleMessages.push(`[pageerror] ${err.message}`);
  });

  await page.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForFunction(
    () =>
      (window as unknown as { __presenterStageConnectionState?: string })
        .__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );

  // Let any async fetches (the initial /stage/snapshot load) settle so a 404
  // network error has time to surface in the console.
  await page.waitForTimeout(3_000);

  expect(consoleMessages).toEqual([]);
});
