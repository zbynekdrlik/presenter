/**
 * Favicon E2E (#361).
 *
 * Browsers automatically request `/favicon.ico` on every navigation. Before the
 * fix this returned 404, logging a console error on every page (operator,
 * tablet, stage, bible, landing) — which also masked any *real* console error a
 * reviewer would otherwise notice.
 *
 * This test loads the plain landing page (no WASM, so no benign Chromium WASM
 * warnings to filter) and asserts:
 *   1. `/favicon.ico` responds 200 with an image content-type.
 *   2. Zero browser console errors/warnings on the route — locking in the
 *      "zero console errors" invariant per ci/browser-console-zero-errors.md.
 */

import { test, expect } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
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
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

test("favicon.ico is served (no 404) and the landing page console is clean", async ({
  page,
}) => {
  const consoleErrors: string[] = [];
  attachConsoleErrorCollector(page, consoleErrors);

  // The favicon request fires automatically as part of loading the document.
  await page.goto(`${baseURL}/`, { waitUntil: "networkidle" });

  // Directly assert the favicon route is healthy.
  const faviconResponse = await page.request.get(`${baseURL}/favicon.ico`);
  expect(faviconResponse.status()).toBe(200);
  expect(faviconResponse.headers()["content-type"] ?? "").toMatch(/^image\//);

  // No favicon 404 (or anything else) in the console.
  expect(consoleErrors).toEqual([]);
});
