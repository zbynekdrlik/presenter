import { test, expect, Page } from "@playwright/test";
import * as path from "path";
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
let dbUrl: string;
let port: number;

async function waitForOperatorReady(page: Page) {
  await page.goto(new URL("/ui/operator", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForLoadState("networkidle");
  await page.waitForFunction(
    () => (window as any).__presenterLiveConnected === true,
    { timeout: 30_000 },
  );
}

async function selectLibrary(page: Page): Promise<string> {
  const libraryItem = page.locator('[data-role="library-item"]').first();
  await expect(libraryItem).toBeVisible({ timeout: 10_000 });
  await libraryItem.scrollIntoViewIfNeeded();
  await libraryItem.click({ force: true });
  await page.waitForTimeout(300);
  const libraryId = await libraryItem.getAttribute("data-library-id");
  expect(libraryId).toBeTruthy();
  return libraryId!;
}

async function selectPlaylist(page: Page): Promise<string> {
  const playlistItem = page.locator('[data-role="playlist-item"]').first();
  await expect(playlistItem).toBeVisible({ timeout: 10_000 });
  await playlistItem.click();
  const playlistId = await playlistItem.getAttribute("data-playlist-id");
  expect(playlistId).toBeTruthy();
  return playlistId!;
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

test("delete button visible in library context, hidden in playlist context", async ({
  page,
}) => {
  await waitForOperatorReady(page);

  // Select a library and switch to edit mode
  await selectLibrary(page);
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();

  // Click the rename button on the first presentation
  const renameButton = page
    .locator(
      '[data-role="presentation-list"] [data-action="presentation-rename"]',
    )
    .first();
  await expect(renameButton).toBeVisible({ timeout: 10_000 });
  await renameButton.click();

  // The presentation edit modal should be open with delete visible
  const editModal = page.locator('[data-role="presentation-edit-modal"]');
  await expect(editModal).toHaveAttribute("data-open", "true");
  const deleteButton = page.locator('[data-role="presentation-edit-delete"]');
  await expect(deleteButton).toBeVisible();

  // Close the modal
  await page.locator('[data-role="presentation-edit-cancel"]').click();

  // Now switch to a playlist
  await selectPlaylist(page);

  // In playlist view, click a presentation rename button
  const playlistRenameButton = page
    .locator(
      '[data-role="presentation-list"] [data-action="presentation-rename"]',
    )
    .first();
  // It may or may not be visible (depends on edit mode for playlist view),
  // but if we can click it, delete should be hidden
  const isRenameVisible = await playlistRenameButton
    .isVisible()
    .catch(() => false);
  if (isRenameVisible) {
    await playlistRenameButton.click();
    await expect(editModal).toHaveAttribute("data-open", "true");
    // Delete button should be hidden (display:none) in playlist context
    await expect(deleteButton).toBeHidden();
    await page.locator('[data-role="presentation-edit-cancel"]').click();
  }
});

test("edit pen button hidden in live mode, visible in edit mode", async ({
  page,
}) => {
  await waitForOperatorReady(page);

  // Select a library
  await selectLibrary(page);

  // In live mode, no rename buttons should be visible
  await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
  await page.waitForTimeout(200);
  const renameButtons = page.locator(
    '[data-role="presentation-list"] [data-action="presentation-rename"]',
  );
  await expect(renameButtons).toHaveCount(0);

  // Switch to edit mode
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
  await page.waitForTimeout(200);

  // Now rename buttons should be visible
  const editRenameButtons = page.locator(
    '[data-role="presentation-list"] [data-action="presentation-rename"]',
  );
  const count = await editRenameButtons.count();
  expect(count).toBeGreaterThan(0);
});

test("create blank presentation via modal", async ({ page }) => {
  await waitForOperatorReady(page);

  // Select existing library and use it
  const libraryId = await selectLibrary(page);

  // Open create modal
  const createButton = page.locator('[data-role="presentation-create"]');
  await expect(createButton).toBeVisible();
  await createButton.click();

  // The create modal should be open
  const createModal = page.locator('[data-role="presentation-create-modal"]');
  await expect(createModal).toHaveAttribute("data-open", "true");

  // Set a name
  const nameInput = page.locator('[data-role="presentation-create-name"]');
  await nameInput.fill("E2E Blank Song");

  // Click the Blank option
  await page.locator('[data-role="presentation-create-blank"]').click();

  // Modal should close
  await expect(createModal).not.toHaveAttribute("data-open", "true", {
    timeout: 10_000,
  });

  // Verify via API that the presentation was created
  const detailResp = await page.request.get(
    new URL("/libraries", baseURL).toString(),
  );
  expect(detailResp.ok()).toBeTruthy();
  const libs: Array<{
    id: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await detailResp.json();
  const updatedLib = libs.find((l) => l.id === libraryId);
  expect(updatedLib).toBeTruthy();
  const created = updatedLib!.presentations.find(
    (p) => p.name === "E2E Blank Song",
  );
  expect(created).toBeTruthy();

  // Fetch presentation detail to verify it has the default blank slide
  const presResp = await page.request.get(
    new URL(`/presentations/${created!.id}`, baseURL).toString(),
  );
  expect(presResp.ok()).toBeTruthy();
  const presData: { presentation: { slides: Array<any> } } =
    await presResp.json();
  // Blank creation sends slides: [] which results in one default empty slide
  expect(presData.presentation.slides.length).toBe(1);
});

test("create presentation from pasted song text", async ({ page }) => {
  await waitForOperatorReady(page);

  // Select existing library
  const libraryId = await selectLibrary(page);

  // Open create modal
  await page.locator('[data-role="presentation-create"]').click();
  const createModal = page.locator('[data-role="presentation-create-modal"]');
  await expect(createModal).toHaveAttribute("data-open", "true");

  // Leave name input empty — Title line from pasted text should be used
  const expectedName = `E2E Title Song ${Date.now()}`;

  // Click Paste option
  await page.locator('[data-role="presentation-create-paste"]').click();

  // Options should be hidden, paste area visible
  await expect(
    page.locator('[data-role="presentation-create-options"]'),
  ).toBeHidden();
  await expect(
    page.locator('[data-role="presentation-create-paste-area"]'),
  ).toBeVisible();

  // Paste song text with Title line and groups
  const songText = [
    `Title: ${expectedName}`,
    "Verse 1",
    "Amazing grace how sweet the sound",
    "That saved a wretch like me",
    "",
    "Chorus",
    "I once was lost but now am found",
    "Was blind but now I see",
    "",
    "Verse 2",
    "Through many dangers toils and snares",
    "I have already come",
  ].join("\n");

  await page
    .locator('[data-role="presentation-create-paste-text"]')
    .fill(songText);

  // Click Create
  await page.locator('[data-role="presentation-create-paste-confirm"]').click();

  // Modal should close
  await expect(createModal).not.toHaveAttribute("data-open", "true", {
    timeout: 10_000,
  });

  // Verify presentation was created with the title from pasted text
  const libs: Array<{
    id: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await (
    await page.request.get(new URL("/libraries", baseURL).toString())
  ).json();
  const updatedLib = libs.find((l) => l.id === libraryId);
  expect(updatedLib).toBeTruthy();
  const created = updatedLib!.presentations.find(
    (p) => p.name === expectedName,
  );
  expect(created).toBeTruthy();

  // Check slides have correct groups
  const presResp = await page.request.get(
    new URL(`/presentations/${created!.id}`, baseURL).toString(),
  );
  expect(presResp.ok()).toBeTruthy();
  const presData: {
    presentation: {
      slides: Array<{
        content: {
          main: { value: string };
          group?: { name: string } | null;
        };
      }>;
    };
  } = await presResp.json();
  const slides = presData.presentation.slides;
  // 3 content slides + empty first + empty last = 5
  expect(slides.length).toBe(5);
  // First slide is empty
  expect(slides[0].content.main.value).toBe("");
  // Content slides
  expect(slides[1].content.group?.name).toBe("Verse 1");
  expect(slides[1].content.main.value).toContain("Amazing grace");
  expect(slides[2].content.group?.name).toBe("Chorus");
  expect(slides[2].content.main.value).toContain("I once was lost");
  expect(slides[3].content.group?.name).toBe("Verse 2");
  expect(slides[3].content.main.value).toContain("Through many dangers");
  // Last slide is empty
  expect(slides[4].content.main.value).toBe("");
});

test("parseSongText handles Title extraction and Misc skipping", async ({
  page,
}) => {
  await waitForOperatorReady(page);

  const result = await page.evaluate(() => {
    const helpers = (window as any).__presenterOperatorTestHelpers;
    if (!helpers || !helpers.parseSongText) {
      throw new Error("parseSongText not available");
    }
    return helpers.parseSongText(
      [
        "Title: My Song",
        "Misc 1",
        "Author: Test",
        "",
        "Verse",
        "First verse lyrics",
        "",
        "Chorus",
        "Chorus lyrics here",
      ].join("\n"),
    );
  });

  expect(result.title).toBe("My Song");
  expect(result.slides.length).toBe(2);
  expect(result.slides[0].group).toBe("Verse");
  expect(result.slides[0].main).toBe("First verse lyrics");
  expect(result.slides[1].group).toBe("Chorus");
  expect(result.slides[1].main).toBe("Chorus lyrics here");
});

test("parseSongText chunks long groups into 2-line slides", async ({
  page,
}) => {
  await waitForOperatorReady(page);

  const result = await page.evaluate(() => {
    const helpers = (window as any).__presenterOperatorTestHelpers;
    return helpers.parseSongText(
      [
        "Verse 1",
        "Line one",
        "Line two",
        "Line three",
        "Line four",
        "Line five",
      ].join("\n"),
    );
  });

  expect(result.slides.length).toBe(3);
  expect(result.slides[0].group).toBe("Verse 1");
  expect(result.slides[0].main).toBe("Line one\nLine two");
  expect(result.slides[1].group).toBe("Verse 1");
  expect(result.slides[1].main).toBe("Line three\nLine four");
  expect(result.slides[2].group).toBe("Verse 1");
  expect(result.slides[2].main).toBe("Line five");
});

test("import .pro file via create modal", async ({ page }) => {
  await waitForOperatorReady(page);

  // Select existing library
  const libraryId = await selectLibrary(page);

  // Count existing presentations before import
  const beforeLibs: Array<{
    id: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await (
    await page.request.get(new URL("/libraries", baseURL).toString())
  ).json();
  const beforeLib = beforeLibs.find((l) => l.id === libraryId);
  const beforeCount = beforeLib ? beforeLib.presentations.length : 0;

  // Open create modal
  await page.locator('[data-role="presentation-create"]').click();
  const createModal = page.locator('[data-role="presentation-create-modal"]');
  await expect(createModal).toHaveAttribute("data-open", "true");

  // Click Import option
  await page.locator('[data-role="presentation-create-import"]').click();

  // Options should be hidden, import area visible
  await expect(
    page.locator('[data-role="presentation-create-options"]'),
  ).toBeHidden();
  await expect(
    page.locator('[data-role="presentation-create-import-area"]'),
  ).toBeVisible();

  // Upload the .pro file
  const fileInput = page.locator(
    '[data-role="presentation-create-import-file"]',
  );
  const fixturePath = path.resolve(__dirname, "fixtures", "test-import.pro");
  await fileInput.setInputFiles(fixturePath);

  // Click Import button
  await page
    .locator('[data-role="presentation-create-import-confirm"]')
    .click();

  // Modal should close
  await expect(createModal).not.toHaveAttribute("data-open", "true", {
    timeout: 15_000,
  });

  // Verify presentation was created in the library
  const libs: Array<{
    id: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await (
    await page.request.get(new URL("/libraries", baseURL).toString())
  ).json();
  const updatedLib = libs.find((l) => l.id === libraryId);
  expect(updatedLib).toBeTruthy();
  expect(updatedLib!.presentations.length).toBeGreaterThan(beforeCount);

  // The imported presentation should have slides
  const importedPres = updatedLib!.presentations.find(
    (p) => !beforeLib?.presentations.some((bp) => bp.id === p.id),
  );
  expect(importedPres).toBeTruthy();
  const presResp = await page.request.get(
    new URL(`/presentations/${importedPres!.id}`, baseURL).toString(),
  );
  expect(presResp.ok()).toBeTruthy();
  const presData: { presentation: { slides: Array<any>; name: string } } =
    await presResp.json();
  expect(presData.presentation.slides.length).toBeGreaterThan(0);
  expect(presData.presentation.name).toBeTruthy();
});

test("create modal Escape closes it", async ({ page }) => {
  await waitForOperatorReady(page);

  // Select a library
  await selectLibrary(page);

  // Open create modal
  await page.locator('[data-role="presentation-create"]').click();
  const createModal = page.locator('[data-role="presentation-create-modal"]');
  await expect(createModal).toHaveAttribute("data-open", "true");

  // Press Escape
  await page.keyboard.press("Escape");

  // Modal should close
  await expect(createModal).not.toHaveAttribute("data-open", "true");
});

test("paste area Back button returns to options", async ({ page }) => {
  await waitForOperatorReady(page);

  await selectLibrary(page);

  await page.locator('[data-role="presentation-create"]').click();
  const createModal = page.locator('[data-role="presentation-create-modal"]');
  await expect(createModal).toHaveAttribute("data-open", "true");

  // Click Paste
  await page.locator('[data-role="presentation-create-paste"]').click();
  await expect(
    page.locator('[data-role="presentation-create-paste-area"]'),
  ).toBeVisible();
  await expect(
    page.locator('[data-role="presentation-create-options"]'),
  ).toBeHidden();

  // Click Back
  await page.locator('[data-role="presentation-create-paste-back"]').click();
  await expect(
    page.locator('[data-role="presentation-create-paste-area"]'),
  ).toBeHidden();
  await expect(
    page.locator('[data-role="presentation-create-options"]'),
  ).toBeVisible();

  // Close modal
  await page.locator('[data-role="presentation-create-cancel"]').click();
  await expect(createModal).not.toHaveAttribute("data-open", "true");
});
