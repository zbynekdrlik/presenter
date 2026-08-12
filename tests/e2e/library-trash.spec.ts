import { test, expect } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #644 — deleting a LIBRARY is a soft delete too (mirrors song-trash.spec.ts):
// it lands in the settings-page trash under "Zmazané knižnice" and restoring
// it is CASCADE-SCOPED — it brings back the library AND every song ITS OWN
// deletion tombstoned along with it, but a song trashed INDEPENDENTLY (before
// the library was deleted) stays trashed. This spec proves that end to end
// through the real UI, not just the repository unit tests.

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

async function createLibrary(request: any, name: string): Promise<string> {
  const res = await request.post(new URL("/libraries", baseURL).toString(), {
    data: { name },
  });
  expect(res.status()).toBe(200);
  const body = await res.json();
  return body.id as string;
}

async function createSong(
  request: any,
  libraryId: string,
  name: string,
): Promise<string> {
  const res = await request.post(
    new URL(`/libraries/${libraryId}/presentations`, baseURL).toString(),
    { data: { name, slides: [{ main: "library trash test verse" }] } },
  );
  expect(res.status()).toBe(200);
  const body = await res.json();
  return body.presentation.id as string;
}

async function libraryNames(request: any): Promise<string[]> {
  const res = await request.get(
    new URL("/libraries/summary", baseURL).toString(),
  );
  expect(res.status()).toBe(200);
  const libs = await res.json();
  return libs.map((l: any) => l.name);
}

async function songNamesIn(request: any, libraryName: string): Promise<string[]> {
  const res = await request.get(
    new URL("/libraries/summary", baseURL).toString(),
  );
  expect(res.status()).toBe(200);
  const libs = await res.json();
  const lib = libs.find((l: any) => l.name === libraryName);
  return lib ? lib.presentations.map((p: any) => p.name) : [];
}

test("a deleted library lands in the trash and Obnoviť restores it AND its cascaded song — but not an independently trashed song", async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);

  const stamp = Date.now();
  const libName = `Library Trash Roundtrip ${stamp}`;
  const cascadedSongName = `Cascaded Song ${stamp}`;
  const independentSongName = `Independent Trash Song ${stamp}`;

  const libraryId = await createLibrary(request, libName);
  await createSong(request, libraryId, cascadedSongName);
  const independentSongId = await createSong(
    request,
    libraryId,
    independentSongName,
  );

  // Trash ONE song individually BEFORE the library itself is deleted — it
  // must stay trashed after the library is later restored.
  const preDel = await request.delete(
    new URL(`/presentations/${independentSongId}`, baseURL).toString(),
  );
  expect(preDel.status()).toBe(204);

  // Delete the LIBRARY — cascades the remaining live song into the SAME
  // tombstone.
  const del = await request.delete(
    new URL(`/libraries/${libraryId}`, baseURL).toString(),
  );
  expect(del.status()).toBe(204);
  expect(await libraryNames(request)).not.toContain(libName);

  // The settings trash card shows the trashed library.
  await page.goto(new URL("/ui/settings", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 60_000,
  });
  const libRow = page.locator(
    `[data-role="trash-library-row"][data-library-name="${libName}"]`,
  );
  await expect(libRow).toBeVisible({ timeout: 30_000 });

  // The independently-trashed song shows in the SONG trash list too.
  const songRow = page.locator(
    `[data-role="trash-row"][data-song-name="${independentSongName}"]`,
  );
  await expect(songRow).toBeVisible({ timeout: 30_000 });

  // Restore the library via the UI button.
  await libRow.locator('[data-role="restore-library-btn"]').click();
  await expect(libRow).toHaveCount(0, { timeout: 30_000 });

  // The library AND its cascaded song are back.
  await expect
    .poll(async () => libraryNames(request), { timeout: 30_000 })
    .toContain(libName);
  await expect
    .poll(async () => songNamesIn(request, libName), { timeout: 30_000 })
    .toContain(cascadedSongName);

  // The independently-trashed song is STILL trashed — the library restore
  // must not have resurrected it.
  expect(await songNamesIn(request, libName)).not.toContain(
    independentSongName,
  );
  await expect(songRow).toBeVisible();

  expect(errors, `console errors: ${errors.join(" | ")}`).toEqual([]);
});
