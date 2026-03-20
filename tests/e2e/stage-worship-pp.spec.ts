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
  await page.goto(new URL("/ui/operator", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });
}

async function openStage(context: BrowserContext) {
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-pp" },
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

test("worship-pp stage displays playlist sidebar when triggered from playlist", async ({
  page,
  context,
}) => {
  await waitForOperatorReady(page);

  // Create a playlist with dashboard visibility
  await page.locator('[data-role="playlist-create"]').click();
  const playlistModal = page.locator('[data-role="playlist-edit-modal"]');
  await expect(playlistModal).toHaveAttribute("data-open", "true");
  const playlistName = `Stage E2E ${Date.now()}`;
  await page.locator('[data-role="playlist-edit-name"]').fill(playlistName);
  // Enable "Show in dashboard" so the playlist appears in the sidebar
  const dashboardCheckbox = page.locator(
    '[data-role="playlist-edit-dashboard"]',
  );
  if (!(await dashboardCheckbox.isChecked())) {
    await dashboardCheckbox.check();
  }
  await page.locator('[data-role="playlist-edit-save"]').click();
  await expect(playlistModal).toHaveAttribute("data-open", "false");

  // Wait for playlist to appear and click to select it
  const newPlaylist = page.locator('[data-role="playlist-item"]', {
    hasText: playlistName,
  });
  await expect(newPlaylist).toBeVisible({ timeout: 10_000 });
  await newPlaylist.click();

  const activePlaylist = page.locator(
    '[data-role="playlist-item"][data-active="true"]',
  );
  await expect(activePlaylist).toContainText(playlistName);

  // Find a library with presentations via API
  const librariesResponse = await page.request.get(
    new URL("/libraries", baseURL).toString(),
    { timeout: 60_000 },
  );
  expect(librariesResponse.ok()).toBeTruthy();
  const libraries: Array<{
    id: string;
    name: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await librariesResponse.json();
  const sourceLibrary = libraries.find(
    (lib) => Array.isArray(lib.presentations) && lib.presentations.length >= 3,
  );
  if (!sourceLibrary) {
    throw new Error("Expected at least one library with 3+ presentations");
  }

  // Add 3 presentations to the playlist via API (more reliable than drag)
  const playlistsBeforeAdd = await page.request.get(
    new URL("/playlists", baseURL).toString(),
  );
  const playlistsBefore: Array<{ id: string; name: string }> =
    await playlistsBeforeAdd.json();
  const targetPlaylist = playlistsBefore.find((p) => p.name === playlistName);
  expect(targetPlaylist).toBeTruthy();
  const playlistId = targetPlaylist!.id;

  const presIds = sourceLibrary.presentations.slice(0, 3).map((p) => p.id);
  const playlistEntries = presIds.map((id) => ({
    type: "presentation",
    presentation_id: id,
  }));
  const replaceResp = await page.request.put(
    new URL(`/playlists/${playlistId}/entries`, baseURL).toString(),
    { data: { entries: playlistEntries } },
  );
  expect(replaceResp.ok()).toBeTruthy();

  // Reload page to see updated playlist
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForSelector('[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });

  // Re-select the playlist
  const reloadedPlaylist = page.locator('[data-role="playlist-item"]', {
    hasText: playlistName,
  });
  await expect(reloadedPlaylist).toBeVisible({ timeout: 10_000 });
  await reloadedPlaylist.click();

  // Verify 3 items in playlist
  const playlistItems = page
    .locator('[data-role="presentation-list"]')
    .locator('[data-role="presentation-item"]');
  await expect(playlistItems).toHaveCount(3, { timeout: 15_000 });

  // Get first presentation's slide for triggering via API
  const firstPresId = sourceLibrary.presentations[0].id;
  const presResponse = await page.request.get(
    new URL(`/presentations/${firstPresId}`, baseURL).toString(),
  );
  expect(presResponse.ok()).toBeTruthy();
  const presDetail: {
    presentation: {
      slides: Array<{
        id: string;
        content: { main: { value: string } };
      }>;
    };
  } = await presResponse.json();
  // Find a slide with non-empty main text
  const slideWithContent = presDetail.presentation.slides.find(
    (s) => s.content.main.value.trim().length > 0,
  );
  expect(slideWithContent).toBeTruthy();
  const slideId = slideWithContent!.id;

  // Open the stage display with worship-pp layout
  const stagePage = await openStage(context);

  // Trigger the stage state via API (with playlistId)
  const triggerResponse = await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: firstPresId,
        currentSlideId: slideId,
        playlistId: playlistId,
      },
    },
  );
  expect(triggerResponse.ok()).toBeTruthy();

  // Verify the stage shows current slide content
  const currentMain = stagePage.locator("#current-main");
  await expect(currentMain).not.toHaveText("", { timeout: 15_000 });

  // Verify the playlist sidebar is visible
  const sidebar = stagePage.locator("#playlist-sidebar");
  await expect(sidebar).toBeVisible();

  // Verify the section has data-has-playlist="true"
  const section = stagePage.locator(".stage__worship-pp");
  await expect(section).toHaveAttribute("data-has-playlist", "true");

  // Verify playlist name is shown
  const playlistNameEl = stagePage.locator("#playlist-name");
  await expect(playlistNameEl).toHaveText(playlistName);

  // Verify playlist entries are rendered
  const entries = stagePage.locator(".stage__worship-pp-playlist-entry");
  await expect(entries).toHaveCount(3, { timeout: 10_000 });

  // Verify exactly one entry is active
  const activeEntries = stagePage.locator(
    '.stage__worship-pp-playlist-entry[data-active="true"]',
  );
  await expect(activeEntries).toHaveCount(1);

  // Get the playlist entries from API to find the second entry's presentation ID
  const updatedPlaylistsResponse = await page.request.get(
    new URL("/playlists", baseURL).toString(),
  );
  const updatedPlaylists = await updatedPlaylistsResponse.json();
  const updatedPlaylist = updatedPlaylists.find(
    (p: { name: string }) => p.name === playlistName,
  );
  expect(updatedPlaylist).toBeTruthy();
  // Find the second presentation entry in the playlist
  const presEntries = updatedPlaylist.entries.filter(
    (e: { type: string }) => e.type === "presentation",
  );
  expect(presEntries.length).toBeGreaterThanOrEqual(2);
  const secondPresIdFromPlaylist = presEntries[1].presentation_id;

  // Get second presentation's slides
  const secondPresResponse = await page.request.get(
    new URL(`/presentations/${secondPresIdFromPlaylist}`, baseURL).toString(),
  );
  const secondPresDetail: {
    presentation: {
      slides: Array<{
        id: string;
        content: { main: { value: string } };
      }>;
    };
  } = await secondPresResponse.json();
  const secondSlide = secondPresDetail.presentation.slides.find(
    (s) => s.content.main.value.trim().length > 0,
  );
  expect(secondSlide).toBeTruthy();

  const secondTrigger = await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: secondPresIdFromPlaylist,
        currentSlideId: secondSlide!.id,
        playlistId: playlistId,
      },
    },
  );
  expect(secondTrigger.ok()).toBeTruthy();

  // Verify the active entry changed: second entry becomes active
  const secondEntry = stagePage.locator("#playlist-list li").nth(1);
  await expect(secondEntry).toHaveAttribute("data-active", "true", {
    timeout: 15_000,
  });

  // First entry should no longer be active
  const firstEntry = stagePage.locator("#playlist-list li").first();
  await expect(firstEntry).toHaveAttribute("data-active", "false");

  // Still exactly one active entry
  const activeEntriesAfterSwitch = stagePage.locator(
    '.stage__worship-pp-playlist-entry[data-active="true"]',
  );
  await expect(activeEntriesAfterSwitch).toHaveCount(1);

  await stagePage.close();
});

test("worship-pp stage hides playlist sidebar when no playlist context", async ({
  page,
  context,
}) => {
  await waitForOperatorReady(page);

  // Find a library with presentations via API
  const librariesResponse = await page.request.get(
    new URL("/libraries", baseURL).toString(),
    { timeout: 60_000 },
  );
  const libraries: Array<{
    id: string;
    name: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await librariesResponse.json();
  const sourceLibrary = libraries.find(
    (lib) => Array.isArray(lib.presentations) && lib.presentations.length > 0,
  );
  if (!sourceLibrary) {
    throw new Error("Expected at least one library with presentations");
  }

  // Get first presentation's slide
  const presId = sourceLibrary.presentations[0].id;
  const presResponse = await page.request.get(
    new URL(`/presentations/${presId}`, baseURL).toString(),
  );
  const presDetail: {
    presentation: {
      slides: Array<{
        id: string;
        content: { main: { value: string } };
      }>;
    };
  } = await presResponse.json();
  const slideWithContent = presDetail.presentation.slides.find(
    (s) => s.content.main.value.trim().length > 0,
  );
  expect(slideWithContent).toBeTruthy();

  // Open stage display
  const stagePage = await openStage(context);

  // Trigger via API without playlistId (library context)
  const triggerResponse = await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: presId,
        currentSlideId: slideWithContent!.id,
      },
    },
  );
  expect(triggerResponse.ok()).toBeTruthy();

  // Verify current slide has content
  const currentMain = stagePage.locator("#current-main");
  await expect(currentMain).not.toHaveText("", { timeout: 15_000 });

  // Verify the section has data-has-playlist="false" (sidebar hidden)
  const section = stagePage.locator(".stage__worship-pp");
  await expect(section).toHaveAttribute("data-has-playlist", "false");

  // Verify no playlist entries
  const entries = stagePage.locator(".stage__worship-pp-playlist-entry");
  await expect(entries).toHaveCount(0);

  await stagePage.close();
});

test("worship-pp stage prefers stage text over main text", async ({
  page,
  context,
}) => {
  await waitForOperatorReady(page);

  // Find a library with presentations via API
  const librariesResponse = await page.request.get(
    new URL("/libraries", baseURL).toString(),
    { timeout: 60_000 },
  );
  const libraries: Array<{
    id: string;
    name: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await librariesResponse.json();
  const sourceLibrary = libraries.find(
    (lib) => Array.isArray(lib.presentations) && lib.presentations.length > 0,
  );
  if (!sourceLibrary) {
    throw new Error("Expected at least one library with presentations");
  }

  // Get first presentation's slides
  const presId = sourceLibrary.presentations[0].id;
  const presResponse = await page.request.get(
    new URL(`/presentations/${presId}`, baseURL).toString(),
  );
  const presDetail: {
    presentation: {
      slides: Array<{
        id: string;
        content: { main: { value: string }; stage: { value: string } };
      }>;
    };
  } = await presResponse.json();
  expect(presDetail.presentation.slides.length).toBeGreaterThan(0);
  const targetSlide = presDetail.presentation.slides[0];
  const slideId = targetSlide.id;
  const existingMain = targetSlide.content.main.value;
  const existingTranslation = targetSlide.content?.translation?.value ?? "";

  // Update the slide to have distinct stage text (all fields required)
  const stageText = `Stage Priority ${Date.now()}`;
  const updateResponse = await page.request.patch(
    new URL(`/presentations/${presId}/slides/${slideId}`, baseURL).toString(),
    {
      data: {
        main: existingMain,
        translation: existingTranslation,
        stage: stageText,
      },
    },
  );
  expect(updateResponse.ok()).toBeTruthy();

  // Open the stage display with worship-pp layout
  const stagePage = await openStage(context);

  // Trigger the slide via API
  const triggerResponse = await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: presId,
        currentSlideId: slideId,
      },
    },
  );
  expect(triggerResponse.ok()).toBeTruthy();

  // Verify the stage shows the stage text (not main text)
  const currentMain = stagePage.locator("#current-main");
  await expect(currentMain).toHaveText(stageText, { timeout: 15_000 });

  await stagePage.close();
});
