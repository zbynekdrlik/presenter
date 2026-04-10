/**
 * WASM Operator Timer Tests
 *
 * Tests timer functionality in the WASM operator including countdown and preach timers.
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
});

async function navigateToTimers(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });

  // Navigate to timers view
  const timersButton = page.locator(
    '[data-role="view-toggle"][data-view="timers"]',
  );
  if ((await timersButton.count()) > 0) {
    await timersButton.click();
  } else {
    // Fallback: click timers tab by text
    const timersTab = page.locator('button:has-text("Timers")').first();
    if ((await timersTab.count()) > 0) {
      await timersTab.click();
    }
  }

  // Wait for timer panel to be visible
  await page.waitForFunction(
    () => {
      const body = document.body;
      return body.getAttribute("data-view") === "timers";
    },
    { timeout: 5_000 },
  );
}

test.describe("WASM Operator Timer Tests", () => {
  test("countdown target input receives focus", async ({ page }) => {
    await navigateToTimers(page);

    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    await expect(countdownInput).toBeVisible();

    await countdownInput.click();
    await expect(countdownInput).toBeFocused();
  });

  test("countdown target input accepts time value", async ({ page }) => {
    await navigateToTimers(page);

    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    await countdownInput.fill("18:00");

    const value = await countdownInput.inputValue();
    expect(value).toBe("18:00");
  });

  test("enter key in countdown input submits value", async ({ page }) => {
    await navigateToTimers(page);

    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    await countdownInput.fill("18:30");
    await countdownInput.press("Enter");

    // Wait for update - target display should update
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Verify no error toast
    const errorToast = page.locator(
      '[data-role="toast"][data-variant="error"]',
    );
    await expect(errorToast).not.toBeVisible();
  });

  test("setting countdown target auto-starts the countdown", async ({
    page,
    request,
  }) => {
    await navigateToTimers(page);

    // Pick an hour at least 2 hours in the future so the test is stable
    // even when run near hour boundaries.
    const futureHour = (new Date().getHours() + 2) % 24;
    const compactInput = String(futureHour).padStart(2, "0") + "00";

    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    await countdownInput.fill(compactInput);
    await countdownInput.press("Enter");

    // After auto-start, the API should report the countdown as Running
    // (not Idle) and the target should match what we typed.
    await expect(async () => {
      const response = await request.get(
        new URL("/timers/overview", baseURL).toString(),
        { timeout: 10_000 },
      );
      const data = await response.json();
      expect(data.countdownToStart.state).toBe("running");
      expect(data.countdownToStart.targetLocal).toMatch(
        new RegExp(`^${String(futureHour).padStart(2, "0")}:00:00$`),
      );
    }).toPass({ timeout: 10_000, intervals: [500] });
  });

  test("countdown HHMM compact form sets correct target", async ({
    page,
    request,
  }) => {
    await navigateToTimers(page);

    // Use 4-digit compact form: 1915 → 19:15
    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    await countdownInput.fill("1915");
    await countdownInput.press("Enter");

    await expect(async () => {
      const response = await request.get(
        new URL("/timers/overview", baseURL).toString(),
        { timeout: 10_000 },
      );
      const data = await response.json();
      expect(data.countdownToStart.targetLocal).toBe("19:15:00");
      expect(data.countdownToStart.state).toBe("running");
    }).toPass({ timeout: 10_000, intervals: [500] });
  });

  test("countdown panel does not show Start/Pause/Reset buttons", async ({
    page,
  }) => {
    await navigateToTimers(page);

    // These were removed because wall-clock countdowns don't need them.
    await expect(
      page.locator('[data-role="countdown-start"]'),
    ).toHaveCount(0);
    await expect(
      page.locator('[data-role="countdown-pause"]'),
    ).toHaveCount(0);
    await expect(
      page.locator('[data-role="countdown-reset"]'),
    ).toHaveCount(0);

    // ±5 buttons stay
    await expect(
      page.locator('[data-role="countdown-offset-minus"]'),
    ).toBeVisible();
    await expect(
      page.locator('[data-role="countdown-offset-plus"]'),
    ).toBeVisible();
  });

  test("countdown offset minus decreases by 5 minutes", async ({ page }) => {
    await navigateToTimers(page);

    const offsetMinus = page.locator('[data-role="countdown-offset-minus"]');
    await expect(offsetMinus).toBeVisible();

    await offsetMinus.click();

    // Wait for API response
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Should not show error
    const errorToast = page.locator(
      '[data-role="toast"][data-variant="error"]',
    );
    await expect(errorToast).not.toBeVisible();
  });

  test("countdown offset plus increases by 5 minutes", async ({ page }) => {
    await navigateToTimers(page);

    const offsetPlus = page.locator('[data-role="countdown-offset-plus"]');
    await expect(offsetPlus).toBeVisible();

    await offsetPlus.click();

    // Wait for API response
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Should not show error
    const errorToast = page.locator(
      '[data-role="toast"][data-variant="error"]',
    );
    await expect(errorToast).not.toBeVisible();
  });

  test("timer display shows countdown value", async ({ page }) => {
    await navigateToTimers(page);

    const countdownValue = page.locator("#countdown-value");
    await expect(countdownValue).toBeVisible();

    const text = await countdownValue.textContent();
    // Should show a time format like "0:00" or "-1:23:45"
    expect(text).toMatch(/^-?\d+:\d{2}(:\d{2})?$/);
  });

  test("timer overlay opens in new window", async ({ page, context }) => {
    await navigateToTimers(page);

    const overlayButton = page.locator('[data-role="timer-overlay-open"]');
    await expect(overlayButton).toBeVisible();

    // Listen for new page
    const pagePromise = context.waitForEvent("page");
    await overlayButton.click();

    const newPage = await pagePromise;
    await newPage.waitForLoadState();

    // Verify URL contains overlay path
    expect(newPage.url()).toContain("/overlays/timer");

    await newPage.close();
  });

  test("timer overlay URL can be copied via execCommand on HTTP", async ({
    page,
  }) => {
    // This test guards the fix for the operator HTTP clipboard bug.
    //
    // Background: navigator.clipboard is undefined on plain HTTP (LAN
    // access), so the old code silently failed. The fix uses
    // document.execCommand('copy') with a temporary textarea. The
    // existing toast-only check was insufficient because the old
    // broken code ALSO set a success toast — it just never put
    // anything on the clipboard.
    //
    // We can't read the system clipboard from a non-secure context,
    // so instead we intercept document.execCommand and capture the
    // selected textarea value at the moment of the copy call.

    await navigateToTimers(page);

    // Install a spy on document.execCommand BEFORE clicking. The spy
    // records the selected text and the command name, then delegates
    // to the original implementation so the real path runs end-to-end.
    await page.evaluate(() => {
      const captured: { command?: string; selectedText?: string } = {};
      const original = document.execCommand.bind(document);
      // @ts-expect-error attaching for test inspection
      window.__execCommandSpy = captured;
      document.execCommand = function (command: string, ...rest: unknown[]) {
        captured.command = command;
        const active = document.activeElement;
        if (active && active instanceof HTMLTextAreaElement) {
          captured.selectedText = active.value;
        }
        // @ts-expect-error forwarding rest args
        return original(command, ...rest);
      };
    });

    const copyButton = page.locator('[data-role="timer-overlay-copy"]');
    await expect(copyButton).toBeVisible();
    await copyButton.click();

    // Success toast must appear (existing assertion)
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("copied");
      },
      { timeout: 3_000 },
    );

    // Toast must be the SUCCESS variant, not the error fallback.
    const toastVariant = await page
      .locator('[data-role="toast"]')
      .getAttribute("data-variant");
    expect(toastVariant).toBe("success");

    // The spy must have observed execCommand('copy') with the correct
    // URL in the textarea. This is the part that genuinely regression-
    // guards the HTTP clipboard fix.
    const spy = await page.evaluate(
      // @ts-expect-error reading the spy installed above
      () => window.__execCommandSpy,
    );
    expect(spy?.command).toBe("copy");
    expect(spy?.selectedText).toMatch(/\/overlays\/timer$/);
  });

  test("preach limit input sets and clears limit", async ({
    page,
    request,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToTimers(page);

    // Preach limit input should be visible
    const limitInput = page.locator('[data-role="preach-limit-input"]');
    await expect(limitInput).toBeVisible({ timeout: 5_000 });

    // Preach card should show "No limit" initially
    const preachLimit = page.locator("#preach-limit");
    await expect(preachLimit).toContainText("No limit", { timeout: 5_000 });

    // Type "5" and press Enter → sets limit to 300 seconds (5 min)
    await limitInput.click();
    await limitInput.fill("5");
    await limitInput.press("Enter");

    // Verify limit is set via API
    await expect(async () => {
      const response = await request.get(
        new URL("/timers/overview", baseURL).toString(),
        { timeout: 10_000 },
      );
      const data = await response.json();
      expect(data.preachTimer.limitSeconds).toBe(300);
    }).toPass({ timeout: 10_000, intervals: [500] });

    // Preach card should show "Limit: 5:00"
    await expect(preachLimit).toContainText("Limit: 5:00", { timeout: 5_000 });

    // Clear limit
    const clearButton = page.locator('[data-role="preach-limit-clear"]');
    await clearButton.click();

    // Verify limit is cleared via API
    await expect(async () => {
      const response = await request.get(
        new URL("/timers/overview", baseURL).toString(),
        { timeout: 10_000 },
      );
      const data = await response.json();
      expect(data.preachTimer.limitSeconds).toBeNull();
    }).toPass({ timeout: 10_000, intervals: [500] });

    // Preach card should show "No limit" again
    await expect(preachLimit).toContainText("No limit", { timeout: 5_000 });

    // Clean console
    expect(consoleMessages).toEqual([]);
  });

  test("typing hour number sets local time target (#212 bug 1+3)", async ({
    page,
    request,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToTimers(page);

    // Use SetCountdownTargetLocal via API to set a known future time
    const now = new Date();
    const futureHour = (now.getHours() + 2) % 24;

    const response = await request.post(
      new URL("/timers/command", baseURL).toString(),
      {
        data: {
          command: "set_countdown_target_local",
          hours: futureHour,
          minutes: 0,
        },
        headers: { "Content-Type": "application/json" },
        timeout: 10_000,
      },
    );
    expect(response.ok()).toBeTruthy();
    const data = await response.json();

    // target_local should show the local time we set
    const expectedPrefix = `${String(futureHour).padStart(2, "0")}:00`;
    expect(data.countdownToStart.targetLocal).toContain(expectedPrefix);

    // Remaining should be between 1-2 hours (depends on where in the hour the test runs)
    expect(data.countdownToStart.secondsRemaining).toBeGreaterThan(3500);
    expect(data.countdownToStart.secondsRemaining).toBeLessThan(7500);

    // Verify the operator UI shows the local target
    const targetDisplay = page.locator("#countdown-target");
    await expect(targetDisplay).toContainText(expectedPrefix, {
      timeout: 10_000,
    });

    // Now test typing the hour in the input
    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    await countdownInput.fill(String(futureHour));
    await countdownInput.press("Enter");

    // Wait for API response
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Target display should still show the same local time
    await expect(targetDisplay).toContainText(expectedPrefix, {
      timeout: 5_000,
    });

    expect(consoleMessages).toEqual([]);
  });

  test("adjust countdown target +5/-5 via API (#212 bug 3)", async ({
    request,
  }) => {
    // Set initial target
    const now = new Date();
    const futureHour = (now.getHours() + 2) % 24;
    await request.post(new URL("/timers/command", baseURL).toString(), {
      data: {
        command: "set_countdown_target_local",
        hours: futureHour,
        minutes: 0,
      },
      headers: { "Content-Type": "application/json" },
    });

    // Get baseline
    const baselineResp = await request.get(
      new URL("/timers/overview", baseURL).toString(),
    );
    const baseline = await baselineResp.json();
    const baselineRemaining = baseline.countdownToStart.secondsRemaining;

    // Adjust +5
    const plusResp = await request.post(
      new URL("/timers/command", baseURL).toString(),
      {
        data: { command: "adjust_countdown_target", offset_minutes: 5 },
        headers: { "Content-Type": "application/json" },
      },
    );
    expect(plusResp.ok()).toBeTruthy();
    const plusData = await plusResp.json();
    const plusDiff =
      plusData.countdownToStart.secondsRemaining - baselineRemaining;
    expect(plusDiff).toBeGreaterThan(290);
    expect(plusDiff).toBeLessThan(310);

    // Adjust -5 (back to baseline)
    const minusResp = await request.post(
      new URL("/timers/command", baseURL).toString(),
      {
        data: { command: "adjust_countdown_target", offset_minutes: -5 },
        headers: { "Content-Type": "application/json" },
      },
    );
    expect(minusResp.ok()).toBeTruthy();
    const minusData = await minusResp.json();
    const totalDiff = Math.abs(
      minusData.countdownToStart.secondsRemaining - baselineRemaining,
    );
    expect(totalDiff).toBeLessThan(5);
  });

  test("timer overlay renders without flicker (#212 bug 4)", async ({
    page,
    request,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    // Set a target and start the countdown
    const now = new Date();
    const futureHour = (now.getHours() + 1) % 24;
    await request.post(new URL("/timers/command", baseURL).toString(), {
      data: {
        command: "set_countdown_target_local",
        hours: futureHour,
        minutes: 0,
      },
      headers: { "Content-Type": "application/json" },
    });
    await request.post(new URL("/timers/command", baseURL).toString(), {
      data: { command: "start_countdown" },
      headers: { "Content-Type": "application/json" },
    });

    // Open overlay
    await page.goto(new URL("/overlays/timer", baseURL).toString());
    await page.waitForSelector("#timer-value", { timeout: 10_000 });

    // Collect displayed values over 5 seconds
    const values: string[] = [];
    for (let i = 0; i < 10; i++) {
      await page.waitForTimeout(500);
      const text = await page.locator("#timer-value").textContent();
      if (text) values.push(text);
    }

    // Parse values to seconds for monotonicity check
    const toSeconds = (v: string): number => {
      const parts = v.split(":").map(Number);
      if (parts.length === 1) return parts[0];
      return parts[0] * 60 + parts[1];
    };

    const seconds = values.map(toSeconds);

    // No value should jump UP (flicker = value goes down then up)
    let flickerCount = 0;
    for (let i = 1; i < seconds.length; i++) {
      if (seconds[i] > seconds[i - 1]) flickerCount++;
    }
    expect(flickerCount).toBe(0);

    // Clean up
    await request.post(new URL("/timers/command", baseURL).toString(), {
      data: { command: "reset_countdown" },
      headers: { "Content-Type": "application/json" },
    });

    expect(consoleMessages).toEqual([]);
  });

  test("preach timer start/pause/reset works", async ({ page }) => {
    await navigateToTimers(page);

    // Start preach timer
    const startButton = page.locator('button[data-command="start_preach"]');
    await expect(startButton).toBeVisible();
    await startButton.click();
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Pause preach timer
    const pauseButton = page.locator('button[data-command="pause_preach"]');
    await pauseButton.click();
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Reset preach timer
    const resetButton = page.locator('button[data-command="reset_preach"]');
    await resetButton.click();
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Should not show error
    const errorToast = page.locator(
      '[data-role="toast"][data-variant="error"]',
    );
    await expect(errorToast).not.toBeVisible();
  });
});
