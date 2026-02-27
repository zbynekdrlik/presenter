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

test.describe.configure({ timeout: 600_000 });

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

test.describe("Stage Design Editor", () => {
  test("editor page loads with layout tabs", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-design`);
    await expect(page.locator("h1")).toHaveText("Stage Design Editor");
    // Verify all four layout tabs are present
    await expect(page.locator('[data-role="layout-tab"]')).toHaveCount(4);
    await expect(
      page.locator('[data-role="layout-tab"][data-layout="worship-snv"]'),
    ).toBeVisible();
    await expect(
      page.locator('[data-role="layout-tab"][data-layout="worship-pp"]'),
    ).toBeVisible();
    await expect(
      page.locator('[data-role="layout-tab"][data-layout="timer"]'),
    ).toBeVisible();
    await expect(
      page.locator('[data-role="layout-tab"][data-layout="preach"]'),
    ).toBeVisible();
  });

  test("canvas shows boxes for worship-snv layout", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-design`);
    // Wait for canvas to render
    await page.waitForTimeout(500);
    const canvas = page.locator("#design-canvas");
    await expect(canvas).toBeVisible();
    // Verify boxes are rendered
    await expect(
      canvas.locator('.sd__box[data-box-type="current_slide"]'),
    ).toBeVisible();
    await expect(
      canvas.locator('.sd__box[data-box-type="next_slide"]'),
    ).toBeVisible();
    await expect(
      canvas.locator('.sd__box[data-box-type="clock"]'),
    ).toBeVisible();
  });

  test("switching tabs renders different boxes", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-design`);
    await page.waitForTimeout(500);
    // Switch to timer layout
    await page.locator('[data-role="layout-tab"][data-layout="timer"]').click();
    await page.waitForTimeout(300);
    // Timer layout should have countdown_timer box
    const canvas = page.locator("#design-canvas");
    await expect(
      canvas.locator('.sd__box[data-box-type="countdown_timer"]'),
    ).toBeVisible();
    // Should not have current_slide
    await expect(
      canvas.locator('.sd__box[data-box-type="current_slide"]'),
    ).toHaveCount(0);
  });

  test("clicking box selects it", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-design`);
    await page.waitForTimeout(500);
    const canvas = page.locator("#design-canvas");
    const box = canvas.locator('.sd__box[data-box-type="current_slide"]');
    await box.click();
    await expect(box).toHaveAttribute("data-selected", "true");
    // Properties panel should show the box info
    const panel = page.locator("#properties-panel");
    await expect(panel.locator(".sd__prop-header")).toContainText(
      "Current Slide",
    );
  });

  test("clicking canvas background deselects box", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-design`);
    await page.waitForTimeout(500);
    const canvas = page.locator("#design-canvas");
    const box = canvas.locator('.sd__box[data-box-type="current_slide"]');
    // Select the box
    await box.click();
    await expect(box).toHaveAttribute("data-selected", "true");
    // Click on canvas background
    await canvas.click({ position: { x: 5, y: 5 } });
    await page.waitForTimeout(200);
    // Box should be deselected
    await expect(box).toHaveAttribute("data-selected", "false");
    // Properties panel should show hint
    const panel = page.locator("#properties-panel");
    await expect(panel.locator(".sd__hint")).toBeVisible();
  });

  test("properties panel updates box position", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-design`);
    await page.waitForTimeout(500);
    const canvas = page.locator("#design-canvas");
    const box = canvas.locator('.sd__box[data-box-type="current_slide"]');
    await box.click();

    // Get initial position
    const initialStyle = await box.getAttribute("style");

    // Change X position via properties panel
    const xInput = page.locator('[data-prop="x"]');
    await xInput.fill("10");
    await xInput.blur();
    await page.waitForTimeout(200);

    // Verify box position changed
    await expect(box).toHaveCSS("left", /10%/);
  });
});

test.describe("Stage Design API", () => {
  test("GET returns default design for layout", async ({ page }) => {
    const response = await page.request.get(
      `${baseURL}/stage/design/worship-snv`,
    );
    expect(response.status()).toBe(200);
    const body = await response.json();
    expect(body.layoutCode).toBe("worship-snv");
    expect(body.boxes).toBeInstanceOf(Array);
    expect(body.boxes.length).toBeGreaterThan(0);
    // Verify expected boxes exist
    const boxIds = body.boxes.map((b: { id: string }) => b.id);
    expect(boxIds).toContain("current-slide");
    expect(boxIds).toContain("next-slide");
    expect(boxIds).toContain("clock");
  });

  test("PUT saves custom design", async ({ page }) => {
    const customDesign = {
      layoutCode: "worship-snv",
      boxes: [
        {
          id: "current-slide",
          boxType: "current_slide",
          x: 5,
          y: 15,
          width: 90,
          height: 50,
          textColor: "#ffffff",
          textAlign: "center",
          fontWeight: 700,
          minFontPx: 12,
          maxFontPx: 100,
          visible: true,
          zIndex: 0,
        },
        {
          id: "next-slide",
          boxType: "next_slide",
          x: 10,
          y: 70,
          width: 80,
          height: 25,
          textColor: "#cbd5f5",
          textAlign: "center",
          fontWeight: 700,
          minFontPx: 12,
          maxFontPx: 80,
          visible: true,
          zIndex: 0,
        },
      ],
      backgroundColor: "#000000",
    };

    const response = await page.request.put(
      `${baseURL}/stage/design/worship-snv`,
      { data: customDesign },
    );
    expect(response.status()).toBe(204);

    // Verify retrieval
    const getResponse = await page.request.get(
      `${baseURL}/stage/design/worship-snv`,
    );
    expect(getResponse.status()).toBe(200);
    const body = await getResponse.json();
    expect(body.boxes.length).toBe(2);
    const currentSlide = body.boxes.find(
      (b: { id: string }) => b.id === "current-slide",
    );
    expect(currentSlide.x).toBe(5);
    expect(currentSlide.y).toBe(15);
    expect(currentSlide.width).toBe(90);
  });

  test("PUT rejects mismatched layout code", async ({ page }) => {
    const response = await page.request.put(
      `${baseURL}/stage/design/worship-snv`,
      {
        data: {
          layoutCode: "timer", // Mismatch!
          boxes: [],
          backgroundColor: "#000000",
        },
      },
    );
    expect(response.status()).toBe(400);
  });

  test("POST reset returns default design", async ({ page }) => {
    // First save a custom design
    await page.request.put(`${baseURL}/stage/design/timer`, {
      data: {
        layoutCode: "timer",
        boxes: [
          {
            id: "countdown-timer",
            boxType: "countdown_timer",
            x: 20,
            y: 40,
            width: 60,
            height: 20,
            textColor: "#38bdf8",
            textAlign: "center",
            fontWeight: 700,
            minFontPx: 12,
            maxFontPx: 150,
            visible: true,
            zIndex: 0,
          },
        ],
        backgroundColor: "#111111",
      },
    });

    // Reset
    const resetResponse = await page.request.post(
      `${baseURL}/stage/design/timer/reset`,
    );
    expect(resetResponse.status()).toBe(200);
    const body = await resetResponse.json();
    expect(body.layoutCode).toBe("timer");
    // Should have default boxes back
    const countdownBox = body.boxes.find(
      (b: { id: string }) => b.id === "countdown-timer",
    );
    expect(countdownBox.x).toBe(10); // Default x
    expect(countdownBox.width).toBe(80); // Default width
  });
});

test.describe("Stage Design Save/Reset via UI", () => {
  test("save button persists design", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-design`);
    await page.waitForTimeout(500);

    // Click save
    await page.locator('[data-role="save"]').click();

    // Verify toast appears
    const toast = page.locator('[data-role="toast"]');
    await expect(toast).toHaveAttribute("data-visible", "true");
    await expect(toast).toContainText("saved");
  });

  test("reset button restores defaults", async ({ page }) => {
    // First save a custom design via API
    await page.request.put(`${baseURL}/stage/design/preach`, {
      data: {
        layoutCode: "preach",
        boxes: [
          {
            id: "preach-timer",
            boxType: "preach_timer",
            x: 25,
            y: 45,
            width: 50,
            height: 10,
            textColor: "#ff0000",
            textAlign: "left",
            fontWeight: 400,
            minFontPx: 12,
            maxFontPx: 80,
            visible: true,
            zIndex: 0,
          },
        ],
        backgroundColor: "#222222",
      },
    });

    await page.goto(`${baseURL}/ui/stage-design`);
    // Switch to preach tab
    await page
      .locator('[data-role="layout-tab"][data-layout="preach"]')
      .click();
    await page.waitForTimeout(500);

    // Click reset
    await page.locator('[data-role="reset"]').click();

    // Verify toast
    const toast = page.locator('[data-role="toast"]');
    await expect(toast).toHaveAttribute("data-visible", "true");
    await expect(toast).toContainText("Reset");

    // Verify defaults via API
    const getResponse = await page.request.get(
      `${baseURL}/stage/design/preach`,
    );
    const body = await getResponse.json();
    const preachTimer = body.boxes.find(
      (b: { id: string }) => b.id === "preach-timer",
    );
    expect(preachTimer.x).toBe(10); // Default
    expect(preachTimer.width).toBe(80); // Default
    expect(preachTimer.textColor).toBe("#34d399"); // Default green
  });
});

test.describe("Stage Design WebSocket Updates", () => {
  test("stage display receives design updates", async ({ page, context }) => {
    // Reset design first
    await page.request.post(`${baseURL}/stage/design/worship-snv/reset`);

    // Open stage display
    await page.goto(`${baseURL}/stage`);
    await page.waitForTimeout(1500);

    // Now update via API (triggers WS broadcast)
    const settingsPage = await context.newPage();
    await settingsPage.request.put(`${baseURL}/stage/design/worship-snv`, {
      data: {
        layoutCode: "worship-snv",
        boxes: [
          {
            id: "current-slide",
            boxType: "current_slide",
            x: 5,
            y: 5,
            width: 90,
            height: 45,
            textColor: "#ff00ff",
            textAlign: "center",
            fontWeight: 700,
            minFontPx: 12,
            maxFontPx: 100,
            visible: true,
            zIndex: 0,
          },
        ],
        backgroundColor: "#000000",
      },
    });

    // Wait for WebSocket to propagate
    await page.waitForTimeout(1500);

    // Verify the design was received
    const design = await page.evaluate(
      () =>
        (window as unknown as { __presenterStageDesign?: unknown })
          .__presenterStageDesign,
    );
    expect(design).toBeTruthy();

    await settingsPage.close();
  });
});
