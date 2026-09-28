import { test, expect, Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

/**
 * #799 — songplayer translation + API text mode + ambient CG-video layout.
 *
 * A real operator picks the `api-ambient` layout and each text mode from the
 * header controls; the real `/stage` display must show the matching text(s)
 * live, hide the overlay entirely when the API text is cleared (pure video),
 * and the classic `api` layout must follow the same mode. Clean console on
 * every page.
 */

test.describe.configure({ timeout: 180_000 });

const ALLOWED_CONSOLE_NOISE = [
  /integrity.*ignored.*preload/i,
  /ResizeObserver loop/i,
];

function collectConsoleErrors(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!ALLOWED_CONSOLE_NOISE.some((pattern) => pattern.test(text))) {
        messages.push(`[${msg.type()}] ${text}`);
      }
    }
  });
  return messages;
}

// Each line stays under the stage's 26-char auto-break threshold so the
// rendered text is exactly the sent line (no inserted line break).
const EN_CURRENT = "Amazing grace";
const SK_CURRENT = "Úžasná milosť";
const EN_NEXT = "How sweet the sound";
const SK_NEXT = "Aký sladký zvuk";

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

function url(path: string): string {
  return new URL(path, baseURL).toString();
}

async function openOperator(page: Page) {
  await page.goto(url("/ui/operator"), { waitUntil: "domcontentloaded" });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  const layoutSelect = page.locator('[data-role="stage-layout-select"]');
  await expect(layoutSelect).toBeVisible({ timeout: 15_000 });
  await expect(layoutSelect.locator('option[value="api-ambient"]')).toHaveCount(
    1,
    { timeout: 15_000 },
  );
  return layoutSelect;
}

async function openStage(page: Page, layout: string) {
  await page.goto(url("/stage"), { waitUntil: "domcontentloaded" });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForFunction(
    () =>
      (window as unknown as { __presenterStageConnectionState?: string })
        .__presenterStageConnectionState === "connected",
    undefined,
    { timeout: 30_000 },
  );
  await expect(page.locator("body")).toHaveAttribute(
    "data-layout-code",
    layout,
    { timeout: 15_000 },
  );
}

async function selectTextMode(operator: Page, mode: string) {
  const modeSelect = operator.locator('[data-role="stage-text-mode-select"]');
  await expect(modeSelect).toBeVisible({ timeout: 10_000 });
  await modeSelect.selectOption(mode);
  // The operator control really reached the server (persisted setting).
  await expect
    .poll(
      async () => (await (await fetch(url("/stage/text-mode"))).json()).mode,
      { timeout: 10_000 },
    )
    .toBe(mode);
}

test("api-ambient shows the mode-selected text over video and hides when empty", async ({
  context,
  request,
}) => {
  const put = await request.put(url("/api/stage"), {
    data: {
      currentText: EN_CURRENT,
      currentTranslation: SK_CURRENT,
      nextText: EN_NEXT,
      nextTranslation: SK_NEXT,
      currentSong: "Amazing Grace",
    },
  });
  expect(put.status()).toBe(204);

  // The operator picks the ambient layout from the real header control.
  const operator = await context.newPage();
  const operatorConsole = collectConsoleErrors(operator);
  const layoutSelect = await openOperator(operator);
  // The text-mode control only appears for API layouts.
  await expect(
    operator.locator('[data-role="stage-text-mode-select"]'),
  ).toHaveCount(0);
  await layoutSelect.selectOption("api-ambient");

  const stage = await context.newPage();
  const stageConsole = collectConsoleErrors(stage);
  await openStage(stage, "api-ambient");

  const lyrics = stage.locator('[data-role="ambient-lyrics"]');
  const primary = stage.locator('[data-role="ambient-primary"]');
  const secondary = stage.locator('[data-role="ambient-secondary"]');

  // The ambient layout keeps the bottom status bar exactly like
  // ndi-fullscreen (owner ruling on #799): clock + connection readout,
  // no live pill, no song number.
  const ambient = stage.locator('[data-layout="api-ambient"]');
  await expect(ambient).toBeVisible();
  const clock = ambient.locator(".stage__clock");
  const connection = ambient.locator(".stage__connection");
  await expect(clock).toBeVisible();
  await expect(clock).toHaveText(/\d{2}:\d{2}:\d{2}/);
  await expect(connection).toBeVisible();
  await expect(ambient.locator(".stage__live-pill")).toHaveCount(0);
  await expect(ambient.locator('[data-role="song-number"]')).toHaveCount(0);

  // Default mode = both: original large, translation smaller below.
  await expect(lyrics).toBeVisible({ timeout: 10_000 });
  await expect(lyrics).toHaveAttribute("data-visible", "true");
  await expect(primary).toHaveText(EN_CURRENT);
  await expect(secondary).toBeVisible();
  await expect(secondary).toHaveText(SK_CURRENT);
  // Autofit runs on the next animation frame — poll the settled sizes.
  await expect
    .poll(
      async () => {
        const [primaryPx, secondaryPx] = await Promise.all([
          primary.evaluate((el) => parseFloat(getComputedStyle(el).fontSize)),
          secondary.evaluate((el) => parseFloat(getComputedStyle(el).fontSize)),
        ]);
        return primaryPx > secondaryPx;
      },
      { timeout: 10_000 },
    )
    .toBe(true);
  // The lyric overlay sits ABOVE the status bar — its area ends where the
  // bar begins, it never overlaps the clock or the connection readout.
  const lyricsBox = await lyrics.boundingBox();
  const clockBox = await clock.boundingBox();
  const connectionBox = await connection.boundingBox();
  expect(lyricsBox).not.toBeNull();
  expect(clockBox).not.toBeNull();
  expect(connectionBox).not.toBeNull();
  const lyricsBottom = lyricsBox!.y + lyricsBox!.height;
  expect(lyricsBottom).toBeLessThanOrEqual(clockBox!.y + 1);
  expect(lyricsBottom).toBeLessThanOrEqual(connectionBox!.y + 1);
  await expect(connection).toContainText("CONNECTED");

  // The latency readouts on the right: with the NDI/CG source active the
  // server→display video-latency readout shows too (driven by the stage's
  // #479/#512 test hooks — the e2e lane has no NDI source), also below the
  // overlay.
  await stage.evaluate(() => {
    const w = window as unknown as {
      __presenterStageSetNdiActive?: (v: boolean) => void;
      __presenterStageSetVideoLatency?: (v: number | null) => void;
    };
    w.__presenterStageSetNdiActive?.(true);
    w.__presenterStageSetVideoLatency?.(42);
  });
  const videoLatency = ambient.locator(".stage__video-latency");
  await expect(videoLatency).toBeVisible();
  await expect(videoLatency).toContainText(/server→displej\s*·\s*42\s*ms/);
  const videoLatencyBox = await videoLatency.boundingBox();
  expect(videoLatencyBox).not.toBeNull();
  expect(lyricsBottom).toBeLessThanOrEqual(videoLatencyBox!.y + 1);
  await stage.evaluate(() => {
    const w = window as unknown as {
      __presenterStageSetNdiActive?: (v: boolean) => void;
      __presenterStageSetVideoLatency?: (v: number | null) => void;
    };
    w.__presenterStageSetVideoLatency?.(null);
    w.__presenterStageSetNdiActive?.(false);
  });
  await expect(videoLatency).toHaveCount(0);
  // Still no song number / on-air pill on the ambient layout.
  await expect(ambient.locator(".stage__live-pill")).toHaveCount(0);
  await expect(ambient.locator('[data-role="song-number"]')).toHaveCount(0);

  // The next line is never previewed on an ambient display.
  await expect(stage.getByText(EN_NEXT)).toHaveCount(0);

  // original → only the original line, live (no reload).
  await selectTextMode(operator, "original");
  await expect(lyrics).toHaveAttribute("data-both", "false", {
    timeout: 10_000,
  });
  await expect(primary).toHaveText(EN_CURRENT);
  await expect(secondary).toBeHidden();

  // translation → only the translation.
  await selectTextMode(operator, "translation");
  await expect(primary).toHaveText(SK_CURRENT, { timeout: 10_000 });
  await expect(secondary).toBeHidden();

  // both again.
  await selectTextMode(operator, "both");
  await expect(primary).toHaveText(EN_CURRENT, { timeout: 10_000 });
  await expect(secondary).toHaveText(SK_CURRENT);
  await expect(secondary).toBeVisible();

  // A long lyric line wraps and must be auto-fitted INTO the overlay box
  // (shrunk below the max font, nothing clipped) — both lines.
  const LONG_EN =
    "And when this flesh and heart shall fail and mortal life shall cease, I shall possess within the veil a life of joy and peace";
  const LONG_SK =
    "A keď mi telo zlyhá raz a smrteľný sa skončí čas, za oponou ja prijmem dar, život radosti a pokoja v nás";
  await request.put(url("/api/stage"), {
    data: { currentText: LONG_EN, currentTranslation: LONG_SK },
  });
  await expect(primary).toHaveText(LONG_EN, { timeout: 10_000 });
  for (const [box, maxPx] of [
    [primary, 160],
    [secondary, 90],
  ] as const) {
    await expect
      .poll(
        () =>
          box.evaluate((el, max) => {
            const px = parseFloat(getComputedStyle(el).fontSize);
            return px < max && el.scrollHeight <= el.clientHeight;
          }, maxPx),
        { timeout: 10_000 },
      )
      .toBe(true);
  }
  await request.put(url("/api/stage"), {
    data: { currentText: EN_CURRENT, currentTranslation: SK_CURRENT },
  });
  await expect(primary).toHaveText(EN_CURRENT, { timeout: 10_000 });

  // Empty API text → the overlay fades out and is hidden: pure video.
  const clear = await request.put(url("/api/stage"), { data: {} });
  expect(clear.status()).toBe(204);
  await expect(lyrics).toHaveAttribute("data-visible", "false", {
    timeout: 10_000,
  });
  await expect(lyrics).toBeHidden({ timeout: 5_000 });

  // Text comes back → overlay fades in again.
  await request.put(url("/api/stage"), {
    data: { currentText: EN_CURRENT, currentTranslation: SK_CURRENT },
  });
  await expect(lyrics).toBeVisible({ timeout: 10_000 });
  await expect(primary).toHaveText(EN_CURRENT);

  // A reloaded display (reconnect resync path) keeps the layout + mode.
  await selectTextMode(operator, "translation");
  await stage.reload({ waitUntil: "domcontentloaded" });
  await openStage(stage, "api-ambient");
  await expect(primary).toHaveText(SK_CURRENT, { timeout: 10_000 });

  await stage.close();
  await operator.close();
  expect(stageConsole).toEqual([]);
  expect(operatorConsole).toEqual([]);
});

test("the classic api layout follows the text mode too", async ({
  context,
  request,
}) => {
  await request.put(url("/api/stage"), {
    data: {
      currentText: EN_CURRENT,
      currentTranslation: SK_CURRENT,
      nextText: EN_NEXT,
      nextTranslation: SK_NEXT,
    },
  });

  const operator = await context.newPage();
  const operatorConsole = collectConsoleErrors(operator);
  const layoutSelect = await openOperator(operator);
  await layoutSelect.selectOption("api");

  const stage = await context.newPage();
  const stageConsole = collectConsoleErrors(stage);
  await openStage(stage, "api");
  const current = stage.locator(".stage__current-slide .stage__slide-text");
  const next = stage.locator(".stage__next-slide .stage__slide-text");

  await selectTextMode(operator, "translation");
  await expect(current).toHaveText(SK_CURRENT, { timeout: 10_000 });
  await expect(next).toHaveText(SK_NEXT);

  await selectTextMode(operator, "original");
  await expect(current).toHaveText(EN_CURRENT, { timeout: 10_000 });
  await expect(next).toHaveText(EN_NEXT);

  await selectTextMode(operator, "both");
  await expect(current).toContainText(EN_CURRENT, { timeout: 10_000 });
  await expect(current).toContainText(SK_CURRENT);

  await stage.close();
  await operator.close();
  expect(stageConsole).toEqual([]);
  expect(operatorConsole).toEqual([]);
});
