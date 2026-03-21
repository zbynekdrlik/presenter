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
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('[data-role="presentation-list"]', {
    state: "visible",
    timeout: 20_000,
  });

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

test("tablet handles slides without Bible metadata gracefully", async ({
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

  // --- Setup: create a presentation with slides that have NO metadata ---
  const presentationName = `NoMeta E2E ${Date.now()}`;
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

  // Append slides WITHOUT metadata — simulates manually-created slides
  const appendResponse = await request.post(
    new URL(
      `/bible/presentations/${presentationId}/append`,
      baseURL,
    ).toString(),
    {
      data: {
        slides: [
          {
            main: "For God so loved the world",
            translation: "",
            stage: "For God so loved the world",
            group: null,
          },
          {
            main: "That he gave his only begotten Son",
            translation: "Secondary text here",
            stage: "That he gave his only begotten Son",
            group: null,
          },
        ],
      },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );
  expect(appendResponse.ok()).toBeTruthy();

  // --- Navigate to tablet UI ---
  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('[data-role="presentation-list"]', {
    state: "visible",
    timeout: 20_000,
  });

  // --- Select the presentation ---
  const presentationButton = page.locator(
    `[data-role="presentation-button"][data-presentation-id="${presentationId}"]`,
  );
  await presentationButton.waitFor({ state: "visible", timeout: 10_000 });
  await presentationButton.click();

  // Verify slides render
  const slideCards = page.locator('[data-role="tablet-slide"]');
  await expect(slideCards).toHaveCount(2, { timeout: 10_000 });

  // Verify first slide content
  await expect(slideCards.first().locator(".tablet-slide__main")).toContainText(
    "For God so loved the world",
  );

  // --- Click the metadata-less slide — this is the bug repro ---
  await slideCards.first().click();

  // Verify toast shows success (NOT an error like "Slide has no Bible metadata")
  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });
  await expect(toast).toContainText("Slide triggered");

  // Verify the slide output was sent via the trigger-slide endpoint
  await expect(async () => {
    const activeSlideResponse = await request.get(
      new URL("/bible/active-slide", baseURL).toString(),
      { timeout: 15_000 },
    );
    expect(activeSlideResponse.ok()).toBeTruthy();
    const activeSlide = await activeSlideResponse.json();
    expect(activeSlide).not.toBeNull();
    expect(activeSlide.mainText).toBe("For God so loved the world");
  }).toPass({ timeout: 15_000, intervals: [300] });

  // --- Click second metadata-less slide with translation text ---
  await slideCards.nth(1).click();
  await expect(toast).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });
  await expect(toast).toContainText("Slide triggered");

  // Verify second slide output includes secondary text
  await expect(async () => {
    const activeSlideResponse = await request.get(
      new URL("/bible/active-slide", baseURL).toString(),
      { timeout: 15_000 },
    );
    expect(activeSlideResponse.ok()).toBeTruthy();
    const activeSlide = await activeSlideResponse.json();
    expect(activeSlide).not.toBeNull();
    expect(activeSlide.mainText).toBe("That he gave his only begotten Son");
    expect(activeSlide.secondaryText).toBe("Secondary text here");
  }).toPass({ timeout: 15_000, intervals: [300] });
});

test("tablet text scale slider updates CSS and persists", async ({
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
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Get initial scale value
  const scaleSlider = page.locator('[data-role="scale-slider"]');
  await scaleSlider.waitFor({ state: "visible", timeout: 10_000 });
  const initialValue = await scaleSlider.inputValue();
  expect(Number(initialValue)).toBeGreaterThanOrEqual(50);
  expect(Number(initialValue)).toBeLessThanOrEqual(200);

  // Change scale to 150%
  await scaleSlider.fill("150");
  await scaleSlider.dispatchEvent("input");

  // Verify CSS custom property updated
  await expect(async () => {
    const scale = await page.evaluate(() =>
      document.body.style.getPropertyValue("--tablet-scale"),
    );
    expect(scale).toBe("1.5");
  }).toPass({ timeout: 5_000, intervals: [200] });

  // Verify display shows 150%
  await expect(page.locator('[data-role="scale-value"]')).toHaveText("150%");

  // Reload page and verify scale persists
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  await expect(async () => {
    const scale = await page.evaluate(() =>
      document.body.style.getPropertyValue("--tablet-scale"),
    );
    expect(scale).toBe("1.5");
  }).toPass({ timeout: 10_000, intervals: [300] });

  await expect(page.locator('[data-role="scale-value"]')).toHaveText("150%");
});

test("tablet sidebar collapse and expand", async ({ page, request }) => {
  // Wait for server readiness
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  // Create a presentation so there's something to click
  const presentationName = `Sidebar E2E ${Date.now()}`;
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

  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('[data-role="presentation-list"]', {
    state: "visible",
    timeout: 20_000,
  });

  // Sidebar should be open initially
  const sidebar = page.locator(".tablet-sidebar");
  await expect(sidebar).not.toHaveClass(/is-collapsed/);

  // Click a presentation — sidebar should collapse
  const presentationButton = page.locator(
    `[data-role="presentation-button"][data-presentation-id="${presentationId}"]`,
  );
  await presentationButton.waitFor({ state: "visible", timeout: 10_000 });
  await presentationButton.click();

  await expect(sidebar).toHaveClass(/is-collapsed/, { timeout: 5_000 });

  // Toggle button should be visible when sidebar is collapsed
  const toggleButton = page.locator('[data-role="sidebar-toggle"]');
  await expect(toggleButton).toBeVisible({ timeout: 5_000 });

  // Click toggle to re-open sidebar
  await toggleButton.click();
  await expect(sidebar).not.toHaveClass(/is-collapsed/, { timeout: 5_000 });

  // Close button should work
  const closeButton = page.locator('[data-role="sidebar-close"]');
  await closeButton.click();
  await expect(sidebar).toHaveClass(/is-collapsed/, { timeout: 5_000 });
});

test("tablet shows empty message for presentation with no slides", async ({
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

  // Create an empty presentation (no slides)
  const presentationName = `Empty E2E ${Date.now()}`;
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

  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('[data-role="presentation-list"]', {
    state: "visible",
    timeout: 20_000,
  });

  // Click the empty presentation
  const presentationButton = page.locator(
    `[data-role="presentation-button"][data-presentation-id="${presentationId}"]`,
  );
  await presentationButton.waitFor({ state: "visible", timeout: 10_000 });
  await presentationButton.click();

  // Wait for context title to confirm presentation is selected
  await expect(page.locator('[data-role="context-title"]')).toHaveText(
    presentationName,
    { timeout: 10_000 },
  );

  // Verify empty state message (use polling to handle async slide loading)
  await expect(async () => {
    const emptyMsg = page.locator(".tablet-slides__empty");
    await expect(emptyMsg).toContainText("No slides in this presentation");
  }).toPass({ timeout: 10_000, intervals: [300] });

  // Verify no slide cards rendered
  await expect(page.locator('[data-role="tablet-slide"]')).toHaveCount(0);
});

test("tablet switches between multiple presentations", async ({
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

  // Create two presentations with different slides
  const name1 = `Multi1 E2E ${Date.now()}`;
  const name2 = `Multi2 E2E ${Date.now()}`;

  const create1 = await request.post(
    new URL("/bible/presentations", baseURL).toString(),
    {
      data: { name: name1 },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );
  expect(create1.ok()).toBeTruthy();
  const pres1 = await create1.json();

  const create2 = await request.post(
    new URL("/bible/presentations", baseURL).toString(),
    {
      data: { name: name2 },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );
  expect(create2.ok()).toBeTruthy();
  const pres2 = await create2.json();

  // Add different slides to each
  await request.post(
    new URL(`/bible/presentations/${pres1.id}/append`, baseURL).toString(),
    {
      data: {
        slides: [
          {
            main: "First presentation slide one",
            translation: "",
            stage: "First presentation slide one",
          },
        ],
      },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );
  await request.post(
    new URL(`/bible/presentations/${pres2.id}/append`, baseURL).toString(),
    {
      data: {
        slides: [
          {
            main: "Second presentation slide one",
            translation: "",
            stage: "Second presentation slide one",
          },
          {
            main: "Second presentation slide two",
            translation: "",
            stage: "Second presentation slide two",
          },
        ],
      },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );

  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('[data-role="presentation-list"]', {
    state: "visible",
    timeout: 20_000,
  });

  // Select first presentation
  const btn1 = page.locator(
    `[data-role="presentation-button"][data-presentation-id="${pres1.id}"]`,
  );
  await btn1.waitFor({ state: "visible", timeout: 10_000 });
  await btn1.click();

  // Verify first presentation slides
  const slideCards = page.locator('[data-role="tablet-slide"]');
  await expect(slideCards).toHaveCount(1, { timeout: 10_000 });
  await expect(slideCards.first().locator(".tablet-slide__main")).toContainText(
    "First presentation slide one",
  );

  // Re-open sidebar and switch to second presentation
  const toggleButton = page.locator('[data-role="sidebar-toggle"]');
  await toggleButton.click();
  await page.waitForSelector('[data-role="presentation-list"]', {
    state: "visible",
    timeout: 5_000,
  });

  const btn2 = page.locator(
    `[data-role="presentation-button"][data-presentation-id="${pres2.id}"]`,
  );
  await btn2.click();

  // Verify second presentation slides
  await expect(slideCards).toHaveCount(2, { timeout: 10_000 });
  await expect(slideCards.first().locator(".tablet-slide__main")).toContainText(
    "Second presentation slide one",
  );
  await expect(slideCards.nth(1).locator(".tablet-slide__main")).toContainText(
    "Second presentation slide two",
  );

  // Verify context title updated
  await expect(page.locator('[data-role="context-title"]')).toHaveText(name2);
});
