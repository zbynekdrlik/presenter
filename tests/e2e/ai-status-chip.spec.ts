/**
 * E2E for #598 / #762: the operator-header AI connection indicator.
 *
 * Since #762 removed the bundled CLIProxyAPI proxy + Claude OAuth, the chip is
 * backend-agnostic — it reads only the flat `connected`/`error` fields of
 * `/ai/status` (no nested `proxy` object, no Claude login state). Three states:
 * `ok`, `unavailable`, and `checking`. Every scenario drives `/ai/status` via
 * `page.route` (the real backend verdict depends on OpenRouter reachability,
 * not something a test should depend on), the same technique
 * `operator-version-recovery.spec.ts` uses for `/healthz`.
 *
 * #598 gotcha: never mock a NON-2xx response in a zero-console spec — Chrome
 * itself logs a "Failed to load resource" console error for any non-2xx fetch,
 * which the zero-console assertion (rightly) treats as a bug. `unavailable` is
 * a well-formed 200 body with `connected:false`; a POLL FAILURE is simulated
 * with a malformed 200 body (client-side deserialize error, clean console).
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

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

/** Mock the backend-agnostic `/ai/status` payload (well-formed 200). */
async function mockAiStatus(
  page: Page,
  status: { connected: boolean; error?: string | null },
) {
  await page.route("**/ai/status", async (route) => {
    await route.fulfill({
      json: {
        connected: status.connected,
        error: status.error ?? null,
        modelValid: true,
      },
    });
  });
}

test("mounted in the top brand row, never next to Stage Output", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { connected: true });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  await expect(page.locator('[data-role="ai-status-chip"]')).toHaveCount(1);
  await expect(
    page.locator('.operator__header-brand [data-role="ai-status-chip"]'),
  ).toHaveCount(1);
  await expect(
    page.locator('.operator__header-right [data-role="ai-status-chip"]'),
  ).toHaveCount(0);

  expect(consoleMessages).toEqual([]);
});

test("connected shows the ok state and links to the AI panel", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { connected: true });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "ok", { timeout: 30_000 });
  await expect(chip).toHaveText("AI: pripojené");
  await expect(chip).toHaveAttribute("href", "/ui/operator/ai");
  await expect(chip).toHaveAttribute("title", /pripojená/);

  expect(consoleMessages).toEqual([]);
});

test("a failed last call (fold window) shows the yellow last-call-failed state with the reason in the tooltip", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  // #764: a metered backend that 200s /models but 403s completions (exhausted
  // budget) reports connected:false with the reason in `error`. A well-formed
  // 200 body, so the zero-console assertion still holds.
  // #784: the `posledné AI volanie zlyhalo:` prefix marks a FOLD-WINDOW failure
  // (one request failed in the last 15 min) — the chip must NOT claim the AI is
  // down; that framing is reserved for a probe failure (next test).
  await mockAiStatus(page, {
    connected: false,
    error: "posledné AI volanie zlyhalo: Workspace daily budget exceeded",
  });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "last_call_failed", { timeout: 30_000 });
  await expect(chip).toHaveText("AI: posledné volanie zlyhalo");
  await expect(chip).toHaveAttribute("title", /budget/);

  expect(consoleMessages).toEqual([]);
});

test("clicking the chip navigates straight to the AI panel", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await mockAiStatus(page, { connected: false, error: "AI backend unreachable" });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "unavailable", { timeout: 30_000 });
  await chip.click();
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  await expect(page).toHaveURL(/\/ui\/operator\/ai$/);
  const aiButton = page.locator('[data-role="view-toggle"][data-view="ai"]');
  await expect(aiButton).toHaveAttribute("data-active", "true");

  expect(consoleMessages).toEqual([]);
});

test("a failed poll shows the neutral checking state, never a false failure claim, with a clean console", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);

  // #622 post-merge review finding 4: prove an actual OK -> checking TRANSITION
  // caused by real poll failures — start connected (chip reaches "ok"), then
  // break the route and wait for >=2 poll ticks (5s interval,
  // STALE_AFTER_FAILURES=2) so the chip is FORCED to fall back to "checking".
  //
  // A non-2xx status here would make Chrome itself log a "Failed to load
  // resource" console error (browser-generated, unavoidable), so "mock a 500"
  // + "assert zero console" can never pass. A malformed 200 body exercises the
  // exact same client failure branch — `check_status()` returns
  // `Err(ApiError::Deserialize(..))`, handled identically — with a clean console.
  await mockAiStatus(page, { connected: true });

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForLoadState("networkidle");

  const chip = page.locator('[data-role="ai-status-chip"]');
  await expect(chip).toHaveAttribute("data-state", "ok", { timeout: 30_000 });

  await page.unroute("**/ai/status");
  await page.route("**/ai/status", async (route) => {
    await route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
  });

  // AI_STATUS_REFRESH_MS = 5_000, STALE_AFTER_FAILURES = 2 — two consecutive
  // failed ticks land at ~10s; wait well past that for the transition.
  await expect(chip).toHaveAttribute("data-state", "checking", { timeout: 20_000 });
  await expect(chip).toHaveText("AI: kontrolujem…");

  expect(consoleMessages).toEqual([]);
});
