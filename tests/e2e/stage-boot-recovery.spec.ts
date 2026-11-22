/**
 * A failed or stuck UI start reloads the page by itself (#990).
 *
 * SNV SD2, 2026-11-21: the stage TV loaded `/stage` at boot, the 1.1 MB wasm
 * download got `net::ERR_CONNECTION_RESET`, Trunk's inline loader rejected
 * (`WebAssembly compilation aborted: Network error`) and nothing ever retried —
 * a dark display until an operator power-cycled it.
 *
 * The fix is an inline boot guard in `crates/presenter-ui/index.html`, placed
 * before Trunk's loader: an unhandled rejection / error before
 * `body[data-wasm-ready="true"]`, or no ready within 20 s, reloads the page
 * after a back-off (5 s, doubling to 60 s; the attempt count lives in
 * sessionStorage and is cleared once the app is ready).
 *
 * Console: the FAILED attempt legitimately logs the network error, the
 * rejected loader promise and the guard's own single warning — the test
 * records them per page load and asserts exactly that shape, then asserts the
 * RECOVERED load has a clean console (browser-console-zero-errors rule).
 */

import { test, expect, type Page, type Route } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

const WASM_ROUTE = /\/ui-pkg\/presenter-ui-[^/]*_bg\.wasm/;
const ATTEMPTS_KEY = "presenter.bootAttempts";
const GUARD_WARNING = /presenter boot failed .*#990/;

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

/** Console errors/warnings + page errors, grouped by main-frame page load. */
type LoadLog = { url: string; messages: string[] };

function recordConsolePerLoad(page: Page): LoadLog[] {
  const loads: LoadLog[] = [];
  page.on("framenavigated", (frame) => {
    if (frame === page.mainFrame()) loads.push({ url: frame.url(), messages: [] });
  });
  const push = (line: string) => {
    if (loads.length === 0) loads.push({ url: "(before first load)", messages: [] });
    loads[loads.length - 1].messages.push(line);
  };
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (err) => push(`[pageerror] ${err.message}`));
  return loads;
}

async function readAttempts(page: Page): Promise<string | null> {
  return page.evaluate((key) => window.sessionStorage.getItem(key), ATTEMPTS_KEY);
}

test.describe("Boot guard: a failed UI start recovers by itself (#990)", () => {
  test("a reset wasm download reloads the stage page and it comes up", async ({ page }) => {
    const loads = recordConsolePerLoad(page);
    let wasmRequests = 0;
    await page.route(WASM_ROUTE, async (route: Route) => {
      wasmRequests += 1;
      if (wasmRequests === 1) {
        // The SD2 failure: the connection is reset during the wasm download.
        await route.abort("connectionreset");
        return;
      }
      await route.continue();
    });

    await page.goto(`${baseURL}/stage`);
    // 5 s back-off + a normal start; well inside 30 s.
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

    expect(wasmRequests, "the wasm is requested again after the reload").toBe(2);
    expect(loads.length, "exactly one reload: the failed load + the recovered one").toBe(2);
    // The stage really works after the recovery: its live socket is up.
    await expect(page.locator(".stage__connection")).toContainText("CONNECTED");
    expect(await readAttempts(page), "the attempt count is cleared once ready").toBeNull();

    // The failed attempt: exactly ONE guard warning, plus only the failure it
    // reacted to (the reset request and the rejected loader promise).
    const failed = loads[0].messages;
    const guardWarnings = failed.filter((m) => m.startsWith("[warning]") && GUARD_WARNING.test(m));
    expect(guardWarnings, `failed load console: ${JSON.stringify(failed)}`).toHaveLength(1);
    const unexpected = failed.filter(
      (m) =>
        !GUARD_WARNING.test(m) &&
        !/ERR_CONNECTION_RESET|WebAssembly|Failed to load resource|Failed to fetch|NetworkError|network error/i.test(
          m,
        ),
    );
    expect(unexpected, "only the wasm failure + the guard's warning on the failed load").toEqual([]);

    // The recovered load: a clean console.
    await page.waitForTimeout(1_000);
    expect(loads[1].messages).toEqual([]);
  });

  test("a wasm download that never finishes reloads after the ready timeout", async ({ page }) => {
    const loads = recordConsolePerLoad(page);
    let wasmRequests = 0;
    let held: Route | undefined;
    await page.route(WASM_ROUTE, async (route: Route) => {
      wasmRequests += 1;
      if (wasmRequests === 1) {
        // Stuck: the response never comes (released in the finally below).
        held = route;
        return;
      }
      await route.continue();
    });

    try {
      await page.goto(`${baseURL}/stage`, { waitUntil: "commit" });
      // 20 s ready timeout + 5 s back-off + a normal start.
      await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 60_000 });
    } finally {
      await held?.abort("aborted").catch(() => undefined);
    }

    expect(wasmRequests).toBe(2);
    expect(loads.length).toBe(2);
    await expect(page.locator(".stage__connection")).toContainText("CONNECTED");
    expect(await readAttempts(page)).toBeNull();
    const failed = loads[0].messages;
    expect(failed, `failed load console: ${JSON.stringify(failed)}`).toHaveLength(1);
    expect(failed[0]).toMatch(/^\[warning\] presenter boot failed \(not ready in 20 s\).*#990/);

    await page.waitForTimeout(1_000);
    expect(loads[1].messages).toEqual([]);
  });

  test("a normal start logs nothing and leaves no attempt count", async ({ page }) => {
    const loads = recordConsolePerLoad(page);
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(page.locator('[data-role="library-list"]')).toBeVisible();
    expect(await readAttempts(page)).toBeNull();
    await page.waitForTimeout(1_000);
    expect(loads.length).toBe(1);
    expect(loads[0].messages).toEqual([]);
  });
});
