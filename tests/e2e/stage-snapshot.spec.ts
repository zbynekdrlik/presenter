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

test("stage snapshot returns JSON with expected fields when slide is triggered", async ({
  request,
}) => {
  // Create a library and presentation with content
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `Snapshot Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Snapshot Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;
  const slideId = presPayload.presentation.slides[0].id;

  // Add content to the slide
  const updateResp = await request.patch(
    new URL(
      `/presentations/${presentationId}/slides/${slideId}`,
      baseURL,
    ).toString(),
    {
      data: {
        main: "Amazing Grace",
        translation: "Úžasná Milosť",
        stage: "verse 1 notes",
        group: "Verse 1",
      },
    },
  );
  expect(updateResp.ok()).toBeTruthy();

  // Trigger the slide via stage state
  const triggerResp = await request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId,
        currentSlideId: slideId,
      },
    },
  );
  expect(triggerResp.status()).toBe(204);

  // Fetch snapshot and verify fields
  const snapshotResp = await request.get(
    new URL("/stage/snapshot", baseURL).toString(),
  );
  expect(snapshotResp.ok()).toBeTruthy();
  const snapshot = await snapshotResp.json();

  expect(snapshot).toHaveProperty("layout");
  expect(snapshot).toHaveProperty("generatedAt");
  expect(snapshot.current).toBeTruthy();
  expect(snapshot.current.main).toBe("Amazing Grace");
  expect(snapshot.current.translation).toBe("Úžasná Milosť");
  expect(snapshot.current.group).toBe("Verse 1");
  expect(snapshot.presentationName).toBe("Snapshot Song");
});

test("stage snapshot reflects cleared state after stage clear", async ({
  request,
}) => {
  // Create and trigger a slide first
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `ClearSnap Lib ${Date.now()}` } },
  );
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "ClearSnap Song" } },
  );
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();

  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: {
      presentationId: presPayload.presentation.id,
      currentSlideId: presPayload.presentation.slides[0].id,
    },
  });

  // Clear stage
  const clearResp = await request.post(
    new URL("/stage/clear", baseURL).toString(),
  );
  expect(clearResp.status()).toBe(204);

  // Snapshot should have no current slide
  const snapshotResp = await request.get(
    new URL("/stage/snapshot", baseURL).toString(),
  );
  expect(snapshotResp.ok()).toBeTruthy();
  const snapshot = await snapshotResp.json();
  expect(snapshot.current).toBeNull();
  expect(snapshot.presentationId).toBeUndefined();
});
