import { test, expect, BrowserContext } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

async function openStageDisplay(context: BrowserContext) {
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
  const stagePage = await context.newPage();
  await stagePage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await stagePage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );
  return stagePage;
}

test("worship-snv shows current song name in amber box", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Create a library with a numbered presentation
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `SongName Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "042 Hodny Chvaly" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;
  const slideId = presPayload.presentation.slides[0].id;

  // Trigger the slide
  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: { presentationId, currentSlideId: slideId },
  });

  // Open stage display
  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Wait for the current song box to show the sanitized song name
  // (number prefix stripped per #312 — see build_stage_snapshot)
  const currentSongBox = stagePage.locator(
    ".stage__current-song .stage__song-name-text",
  );
  await expect(currentSongBox).toBeVisible({ timeout: 10_000 });
  await expect(currentSongBox).toContainText("Hodny Chvaly", {
    timeout: 10_000,
  });
  await expect(currentSongBox).not.toContainText("042", { timeout: 5_000 });

  // Verify amber color
  const color = await currentSongBox.evaluate(
    (el) => window.getComputedStyle(el).color,
  );
  expect(color).toBe("rgb(251, 191, 36)");

  // Verify next-song box exists (empty since no AbleSet/playlist)
  const nextSongBox = stagePage.locator(
    ".stage__next-song .stage__song-name-text",
  );
  await expect(nextSongBox).toBeVisible({ timeout: 5_000 });

  // Verify group pills are left-aligned (not centered)
  const groupLeft = await stagePage
    .locator(".stage__current-group")
    .evaluate((el) => window.getComputedStyle(el).left);
  // left:2% of viewport — should be a small pixel value, not 25% (~320px on 1280px viewport)
  expect(parseInt(groupLeft)).toBeLessThan(100);

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});

test("worship-snv shows next song from playlist", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Create a library with two presentations
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `NextSong Lib ${Date.now()}` } },
  );
  const library: { id: string } = await libResp.json();

  const pres1Resp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "001 First Song" } },
  );
  const pres1: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await pres1Resp.json();

  const pres2Resp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "002 Second Song" } },
  );
  const pres2: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await pres2Resp.json();

  // Create a playlist and add entries
  const playlistResp = await request.post(
    new URL("/playlists", baseURL).toString(),
    { data: { name: `Test Playlist ${Date.now()}` } },
  );
  expect(playlistResp.ok()).toBeTruthy();
  const playlist: { id: string } = await playlistResp.json();

  const entriesResp = await request.put(
    new URL(`/playlists/${playlist.id}/entries`, baseURL).toString(),
    {
      data: {
        entries: [
          { type: "presentation", presentationId: pres1.presentation.id },
          { type: "presentation", presentationId: pres2.presentation.id },
        ],
      },
    },
  );
  expect(entriesResp.ok()).toBeTruthy();

  // Trigger first song with playlist context
  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: {
      presentationId: pres1.presentation.id,
      currentSlideId: pres1.presentation.slides[0].id,
      playlistId: playlist.id,
    },
  });

  // Open stage display
  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Current song should show "First Song" (number prefix stripped per #312, uppercased by CSS)
  const currentSongBox = stagePage.locator(
    ".stage__current-song .stage__song-name-text",
  );
  await expect(currentSongBox).toContainText("First Song", {
    timeout: 10_000,
  });
  await expect(currentSongBox).not.toContainText("001", { timeout: 5_000 });

  // Next song should show "Second Song" (from playlist, prefix stripped per #312)
  const nextSongBox = stagePage.locator(
    ".stage__next-song .stage__song-name-text",
  );
  await expect(nextSongBox).toContainText("Second Song", {
    timeout: 10_000,
  });
  await expect(nextSongBox).not.toContainText("002", { timeout: 5_000 });

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});
