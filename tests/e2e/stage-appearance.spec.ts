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
  serverHandle = undefined;
});

test.describe("Stage Appearance Settings", () => {
  test("settings page loads with layout tabs", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-settings`);
    await expect(page.locator("h1")).toHaveText("Stage Appearance Settings");
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
    // worship-snv section should be visible by default
    await expect(
      page.locator('[data-role="layout-section"][data-layout="worship-snv"]'),
    ).toHaveAttribute("data-visible", "true");
    await expect(
      page.locator('[data-role="layout-section"][data-layout="worship-pp"]'),
    ).toHaveAttribute("data-visible", "false");
  });

  test("switching tabs shows correct layout section", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-settings`);
    // Click worship-pp tab
    await page
      .locator('[data-role="layout-tab"][data-layout="worship-pp"]')
      .click();
    await expect(
      page.locator('[data-role="layout-section"][data-layout="worship-pp"]'),
    ).toHaveAttribute("data-visible", "true");
    await expect(
      page.locator('[data-role="layout-section"][data-layout="worship-snv"]'),
    ).toHaveAttribute("data-visible", "false");
    // Click timer tab
    await page.locator('[data-role="layout-tab"][data-layout="timer"]').click();
    await expect(
      page.locator('[data-role="layout-section"][data-layout="timer"]'),
    ).toHaveAttribute("data-visible", "true");
    await expect(
      page.locator('[data-role="layout-section"][data-layout="worship-pp"]'),
    ).toHaveAttribute("data-visible", "false");
  });

  test("sliders show default values for worship-snv", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-settings`);
    // Check default currentMaxFont for worship-snv is 120
    const slider = page.locator(
      '[data-role="layout-section"][data-layout="worship-snv"] [data-param="currentMaxFont"]',
    );
    await expect(slider).toHaveValue("120");
    const display = slider.locator("..").locator('[data-role="value-display"]');
    await expect(display).toHaveText("120");
  });

  test("worship-pp shows playlist-specific sliders", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-settings`);
    await page
      .locator('[data-role="layout-tab"][data-layout="worship-pp"]')
      .click();
    // Playlist font size slider should exist
    await expect(
      page.locator(
        '[data-role="layout-section"][data-layout="worship-pp"] [data-param="playlistFontSize"]',
      ),
    ).toBeVisible();
    await expect(
      page.locator(
        '[data-role="layout-section"][data-layout="worship-pp"] [data-param="slidesPlaylistRatio"]',
      ),
    ).toBeVisible();
  });

  test("timer layout does not show playlist sliders", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-settings`);
    await page.locator('[data-role="layout-tab"][data-layout="timer"]').click();
    await expect(
      page.locator(
        '[data-role="layout-section"][data-layout="timer"] [data-param="playlistFontSize"]',
      ),
    ).toHaveCount(0);
  });

  test("save appearance via API and verify persistence", async ({ page }) => {
    // Save custom appearance via the API
    const response = await page.request.put(
      `${baseURL}/stage/appearance/worship-snv`,
      {
        data: {
          bodyPaddingV: 3.0,
          bodyPaddingH: 4.0,
          currentMaxFont: 90,
          nextMaxFont: 60,
          nextRatio: 0.7,
          groupFontSize: 2.0,
          lyricsGap: 1.0,
          nextPaddingBottom: 3.0,
          baseChars: 30,
          minFont: 14,
          playlistFontSize: 1.5,
          playlistHeaderSize: 1.2,
          playlistPadding: 1.5,
          slidesPlaylistRatio: "6fr 4fr",
        },
      },
    );
    expect(response.status()).toBe(204);

    // Verify retrieval
    const getResponse = await page.request.get(
      `${baseURL}/stage/appearance/worship-snv`,
    );
    expect(getResponse.status()).toBe(200);
    const body = await getResponse.json();
    expect(body.currentMaxFont).toBe(90);
    expect(body.bodyPaddingV).toBe(3.0);
    expect(body.baseChars).toBe(30);
  });

  test("save button persists slider changes", async ({ page }) => {
    await page.goto(`${baseURL}/ui/stage-settings`);
    // Switch to worship-pp
    await page
      .locator('[data-role="layout-tab"][data-layout="worship-pp"]')
      .click();

    // Change a slider value
    const slider = page.locator(
      '[data-role="layout-section"][data-layout="worship-pp"] [data-param="currentMaxFont"]',
    );
    await slider.fill("85");

    // Click save
    await page
      .locator(
        '[data-role="layout-section"][data-layout="worship-pp"] [data-role="save"]',
      )
      .click();

    // Verify toast appears
    const toast = page.locator('[data-role="toast"]');
    await expect(toast).toHaveAttribute("data-visible", "true");
    await expect(toast).toContainText("Saved");

    // Verify the value persisted via API
    const getResponse = await page.request.get(
      `${baseURL}/stage/appearance/worship-pp`,
    );
    const body = await getResponse.json();
    expect(body.currentMaxFont).toBe(85);
  });

  test("reset button restores defaults", async ({ page }) => {
    // First set a custom value
    await page.request.put(`${baseURL}/stage/appearance/worship-pp`, {
      data: {
        bodyPaddingV: 5.0,
        bodyPaddingH: 5.0,
        currentMaxFont: 50,
        nextMaxFont: 30,
        nextRatio: 0.5,
        groupFontSize: 3.0,
        lyricsGap: 2.0,
        nextPaddingBottom: 5.0,
        baseChars: 40,
        minFont: 20,
        playlistFontSize: 2.5,
        playlistHeaderSize: 2.0,
        playlistPadding: 3.0,
        slidesPlaylistRatio: "5fr 5fr",
      },
    });

    await page.goto(`${baseURL}/ui/stage-settings`);
    await page
      .locator('[data-role="layout-tab"][data-layout="worship-pp"]')
      .click();

    // Click reset
    await page
      .locator(
        '[data-role="layout-section"][data-layout="worship-pp"] [data-role="reset"]',
      )
      .click();

    // Verify toast
    const toast = page.locator('[data-role="toast"]');
    await expect(toast).toHaveAttribute("data-visible", "true");
    await expect(toast).toContainText("Reset");

    // Verify default values via API
    const getResponse = await page.request.get(
      `${baseURL}/stage/appearance/worship-pp`,
    );
    const body = await getResponse.json();
    expect(body.currentMaxFont).toBe(100); // worship-pp default
    expect(body.nextMaxFont).toBe(64); // worship-pp default
    expect(body.bodyPaddingV).toBe(1.0);
  });

  test("GET returns defaults for unknown layout", async ({ page }) => {
    const response = await page.request.get(
      `${baseURL}/stage/appearance/nonexistent`,
    );
    expect(response.status()).toBe(200);
    const body = await response.json();
    // Should get the generic defaults
    expect(body.currentMaxFont).toBe(120);
    expect(body.nextMaxFont).toBe(80);
  });

  test("stage display applies appearance CSS vars", async ({ page }) => {
    // Set custom appearance
    await page.request.put(`${baseURL}/stage/appearance/worship-snv`, {
      data: {
        bodyPaddingV: 5.0,
        bodyPaddingH: 6.0,
        currentMaxFont: 90,
        nextMaxFont: 60,
        nextRatio: 0.7,
        groupFontSize: 2.0,
        lyricsGap: 1.5,
        nextPaddingBottom: 4.0,
        baseChars: 30,
        minFont: 14,
        playlistFontSize: 1.3,
        playlistHeaderSize: 1.1,
        playlistPadding: 1.0,
        slidesPlaylistRatio: "7fr 3fr",
      },
    });

    // Open stage display (worship-snv is default layout)
    await page.goto(`${baseURL}/stage`);
    // Wait for appearance to be fetched and applied
    await page.waitForTimeout(1500);

    // Verify CSS custom properties are set on body
    const bodyPadV = await page.evaluate(() =>
      document.body.style.getPropertyValue("--body-pad-v"),
    );
    expect(bodyPadV).toBe("5vh");

    const bodyPadH = await page.evaluate(() =>
      document.body.style.getPropertyValue("--body-pad-h"),
    );
    expect(bodyPadH).toBe("6vw");

    const lyricsGap = await page.evaluate(() =>
      document.body.style.getPropertyValue("--lyrics-gap"),
    );
    expect(lyricsGap).toBe("1.5rem");
  });

  test("live appearance update via WebSocket", async ({ page, context }) => {
    // Reset worship-snv to defaults first
    await page.request.put(`${baseURL}/stage/appearance/worship-snv`, {
      data: {
        bodyPaddingV: 1.0,
        bodyPaddingH: 2.0,
        currentMaxFont: 120,
        nextMaxFont: 80,
        nextRatio: 0.8,
        groupFontSize: 1.6,
        lyricsGap: 0.5,
        nextPaddingBottom: 2.0,
        baseChars: 25,
        minFont: 12,
        playlistFontSize: 1.3,
        playlistHeaderSize: 1.1,
        playlistPadding: 1.0,
        slidesPlaylistRatio: "7fr 3fr",
      },
    });

    // Open stage display
    await page.goto(`${baseURL}/stage`);
    await page.waitForTimeout(1500);

    // Verify initial CSS var
    const initialPad = await page.evaluate(() =>
      document.body.style.getPropertyValue("--body-pad-v"),
    );
    expect(initialPad).toBe("1vh");

    // Now update via API (triggers WS broadcast)
    const settingsPage = await context.newPage();
    await settingsPage.request.put(`${baseURL}/stage/appearance/worship-snv`, {
      data: {
        bodyPaddingV: 8.0,
        bodyPaddingH: 2.0,
        currentMaxFont: 120,
        nextMaxFont: 80,
        nextRatio: 0.8,
        groupFontSize: 1.6,
        lyricsGap: 0.5,
        nextPaddingBottom: 2.0,
        baseChars: 25,
        minFont: 12,
        playlistFontSize: 1.3,
        playlistHeaderSize: 1.1,
        playlistPadding: 1.0,
        slidesPlaylistRatio: "7fr 3fr",
      },
    });

    // Wait for WebSocket to propagate
    await page.waitForTimeout(1000);

    // Verify the stage display received the update
    const updatedPad = await page.evaluate(() =>
      document.body.style.getPropertyValue("--body-pad-v"),
    );
    expect(updatedPad).toBe("8vh");

    await settingsPage.close();
  });
});
