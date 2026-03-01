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

test("slide CRUD, reorder, and content update via presentations API", async ({
  request,
}) => {
  // Create a new library, then a presentation in it
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    {
      data: { name: `E2E Library ${Date.now()}` },
    },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string; name: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "E2E Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; name: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;

  // Insert a blank slide at position 0
  const insertResp = await request.post(
    new URL(`/presentations/${presentationId}/slides`, baseURL).toString(),
    { data: { position: 0 } },
  );
  expect(insertResp.ok()).toBeTruthy();
  const slidesAfterInsert: Array<{ id: string }> = await insertResp.json();
  expect(slidesAfterInsert.length).toBeGreaterThanOrEqual(2);

  const firstSlideId = slidesAfterInsert[0].id;

  // Duplicate the first slide
  const dupResp = await request.post(
    new URL(
      `/presentations/${presentationId}/slides/${firstSlideId}/duplicate`,
      baseURL,
    ).toString(),
  );
  expect(dupResp.ok()).toBeTruthy();
  const slidesAfterDup: Array<{ id: string }> = await dupResp.json();
  expect(slidesAfterDup.length).toBe(slidesAfterInsert.length + 1);

  // Update content on the second slide
  const secondSlideId = slidesAfterDup[1].id;
  const updateResp = await request.patch(
    new URL(
      `/presentations/${presentationId}/slides/${secondSlideId}`,
      baseURL,
    ).toString(),
    {
      data: {
        main: "Main line",
        translation: "Preklad",
        stage: "Stage text",
        group: "Verse 1",
      },
    },
  );
  expect(updateResp.ok()).toBeTruthy();
  const updated: {
    id: string;
    content: {
      main: { value: string };
      translation: { value: string };
      stage: { value: string };
      group?: { name: string };
    };
  } = await updateResp.json();
  expect(updated.content.main.value).toBe("Main line");
  expect(updated.content.translation.value).toBe("Preklad");
  expect(updated.content.stage.value).toBe("Stage text");
  expect(updated.content.group?.name).toBe("Verse 1");

  // Reorder slides (reverse)
  const detailBefore = await request.get(
    new URL(`/presentations/${presentationId}`, baseURL).toString(),
  );
  expect(detailBefore.ok()).toBeTruthy();
  const beforePayload: { presentation: { slides: Array<{ id: string }> } } =
    await detailBefore.json();
  const reversed = [...beforePayload.presentation.slides]
    .reverse()
    .map((s) => s.id);
  const reorderResp = await request.post(
    new URL(
      `/presentations/${presentationId}/slides/reorder`,
      baseURL,
    ).toString(),
    { data: { slideIds: reversed } },
  );
  expect(reorderResp.ok()).toBeTruthy();
  const reordered: Array<{ id: string }> = await reorderResp.json();
  expect(reordered.map((s) => s.id)).toEqual(reversed);

  // Delete a slide
  const deleteTarget = reordered[0].id;
  const delResp = await request.delete(
    new URL(
      `/presentations/${presentationId}/slides/${deleteTarget}`,
      baseURL,
    ).toString(),
  );
  expect(delResp.ok()).toBeTruthy();
  const afterDelete: Array<{ id: string }> = await delResp.json();
  expect(afterDelete.find((s) => s.id === deleteTarget)).toBeFalsy();

  // Rename presentation
  const renameResp = await request.patch(
    new URL(`/presentations/${presentationId}`, baseURL).toString(),
    { data: { name: "E2E Song Renamed" } },
  );
  expect(renameResp.ok()).toBeTruthy();
  const detailAfter = await request.get(
    new URL(`/presentations/${presentationId}`, baseURL).toString(),
  );
  const afterPayload: { presentation: { name: string } } =
    await detailAfter.json();
  expect(afterPayload.presentation.name).toBe("E2E Song Renamed");

  // Delete presentation
  const deletePresResp = await request.delete(
    new URL(`/presentations/${presentationId}`, baseURL).toString(),
  );
  expect(deletePresResp.status()).toBe(204);

  // Verify deleted presentation returns 404
  const detailGone = await request.get(
    new URL(`/presentations/${presentationId}`, baseURL).toString(),
  );
  expect(detailGone.status()).toBe(404);
});
