import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #555 — deleting a song is now a SOFT delete: it lands in the settings-page
// trash ("Zmazané piesne") and can be restored for 30 days. This spec proves the
// full user path: delete via API (as the operator UI does) → the song vanishes
// from the libraries → it appears in the trash card → clicking "Obnoviť" brings
// it back. The test server has no PRESENTER_SYNC_PEER_URL, so the sync loop is
// disabled — the trash is a purely local feature here.

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";
let dbUrl = "";
let port = 0;

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

async function createSong(request: any, name: string) {
  const libsRes = await request.get(
    new URL("/libraries/summary", baseURL).toString(),
  );
  expect(libsRes.status()).toBe(200);
  const libs = await libsRes.json();
  expect(libs.length).toBeGreaterThan(0);
  const res = await request.post(
    new URL(`/libraries/${libs[0].id}/presentations`, baseURL).toString(),
    { data: { name, slides: [{ main: "trash test verse" }] } },
  );
  expect(res.status()).toBe(200);
  const body = await res.json();
  return body.presentation.id as string;
}

async function songNames(request: any): Promise<string[]> {
  const res = await request.get(
    new URL("/libraries/summary", baseURL).toString(),
  );
  expect(res.status()).toBe(200);
  const libs = await res.json();
  return libs.flatMap((l: any) => l.presentations.map((p: any) => p.name));
}

test("a deleted song lands in the trash and Obnoviť restores it", async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  const name = `Trash Roundtrip ${Date.now()}`;
  const id = await createSong(request, name);

  // Delete → gone from the libraries.
  const del = await request.delete(
    new URL(`/presentations/${id}`, baseURL).toString(),
  );
  expect(del.status()).toBe(204);
  expect(await songNames(request)).not.toContain(name);

  // The settings trash card shows it.
  await page.goto(new URL("/ui/settings", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 60_000,
  });
  const row = page.locator(`[data-role="trash-row"][data-song-name="${name}"]`);
  await expect(row).toBeVisible({ timeout: 30_000 });

  // Restore via the UI button.
  await row.locator('[data-role="restore-btn"]').click();
  await expect(row).toHaveCount(0, { timeout: 30_000 });

  // And the song is back in the library.
  await expect
    .poll(async () => songNames(request), { timeout: 30_000 })
    .toContain(name);

  expect(errors, `console errors: ${errors.join(" | ")}`).toEqual([]);
});
