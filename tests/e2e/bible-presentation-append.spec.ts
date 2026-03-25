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

test("append slides to Bible presentation increases slide count", async ({
  request,
}) => {
  // Create a Bible presentation
  const createResp = await request.post(
    new URL("/bible/presentations", baseURL).toString(),
    { data: { name: `Append Test ${Date.now()}` } },
  );
  expect(createResp.ok()).toBeTruthy();
  const created: { id: string; slides: Array<{ id: string }> } =
    await createResp.json();
  const presId = created.id;

  // Append slides (note: append filters out empty placeholder slides from creation)
  const appendResp = await request.post(
    new URL(`/bible/presentations/${presId}/append`, baseURL).toString(),
    {
      data: {
        slides: [
          {
            bibleMain: "For God so loved the world",
            bibleTranslation: "Neboť Bůh tak miloval svět",
            bibleMainReference: "John 3:16",
            bibleTranslationReference: "",
          },
          {
            bibleMain: "that he gave his one and only Son",
            bibleTranslation: "že dal svého jediného Syna",
            bibleMainReference: "John 3:16b",
            bibleTranslationReference: "",
          },
        ],
      },
    },
  );
  expect(appendResp.ok()).toBeTruthy();
  const appended: { id: string; slides: Array<{ id: string }> } =
    await appendResp.json();
  // Empty placeholder slide is removed, so we get exactly 2 appended slides
  expect(appended.slides.length).toBe(2);

  // Verify the presentation detail reflects the appended slides
  const detailResp = await request.get(
    new URL(`/bible/presentations/${presId}`, baseURL).toString(),
  );
  expect(detailResp.ok()).toBeTruthy();
  const detail: { slides: Array<{ id: string }> } = await detailResp.json();
  expect(detail.slides.length).toBe(2);
});

test("append empty slides array returns error", async ({ request }) => {
  // Create a Bible presentation
  const createResp = await request.post(
    new URL("/bible/presentations", baseURL).toString(),
    { data: { name: `Empty Append ${Date.now()}` } },
  );
  const created: { id: string } = await createResp.json();

  const appendResp = await request.post(
    new URL(`/bible/presentations/${created.id}/append`, baseURL).toString(),
    { data: { slides: [] } },
  );
  expect(appendResp.status()).toBe(400);
});
