/**
 * #853 (prod SNV, 2026-10-13): while the stage changes slides every few seconds (AbleSet driving the
 * lyrics during a rehearsal), an operator working in the operator page must keep their place — no
 * reload, no scroll jump in the song list or the playlist, and, in Edit mode, no yank of the slide
 * list away from the slide being edited, even when that song is the one on stage or AbleSet Follow
 * switches the stage to another song.
 */
import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 240_000 });

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let libId: string;
let playlistId: string;
const presIds: string[] = [];
const slideIds: Record<string, string[]> = {};

async function send(method: string, path: string, body: unknown): Promise<any> {
  const resp = await fetch(new URL(path, baseURL).toString(), {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await resp.text();
  expect(resp.ok, `${method} ${path} → ${resp.status} ${text.slice(0, 200)}`).toBeTruthy();
  return text ? JSON.parse(text) : null;
}

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);

  const lib = await send("POST", "/libraries", { name: "_E2E Stage Churn" });
  libId = lib.id;
  for (let p = 0; p < 24; p++) {
    const slides = Array.from({ length: 16 }, (_, i) => ({ main: `Song ${p + 1} slide ${i + 1}\nSecond line` }));
    const data = await send("POST", `/libraries/${libId}/presentations`, {
      name: `Churn Song ${String(p + 1).padStart(2, "0")}`,
      slides,
    });
    presIds.push(data.presentation.id);
    slideIds[data.presentation.id] = data.presentation.slides.map((s: { id: string }) => s.id);
  }
  const pl = await send("POST", "/playlists", { name: "_E2E Churn Set", showInDashboard: true });
  playlistId = pl.id;
  await send("PUT", `/playlists/${playlistId}/entries`, {
    entries: presIds.map((id) => ({ type: "presentation", presentationId: id })),
  });
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

/** Puts an unrelated song (24) on stage so no test inherits the previous test's stage. */
async function resetStage() {
  await send("POST", "/stage/state", { presentationId: presIds[23], currentSlideId: slideIds[presIds[23]][0] });
}

/** AbleSet-like churn: `count` stage slide changes of `pres`, 0.7 s apart. */
async function churnStage(pres: string, count: number) {
  for (let i = 0; i < count; i++) {
    await send("POST", "/stage/state", { presentationId: pres, currentSlideId: slideIds[pres][i % 16] });
    await new Promise((r) => setTimeout(r, 700));
  }
}

/** Counts the `stage` frames the operator page's own live socket receives (not the preview iframe's). */
function countStageFrames(page: Page): { count: number } {
  const seen = { count: 0 };
  page.on("websocket", (ws) => {
    if (ws.url().includes("preview")) return;
    ws.on("framereceived", (frame) => {
      try {
        if (JSON.parse(String(frame.payload)).type === "stage") seen.count += 1;
      } catch {
        // binary or non-JSON frame
      }
    });
  });
  return seen;
}

async function scrollTops(page: Page) {
  // Let the 0 ms scroll timeout and a frame run before reading.
  return page.evaluate(
    () =>
      new Promise<{ list: number; slides: number; marker: boolean }>((resolve) =>
        setTimeout(
          () =>
            requestAnimationFrame(() =>
              resolve({
                list: document.querySelector(".operator__presentation-list")?.scrollTop ?? -1,
                slides: document.querySelector(".operator__slides")?.scrollTop ?? -1,
                marker: (window as any).__churnMarker === 1,
              }),
            ),
          0,
        ),
      ),
  );
}

async function openOperator(page: Page) {
  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
}

async function openTestLibrary(page: Page) {
  await page.locator('[data-role="library-more"]').click();
  await page.locator(`[data-role="library-row"][data-library-id="${libId}"] .operator__list-button`).click();
  await page.waitForSelector('[data-role="presentation-item"]');
}

/** Edit mode, song `songNumber` open, the caret at the END of slide 14's main text, " typing" typed. */
async function editSlide(page: Page, songNumber: number) {
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
  const label = `Churn Song ${String(songNumber).padStart(2, "0")}`;
  await page.locator('[data-role="presentation-item"]', { hasText: label }).first().click();
  await page.waitForSelector(".operator__slides [data-slide-id] textarea");
  const target = slideIds[presIds[songNumber - 1]][13];
  const area = page.locator(`.operator__slides [data-slide-id="${target}"] textarea`).first();
  await area.scrollIntoViewIfNeeded();
  await area.click();
  await page.keyboard.press("End");
  await page.keyboard.type(" typing");
  return { target, area };
}

for (const source of ["library", "playlist"] as const) {
  test(`stage churn keeps the operator's place (${source})`, async ({ page }) => {
    const consoleMessages: string[] = [];
    attachConsoleErrorCollector(page, consoleMessages);
    const frames = countStageFrames(page);
    await resetStage();
    await openOperator(page);
    if (source === "library") {
      await openTestLibrary(page);
    } else {
      await page.locator(`[data-role="playlist-item"][data-playlist-id="${playlistId}"]`).first().click();
      await page.waitForSelector('[data-role="presentation-item"]');
    }
    // The operator opens song 20 (not the one on stage) and scrolls both lists.
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 20" }).first().click();
    await page.waitForSelector(".operator__slides [data-slide-id]");
    await page.evaluate(() => {
      (window as any).__churnMarker = 1;
      (document.querySelector(".operator__presentation-list") as HTMLElement).scrollTop = 300;
      (document.querySelector(".operator__slides") as HTMLElement).scrollTop = 400;
    });
    const before = await scrollTops(page);
    expect(before.list, "the song list must be scrollable for this test").toBeGreaterThan(0);
    expect(before.slides, "the slide list must be scrollable for this test").toBeGreaterThan(0);

    const framesBefore = frames.count;
    await churnStage(presIds[1], 8);
    await expect.poll(() => frames.count - framesBefore, { message: "stage events reach the page" }).toBeGreaterThanOrEqual(8);
    const after = await scrollTops(page);
    console.log(`[churn ${source}] before ${JSON.stringify(before)} after ${JSON.stringify(after)}`);
    expect(after.marker, "the page must not reload").toBe(true);
    expect(after.list, "song list keeps its scroll").toBe(before.list);
    expect(after.slides, "slide list keeps its scroll").toBe(before.slides);
    expect(consoleMessages).toEqual([]);
  });
}

for (const which of ["live", "other"] as const) {
  test(`editing slides while AbleSet advances the stage keeps the editor in place (${which} song)`, async ({ page }) => {
    const consoleMessages: string[] = [];
    attachConsoleErrorCollector(page, consoleMessages);
    const frames = countStageFrames(page);
    await resetStage();
    await openOperator(page);
    await openTestLibrary(page);
    // The operator edits song 3; AbleSet drives song 3 ("live") or song 2 ("other").
    const { target, area } = await editSlide(page, 3);
    const live = which === "live" ? presIds[2] : presIds[1];
    await page.evaluate(() => {
      (window as any).__churnMarker = 1;
    });
    const before = await scrollTops(page);
    expect(before.slides, "slide 14 must sit below the top of the list").toBeGreaterThan(0);
    if (which === "live") {
      // Slide 1 must be above the visible area, so a pull toward the stage slide is observable.
      const firstAbove = await page.evaluate((firstId) => {
        const list = document.querySelector(".operator__slides") as HTMLElement;
        const first = document.querySelector(`.operator__slides [data-slide-id="${firstId}"]`) as HTMLElement;
        return first.getBoundingClientRect().bottom < list.getBoundingClientRect().top;
      }, slideIds[live][0]);
      expect(firstAbove, "slide 1 starts above the visible list").toBe(true);
    }

    const framesBefore = frames.count;
    await churnStage(live, 6);
    await expect.poll(() => frames.count - framesBefore, { message: "stage events reach the page" }).toBeGreaterThanOrEqual(6);
    if (which === "live") {
      await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[live][5]}"]`)).toHaveClass(/is-active/);
    }
    const after = await scrollTops(page);
    const focused = await page.evaluate((sid) => {
      const ta = document.querySelector(`.operator__slides [data-slide-id="${sid}"] textarea`);
      return !!ta && document.activeElement === ta;
    }, target);
    console.log(`[edit ${which}] before ${JSON.stringify(before)} after ${JSON.stringify(after)} focused ${focused}`);
    expect(after.marker, "the page must not reload").toBe(true);
    expect(focused, "the slide being edited keeps focus").toBe(true);
    await expect(area).toHaveValue(/ typing$/);
    expect(after.slides, "the slide list must not move away from the edited slide").toBe(before.slides);
    expect(consoleMessages).toEqual([]);
  });
}

test("editing with AbleSet Follow on: a stage song change does not replace the song being edited", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const frames = countStageFrames(page);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await openTestLibrary(page);
    const { target, area } = await editSlide(page, 3);
    await page.evaluate(() => {
      (window as any).__churnMarker = 1;
    });
    const before = await scrollTops(page);
    expect(before.slides, "slide 14 must sit below the top of the list").toBeGreaterThan(0);

    // AbleSet moves on to song 2: with Follow on, Live mode would open it.
    const framesBefore = frames.count;
    await churnStage(presIds[1], 3);
    await expect.poll(() => frames.count - framesBefore, { message: "stage events reach the page" }).toBeGreaterThanOrEqual(3);
    const after = await scrollTops(page);
    const state = await page.evaluate((sid) => {
      const ta = document.querySelector(`.operator__slides [data-slide-id="${sid}"] textarea`);
      return { stillOpen: !!ta, focused: !!ta && document.activeElement === ta };
    }, target);
    console.log(`[follow edit] before ${JSON.stringify(before)} after ${JSON.stringify(after)} state ${JSON.stringify(state)}`);
    expect(after.marker, "the page must not reload").toBe(true);
    expect(state.stillOpen, "the edited song stays open").toBe(true);
    expect(state.focused, "the slide being edited keeps focus").toBe(true);
    await expect(area).toHaveValue(/ typing$/);
    expect(after.slides, "the slide list keeps its scroll").toBe(before.slides);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});
