/**
 * WASM Stage Display Tests
 *
 * Tests the WASM-based stage display: loading, WebSocket connection,
 * slide display, layout switching, and clean console.
 */

import { test, expect, BrowserContext } from "@playwright/test";
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

async function openStageDisplay(
  context: BrowserContext,
  layout = "worship-snv",
) {
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: layout },
  });
  const stagePage = await context.newPage();

  const consoleMessages: string[] = [];
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await stagePage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await stagePage.waitForFunction(
    () =>
      (window as unknown as { __presenterStageConnectionState?: string })
        .__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );
  return { stagePage, consoleMessages };
}

test.describe("WASM Stage Display", () => {
  test("loads and connects via WebSocket", async ({ context }) => {
    const { stagePage, consoleMessages } = await openStageDisplay(context);

    // Status bar shows "CONNECTED"
    const connection = stagePage.locator(".stage__connection");
    await expect(connection).toContainText("CONNECTED");

    // Clock is visible and updating
    const clock = stagePage.locator(".stage__clock");
    await expect(clock).toBeVisible();
    const clockText = await clock.textContent();
    expect(clockText).toMatch(/\d{2}:\d{2}:\d{2}/);

    // Live indicator visible
    const livePill = stagePage.locator(".stage__live-pill");
    await expect(livePill).toBeVisible();

    expect(consoleMessages).toEqual([]);
    await stagePage.close();
  });

  test("displays current and next slide text after trigger", async ({
    context,
  }) => {
    const { stagePage, consoleMessages } = await openStageDisplay(context);

    // Get a presentation with slides
    const libsResp = await context.request.get(
      new URL("/libraries", baseURL).toString(),
    );
    const libs = await libsResp.json();

    if (libs.length > 0) {
      const firstLib = libs[0];
      const presResp = await context.request.get(
        new URL(
          `/libraries/${firstLib.id}/presentations`,
          baseURL,
        ).toString(),
      );
      const presentations = await presResp.json();

      if (presentations.length > 0) {
        const pres = presentations[0];
        const detailResp = await context.request.get(
          new URL(`/presentations/${pres.id}`, baseURL).toString(),
        );
        const detail = await detailResp.json();

        if (detail.slides && detail.slides.length >= 2) {
          // Trigger a slide
          await context.request.post(
            new URL("/stage/state", baseURL).toString(),
            {
              data: {
                presentationId: pres.id,
                currentSlideId: detail.slides[0].id,
                nextSlideId: detail.slides[1].id,
              },
            },
          );

          // Wait for current slide text to appear
          const currentSlide = stagePage.locator(
            ".stage__current-slide .stage__slide-text",
          );
          await expect(currentSlide).not.toBeEmpty({ timeout: 5_000 });

          // Next slide should also have text
          const nextSlide = stagePage.locator(
            ".stage__next-slide .stage__slide-text",
          );
          await expect(nextSlide).not.toBeEmpty({ timeout: 5_000 });
        }
      }
    }

    expect(consoleMessages).toEqual([]);
    await stagePage.close();
  });

  test("layout switching works reactively", async ({ context }) => {
    const { stagePage, consoleMessages } = await openStageDisplay(
      context,
      "worship-snv",
    );

    // Verify initial layout
    const container = stagePage.locator(".stage-container");
    await expect(container).toHaveAttribute("data-layout", "worship-snv");

    // Switch to timer layout via API
    await context.request.post(
      new URL("/stage/layout", baseURL).toString(),
      { data: { code: "timer" } },
    );

    // WASM should reactively switch layout (no page reload)
    await expect(container).toHaveAttribute("data-layout", "timer", {
      timeout: 5_000,
    });

    // Switch back
    await context.request.post(
      new URL("/stage/layout", baseURL).toString(),
      { data: { code: "worship-snv" } },
    );
    await expect(container).toHaveAttribute("data-layout", "worship-snv", {
      timeout: 5_000,
    });

    expect(consoleMessages).toEqual([]);
    await stagePage.close();
  });

  test("clean console — no errors or warnings", async ({ context }) => {
    const { stagePage, consoleMessages } = await openStageDisplay(context);

    // Wait a few seconds for any async errors to surface
    await stagePage.waitForTimeout(3_000);

    expect(consoleMessages).toEqual([]);
    await stagePage.close();
  });
});
