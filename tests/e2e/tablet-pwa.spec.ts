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

test.describe("Tablet PWA Configuration", () => {
  test("manifest.json returns valid PWA manifest", async ({ request }) => {
    // Wait for server readiness
    await expect(async () => {
      const response = await request.get(
        new URL("/healthz", baseURL).toString(),
        { timeout: 120_000 },
      );
      expect(response.ok()).toBeTruthy();
    }).toPass({ timeout: 180_000 });

    // Fetch manifest
    const response = await request.get(
      new URL("/ui/tablet/manifest.json", baseURL).toString(),
      { timeout: 30_000 },
    );
    expect(response.ok()).toBeTruthy();
    expect(response.headers()["content-type"]).toContain(
      "application/manifest+json",
    );

    const manifest = await response.json();

    // Verify required manifest fields
    expect(manifest.name).toBe("Bible Tablet");
    expect(manifest.short_name).toBe("Bible");
    expect(manifest.start_url).toBe("/ui/tablet");
    expect(manifest.display).toBe("standalone");
    expect(manifest.background_color).toBe("#0f172a");
    expect(manifest.theme_color).toBe("#0f172a");

    // Verify icons array
    expect(manifest.icons).toHaveLength(2);
    expect(manifest.icons[0]).toMatchObject({
      src: "/ui/tablet/icon-192.png",
      sizes: "192x192",
      type: "image/png",
    });
    expect(manifest.icons[1]).toMatchObject({
      src: "/ui/tablet/icon-512.png",
      sizes: "512x512",
      type: "image/png",
    });
  });

  test("icon-192.png returns valid PNG", async ({ request }) => {
    const response = await request.get(
      new URL("/ui/tablet/icon-192.png", baseURL).toString(),
      { timeout: 30_000 },
    );
    expect(response.ok()).toBeTruthy();
    expect(response.headers()["content-type"]).toBe("image/png");

    const body = await response.body();
    // PNG magic bytes
    expect(body[0]).toBe(0x89);
    expect(body[1]).toBe(0x50); // P
    expect(body[2]).toBe(0x4e); // N
    expect(body[3]).toBe(0x47); // G
  });

  test("icon-512.png returns valid PNG", async ({ request }) => {
    const response = await request.get(
      new URL("/ui/tablet/icon-512.png", baseURL).toString(),
      { timeout: 30_000 },
    );
    expect(response.ok()).toBeTruthy();
    expect(response.headers()["content-type"]).toBe("image/png");

    const body = await response.body();
    // PNG magic bytes
    expect(body[0]).toBe(0x89);
    expect(body[1]).toBe(0x50);
    expect(body[2]).toBe(0x4e);
    expect(body[3]).toBe(0x47);
  });

  test("apple-touch-icon.png returns valid PNG", async ({ request }) => {
    const response = await request.get(
      new URL("/ui/tablet/apple-touch-icon.png", baseURL).toString(),
      { timeout: 30_000 },
    );
    expect(response.ok()).toBeTruthy();
    expect(response.headers()["content-type"]).toBe("image/png");

    const body = await response.body();
    // PNG magic bytes
    expect(body[0]).toBe(0x89);
    expect(body[1]).toBe(0x50);
    expect(body[2]).toBe(0x4e);
    expect(body[3]).toBe(0x47);
  });

  test("service worker returns valid JavaScript", async ({ request }) => {
    const response = await request.get(
      new URL("/ui/tablet/sw.js", baseURL).toString(),
      { timeout: 30_000 },
    );
    expect(response.ok()).toBeTruthy();
    expect(response.headers()["content-type"]).toContain("javascript");

    const body = await response.text();
    expect(body).toContain("Service Worker");
    expect(body).toContain("addEventListener");
    expect(body).toContain("skipWaiting");
  });

  test("tablet page has PWA meta tags", async ({ page, request }) => {
    // Wait for server readiness
    await expect(async () => {
      const response = await request.get(
        new URL("/healthz", baseURL).toString(),
        { timeout: 120_000 },
      );
      expect(response.ok()).toBeTruthy();
    }).toPass({ timeout: 180_000 });

    await page.goto(new URL("/ui/tablet", baseURL).toString());
    await page.waitForLoadState("domcontentloaded");

    // Verify PWA manifest link
    const manifestLink = page.locator('link[rel="manifest"]');
    await expect(manifestLink).toHaveAttribute(
      "href",
      "/ui/tablet/manifest.json",
    );

    // Verify iOS meta tags
    const appleWebAppCapable = page.locator(
      'meta[name="apple-mobile-web-app-capable"]',
    );
    await expect(appleWebAppCapable).toHaveAttribute("content", "yes");

    const appleStatusBar = page.locator(
      'meta[name="apple-mobile-web-app-status-bar-style"]',
    );
    await expect(appleStatusBar).toHaveAttribute(
      "content",
      "black-translucent",
    );

    const appleTitle = page.locator('meta[name="apple-mobile-web-app-title"]');
    await expect(appleTitle).toHaveAttribute("content", "Bible Tablet");

    const appleTouchIcon = page.locator('link[rel="apple-touch-icon"]');
    await expect(appleTouchIcon).toHaveAttribute(
      "href",
      "/ui/tablet/apple-touch-icon.png",
    );

    // Verify Android meta tags
    const mobileWebAppCapable = page.locator(
      'meta[name="mobile-web-app-capable"]',
    );
    await expect(mobileWebAppCapable).toHaveAttribute("content", "yes");

    const themeColor = page.locator('meta[name="theme-color"]');
    await expect(themeColor).toHaveAttribute("content", "#0f172a");

    // Verify viewport includes PWA-specific settings
    const viewport = page.locator('meta[name="viewport"]');
    const viewportContent = await viewport.getAttribute("content");
    expect(viewportContent).toContain("viewport-fit=cover");
    expect(viewportContent).toContain("user-scalable=no");
  });

  test("slides have proper spacing (no overlap)", async ({ page, request }) => {
    // Wait for server readiness
    await expect(async () => {
      const response = await request.get(
        new URL("/healthz", baseURL).toString(),
        { timeout: 120_000 },
      );
      expect(response.ok()).toBeTruthy();
    }).toPass({ timeout: 180_000 });

    await page.goto(new URL("/ui/tablet", baseURL).toString());
    await page.waitForLoadState("domcontentloaded");

    // Wait for presentation list to load (shows "Loading..." initially)
    await page.waitForTimeout(2000);

    // Check if any presentations exist
    const presentationCount = await page
      .locator('[data-role="presentation-list"] button')
      .count();

    if (presentationCount === 0) {
      // No Bible presentations in test DB - verify CSS properties directly
      const slidesContainer = page.locator(".tablet-slides");
      const display = await slidesContainer.evaluate((el) => {
        return window.getComputedStyle(el).display;
      });
      // Verify block layout (not flex) for older Safari compatibility
      expect(display).toBe("block");
      return;
    }

    // Click first presentation to load slides
    const firstPresentation = page
      .locator('[data-role="presentation-list"] button')
      .first();
    await firstPresentation.click();

    // Wait for slides to load
    const slides = page.locator(".tablet-slide");
    await expect(slides.first()).toBeVisible({ timeout: 10_000 });

    // Verify slides have gaps between them (margin-bottom creates spacing)
    const slideCount = await slides.count();
    if (slideCount >= 2) {
      const gaps = await page.evaluate(() => {
        const slideElements = document.querySelectorAll(".tablet-slide");
        const gaps: number[] = [];
        for (let i = 0; i < slideElements.length - 1; i++) {
          const rect1 = slideElements[i].getBoundingClientRect();
          const rect2 = slideElements[i + 1].getBoundingClientRect();
          gaps.push(rect2.top - rect1.bottom);
        }
        return gaps;
      });

      // All gaps should be at least 20px (we use 1.5rem = 24px)
      for (const gap of gaps) {
        expect(gap).toBeGreaterThanOrEqual(20);
      }
    }
  });

  test("header stays visible when scrolling (sticky)", async ({
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

    await page.goto(new URL("/ui/tablet", baseURL).toString());
    await page.waitForLoadState("domcontentloaded");

    // Verify header has sticky positioning (can test without presentations)
    const header = page.locator(".tablet-main__header");
    await expect(header).toBeVisible({ timeout: 10_000 });

    const position = await header.evaluate((el) => {
      const style = window.getComputedStyle(el);
      return style.position;
    });
    expect(position).toBe("sticky");

    // Verify header has z-index for proper stacking
    const zIndex = await header.evaluate((el) => {
      const style = window.getComputedStyle(el);
      return style.zIndex;
    });
    expect(parseInt(zIndex)).toBeGreaterThanOrEqual(10);
  });
});
