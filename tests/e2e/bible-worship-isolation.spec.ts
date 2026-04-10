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

test("creating a Bible presentation does not add a Bible library to the worship library list", async ({
  request,
}) => {
  // Regression for #227: previously bible content was stored as a Library row
  // named "Bible" which leaked into the worship operator's library list. After
  // the bible/worship separation migration, the bible library row no longer
  // exists at all.

  // Create a bible presentation via the dedicated bible API
  const createResp = await request.post(
    new URL("/bible/presentations", baseURL).toString(),
    { data: { name: `Isolation Test ${Date.now()}` } },
  );
  expect(createResp.ok()).toBeTruthy();

  // Fetch the worship library summary
  const libsResp = await request.get(
    new URL("/libraries/summary", baseURL).toString(),
  );
  expect(libsResp.ok()).toBeTruthy();
  const libs = (await libsResp.json()) as Array<{ name: string }>;

  // Assert NO library is named "Bible" (case-insensitive)
  const bibleLibs = libs.filter(
    (l) => l.name.toLowerCase() === "bible",
  );
  expect(bibleLibs).toHaveLength(0);
});

test("triggering a worship slide with non-empty stage text does not change /bible/active", async ({
  request,
}) => {
  // Regression for the broadcasting leak deleted in Task 7. Previously, any
  // worship slide with non-empty `stage` text triggered a spurious BibleUpdate
  // to Resolume — hijacking the bible reference clips. This test asserts that
  // /bible/active state is unaffected by worship stage broadcasts.

  // Snapshot /bible/active before any worship action
  const beforeResp = await request.get(
    new URL("/bible/active", baseURL).toString(),
  );
  const beforeBody = beforeResp.ok() ? await beforeResp.json() : null;

  // Create a worship library
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `_Bible Isolation Test ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const lib: { id: string } = await libResp.json();

  // Create a worship presentation containing a slide that has non-empty stage
  // text. The `stage` field used to trigger a spurious BibleUpdate in the
  // broadcasting layer that was deleted in Task 7.
  const presResp = await request.post(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      data: {
        name: "Stage Text Trigger Test",
        slides: [
          {
            main: "First line",
            translation: "Translation",
            stage: "John 3:16", // non-empty stage that previously triggered BibleUpdate
            group: "Verse 1",
          },
        ],
      },
    },
  );
  expect(presResp.ok()).toBeTruthy();
  const presData: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presData.presentation.id;
  const slideId = presData.presentation.slides[0].id;

  // Trigger the slide via /stage/state — same path the operator UI uses
  const triggerResp = await request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId,
        currentSlideId: slideId,
        nextSlideId: null,
      },
    },
  );
  expect(triggerResp.ok()).toBeTruthy();

  // After the worship trigger, /bible/active should be unchanged
  const afterResp = await request.get(
    new URL("/bible/active", baseURL).toString(),
  );
  const afterBody = afterResp.ok() ? await afterResp.json() : null;

  expect(afterBody).toEqual(beforeBody);
});
