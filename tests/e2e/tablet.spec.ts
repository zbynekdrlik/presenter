import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 300_000 });

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

test("tablet shows Bible presentations, renders slides, and triggers passages", async ({
  page,
  request,
}) => {
  // Wait for server readiness
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      {
        timeout: 120_000,
      },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  // --- Setup: create a Bible presentation with slides via API ---
  const presentationName = `Tablet E2E ${Date.now()}`;
  const createResponse = await request.post(
    new URL("/bible/presentations", baseURL).toString(),
    {
      data: { name: presentationName },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );
  expect(createResponse.ok()).toBeTruthy();
  const created = await createResponse.json();
  const presentationId: string = created.id;

  // Resolve Bible slides for John 3:16-18 to get slide data with metadata
  const resolveResponse = await request.post(
    new URL("/bible/resolve", baseURL).toString(),
    {
      data: {
        mainTranslation: "eng-kjv",
        book: "John",
        bookCode: "JHN",
        chapter: 3,
        verseStart: 16,
        verseEnd: 18,
      },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );
  expect(resolveResponse.ok()).toBeTruthy();
  const resolved = await resolveResponse.json();
  const resolvedSlides: Array<{
    main: string;
    translation: string;
    stage: string;
    group?: string;
    metadata?: any;
    mainReference?: string;
    translationReference?: string;
  }> = resolved.slides;
  expect(resolvedSlides.length).toBeGreaterThan(0);

  // Append resolved slides to the presentation
  const appendResponse = await request.post(
    new URL(
      `/bible/presentations/${presentationId}/append`,
      baseURL,
    ).toString(),
    {
      data: {
        slides: resolvedSlides.map((slide) => ({
          main: slide.main,
          translation: slide.translation,
          stage: slide.stage,
          group: slide.group || null,
          metadata: slide.metadata || null,
        })),
      },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );
  expect(appendResponse.ok()).toBeTruthy();

  // Fetch final presentation to get slide IDs
  const detailResponse = await request.get(
    new URL(`/bible/presentations/${presentationId}`, baseURL).toString(),
    { timeout: 60_000 },
  );
  expect(detailResponse.ok()).toBeTruthy();
  const detail = await detailResponse.json();
  const slideCount: number = detail.slides.length;
  expect(slideCount).toBeGreaterThan(0);
  const firstSlide = detail.slides[0];

  // --- Navigate to tablet UI ---
  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForLoadState("networkidle");
  await page.waitForFunction(
    () => (window as any).__presenterTabletReady === true,
    {
      timeout: 20_000,
    },
  );

  // --- Verify presentation appears in sidebar ---
  const presentationButton = page.locator(
    `[data-role="presentation-button"][data-presentation-id="${presentationId}"]`,
  );
  await presentationButton.waitFor({ state: "visible", timeout: 10_000 });
  await expect(
    presentationButton.locator(".tablet-button__label"),
  ).toContainText(presentationName);
  await expect(presentationButton.locator(".tablet-button__meta")).toHaveText(
    String(slideCount),
  );

  // --- Click presentation to load slides ---
  await presentationButton.click();

  // Verify context title updates
  await expect(page.locator('[data-role="context-title"]')).toHaveText(
    presentationName,
  );

  // Verify slides render in main area
  const slideCards = page.locator('[data-role="tablet-slide"]');
  await expect(slideCards).toHaveCount(slideCount, { timeout: 10_000 });

  // Verify first slide content
  const firstCard = slideCards.first();
  if (firstSlide.main) {
    await expect(firstCard.locator(".tablet-slide__main")).toContainText(
      firstSlide.main.substring(0, 20),
    );
  }
  if (firstSlide.mainReference) {
    await expect(firstCard.locator(".tablet-slide__ref")).toContainText(
      firstSlide.mainReference,
    );
  }

  // --- Click a slide to trigger Bible passage ---
  await firstCard.click();

  // Verify toast shows success
  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });
  await expect(toast).toContainText("Slide triggered");

  // Verify Bible active endpoint has the triggered passage
  await expect(async () => {
    const activeResponse = await request.get(
      new URL("/bible/active", baseURL).toString(),
      { timeout: 15_000 },
    );
    expect(activeResponse.ok()).toBeTruthy();
    const active = await activeResponse.json();
    expect(active).not.toBeNull();
    expect(active.passage.reference.book).toBe("John");
    expect(active.passage.reference.chapter).toBe(3);
  }).toPass({ timeout: 15_000, intervals: [300] });

  // Verify the clicked slide gets is-active class
  await expect(async () => {
    const hasActive = await firstCard.evaluate((el) =>
      el.classList.contains("is-active"),
    );
    expect(hasActive).toBe(true);
  }).toPass({ timeout: 10_000, intervals: [300] });

  // --- Verify no song/library/playlist content is visible ---
  await expect(page.locator('[data-role="library-list"]')).toHaveCount(0);
  await expect(page.locator('[data-role="playlist-list"]')).toHaveCount(0);
  await expect(page.locator('[data-role="mode-toggle"]')).toHaveCount(0);
  await expect(page.locator('[data-role="editor"]')).toHaveCount(0);

  // --- Click a different slide (if multiple) to verify switching ---
  if (slideCount > 1) {
    const secondCard = slideCards.nth(1);
    await secondCard.click();
    await expect(toast).toHaveAttribute("data-visible", "true", {
      timeout: 10_000,
    });

    // Verify second slide becomes active
    await expect(async () => {
      const secondActive = await secondCard.evaluate((el) =>
        el.classList.contains("is-active"),
      );
      expect(secondActive).toBe(true);
    }).toPass({ timeout: 10_000, intervals: [300] });
  }
});
