import { test, expect } from "@playwright/test";
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

test("GET stage appearance returns default values", async ({ request }) => {
  const resp = await request.get(
    new URL("/stage/appearance/worship-snv", baseURL).toString(),
  );
  expect(resp.ok()).toBeTruthy();
  const appearance = await resp.json();

  // Verify default values match StageAppearance::default()
  expect(appearance.bodyPaddingV).toBe(1.0);
  expect(appearance.bodyPaddingH).toBe(2.0);
  expect(appearance.currentMaxFont).toBe(120.0);
  expect(appearance.nextMaxFont).toBe(80.0);
  expect(appearance.nextRatio).toBeCloseTo(0.8);
  expect(appearance.baseChars).toBe(25);
  expect(appearance.minFont).toBe(12.0);
});

test("PUT stage appearance persists updated values", async ({ request }) => {
  const layout = "worship-snv";
  const customAppearance = {
    bodyPaddingV: 2.5,
    bodyPaddingH: 3.0,
    currentMaxFont: 100.0,
    nextMaxFont: 60.0,
    nextRatio: 0.7,
    groupFontSize: 2.0,
    lyricsGap: 1.0,
    nextPaddingBottom: 3.0,
    baseChars: 30,
    minFont: 14.0,
    playlistFontSize: 1.3,
    playlistHeaderSize: 1.1,
    playlistPadding: 1.0,
    slidesPlaylistRatio: "7fr 3fr",
  };

  const putResp = await request.put(
    new URL(`/stage/appearance/${layout}`, baseURL).toString(),
    { data: customAppearance },
  );
  expect(putResp.status()).toBe(204);

  // Re-fetch and verify persistence
  const getResp = await request.get(
    new URL(`/stage/appearance/${layout}`, baseURL).toString(),
  );
  expect(getResp.ok()).toBeTruthy();
  const persisted = await getResp.json();

  expect(persisted.bodyPaddingV).toBe(2.5);
  expect(persisted.bodyPaddingH).toBe(3.0);
  expect(persisted.currentMaxFont).toBe(100.0);
  expect(persisted.nextMaxFont).toBe(60.0);
  expect(persisted.nextRatio).toBeCloseTo(0.7);
  expect(persisted.baseChars).toBe(30);
  expect(persisted.minFont).toBe(14.0);
});

test("worship-pp layout has its own default appearance", async ({
  request,
}) => {
  const resp = await request.get(
    new URL("/stage/appearance/worship-pp", baseURL).toString(),
  );
  expect(resp.ok()).toBeTruthy();
  const appearance = await resp.json();

  // worship-pp has different defaults than worship-snv
  expect(appearance.currentMaxFont).toBe(100.0);
  expect(appearance.nextMaxFont).toBe(64.0);
});
