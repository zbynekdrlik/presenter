import { test, expect, Page } from "@playwright/test";
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
  await page.waitForFunction(() => window.__presenterLiveConnected === true, {
    timeout: 30_000,
  });
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

test("allows managing playlist entries while in live mode", async ({
  page,
}) => {
  await waitForOperatorReady(page);

  const liveToggle = page.locator(
    '[data-role="mode-toggle"][data-mode="live"]',
  );
  await expect(liveToggle).toHaveAttribute("data-active", "true");

  await page.locator('[data-role="playlist-create"]').click();
  const playlistModal = page.locator('[data-role="playlist-edit-modal"]');
  await expect(playlistModal).toHaveAttribute("data-open", "true");

  const playlistName = `E2E Live Playlist ${Date.now()}`;
  await page.locator('[data-role="playlist-edit-name"]').fill(playlistName);
  await page.locator('[data-role="playlist-edit-save"]').click();
  await expect(playlistModal).toHaveAttribute("data-open", "false");

  const activePlaylistButton = page.locator(
    '[data-role="playlist-item"][data-active="true"]',
  );
  await expect(activePlaylistButton).toContainText(playlistName);

  const librariesResponse = await page.request.get(
    new URL("/libraries", baseURL).toString(),
    {
      timeout: 60_000,
    },
  );
  expect(librariesResponse.ok()).toBeTruthy();
  const libraries: Array<{
    id: string;
    name: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await librariesResponse.json();
  const source = libraries.find(
    (lib) => Array.isArray(lib.presentations) && lib.presentations.length > 0,
  );
  if (!source) {
    throw new Error("Expected at least one library with presentations");
  }
  const presentation = source.presentations[0];
  const searchInput = page.locator('[data-role="global-search-query"]');
  await searchInput.fill(
    presentation.name.slice(0, Math.min(12, presentation.name.length)),
  );

  const searchResult = page
    .locator('[data-role="search-result-item"][data-kind="presentation"]')
    .first();
  await expect(searchResult).toBeVisible({ timeout: 20_000 });

  const presentationList = page.locator('[data-role="presentation-list"]');
  await searchResult.dragTo(presentationList);

  await expect(searchInput).toHaveValue("");
  await expect(
    page.locator('[data-role="global-search-results"]'),
  ).toHaveAttribute("data-visible", "false");

  const playlistItems = presentationList.locator(
    '[data-role="presentation-item"]',
  );
  await expect(playlistItems).toHaveCount(1, { timeout: 15_000 });

  const removeButton = playlistItems
    .first()
    .locator('[data-action="playlist-remove"]');
  await removeButton.click();

  await expect(presentationList.locator("li.empty")).toHaveText(
    /Playlist is empty/i,
  );
  await expect(playlistItems).toHaveCount(0);
  const playlistResponse = await page.request.get(
    new URL("/playlists", baseURL).toString(),
    {
      timeout: 60_000,
    },
  );
  expect(playlistResponse.ok()).toBeTruthy();
  const playlists: Array<{ id: string; name: string }> =
    await playlistResponse.json();
  const createdPlaylist = playlists.find((item) => item.name === playlistName);
  expect(createdPlaylist).toBeTruthy();
  const playlistId = createdPlaylist!.id;
  const playlistButton = page
    .locator(
      `[data-role=\"playlist-item\"][data-playlist-id=\"${playlistId}\"]`,
    )
    .first();
  await expect(playlistButton).toBeVisible();
  await playlistButton.click();

  const sourceLibrary = source;
  await page.locator('[data-role=\"library-more\"]').click();
  const libraryModalRow = page.locator(
    `[data-role="library-modal-list"] [data-role="library-row"][data-library-id="${sourceLibrary.id}"]`,
  );
  await expect(libraryModalRow).toBeVisible();
  const favoriteToggle = libraryModalRow.locator(
    '[data-action="library-favorite"]',
  );
  const pressed = await favoriteToggle.getAttribute("aria-pressed");
  if (pressed !== "true") {
    await favoriteToggle.click();
  }
  const closeButton = page.locator('[data-role="library-modal-close"]');
  if (await closeButton.isVisible()) {
    await closeButton.click();
  } else {
    await page.keyboard.press("Escape");
  }
  await expect(page.locator('[data-role="library-modal"]')).toHaveAttribute(
    "data-open",
    "false",
  );
  const libraryButton = page.locator(
    `[data-role="library-list"] [data-role="library-item"][data-library-id="${sourceLibrary.id}"]`,
  );
  await expect(libraryButton).toBeVisible({ timeout: 10_000 });
  await libraryButton.click();

  await expect(playlistButton).toBeVisible();

  const libraryPresentation = page
    .locator('[data-role="presentation-item"][data-type="presentation"]')
    .first();
  await expect(libraryPresentation).toBeVisible();
  const presentationDropzone = page.locator(
    '[data-dropzone-target="presentations"]',
  );
  await expect(presentationDropzone).toBeVisible();
  await libraryPresentation.dragTo(presentationDropzone);

  await expect
    .poll(
      async () => {
        return await page.evaluate(
          (id) =>
            window.__presenterOperatorTestHelpers.playlistPresentationCount?.(
              id,
            ),
          playlistId,
        );
      },
      { timeout: 10_000 },
    )
    .toBe(1);

  await playlistButton.click();
  const playlistItemsAfter = page.locator(
    '[data-role="presentation-item"][data-type="presentation"]',
  );
  await expect(playlistItemsAfter).toHaveCount(1, { timeout: 10_000 });
  const playlistCountBadge = playlistButton.locator(
    '[data-role="playlist-count"]',
  );
  await expect(playlistCountBadge).toHaveText(/\b1\b/, { timeout: 10_000 });
});

test("playlist separator entries can be added and persisted via API", async ({
  request,
}) => {
  // Create a playlist
  const playlistResp = await request.post(
    new URL("/playlists", baseURL).toString(),
    { data: { name: `Sep Test ${Date.now()}` } },
  );
  expect(playlistResp.ok()).toBeTruthy();
  const playlist: { id: string; name: string } = await playlistResp.json();

  // Create a library and presentation to use as a playlist entry
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `Sep Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Sep Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: { presentation: { id: string } } = await presResp.json();

  // Add entries including a separator
  const entriesResp = await request.put(
    new URL(`/playlists/${playlist.id}/entries`, baseURL).toString(),
    {
      data: {
        entries: [
          {
            type: "presentation",
            presentationId: presPayload.presentation.id,
          },
          {
            type: "separator",
            name: "-- Worship Set --",
          },
        ],
      },
    },
  );
  expect(entriesResp.ok()).toBeTruthy();
  const updated: { entries: Array<{ kind: any }> } = await entriesResp.json();
  expect(updated.entries.length).toBe(2);

  // Verify the separator entry persists on re-fetch
  const playlistsResp = await request.get(
    new URL("/playlists", baseURL).toString(),
  );
  expect(playlistsResp.ok()).toBeTruthy();
  const playlists: Array<{
    id: string;
    entries: Array<{ kind: any }>;
  }> = await playlistsResp.json();
  const found = playlists.find((p) => p.id === playlist.id);
  expect(found).toBeTruthy();
  expect(found!.entries.length).toBe(2);
});

test("stage display status shows connection and latency", async ({ page }) => {
  await page.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
  await page.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });

  await page.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    {
      timeout: 30_000,
    },
  );

  const connectionLabel = page.locator("#stage-status-connection");
  await expect(connectionLabel).toHaveText(/Connected/i);

  const latencyLabel = page.locator("#stage-status-latency");
  await page.waitForTimeout(2500);
  const latencyText = (await latencyLabel.textContent())?.trim() ?? "";
  const latencyVisible = await latencyLabel.getAttribute("data-visible");
  if (latencyVisible === "true") {
    expect(latencyText.length).toBeGreaterThan(0);
    expect(latencyText).not.toBe("—");
  } else {
    expect(latencyText).toBe("");
  }
  await expect(
    page.locator(".stage__box--connection-status"),
  ).not.toContainText("&");
});
