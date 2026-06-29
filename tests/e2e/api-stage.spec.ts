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

async function openApiStage(context: BrowserContext) {
  // Set global layout to "api"
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "api" },
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
  // Wait for the WASM client to fetch and apply the "api" layout code
  await stagePage.waitForFunction(
    () => window.__presenterStageLayout === "api",
    { timeout: 10_000 },
  );
  return stagePage;
}

test("API stage push displays text and group colors", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Push data BEFORE opening the page so the initial snapshot fetch picks it up
  const putResp = await request.put(
    new URL("/api/stage", baseURL).toString(),
    {
      data: {
        currentText: "Haleluja, haleluja",
        nextText: "Spievajte Hospodinovi",
        currentGroup: "Vsetci",
        nextGroup: "Zeny",
        currentSong: "Haleluja",
        nextSong: "Spievajte",
      },
    },
  );
  expect(putResp.status()).toBe(204);

  const stagePage = await openApiStage(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Verify current text
  const currentText = stagePage.locator(".stage__current-slide .stage__slide-text");
  await expect(currentText).toContainText("Haleluja, haleluja", {
    timeout: 10_000,
  });

  // Verify next text
  const nextText = stagePage.locator(".stage__next-slide .stage__slide-text");
  await expect(nextText).toContainText("Spievajte Hospodinovi", {
    timeout: 10_000,
  });

  // Verify current group pill with legacy color for "Vsetci" (#E08A3C = rgb(224, 138, 60))
  const currentGroupPill = stagePage.locator(
    ".stage__current-group .stage__group-pill",
  );
  await expect(currentGroupPill).toBeVisible({ timeout: 10_000 });
  await expect(currentGroupPill).toContainText("Vsetci");
  const bgColor = await currentGroupPill.evaluate(
    (el) => window.getComputedStyle(el).backgroundColor,
  );
  expect(bgColor).toBe("rgb(224, 138, 60)");

  // Verify text color is black (WCAG contrast for light background)
  const textColor = await currentGroupPill.evaluate(
    (el) => window.getComputedStyle(el).color,
  );
  expect(textColor).toBe("rgb(0, 0, 0)");

  // Verify next group pill
  const nextGroupPill = stagePage.locator(
    ".stage__next-group .stage__group-pill",
  );
  await expect(nextGroupPill).toBeVisible({ timeout: 10_000 });
  await expect(nextGroupPill).toContainText("Zeny");

  // Verify current song name
  const songName = stagePage.locator(".stage__current-song .stage__song-name-text");
  await expect(songName).toContainText("Haleluja", { timeout: 10_000 });

  // Verify next song name
  const nextSongName = stagePage.locator(".stage__next-song .stage__song-name-text");
  await expect(nextSongName).toContainText("Spievajte", { timeout: 10_000 });

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});

test("API stage push with empty state clears display", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Push data BEFORE opening the page so initial snapshot has content
  await request.put(new URL("/api/stage", baseURL).toString(), {
    data: {
      currentText: "Some text",
      currentGroup: "Vsetci",
      currentSong: "Song",
    },
  });

  const stagePage = await openApiStage(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  const currentText = stagePage.locator(".stage__current-slide .stage__slide-text");
  await expect(currentText).toContainText("Some text", { timeout: 10_000 });

  // Push empty state
  const putResp = await request.put(
    new URL("/api/stage", baseURL).toString(),
    { data: {} },
  );
  expect(putResp.status()).toBe(204);

  // Wait for the display to clear — current text should become empty
  await expect(currentText).toHaveText("", { timeout: 10_000 });

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});

test("API stage does not interfere with normal stage", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Create a library and presentation for normal stage
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `ApiIsolation Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Normal Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;
  const slideId = presPayload.presentation.slides[0].id;

  // Set slide text
  await request.patch(
    new URL(
      `/presentations/${presentationId}/slides/${slideId}`,
      baseURL,
    ).toString(),
    {
      data: { main: "Normal slide text", translation: "", stage: "" },
    },
  );

  // Set normal stage layout and trigger slide
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: { presentationId, currentSlideId: slideId },
  });

  // Open normal stage page
  const normalPage = await context.newPage();
  normalPage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      // Ignore Chrome's subresource integrity preload warning (browser-level, not app)
      if (msg.text().includes("crbug.com/981419")) return;
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  await normalPage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await normalPage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await normalPage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );
  // Wait for layout to be applied
  await normalPage.waitForFunction(
    () => window.__presenterStageLayout === "worship-snv",
    { timeout: 10_000 },
  );

  // Re-trigger the slide to ensure snapshot is sent after page is connected
  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: { presentationId, currentSlideId: slideId },
  });

  // Verify normal stage shows normal text
  const normalText = normalPage.locator(".stage__current-slide .stage__slide-text");
  await expect(normalText).toContainText("Normal slide text", {
    timeout: 10_000,
  });

  // Push API stage data — should NOT affect the normal stage
  await request.put(new URL("/api/stage", baseURL).toString(), {
    data: { currentText: "API override attempt" },
  });

  // Wait briefly and verify normal stage still shows normal text
  await normalPage.waitForTimeout(2_000);
  await expect(normalText).toContainText("Normal slide text");

  await normalPage.close();
  expect(consoleMessages).toEqual([]);
});

test("api put does not switch preview when layout is worship-snv", async ({
  request,
  page,
  context,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      // Existing operator E2E filters this Chrome integrity-preload warning.
      if (!text.includes("crbug.com/981419")) {
        consoleMessages.push(`[${msg.type()}] ${text}`);
      }
    }
  });

  // 1. Set layout to worship-snv (a non-api layout).
  const setLayoutRes = await request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "worship-snv" } },
  );
  expect(setLayoutRes.ok()).toBeTruthy();

  // 2. Open the operator UI; wait for WASM ready.
  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForLoadState("networkidle");

  // 3. Snapshot the current preview text BEFORE the api PUT.
  //
  // #460: the header preview is now a live `/stage?preview=1` iframe, so the
  // current-slide text lives INSIDE the iframe (the real stage render), read
  // via frameLocator. This is the most faithful form of the isolation check:
  // it verifies api content does/doesn't reach the ACTUAL stage output.
  const currentInFrame = () =>
    page
      .frameLocator("iframe.operator__stage-iframe")
      .locator(".stage__current-slide .stage__slide-text");
  const previewBefore = await currentInFrame()
    .first()
    .textContent()
    .catch(() => "");

  // 4. PUT api/stage with distinctive content. With the gate, this MUST NOT
  // cause the operator preview to update.
  const distinctiveText = "should-not-appear-in-worship-snv-preview-281";
  const putRes = await request.put(
    new URL("/api/stage", baseURL).toString(),
    {
      data: {
        currentText: distinctiveText,
        nextText: "",
        currentGroup: "",
        nextGroup: "",
        currentSong: "",
        nextSong: "",
      },
    },
  );
  expect(putRes.ok()).toBeTruthy();

  // 5. Wait briefly for any potential leak event to land.
  await page.waitForTimeout(500);

  // 6. Verify the operator preview did NOT change.
  const previewAfterPut = await currentInFrame()
    .first()
    .textContent()
    .catch(() => "");
  expect(previewAfterPut ?? "").not.toContain(distinctiveText);
  expect(previewAfterPut).toBe(previewBefore);

  // 7. Switch to api layout — operator preview SHOULD now reflect the
  // stored api content (per the switch-to-api refresh in
  // set_stage_layout_code).
  const switchRes = await request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "api" } },
  );
  expect(switchRes.ok()).toBeTruthy();

  // 8. Wait for the preview to update with the stored api content.
  await expect
    .poll(
      async () => {
        const text = await currentInFrame()
          .first()
          .textContent()
          .catch(() => "");
        return text ?? "";
      },
      { timeout: 10_000 },
    )
    .toContain(distinctiveText);

  // Console must be clean.
  expect(consoleMessages).toEqual([]);

  // Suppress unused warnings for context (we keep it in the signature for
  // symmetry with other tests in this file that use it).
  void context;
});

declare global {
  interface Window {
    __presenterStageLayout?: string;
  }
}
