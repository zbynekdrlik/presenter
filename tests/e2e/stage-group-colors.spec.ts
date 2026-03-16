import { test, expect, Page, BrowserContext } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let dbUrl: string;
let port: number;

test.describe.configure({ timeout: 180_000 });

async function waitForOperatorReady(page: Page) {
  await page.goto(new URL("/legacy", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForLoadState("networkidle");
  await page.waitForFunction(() => window.__presenterLiveConnected === true, {
    timeout: 30_000,
  });
}

async function openStageDisplay(
  context: BrowserContext,
  layoutCode: "worship-snv" | "worship-pp",
) {
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: layoutCode },
  });
  const stagePage = await context.newPage();
  await stagePage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await stagePage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );
  return stagePage;
}

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  dbUrl = config.dbUrl;
  port = config.port;
  await refreshDevData(dbUrl);
  serverHandle = await startTestServer(port, dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

/**
 * Extract computed color from an element's style.
 * Returns the color value as a string (e.g., "#fb7185" or "rgb(251, 113, 133)").
 */
async function getGroupColor(
  page: Page,
  elementId: string,
): Promise<string | null> {
  return page.evaluate((id) => {
    const el = document.getElementById(id);
    if (!el) return null;
    // Get the inline style color which is set by applyGroupColor()
    return el.style.color || null;
  }, elementId);
}

/**
 * Extract computed background color from an element's style.
 */
async function getGroupBgColor(
  page: Page,
  elementId: string,
): Promise<string | null> {
  return page.evaluate((id) => {
    const el = document.getElementById(id);
    if (!el) return null;
    return el.style.backgroundColor || null;
  }, elementId);
}

test("worship-snv stage applies consistent colors to group badges", async ({
  page,
  context,
}) => {
  await waitForOperatorReady(page);

  // Create a presentation with slides that have different groups
  await page.locator('[data-role="presentation-create"]').click();
  const modal = page.locator('[data-role="presentation-create-modal"]');
  await expect(modal).toHaveAttribute("data-open", "true");

  const presName = `Group Colors E2E ${Date.now()}`;
  await page.locator('[data-role="presentation-create-name"]').fill(presName);
  await page.locator('[data-role="presentation-create-blank"]').click();
  await expect(modal).not.toHaveAttribute("data-open", "true", {
    timeout: 10_000,
  });

  // Find the created presentation and get its ID
  const librariesResponse = await page.request.get(
    new URL("/libraries", baseURL).toString(),
    { timeout: 60_000 },
  );
  const libraries: Array<{
    id: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await librariesResponse.json();
  let presId: string | undefined;
  for (const lib of libraries) {
    const found = lib.presentations.find((p) => p.name === presName);
    if (found) {
      presId = found.id;
      break;
    }
  }
  expect(presId).toBeTruthy();

  // Get the presentation slides
  const presResponse = await page.request.get(
    new URL(`/presentations/${presId}`, baseURL).toString(),
  );
  const presDetail: {
    presentation: {
      slides: Array<{ id: string }>;
    };
  } = await presResponse.json();

  // We need at least 3 slides; add more if needed
  const existingSlides = presDetail.presentation.slides;
  while (existingSlides.length < 3) {
    const addResponse = await page.request.post(
      new URL(`/presentations/${presId}/slides`, baseURL).toString(),
      { data: { main: "", translation: "", stage: "" } },
    );
    expect(addResponse.ok()).toBeTruthy();
    const newSlide = await addResponse.json();
    existingSlides.push(newSlide);
  }

  // Re-fetch to get updated slide list
  const updatedPresResponse = await page.request.get(
    new URL(`/presentations/${presId}`, baseURL).toString(),
  );
  const updatedPresDetail: {
    presentation: {
      slides: Array<{ id: string }>;
    };
  } = await updatedPresResponse.json();
  const slides = updatedPresDetail.presentation.slides;
  expect(slides.length).toBeGreaterThanOrEqual(3);

  // Update slides with different groups
  const groups = ["Men", "Women", "All"];
  for (let i = 0; i < 3; i++) {
    const updateResponse = await page.request.patch(
      new URL(
        `/presentations/${presId}/slides/${slides[i].id}`,
        baseURL,
      ).toString(),
      {
        data: {
          main: `Slide ${i + 1} text`,
          translation: "",
          stage: "",
          group: groups[i],
        },
      },
    );
    expect(updateResponse.ok()).toBeTruthy();
  }

  // Open stage display with worship-snv layout
  const stagePage = await openStageDisplay(context, "worship-snv");

  // Trigger first slide (Men = current, Women = next)
  const trigger1 = await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: presId,
        currentSlideId: slides[0].id,
      },
    },
  );
  expect(trigger1.ok()).toBeTruthy();

  // Wait for stage to update
  await stagePage.waitForFunction(
    () =>
      document.getElementById("current-text")?.textContent?.includes("Slide 1"),
    { timeout: 15_000 },
  );

  // Capture Men's color when it's the current group
  const menColorAsCurrent = await getGroupColor(stagePage, "current-group");
  const menBgAsCurrent = await getGroupBgColor(stagePage, "current-group");
  expect(menColorAsCurrent).toBeTruthy();
  expect(menBgAsCurrent).toBeTruthy();

  // Capture Women's color when it's the next group
  const womenColorAsNext = await getGroupColor(stagePage, "next-group");
  const womenBgAsNext = await getGroupBgColor(stagePage, "next-group");
  expect(womenColorAsNext).toBeTruthy();
  expect(womenBgAsNext).toBeTruthy();

  // Men and Women should have DIFFERENT colors
  expect(menColorAsCurrent).not.toBe(womenColorAsNext);

  // Trigger second slide (Women = current, All = next)
  const trigger2 = await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: presId,
        currentSlideId: slides[1].id,
      },
    },
  );
  expect(trigger2.ok()).toBeTruthy();

  // Wait for stage to update
  await stagePage.waitForFunction(
    () =>
      document.getElementById("current-text")?.textContent?.includes("Slide 2"),
    { timeout: 15_000 },
  );

  // Capture Women's color when it's NOW the current group
  const womenColorAsCurrent = await getGroupColor(stagePage, "current-group");
  expect(womenColorAsCurrent).toBeTruthy();

  // CRITICAL: Women's color should be the SAME whether it was next or current
  expect(womenColorAsCurrent).toBe(womenColorAsNext);

  // Capture All's color when it's the next group
  const allColorAsNext = await getGroupColor(stagePage, "next-group");
  expect(allColorAsNext).toBeTruthy();

  // All should have a DIFFERENT color than Women
  expect(allColorAsNext).not.toBe(womenColorAsCurrent);

  // Trigger third slide (All = current, no next)
  const trigger3 = await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: presId,
        currentSlideId: slides[2].id,
      },
    },
  );
  expect(trigger3.ok()).toBeTruthy();

  // Wait for stage to update
  await stagePage.waitForFunction(
    () =>
      document.getElementById("current-text")?.textContent?.includes("Slide 3"),
    { timeout: 15_000 },
  );

  // Capture All's color when it's NOW the current group
  const allColorAsCurrent = await getGroupColor(stagePage, "current-group");
  expect(allColorAsCurrent).toBeTruthy();

  // CRITICAL: All's color should be the SAME whether it was next or current
  expect(allColorAsCurrent).toBe(allColorAsNext);

  await stagePage.close();
});

test("worship-pp stage applies consistent colors to group badges", async ({
  page,
  context,
}) => {
  await waitForOperatorReady(page);

  // Create a presentation with slides that have different groups
  await page.locator('[data-role="presentation-create"]').click();
  const modal = page.locator('[data-role="presentation-create-modal"]');
  await expect(modal).toHaveAttribute("data-open", "true");

  const presName = `Group Colors PP ${Date.now()}`;
  await page.locator('[data-role="presentation-create-name"]').fill(presName);
  await page.locator('[data-role="presentation-create-blank"]').click();
  await expect(modal).not.toHaveAttribute("data-open", "true", {
    timeout: 10_000,
  });

  // Find the created presentation
  const librariesResponse = await page.request.get(
    new URL("/libraries", baseURL).toString(),
    { timeout: 60_000 },
  );
  const libraries: Array<{
    id: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await librariesResponse.json();
  let presId: string | undefined;
  for (const lib of libraries) {
    const found = lib.presentations.find((p) => p.name === presName);
    if (found) {
      presId = found.id;
      break;
    }
  }
  expect(presId).toBeTruthy();

  // Get and extend slides
  const presResponse = await page.request.get(
    new URL(`/presentations/${presId}`, baseURL).toString(),
  );
  const presDetail: {
    presentation: { slides: Array<{ id: string }> };
  } = await presResponse.json();

  const existingSlides = presDetail.presentation.slides;
  while (existingSlides.length < 2) {
    const addResponse = await page.request.post(
      new URL(`/presentations/${presId}/slides`, baseURL).toString(),
      { data: { main: "", translation: "", stage: "" } },
    );
    expect(addResponse.ok()).toBeTruthy();
    const newSlide = await addResponse.json();
    existingSlides.push(newSlide);
  }

  // Re-fetch slides
  const updatedPresResponse = await page.request.get(
    new URL(`/presentations/${presId}`, baseURL).toString(),
  );
  const updatedPresDetail: {
    presentation: { slides: Array<{ id: string }> };
  } = await updatedPresResponse.json();
  const slides = updatedPresDetail.presentation.slides;

  // Update slides with groups (use "Men" and "Women" which are known to have different colors)
  const groups = ["Men", "Women"];
  for (let i = 0; i < 2; i++) {
    const updateResponse = await page.request.patch(
      new URL(
        `/presentations/${presId}/slides/${slides[i].id}`,
        baseURL,
      ).toString(),
      {
        data: {
          main: `PP Slide ${i + 1}`,
          translation: "",
          stage: "",
          group: groups[i],
        },
      },
    );
    expect(updateResponse.ok()).toBeTruthy();
  }

  // Open worship-pp stage
  const stagePage = await openStageDisplay(context, "worship-pp");

  // Trigger first slide
  const trigger1 = await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: presId,
        currentSlideId: slides[0].id,
      },
    },
  );
  expect(trigger1.ok()).toBeTruthy();

  // Wait for stage update
  await stagePage.waitForFunction(
    () =>
      document
        .getElementById("current-main")
        ?.textContent?.includes("PP Slide 1"),
    { timeout: 15_000 },
  );

  // Capture Men color as current
  const menColorAsCurrent = await getGroupColor(stagePage, "current-group");
  expect(menColorAsCurrent).toBeTruthy();

  // Capture Women color as next
  const womenColorAsNext = await getGroupColor(stagePage, "next-group");
  expect(womenColorAsNext).toBeTruthy();

  // Should be different
  expect(menColorAsCurrent).not.toBe(womenColorAsNext);

  // Trigger second slide
  const trigger2 = await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: presId,
        currentSlideId: slides[1].id,
      },
    },
  );
  expect(trigger2.ok()).toBeTruthy();

  // Wait for update
  await stagePage.waitForFunction(
    () =>
      document
        .getElementById("current-main")
        ?.textContent?.includes("PP Slide 2"),
    { timeout: 15_000 },
  );

  // Women is now current - should have same color as when it was next
  const womenColorAsCurrent = await getGroupColor(stagePage, "current-group");
  expect(womenColorAsCurrent).toBe(womenColorAsNext);

  await stagePage.close();
});

test("group color is cleared when group is empty", async ({
  page,
  context,
}) => {
  await waitForOperatorReady(page);

  // Create presentation
  await page.locator('[data-role="presentation-create"]').click();
  const modal = page.locator('[data-role="presentation-create-modal"]');
  await expect(modal).toHaveAttribute("data-open", "true");

  const presName = `No Group E2E ${Date.now()}`;
  await page.locator('[data-role="presentation-create-name"]').fill(presName);
  await page.locator('[data-role="presentation-create-blank"]').click();
  await expect(modal).not.toHaveAttribute("data-open", "true", {
    timeout: 10_000,
  });

  // Find presentation
  const librariesResponse = await page.request.get(
    new URL("/libraries", baseURL).toString(),
  );
  const libraries: Array<{
    id: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await librariesResponse.json();
  let presId: string | undefined;
  for (const lib of libraries) {
    const found = lib.presentations.find((p) => p.name === presName);
    if (found) {
      presId = found.id;
      break;
    }
  }
  expect(presId).toBeTruthy();

  // Get slides
  const presResponse = await page.request.get(
    new URL(`/presentations/${presId}`, baseURL).toString(),
  );
  const presDetail: {
    presentation: { slides: Array<{ id: string }> };
  } = await presResponse.json();
  const slides = presDetail.presentation.slides;

  // Update first slide WITHOUT a group
  await page.request.patch(
    new URL(
      `/presentations/${presId}/slides/${slides[0].id}`,
      baseURL,
    ).toString(),
    {
      data: {
        main: "No group slide",
        translation: "",
        stage: "",
      },
    },
  );

  // Open stage
  const stagePage = await openStageDisplay(context, "worship-snv");

  // Trigger slide
  await page.request.post(new URL("/stage/state", baseURL).toString(), {
    data: {
      presentationId: presId,
      currentSlideId: slides[0].id,
    },
  });

  // Wait for update
  await stagePage.waitForFunction(
    () =>
      document
        .getElementById("current-text")
        ?.textContent?.includes("No group"),
    { timeout: 15_000 },
  );

  // Current group should have no inline color (empty or null)
  const color = await getGroupColor(stagePage, "current-group");
  expect(color).toBeFalsy();

  // Element should be hidden
  const isHidden = await stagePage.evaluate(() => {
    const el = document.getElementById("current-group");
    return el?.dataset.hidden === "true";
  });
  expect(isHidden).toBe(true);

  await stagePage.close();
});
