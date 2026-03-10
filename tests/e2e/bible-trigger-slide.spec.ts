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

test("trigger Bible slide sends text to stage snapshot", async ({
  request,
}) => {
  const triggerResp = await request.post(
    new URL("/bible/trigger-slide", baseURL).toString(),
    {
      data: {
        mainText: "For God so loved the world",
        mainReference: "John 3:16 (NIV)",
        secondaryText: "Neboť Bůh tak miloval svět",
        secondaryReference: "Jan 3:16 (CEP)",
      },
    },
  );
  expect(triggerResp.ok()).toBeTruthy();
  const body = await triggerResp.json();
  expect(body.success).toBe(true);
  expect(body.output.mainText).toBe("For God so loved the world");
  expect(body.output.mainReference).toBe("John 3:16 (NIV)");
  expect(body.output.secondaryText).toBe("Neboť Bůh tak miloval svět");
  expect(body.output.secondaryReference).toBe("Jan 3:16 (CEP)");
});

test("trigger Bible slide with optional metadata fields", async ({
  request,
}) => {
  const triggerResp = await request.post(
    new URL("/bible/trigger-slide", baseURL).toString(),
    {
      data: {
        mainText: "In the beginning God created",
        mainReference: "Genesis 1:1 (NIV)",
        secondaryText: "",
        secondaryReference: "",
        translationCode: "NIV",
        book: "Genesis",
        bookCode: "GEN",
        bookNumber: 1,
        chapter: 1,
        verseStart: 1,
        verseEnd: 1,
      },
    },
  );
  expect(triggerResp.ok()).toBeTruthy();
  const body = await triggerResp.json();
  expect(body.success).toBe(true);
  expect(body.output.mainText).toBe("In the beginning God created");
});

test("clear Bible broadcast after trigger", async ({ request }) => {
  // Trigger a slide
  await request.post(new URL("/bible/trigger-slide", baseURL).toString(), {
    data: {
      mainText: "The Lord is my shepherd",
      mainReference: "Psalm 23:1 (NIV)",
    },
  });

  // Clear
  const clearResp = await request.post(
    new URL("/bible/clear", baseURL).toString(),
  );
  expect(clearResp.status()).toBe(204);
});
