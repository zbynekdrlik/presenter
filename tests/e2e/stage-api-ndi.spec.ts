import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

const ALLOWED_CONSOLE_NOISE = [
  /integrity.*ignored.*preload/i,
  /ResizeObserver loop/i,
];

function collectConsoleErrors(
  page: import("@playwright/test").Page,
  extraAllowed: RegExp[] = [],
): string[] {
  const messages: string[] = [];
  const allowed = [...ALLOWED_CONSOLE_NOISE, ...extraAllowed];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!allowed.some((pattern) => pattern.test(text))) {
        messages.push(`[${msg.type()}] ${text}`);
      }
    }
  });
  return messages;
}

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

test("api layout renders ApiStage wrapper with no NDI source active", async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page);

  // Ensure no video source is active
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  // Switch stage to api layout
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "api" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="api"]', {
    timeout: 10_000,
  });

  // ApiStage wrapper must be in the DOM
  const wrapper = page.locator("div.stage-api");
  await expect(wrapper).toBeAttached();

  // No NDI video when no source is active
  const video = page.locator('[data-role="ndi-video"]');
  await expect(video).toHaveCount(0);

  // WorshipSnv content is nested inside the wrapper
  const slide = page.locator("div.stage-api .stage__current-slide");
  await expect(slide).toBeAttached();

  // Wrapper should be absolutely sized to viewport
  const wrapperStyle = await wrapper.evaluate((el) => {
    const cs = window.getComputedStyle(el);
    return {
      position: cs.position,
      width: cs.width,
      height: cs.height,
    };
  });
  expect(wrapperStyle.position).toBe("relative");

  // Slide text inside .stage-api must have a non-empty text-shadow
  const slideShadow = await page
    .locator("div.stage-api .stage__current-slide .stage__slide-text")
    .evaluate((el) => window.getComputedStyle(el).textShadow);
  expect(slideShadow).not.toBe("none");
  expect(slideShadow).not.toBe("");

  expect(consoleMessages).toEqual([]);
});

test("worship-snv layout is not affected by api stage changes", async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page);

  // Switch back to worship-snv
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "worship-snv" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="worship-snv"]', {
    timeout: 10_000,
  });

  // No api wrapper
  await expect(page.locator("div.stage-api")).toHaveCount(0);
  await expect(page.locator('[data-role="ndi-video"]')).toHaveCount(0);

  // Worship-snv slide text must NOT have a text-shadow (only api layout gets it)
  const slideShadow = await page
    .locator('div.stage-container[data-layout="worship-snv"] .stage__current-slide .stage__slide-text')
    .evaluate((el) => window.getComputedStyle(el).textShadow);
  expect(slideShadow).toBe("none");

  expect(consoleMessages).toEqual([]);
});

test("api layout mounts NDI video but shows NO red overlay for a connecting/not-producing source", async ({ page }) => {
  // #448 changed the api_stage layout: the NDI video here is a BACKGROUND
  // behind the slides, so a NEUTRAL state (connecting / no-signal — the source
  // is configured but not yet producing) must paint NOTHING over the slides.
  // Only a GENUINE failure (`ndi_overlay_kind == Error`: `failed`/`disconnected`)
  // surfaces the red `.stage-api__overlay`. This test replaces the old one,
  // which asserted the now-removed "Connecting" red overlay on activation.
  //
  // We deliberately activate a bogus NDI source so the WS event fires while
  // the page is open. Once `ndi_active=true`, the <NdiVideo> mounts and tries
  // to fetch the WHEP endpoint, which returns 503 because the bogus name has no
  // real stream — that 503 is expected noise for this test only. The
  // reconnect_loop retries with backoff and emits a WARN on each failed
  // attempt; allow that too. The published status stays NEUTRAL the whole time
  // (`connecting` on activate; `no-signal` if a manager classifies it as
  // SourceSilent) — both map to `NdiOverlayKind::Neutral`, so the red overlay
  // must never appear (verified against `ndi_overlay_kind` in
  // crates/presenter-ui/src/components/stage/mod.rs). The Error→red-overlay path
  // is covered by the unit tests there (`failed`/`disconnected` → Error).
  const consoleMessages = collectConsoleErrors(page, [
    /Failed to load resource.*503/i,
    /WHEP connect for.*failed/i,
    /reconnect_loop.*connect_whep failed/i,
  ]);

  // Start clean
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  // Navigate FIRST so the WS is open before we activate
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "api" } },
  );
  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="api"]', {
    timeout: 10_000,
  });

  // Sanity: NdiVideo not present yet, and no overlay
  await expect(page.locator('[data-role="ndi-video"]')).toHaveCount(0);
  await expect(page.locator("div.stage-api__overlay")).toHaveCount(0);

  // Create a bogus video source
  const createResp = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "E2E Stage API NDI Test", ndiName: "BOGUS_DOES_NOT_EXIST" } },
  );
  const source = await createResp.json();

  // Activate. The handler publishes NdiSourceActivated to the live hub
  // BEFORE attempting to start the stream, so even when start_stream fails
  // for a bogus name, the frontend still receives the event. We ignore
  // the HTTP status here and observe DOM effects instead.
  await page.request.post(
    new URL(
      `/integrations/video-sources/${source.id}/activate`,
      baseURL,
    ).toString(),
    { failOnStatusCode: false },
  );

  try {
    // NEW behavior: the <NdiVideo> mounts (ndi_active=true) …
    await expect(page.locator('[data-role="ndi-video"]')).toHaveCount(1, {
      timeout: 10_000,
    });
    // … but the NEUTRAL connecting/not-producing state paints NOTHING over the
    // slides — the red overlay must NOT appear. Give the status time to settle
    // (connecting → possibly no-signal) and assert the overlay stays absent so
    // the slides show through.
    await page.waitForTimeout(2_000);
    await expect(page.locator("div.stage-api__overlay")).toHaveCount(0);
    // The slides remain visible through the (absent) overlay.
    await expect(
      page.locator("div.stage-api .stage__current-slide"),
    ).toBeAttached();
  } finally {
    // Cleanup so subsequent test runs are clean
    await page.request.post(
      new URL("/integrations/video-sources/deactivate", baseURL).toString(),
      { failOnStatusCode: false },
    );
    await page.request.delete(
      new URL(
        `/integrations/video-sources/${source.id}`,
        baseURL,
      ).toString(),
      { failOnStatusCode: false },
    );
  }

  expect(consoleMessages).toEqual([]);
});
