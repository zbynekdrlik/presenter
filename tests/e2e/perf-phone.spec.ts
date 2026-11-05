/**
 * #850 S8 — a cold console on a slow phone (spec §10 `@perf-phone`). Timing on
 * the shared CI runner is load-sensitive, so this runs ONLY from the
 * on-demand `ui-perf.yml` (the #386 ndi-latency pattern) and after a deploy
 * against prod (`PERF_BASE_URL`). Throttling: 9 / 1.5 Mbps, 150 ms RTT, CPU ×4.
 */
import { test, expect } from "@playwright/test";
import { PHONE_390 } from "./support";

const BASE_URL = process.env.PERF_BASE_URL ?? "";
const BUDGET_MS = Number(process.env.PERF_BUDGET_MS ?? "3500");

test("@perf-phone a cold console is usable within the budget and a keystroke shows its row in 50 ms", async ({
  browser,
}) => {
  // Never a skip (test strictness): without a target this fails loudly.
  expect(BASE_URL, "PERF_BASE_URL must name the instance (ui-perf.yml sets it)").not.toBe("");
  const context = await browser.newContext({ ...PHONE_390, baseURL: BASE_URL });
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  await cdp.send("Network.enable");
  await cdp.send("Network.emulateNetworkConditions", {
    offline: false,
    latency: 150,
    downloadThroughput: 9_000_000 / 8,
    uploadThroughput: 1_500_000 / 8,
  });
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  try {
    const started = Date.now();
    await page.goto("/ui/operator/bible");
    await page.locator('[data-role="console-field"]').waitFor({ state: "visible", timeout: 30_000 });
    await expect(page.locator('[data-role="book-list"]')).toHaveAttribute("data-books-translation", /.+/);
    const usableMs = Date.now() - started;
    console.log(`[perf-phone] ${BASE_URL}: console usable after ${usableMs} ms (budget ${BUDGET_MS})`);
    expect(usableMs).toBeLessThanOrEqual(BUDGET_MS);

    const rowMs = await page.evaluate(async () => {
      const field = document.querySelector<HTMLInputElement>('[data-role="console-field"]');
      if (!field) throw new Error("no console field");
      const t0 = performance.now();
      field.value = "zid 11 1";
      field.dispatchEvent(new Event("input", { bubbles: true }));
      await new Promise<void>((resolve) => {
        const check = () =>
          document.querySelector('[data-role="console-row"]') ? resolve() : requestAnimationFrame(check);
        check();
      });
      return performance.now() - t0;
    });
    console.log(`[perf-phone] keystroke → row ${rowMs.toFixed(1)} ms`);
    expect(rowMs).toBeLessThanOrEqual(50);
  } finally {
    await context.close();
  }
});
