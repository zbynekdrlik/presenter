import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL: string;
test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(
    config.port,
    config.dbUrl,
    config.oscPort,
  );
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("tablet timer bar shows clock and responds to preach timer", async ({
  page,
  request,
}) => {
  // Wait for server readiness
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  // Collect console errors
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Navigate to tablet
  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // --- Timer bar should be visible with clock ---
  const timerBar = page.locator('[data-role="timer-bar"]');
  await expect(timerBar).toBeVisible({ timeout: 5_000 });

  // Clock should show HH:MM format
  const clock = page.locator('[data-role="timer-clock"]');
  await expect(clock).toHaveText(/^\d{2}:\d{2}$/, { timeout: 5_000 });

  // Elapsed should show em-dash when idle
  const elapsed = page.locator('[data-role="timer-elapsed"]');
  await expect(elapsed).toHaveText("—", { timeout: 5_000 });

  // State should show IDLE
  const state = page.locator('[data-role="timer-state"]');
  await expect(state).toHaveText("IDLE", { timeout: 5_000 });

  // Zone should be neutral
  await expect(timerBar).toHaveAttribute("data-zone", "neutral");

  // --- Start preach timer via API ---
  const startResponse = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: { command: "start_preach" },
      headers: { "Content-Type": "application/json" },
      timeout: 10_000,
    },
  );
  expect(startResponse.ok()).toBeTruthy();

  // Elapsed should update to show a time value (not em-dash)
  await expect(async () => {
    const text = await elapsed.textContent();
    expect(text).toMatch(/^\d+:\d{2}$/);
  }).toPass({ timeout: 10_000, intervals: [500] });

  // State should show RUNNING
  await expect(state).toHaveText("RUNNING", { timeout: 5_000 });

  // --- Set preach limit and verify color zone ---
  const limitResponse = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: { command: "set_preach_limit", seconds: 3 },
      headers: { "Content-Type": "application/json" },
      timeout: 10_000,
    },
  );
  expect(limitResponse.ok()).toBeTruthy();

  // With 3-second limit, initially green, then transitions
  // Wait for orange zone (at 90% = 2.7s)
  await expect(async () => {
    const zone = await timerBar.getAttribute("data-zone");
    expect(zone).toBe("orange");
  }).toPass({ timeout: 10_000, intervals: [300] });

  // Wait for red zone (at 100% = 3s)
  await expect(async () => {
    const zone = await timerBar.getAttribute("data-zone");
    expect(zone).toBe("red");
  }).toPass({ timeout: 10_000, intervals: [300] });

  // --- Pause preach timer ---
  const pauseResponse = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: { command: "pause_preach" },
      headers: { "Content-Type": "application/json" },
      timeout: 10_000,
    },
  );
  expect(pauseResponse.ok()).toBeTruthy();

  // State should show PAUSED, zone back to neutral
  await expect(state).toHaveText("PAUSED", { timeout: 5_000 });
  await expect(timerBar).toHaveAttribute("data-zone", "neutral", {
    timeout: 5_000,
  });

  // --- Reset preach timer ---
  const resetResponse = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: { command: "reset_preach" },
      headers: { "Content-Type": "application/json" },
      timeout: 10_000,
    },
  );
  expect(resetResponse.ok()).toBeTruthy();

  // Should show IDLE and em-dash again
  await expect(state).toHaveText("IDLE", { timeout: 5_000 });
  await expect(elapsed).toHaveText("—", { timeout: 5_000 });

  // Clean console check
  expect(consoleMessages).toEqual([]);
});
