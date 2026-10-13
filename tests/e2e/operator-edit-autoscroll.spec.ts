/**
 * #853 (prod SNV, 2026-10-13): while the stage changes slides every few seconds (AbleSet driving the
 * lyrics during a rehearsal), an operator working in the operator page must keep their place — no
 * reload, no scroll jump in the song list or the playlist, and, in Edit mode, no yank of the slide
 * list away from the slide being edited, even when that song is the one on stage or AbleSet Follow
 * switches the stage to another song.
 */
import { test, expect, type Page, type Route } from "@playwright/test";
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

/** AbleSet-like churn: `count` stage slide changes of `pres` from slide `from`, 0.7 s apart. */
async function churnStage(pres: string, count: number, from = 0) {
  for (let i = from; i < from + count; i++) {
    await send("POST", "/stage/state", { presentationId: pres, currentSlideId: slideIds[pres][i % 16] });
    await new Promise((r) => setTimeout(r, 700));
  }
}

/**
 * Counts the `stage` frames the operator page's own live socket receives (not the preview iframe's),
 * one per stage change: the always-on camera-crew snapshot each change also sends is skipped. On an
 * API layout a presenter trigger sends ONLY the camera-crew frame (not counted); a songplayer push is.
 */
function countStageFrames(page: Page): { count: number } {
  const seen = { count: 0 };
  page.on("websocket", (ws) => {
    if (ws.url().includes("preview")) return;
    ws.on("framereceived", (frame) => {
      try {
        const msg = JSON.parse(String(frame.payload));
        const snapshot = msg.snapshot ?? msg;
        if (msg.type === "stage" && snapshot.layout?.code !== "camera-crew") seen.count += 1;
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

/** True when the slide card is fully inside the visible part of the slide list. */
async function cardInView(page: Page, slideId: string): Promise<boolean> {
  return page.evaluate((sid) => {
    const list = document.querySelector(".operator__slides") as HTMLElement | null;
    const card = document.querySelector(`.operator__slides [data-slide-id="${sid}"]`) as HTMLElement | null;
    if (!list || !card) return false;
    const l = list.getBoundingClientRect();
    const c = card.getBoundingClientRect();
    return c.top >= l.top - 1 && c.bottom <= l.bottom + 1;
  }, slideId);
}

/** Holds every matching request until the returned `release` is called. */
async function gateRoute(page: Page, url: string): Promise<() => void> {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  await page.route(url, async (route) => {
    await gate;
    await route.continue();
  });
  return release;
}

/** Waits until a response's body has fully arrived (headers alone resolve `waitForResponse`). */
async function landed(response: Promise<import("@playwright/test").Response>) {
  await (await response).finished();
}

/** Drags a slide card onto another with dispatched DragEvents (as wasm-drag-drop.spec.ts). */
async function dragSlide(page: Page, draggedSlideId: string, targetSlideId: string) {
  await page.evaluate(
    ({ draggedSlideId, targetSlideId }) => {
      const handle = document.querySelector(`[data-slide-id="${draggedSlideId}"] [data-role="slide-drag-handle"]`);
      const target = document.querySelector(`[data-slide-id="${targetSlideId}"]`);
      if (!handle || !target) throw new Error("drag handle or target card not found");
      const dataTransfer = new DataTransfer();
      const opts = { bubbles: true, cancelable: true, dataTransfer };
      handle.dispatchEvent(new DragEvent("dragstart", opts));
      if (target.dispatchEvent(new DragEvent("dragover", opts))) throw new Error("dragover was not accepted");
      target.dispatchEvent(new DragEvent("drop", opts));
      handle.dispatchEvent(new DragEvent("dragend", opts));
    },
    { draggedSlideId, targetSlideId },
  );
}

/** Follow's status has loaded as ON and the live socket is connected (a stage frame will arrive). */
async function followReady(page: Page) {
  await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
  await page.waitForFunction(() => (window as any).__presenterLiveConnected?.() === true);
}

/** Lets the WASM handle what just arrived: a 0 ms timeout, then two animation frames. */
async function settle(page: Page) {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        setTimeout(() => requestAnimationFrame(() => requestAnimationFrame(() => resolve())), 0),
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
  await page.keyboard.press("Control+End");
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

    // The stage moves to song 2 (one song change: the list re-highlights) ...
    const framesBefore = frames.count;
    await churnStage(presIds[1], 1);
    const song2Row = page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 02" }).first();
    await expect(song2Row, "the stage song is highlighted").toHaveClass(/is-stage-active/);
    await expect(page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 24" }).first()).not.toHaveClass(/is-stage-active/);
    await page.evaluate(() => {
      (document.querySelector('[data-role="presentation-item"]') as any).__churnTag = 1;
    });
    // ... then AbleSet walks its slides: slide changes must not rebuild the rows.
    await churnStage(presIds[1], 7, 1);
    await expect.poll(() => frames.count - framesBefore, { message: "stage events reach the page" }).toBeGreaterThanOrEqual(8);
    const after = await scrollTops(page);
    const sameRows = await page.evaluate(
      () => (document.querySelector('[data-role="presentation-item"]') as any)?.__churnTag === 1,
    );
    console.log(`[churn ${source}] before ${JSON.stringify(before)} after ${JSON.stringify(after)} sameRows ${sameRows}`);
    expect(after.marker, "the page must not reload").toBe(true);
    expect(sameRows, "slide changes must not rebuild the song list").toBe(true);
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
    await expect(page.locator(`.operator__slides [data-slide-id="${target}"]`), "the edited card stays marked").toHaveClass(/is-focused/);
    await expect(area).toHaveValue(/ typing$/);
    expect(after.slides, "the slide list must not move away from the edited slide").toBe(before.slides);
    if (which === "live") {
      // AbleSet reaches the very slide being edited: its card turns active and must stay marked
      // as the edited one (the old DOM-poked class was dropped when the card's class changed).
      await send("POST", "/stage/state", { presentationId: live, currentSlideId: target });
      const card = page.locator(`.operator__slides [data-slide-id="${target}"]`);
      await expect(card).toHaveClass(/is-active/);
      await expect(card, "the edited card stays marked while it is the stage slide").toHaveClass(/is-focused/);
    }
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
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
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

test("back to Live with AbleSet Follow on: the operator catches up with the stage song at once", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const frames = countStageFrames(page);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    // Edit mode, song 3 open, nothing typed (so leaving Edit saves nothing and sends no stage event).
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // AbleSet moves on to song 2 while the operator is in Edit mode: nothing switches yet.
    const framesBefore = frames.count;
    await churnStage(presIds[1], 3, 11); // slides 12–14 of song 2
    await expect.poll(() => frames.count - framesBefore, { message: "stage events reach the page" }).toBeGreaterThanOrEqual(3);
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`)).toBeVisible();

    // Back to Live: Follow opens song 2 without waiting for another stage event, so an arrow key
    // never sends a slide of the song that was open in Edit mode.
    const framesAtSwitch = frames.count;
    await page.locator('[data-role="mode-toggle"][data-mode="live"]').first().click();
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`),
      "Live mode opens the stage song",
    ).toBeAttached();
    expect(frames.count, "no stage event was needed to catch up").toBe(framesAtSwitch);
    // ... scrolled to the slide that is on stage (slide 14), not left at the top.
    await expect.poll(() => cardInView(page, slideIds[presIds[1]][13]), { message: "the stage slide is in view" }).toBe(true);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow: switching to Edit while the stage song is still loading keeps Edit on its song, and Live catches up", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // Song 2's detail answers slowly; the stage moves to song 2 and the operator clicks Edit meanwhile.
    const releaseSong2 = await gateRoute(page, `**/presentations/${presIds[1]}`);
    const songRequested = page.waitForRequest(`**/presentations/${presIds[1]}`);
    const songAnswered = page.waitForResponse(`**/presentations/${presIds[1]}`);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await songRequested; // Follow is loading song 2 ...
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
    await expect(page.locator("body")).toHaveAttribute("data-mode", "edit");
    releaseSong2();
    await landed(songAnswered); // ... and it lands while in Edit mode
    await settle(page);
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`),
      "Edit mode keeps song 3",
    ).toBeAttached();

    await page.unroute(`**/presentations/${presIds[1]}`);
    await page.locator('[data-role="mode-toggle"][data-mode="live"]').first().click();
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`),
      "back in Live the stage song opens",
    ).toBeAttached();
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow: a song the operator clicks while the stage song's library is loading stays open", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    // From the playlist, so a late library switch would be visible (the library holds the same songs).
    const playlistButton = page.locator(`[data-role="playlist-item"][data-playlist-id="${playlistId}"]`).first();
    await playlistButton.click();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Churn Set");
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // The library list (`GET /libraries/summary`, where Follow reads the song list) answers slowly; the stage moves to song 2 and the
    // operator clicks song 5 meanwhile.
    const releaseLibrary = await gateRoute(page, "**/libraries/summary");
    const libraryRequested = page.waitForRequest("**/libraries/summary");
    const libraryAnswered = page.waitForResponse("**/libraries/summary");
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    // Click while Follow waits for that library.
    await libraryRequested;
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[4]][0]}"]`)).toBeAttached();
    releaseLibrary();
    await landed(libraryAnswered); // the held library lands after the click
    await settle(page);
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[4]][0]}"]`),
      "the clicked song stays open",
    ).toBeAttached();
    await expect(page.locator('[data-role="context-title"]'), "the playlist stays the context").toHaveText("_E2E Churn Set");
    await expect(playlistButton).toHaveAttribute("data-active", "true");
    await page.unroute("**/libraries/summary");
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow: a stage song that fails to load is opened on the next stage change", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // Song 2's detail fails once — a malformed answer, so the browser logs nothing (a 5xx would).
    let failed = false;
    await page.route(`**/presentations/${presIds[1]}`, async (route) => {
      if (failed) return route.continue();
      failed = true;
      return route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    });
    const failedAnswer = page.waitForResponse(`**/presentations/${presIds[1]}`);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await landed(failedAnswer);
    await settle(page);
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`),
      "song 3 stays shown",
    ).toBeAttached();
    await expect(
      page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first(),
      "and stays the selected song (so an arrow key sends what is shown)",
    ).toHaveClass(/is-active/);

    // The next stage change retries.
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][1] });
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`),
      "song 2 opens on the next stage change",
    ).toBeAttached();
    await page.unroute(`**/presentations/${presIds[1]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow: the selection stays on the shown song while the stage song loads, and a stage that comes back leaves it there", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    const song3Row = page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first();
    await song3Row.click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // Song 2 is held; the stage moves to song 2, then back to song 3 before song 2 arrives.
    const releaseSong2 = await gateRoute(page, `**/presentations/${presIds[1]}`);
    const songRequested = page.waitForRequest(`**/presentations/${presIds[1]}`);
    const songAnswered = page.waitForResponse(`**/presentations/${presIds[1]}`);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await songRequested;
    await settle(page);
    await expect(song3Row, "while song 2 loads, the shown song 3 stays selected").toHaveClass(/is-active/);

    await send("POST", "/stage/state", { presentationId: presIds[2], currentSlideId: slideIds[presIds[2]][4] });
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][4]}"]`),
      "the stage is back on song 3",
    ).toHaveClass(/is-active/);
    releaseSong2();
    await landed(songAnswered);
    await settle(page);
    await expect(song3Row, "song 3 is still selected after song 2 arrived").toHaveClass(/is-active/);
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`)).toBeAttached();
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`)).not.toBeAttached();
    await page.unroute(`**/presentations/${presIds[1]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Live, Follow off: clicking the song on stage opens it at its first slide (#271)", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  // One slide per row, so slide 14 is far below the first screen.
  await page.addInitScript(() => localStorage.setItem("presenter:operatorSlideColumns", JSON.stringify("1")));
  await resetStage();
  await openOperator(page);
  await expect(page.locator('[data-role="ableset-follow"]'), "Follow is off").toHaveAttribute("data-state", "off");
  await openTestLibrary(page);
  await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][13] });
  const song2Row = page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 02" }).first();
  await expect(song2Row, "song 2 is on stage").toHaveClass(/is-stage-active/);

  await song2Row.click();
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`);
  await settle(page);
  expect(await cardInView(page, slideIds[presIds[1]][0]), "slide 1 is in view").toBe(true);
  expect(await cardInView(page, slideIds[presIds[1]][13]), "the list did not jump to the stage slide").toBe(false);
  expect((await scrollTops(page)).slides, "the slide list is at the top").toBe(0);
  expect(consoleMessages).toEqual([]);
});

test("Follow: a song clicked just before the stage changed does not replace the stage song Follow opened", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // The operator clicks song 5, whose answer is held; the stage then moves to song 2.
    const releaseSong5 = await gateRoute(page, `**/presentations/${presIds[4]}`);
    const clickRequested = page.waitForRequest(`**/presentations/${presIds[4]}`);
    const clickAnswered = page.waitForResponse(`**/presentations/${presIds[4]}`);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
    await clickRequested;
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    const song2First = page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`);
    await expect(song2First, "Follow opens the stage song").toBeAttached();

    // Song 5's late answer must not replace it.
    releaseSong5();
    await landed(clickAnswered);
    await settle(page);
    await expect(song2First, "song 2 stays shown").toBeAttached();
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[4]][0]}"]`)).not.toBeAttached();
    await expect(
      page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 02" }).first(),
      "and selected",
    ).toHaveClass(/is-active/);
    await page.unroute(`**/presentations/${presIds[4]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow on: a Follow open lands on the stage slide, opening the stage song by hand starts at the top (#271)", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  // One slide per row, so slide 14 is far below the first screen.
  await page.addInitScript(() => localStorage.setItem("presenter:operatorSlideColumns", JSON.stringify("1")));
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    const song2Slide14 = slideIds[presIds[1]][13];
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: song2Slide14 });
    await expect.poll(() => cardInView(page, song2Slide14), { message: "Follow lands on the stage slide" }).toBe(true);

    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[4]][0]}"]`);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 02" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`);
    await settle(page);
    expect(await cardInView(page, slideIds[presIds[1]][0]), "slide 1 is in view").toBe(true);
    expect(await cardInView(page, song2Slide14), "a manual open does not jump to the stage slide").toBe(false);
    expect((await scrollTops(page)).slides, "the slide list is at the top").toBe(0);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow: a stage song whose request hangs is requested again on a later stage change", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // The first request for song 2 never gets an answer.
    let first = true;
    await page.route(`**/presentations/${presIds[1]}`, async (route) => {
      if (!first) return route.continue();
      first = false;
    });
    const hung = page.waitForRequest(`**/presentations/${presIds[1]}`);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await hung;
    // Past the age after which an unanswered open is retried (`STALE_OPEN_MS` in stage_follow.rs).
    await page.waitForTimeout(5_500);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][1] });
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`),
      "the next stage change requests song 2 again and opens it",
    ).toBeAttached();
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow turned on and straight off again does not open the stage song", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: false });
  try {
    await openOperator(page);
    const follow = page.locator('[data-role="ableset-follow"]');
    await expect(follow, "Follow is off").toHaveAttribute("data-state", "off");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    const releaseSong2 = await gateRoute(page, `**/presentations/${presIds[1]}`);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await expect(
      page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 02" }).first(),
      "song 2 is on stage",
    ).toHaveClass(/is-stage-active/);
    // Follow ON opens the stage song at once (held) ...
    const requested = page.waitForRequest(`**/presentations/${presIds[1]}`);
    const answered = page.waitForResponse(`**/presentations/${presIds[1]}`);
    await follow.first().click();
    await expect(follow).toHaveAttribute("data-state", "on");
    await requested;
    // ... and OFF again before it arrives.
    await follow.first().click();
    await expect(follow).toHaveAttribute("data-state", "off");
    releaseSong2();
    await landed(answered);
    await settle(page);
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`), "song 3 stays").toBeAttached();
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`)).not.toBeAttached();
    await page.unroute(`**/presentations/${presIds[1]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow: a playlist the operator opens while the stage song's library loads stays open", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // Follow opens song 2 and waits for its library (held); the operator opens the playlist meanwhile.
    const releaseLibrary = await gateRoute(page, "**/libraries/summary");
    const libraryRequested = page.waitForRequest("**/libraries/summary");
    const libraryAnswered = page.waitForResponse("**/libraries/summary");
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await libraryRequested;
    const playlistButton = page.locator(`[data-role="playlist-item"][data-playlist-id="${playlistId}"]`).first();
    await playlistButton.click();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Churn Set");
    releaseLibrary();
    await landed(libraryAnswered);
    await settle(page);
    await expect(page.locator('[data-role="context-title"]'), "the playlist stays open").toHaveText("_E2E Churn Set");
    await expect(playlistButton).toHaveAttribute("data-active", "true");
    await page.unroute("**/libraries/summary");
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow: after it moves from a playlist to the stage song's library, a reload stays in that library", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    const playlistButton = page.locator(`[data-role="playlist-item"][data-playlist-id="${playlistId}"]`).first();
    await playlistButton.click();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Churn Set");
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await expect(page.locator('[data-role="context-title"]'), "Follow opens the song's library").toHaveText("_E2E Stage Churn");
    await expect(playlistButton).toHaveAttribute("data-active", "false");

    await page.reload();
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`)).toBeAttached();
    await expect(page.locator('[data-role="context-title"]'), "the reload keeps the library").toHaveText("_E2E Stage Churn");
    await expect(playlistButton, "not the old playlist").toHaveAttribute("data-active", "false");
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Edit: the refetch after a raced slide duplicate does not replace a song opened meanwhile", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await openOperator(page);
  await openTestLibrary(page);
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 07" }).first().click();
  const song7 = presIds[6];
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[song7][0]}"]`);

  // Two duplicates whose answers cross: the first is held until the second has landed, so the
  // first answer sees a newer edit and the slide list reconciles with one refetch of song 7.
  let releaseFirstDuplicate!: () => void;
  const firstDuplicateHeld = new Promise<void>((resolve) => (releaseFirstDuplicate = resolve));
  let duplicates = 0;
  await page.route(`**/presentations/${song7}/slides/*/duplicate`, async (route) => {
    duplicates += 1;
    if (duplicates === 1) await firstDuplicateHeld;
    await route.continue();
  });
  const duplicate = page.locator(`.operator__slides [data-slide-id="${slideIds[song7][0]}"] [data-action="duplicate"]`).first();
  const secondAnswered = page.waitForResponse(
    (resp) => resp.url().includes(`/presentations/${song7}/slides/`) && resp.url().endsWith("/duplicate"),
  );
  await duplicate.click();
  await expect.poll(() => duplicates).toBe(1);
  await duplicate.click();
  await landed(secondAnswered);
  await expect(page.locator(".operator__slides [data-slide-id]")).toHaveCount(17);

  // The refetch is held; the operator opens song 8 meanwhile.
  const releaseRefetch = await gateRoute(page, `**/presentations/${song7}`);
  const refetchRequested = page.waitForRequest(`**/presentations/${song7}`);
  const refetchAnswered = page.waitForResponse(`**/presentations/${song7}`);
  releaseFirstDuplicate();
  await refetchRequested;
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 08" }).first().click();
  const song8First = page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[7]][0]}"]`);
  await expect(song8First).toBeAttached();
  releaseRefetch();
  await landed(refetchAnswered);
  await settle(page);
  await expect(song8First, "song 8 stays shown").toBeAttached();
  await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[song7][0]}"]`)).not.toBeAttached();
  await page.unroute(`**/presentations/${song7}`);
  await page.unroute(`**/presentations/${song7}/slides/*/duplicate`);
  expect(consoleMessages).toEqual([]);
});

test("Edit: a slide duplicate answered after another song was opened leaves that song alone", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await openOperator(page);
  await openTestLibrary(page);
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 09" }).first().click();
  const song9 = presIds[8];
  const song10 = presIds[9];
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[song9][0]}"]`);

  // The duplicate's answer is held while the operator opens song 10.
  const releaseDuplicate = await gateRoute(page, `**/presentations/${song9}/slides/*/duplicate`);
  const duplicateRequested = page.waitForRequest((req) => req.url().endsWith("/duplicate"));
  const duplicateAnswered = page.waitForResponse((resp) => resp.url().endsWith("/duplicate"));
  await page.locator(`.operator__slides [data-slide-id="${slideIds[song9][0]}"] [data-action="duplicate"]`).first().click();
  await duplicateRequested;
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 10" }).first().click();
  const song10First = page.locator(`.operator__slides [data-slide-id="${slideIds[song10][0]}"]`);
  await expect(song10First).toBeAttached();
  releaseDuplicate();
  await landed(duplicateAnswered);
  await settle(page);
  await expect(song10First, "song 10 stays shown").toBeAttached();
  await expect(page.locator(".operator__slides [data-slide-id]"), "with its own 16 slides").toHaveCount(16);
  await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[song9][0]}"]`)).not.toBeAttached();
  await page.unroute(`**/presentations/${song9}/slides/*/duplicate`);
  expect(consoleMessages).toEqual([]);
});

test("Follow: a slow stage song still opens when a later stage change re-requests it", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // Each request for song 2 is held on its own gate.
    const gates: Array<() => void> = [];
    await page.route(`**/presentations/${presIds[1]}`, async (route) => {
      await new Promise<void>((resolve) => gates.push(resolve));
      await route.continue();
    });
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await expect.poll(() => gates.length, { message: "the first request is held" }).toBe(1);
    // Past `STALE_OPEN_MS`: the next stage change re-requests the song.
    await page.waitForTimeout(5_500);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][1] });
    await expect.poll(() => gates.length, { message: "a second request is sent" }).toBe(2);
    // The FIRST answer arrives: it must open the song (the second is still held).
    const firstAnswered = page.waitForResponse(`**/presentations/${presIds[1]}`);
    gates[0]();
    await landed(firstAnswered);
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`),
      "the first answer opens song 2",
    ).toBeAttached();
    // Let the second request finish before removing the route (a handler still waiting when the
    // route is removed fails with "Route is already handled").
    const secondAnswered = page.waitForResponse(`**/presentations/${presIds[1]}`);
    gates[1]();
    await landed(secondAnswered);
    await page.unroute(`**/presentations/${presIds[1]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Edit: a Follow ON open whose stage song changed before it arrived does not replace the song being edited", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: false });
  try {
    await openOperator(page);
    const follow = page.locator('[data-role="ableset-follow"]');
    await expect(follow, "Follow is off").toHaveAttribute("data-state", "off");
    await openTestLibrary(page);
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await expect(
      page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 02" }).first(),
    ).toHaveClass(/is-stage-active/);
    // Follow ON in Edit mode opens the stage song (held) ...
    const releaseSong2 = await gateRoute(page, `**/presentations/${presIds[1]}`);
    const requested = page.waitForRequest(`**/presentations/${presIds[1]}`);
    const answered = page.waitForResponse(`**/presentations/${presIds[1]}`);
    await follow.first().click();
    await expect(follow).toHaveAttribute("data-state", "on");
    await requested;
    // ... but the stage moves on to song 4 before it arrives.
    await send("POST", "/stage/state", { presentationId: presIds[3], currentSlideId: slideIds[presIds[3]][0] });
    await expect(
      page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 04" }).first(),
    ).toHaveClass(/is-stage-active/);
    releaseSong2();
    await landed(answered);
    await settle(page);
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`),
      "the song being edited stays",
    ).toBeAttached();
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`)).not.toBeAttached();
    await page.unroute(`**/presentations/${presIds[1]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("a song that fails to load keeps the shown song selected", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await openOperator(page);
  await openTestLibrary(page);
  const song3Row = page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first();
  await song3Row.click();
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

  // Song 5's detail fails (a malformed answer, so the browser logs nothing).
  await page.route(`**/presentations/${presIds[4]}`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
  );
  const failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
  await landed(failed);
  await expect(song3Row, "the selection returns to the song still shown").toHaveClass(/is-active/);
  await expect(
    page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first(),
  ).not.toHaveClass(/is-active/);
  await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`)).toBeAttached();
  await page.unroute(`**/presentations/${presIds[4]}`);
  expect(consoleMessages).toEqual([]);
});

test("Follow: a songplayer push to the API layout does not cancel the stage song Follow is loading", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const frames = countStageFrames(page);
  await resetStage();
  const previousLayout = (await (await fetch(new URL("/stage/layout", baseURL))).json()).code as string;
  try {
    await send("POST", "/stage/layout", { code: "api" });
    await send("POST", "/integrations/ableset/follow", { enabled: true });
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    const releaseSong2 = await gateRoute(page, `**/presentations/${presIds[1]}`);
    const requested = page.waitForRequest(`**/presentations/${presIds[1]}`);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await requested;
    // songplayer pushes text to the API layout while song 2 loads (a stage frame with no song).
    const framesBefore = frames.count;
    const push = await fetch(new URL("/api/stage", baseURL), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ currentText: "Amazing grace", currentSong: "Amazing Grace" }),
    });
    expect(push.status).toBe(204);
    await expect.poll(() => frames.count - framesBefore, { message: "the API push reached the page" }).toBeGreaterThanOrEqual(1);
    releaseSong2();
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`),
      "song 2 still opens",
    ).toBeAttached();
    await page.unroute(`**/presentations/${presIds[1]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
    await send("POST", "/stage/layout", { code: previousLayout });
  }
});

test("Follow: a playlist the operator opens while the stage song itself loads stays open", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    const releaseSong2 = await gateRoute(page, `**/presentations/${presIds[1]}`);
    const requested = page.waitForRequest(`**/presentations/${presIds[1]}`);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await requested;
    const playlistButton = page.locator(`[data-role="playlist-item"][data-playlist-id="${playlistId}"]`).first();
    await playlistButton.click();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Churn Set");
    const libraryAnswered = page.waitForResponse("**/libraries/summary");
    releaseSong2();
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`), "song 2 opens").toBeAttached();
    await landed(libraryAnswered);
    await settle(page);
    await expect(page.locator('[data-role="context-title"]'), "the playlist stays open").toHaveText("_E2E Churn Set");
    await expect(playlistButton).toHaveAttribute("data-active", "true");
    await page.unroute(`**/presentations/${presIds[1]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

// Guard (the fix shipped in 08105c2f without a failing test first).
test("Follow: a search result whose answer comes late does not replace the stage song Follow opened", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    const releaseSong6 = await gateRoute(page, `**/presentations/${presIds[5]}`);
    const searchRequested = page.waitForRequest(`**/presentations/${presIds[5]}`);
    const searchAnswered = page.waitForResponse(`**/presentations/${presIds[5]}`);
    await page.locator('[data-role="global-search-query"]').fill("Churn Song 06");
    await page
      .locator('[data-role="search-result-item"][data-kind="presentation"]', { hasText: "Churn Song 06" })
      .first()
      .click();
    await searchRequested;
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    const song2First = page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`);
    await expect(song2First, "Follow opens the stage song").toBeAttached();
    releaseSong6();
    await landed(searchAnswered);
    await settle(page);
    await expect(song2First, "song 2 stays shown").toBeAttached();
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[5]][0]}"]`)).not.toBeAttached();
    await page.unroute(`**/presentations/${presIds[5]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

// Guard (the fix shipped in 08105c2f without a failing test first).
test("Follow: the session song restored after a reload does not replace a stage song Follow opened first", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await expect(page.locator('[data-role="ableset-follow"]'), "Follow is on").toHaveAttribute("data-state", "on");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);
    // The stage is on the session song, so the reload itself opens nothing new.
    await send("POST", "/stage/state", { presentationId: presIds[2], currentSlideId: slideIds[presIds[2]][0] });

    const releaseSong3 = await gateRoute(page, `**/presentations/${presIds[2]}`);
    const restoreRequested = page.waitForRequest(`**/presentations/${presIds[2]}`);
    const restoreAnswered = page.waitForResponse(`**/presentations/${presIds[2]}`);
    await page.reload();
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await restoreRequested;
    await followReady(page);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    const song2First = page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`);
    await expect(song2First, "Follow opens the stage song").toBeAttached();
    releaseSong3();
    await landed(restoreAnswered);
    await settle(page);
    await expect(song2First, "song 2 stays shown").toBeAttached();
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`)).not.toBeAttached();
    await page.unroute(`**/presentations/${presIds[2]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow, API layout: back to Live opens the stage song even after a songplayer push", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const frames = countStageFrames(page);
  await resetStage();
  const previousLayout = (await (await fetch(new URL("/stage/layout", baseURL))).json()).code as string;
  try {
    await send("POST", "/stage/layout", { code: "api" });
    await send("POST", "/integrations/ableset/follow", { enabled: true });
    await openOperator(page);
    await followReady(page);
    await openTestLibrary(page);
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // In Edit mode AbleSet moves the stage to song 2 (on an API layout only the camera-crew frame
    // carries it — the song list highlights it), then songplayer pushes text (a frame with no song).
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await expect(
      page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 02" }).first(),
      "the stage change reached the page",
    ).toHaveClass(/is-stage-active/);
    const before = frames.count;
    const push = await fetch(new URL("/api/stage", baseURL), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ currentText: "Amazing grace", currentSong: "Amazing Grace" }),
    });
    expect(push.status).toBe(204);
    await expect.poll(() => frames.count - before, { message: "the push reached the page" }).toBeGreaterThanOrEqual(1);

    await page.locator('[data-role="mode-toggle"][data-mode="live"]').first().click();
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`),
      "back in Live the stage song opens",
    ).toBeAttached();
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
    await send("POST", "/stage/layout", { code: previousLayout });
  }
});

test("Follow: a song from another library opened right after Follow's previous library switch gets its own library", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const otherLib = await send("POST", "/libraries", { name: "_E2E Follow Other" });
  const other = await send("POST", `/libraries/${otherLib.id}/presentations`, {
    name: "Other Library Song",
    slides: [{ main: "Other slide 1" }, { main: "Other slide 2" }],
  });
  const otherSong = other.presentation.id as string;
  const otherFirst = other.presentation.slides[0].id as string;
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await followReady(page);
    // From the playlist, so every Follow library switch is a visible context change.
    await page.locator(`[data-role="playlist-item"][data-playlist-id="${playlistId}"]`).first().click();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Churn Set");
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // Each library read is held on its own gate; the other song's detail too.
    const libraryGates: Array<() => void> = [];
    await page.route("**/libraries/summary", async (route) => {
      await new Promise<void>((resolve) => libraryGates.push(resolve));
      await route.continue();
    });
    const releaseOther = await gateRoute(page, `**/presentations/${otherSong}`);
    const otherRequested = page.waitForRequest(`**/presentations/${otherSong}`);

    // Song 2 opens and waits for its library; the stage moves on to the other library's song.
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`)).toBeAttached();
    await expect.poll(() => libraryGates.length).toBe(1);
    await send("POST", "/stage/state", { presentationId: otherSong, currentSlideId: otherFirst });
    await otherRequested;
    // Song 2's library step lands (Follow switches to its library) ...
    libraryGates[0]();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Stage Churn");
    // ... then the other song arrives, and its library step must not mistake that switch for an operator pick.
    releaseOther();
    await expect(page.locator(`.operator__slides [data-slide-id="${otherFirst}"]`)).toBeAttached();
    await expect.poll(() => libraryGates.length).toBe(2);
    libraryGates[1]();
    await expect(page.locator('[data-role="context-title"]'), "the other song's library opens").toHaveText("_E2E Follow Other");
    await page.unroute("**/libraries/summary");
    await page.unroute(`**/presentations/${otherSong}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow: an older request failing after the song was re-requested does not lose the open", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await followReady(page);
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    const held: Route[] = [];
    await page.route(`**/presentations/${presIds[1]}`, (route) => {
      held.push(route);
    });
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await expect.poll(() => held.length).toBe(1);
    await page.waitForTimeout(5_500); // past `STALE_OPEN_MS`
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][1] });
    await expect.poll(() => held.length, { message: "the song is requested again" }).toBe(2);
    // The OLD request fails (a malformed answer: no console noise), then the new one answers.
    const oldFailed = page.waitForResponse(`**/presentations/${presIds[1]}`);
    await held[0].fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    await landed(oldFailed);
    await settle(page);
    await held[1].continue();
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`),
      "the new request's answer still opens song 2",
    ).toBeAttached();
    await page.unroute(`**/presentations/${presIds[1]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("a failed open in a playlist keeps the shown occurrence for the next trigger (#496)", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const repeat = await send("POST", "/playlists", { name: "_E2E Repeat Set", showInDashboard: true });
  await send("PUT", `/playlists/${repeat.id}/entries`, {
    entries: [presIds[2], presIds[4], presIds[2]].map((id) => ({ type: "presentation", presentationId: id })),
  });
  await resetStage();
  await openOperator(page);
  await page.locator(`[data-role="playlist-item"][data-playlist-id="${repeat.id}"]`).first().click();
  await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Repeat Set");
  const rows = page.locator('[data-role="presentation-item"]');
  await expect(rows).toHaveCount(3);
  // The second occurrence of song 3 (entry index 2).
  await rows.nth(2).click();
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);
  // Song 5 (entry index 1) fails to load.
  await page.route(`**/presentations/${presIds[4]}`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
  );
  const failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
  await rows.nth(1).click();
  await landed(failed);
  // The selection (and with it the occurrence) is back on the shown entry.
  await expect(rows.nth(2)).toHaveClass(/is-active/);
  // A slide of the shown song goes to the stage: it must carry song 3's occurrence, not song 5's.
  await page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][1]}"]`).click();
  await expect
    .poll(async () => {
      const snap = await (await fetch(new URL("/stage/snapshot", baseURL))).json();
      return { song: snap.presentationId, entry: snap.activeEntryIndex };
    }, { message: "the stage marks the occurrence that is shown" })
    .toEqual({ song: presIds[2], entry: 2 });
  await page.unroute(`**/presentations/${presIds[4]}`);
  expect(consoleMessages).toEqual([]);
});

test("Edit: a song clicked while a Follow ON open loads cancels that open, even if the click fails", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: false });
  try {
    await openOperator(page);
    const follow = page.locator('[data-role="ableset-follow"]');
    await expect(follow, "Follow is off").toHaveAttribute("data-state", "off");
    await openTestLibrary(page);
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await expect(page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 02" }).first()).toHaveClass(/is-stage-active/);

    // Follow ON opens the stage song (held) ...
    const releaseSong2 = await gateRoute(page, `**/presentations/${presIds[1]}`);
    const requested = page.waitForRequest(`**/presentations/${presIds[1]}`);
    const answered = page.waitForResponse(`**/presentations/${presIds[1]}`);
    await follow.first().click();
    await expect(follow).toHaveAttribute("data-state", "on");
    await requested;
    // ... the operator clicks song 5, which fails (the selection returns to song 3) ...
    await page.route(`**/presentations/${presIds[4]}`, (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
    );
    const failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
    await landed(failed);
    await expect(page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first()).toHaveClass(/is-active/);
    // ... so the Follow open is no longer wanted.
    releaseSong2();
    await landed(answered);
    await settle(page);
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`), "song 3 stays").toBeAttached();
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`)).not.toBeAttached();
    await page.unroute(`**/presentations/${presIds[1]}`);
    await page.unroute(`**/presentations/${presIds[4]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("a search result that fails to load keeps the shown song selected", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await openOperator(page);
  await openTestLibrary(page);
  const song3Row = page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first();
  await song3Row.click();
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);
  await page.route(`**/presentations/${presIds[5]}`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
  );
  const failed = page.waitForResponse(`**/presentations/${presIds[5]}`);
  await page.locator('[data-role="global-search-query"]').fill("Churn Song 06");
  await page
    .locator('[data-role="search-result-item"][data-kind="presentation"]', { hasText: "Churn Song 06" })
    .first()
    .click();
  await landed(failed);
  await expect(song3Row, "the selection returns to the song still shown").toHaveClass(/is-active/);
  await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`)).toBeAttached();
  await page.unroute(`**/presentations/${presIds[5]}`);
  expect(consoleMessages).toEqual([]);
});

test("Follow: a session song that fails to load after a reload is opened by the next stage change", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await followReady(page);
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);
    await send("POST", "/stage/state", { presentationId: presIds[2], currentSlideId: slideIds[presIds[2]][0] });

    // After the reload the session song's first request fails; later ones answer.
    let failedOnce = false;
    await page.route(`**/presentations/${presIds[2]}`, async (route) => {
      if (failedOnce) return route.continue();
      failedOnce = true;
      return route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    });
    const restoreFailed = page.waitForResponse(`**/presentations/${presIds[2]}`);
    await page.reload();
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await landed(restoreFailed);
    await followReady(page);
    await send("POST", "/stage/state", { presentationId: presIds[2], currentSlideId: slideIds[presIds[2]][1] });
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`),
      "Follow opens the stage song the session could not load",
    ).toBeAttached();
    await page.unroute(`**/presentations/${presIds[2]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

// Guard (the reorder check shipped in f5da384b without a failing test first).
test("Edit: a slide reorder answered after another song was opened leaves that song alone", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await openOperator(page);
  await openTestLibrary(page);
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 11" }).first().click();
  const song11 = presIds[10];
  const song12 = presIds[11];
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[song11][0]}"]`);

  const releaseReorder = await gateRoute(page, `**/presentations/${song11}/slides/reorder`);
  const reorderRequested = page.waitForRequest((req) => req.url().endsWith("/slides/reorder"));
  const reorderAnswered = page.waitForResponse((resp) => resp.url().endsWith("/slides/reorder"));
  await dragSlide(page, slideIds[song11][0], slideIds[song11][2]);
  await reorderRequested;
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 12" }).first().click();
  const song12First = page.locator(`.operator__slides [data-slide-id="${slideIds[song12][0]}"]`);
  await expect(song12First).toBeAttached();
  releaseReorder();
  await landed(reorderAnswered);
  await settle(page);
  await expect(song12First, "song 12 stays shown").toBeAttached();
  await expect(page.locator(".operator__slides [data-slide-id]"), "with its own 16 slides").toHaveCount(16);
  await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[song11][0]}"]`)).not.toBeAttached();
  await page.unroute(`**/presentations/${song11}/slides/reorder`);
  expect(consoleMessages).toEqual([]);
});

test("Follow: a stage song that starts loading while a clicked song fails still opens", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await followReady(page);
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // The operator clicks song 5 (held, then failing); the stage moves to song 2 meanwhile (held).
    const song5: Route[] = [];
    await page.route(`**/presentations/${presIds[4]}`, (route) => {
      song5.push(route);
    });
    const releaseSong2 = await gateRoute(page, `**/presentations/${presIds[1]}`);
    const song2Requested = page.waitForRequest(`**/presentations/${presIds[1]}`);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
    await expect.poll(() => song5.length).toBe(1);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await song2Requested;
    // Song 5 fails: the selection returns to song 3 ...
    const song5Failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
    await song5[0].fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    await landed(song5Failed);
    await expect(page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first()).toHaveClass(/is-active/);
    // ... and the Follow open that started during that load must still land.
    releaseSong2();
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`),
      "song 2 opens",
    ).toBeAttached();
    await page.unroute(`**/presentations/${presIds[4]}`);
    await page.unroute(`**/presentations/${presIds[1]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("a failed click keeps the occurrence of the song actually shown, not of a click still loading (#496)", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const set = await send("POST", "/playlists", { name: "_E2E Three Set", showInDashboard: true });
  await send("PUT", `/playlists/${set.id}/entries`, {
    entries: [presIds[2], presIds[4], presIds[5]].map((id) => ({ type: "presentation", presentationId: id })),
  });
  await resetStage();
  await openOperator(page);
  await page.locator(`[data-role="playlist-item"][data-playlist-id="${set.id}"]`).first().click();
  await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Three Set");
  const rows = page.locator('[data-role="presentation-item"]');
  await expect(rows).toHaveCount(3);
  await rows.nth(0).click(); // song 3, occurrence 0
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

  // Song 5 (occurrence 1) is clicked and held; song 6 (occurrence 2) is clicked and fails.
  const releaseSong5 = await gateRoute(page, `**/presentations/${presIds[4]}`);
  const song5Requested = page.waitForRequest(`**/presentations/${presIds[4]}`);
  await rows.nth(1).click();
  await song5Requested;
  await page.route(`**/presentations/${presIds[5]}`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
  );
  const song6Failed = page.waitForResponse(`**/presentations/${presIds[5]}`);
  await rows.nth(2).click();
  await landed(song6Failed);
  await expect(rows.nth(0), "back on the song still shown").toHaveClass(/is-active/);
  const song5Answered = page.waitForResponse(`**/presentations/${presIds[4]}`);
  releaseSong5(); // too late: song 5 is no longer selected
  await landed(song5Answered);
  await settle(page);
  await expect(rows.nth(0), "song 5's late answer changes nothing").toHaveClass(/is-active/);

  await page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][1]}"]`).click();
  await expect
    .poll(async () => {
      const snap = await (await fetch(new URL("/stage/snapshot", baseURL))).json();
      return { song: snap.presentationId, entry: snap.activeEntryIndex };
    }, { message: "the stage marks song 3's own occurrence" })
    .toEqual({ song: presIds[2], entry: 0 });
  await page.unroute(`**/presentations/${presIds[4]}`);
  await page.unroute(`**/presentations/${presIds[5]}`);
  expect(consoleMessages).toEqual([]);
});

test("Follow: catching up with a stage that has no song keeps the occurrence the operator picked (#496)", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const set = await send("POST", "/playlists", { name: "_E2E Repeat Set 2", showInDashboard: true });
  await send("PUT", `/playlists/${set.id}/entries`, {
    entries: [presIds[2], presIds[4], presIds[2]].map((id) => ({ type: "presentation", presentationId: id })),
  });
  // A song that goes on stage and is then deleted: clearing the stage then leaves no song at all.
  const gone = await send("POST", `/libraries/${libId}/presentations`, {
    name: "Churn Gone Song",
    slides: [{ main: "Gone 1" }, { main: "Gone 2" }],
  });
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await send("POST", "/stage/state", {
      presentationId: gone.presentation.id,
      currentSlideId: gone.presentation.slides[0].id,
    });
    await send("DELETE", `/presentations/${gone.presentation.id}`, {});
    await send("POST", "/stage/clear", {});
    const snap = await (await fetch(new URL("/stage/snapshot", baseURL))).json();
    expect(snap.presentationId ?? null, "the stage has no song").toBeNull();

    await openOperator(page);
    await followReady(page);
    await page.locator(`[data-role="playlist-item"][data-playlist-id="${set.id}"]`).first().click();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Repeat Set 2");
    const rows = page.locator('[data-role="presentation-item"]');
    await expect(rows).toHaveCount(3);
    // In Edit mode the operator picks the SECOND occurrence of song 3, then goes back to Live.
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
    await rows.nth(2).click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);
    await page.locator('[data-role="mode-toggle"][data-mode="live"]').first().click();
    await settle(page);

    await page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][1]}"]`).click();
    await expect
      .poll(async () => {
        const s2 = await (await fetch(new URL("/stage/snapshot", baseURL))).json();
        return { song: s2.presentationId, entry: s2.activeEntryIndex };
      }, { message: "the stage marks the occurrence the operator picked" })
      .toEqual({ song: presIds[2], entry: 2 });
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Follow: the occurrence the stage mirrored while a clicked song loaded is the one a later failed click restores (#496)", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const set = await send("POST", "/playlists", { name: "_E2E Repeat Set 3", showInDashboard: true });
  await send("PUT", `/playlists/${set.id}/entries`, {
    entries: [presIds[2], presIds[4], presIds[2]].map((id) => ({ type: "presentation", presentationId: id })),
  });
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await followReady(page);
    await page.locator(`[data-role="playlist-item"][data-playlist-id="${set.id}"]`).first().click();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Repeat Set 3");
    const rows = page.locator('[data-role="presentation-item"]');
    await expect(rows).toHaveCount(3);
    await rows.nth(1).click(); // song 5
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[4]][0]}"]`);

    // The operator clicks the SECOND song 3 (occurrence 2), held; the stage plays the FIRST one meanwhile.
    const releaseSong3 = await gateRoute(page, `**/presentations/${presIds[2]}`);
    const song3Requested = page.waitForRequest(`**/presentations/${presIds[2]}`);
    const song3Answered = page.waitForResponse(`**/presentations/${presIds[2]}`);
    await rows.nth(2).click();
    await song3Requested;
    await send("POST", "/stage/state", {
      presentationId: presIds[2],
      currentSlideId: slideIds[presIds[2]][0],
      playlistId: set.id,
      entryIndex: 0,
    });
    await expect(rows.nth(0), "the stage is on song 3").toHaveClass(/is-stage-active/);
    releaseSong3();
    await landed(song3Answered);
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // A click on song 5 fails: back to song 3 with the occurrence it is shown as (the stage's 0).
    await page.route(`**/presentations/${presIds[4]}`, (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
    );
    const failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
    await rows.nth(1).click();
    await landed(failed);
    await expect(rows.nth(2)).toHaveClass(/is-active/);
    await page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][1]}"]`).click();
    await expect
      .poll(async () => {
        const snap = await (await fetch(new URL("/stage/snapshot", baseURL))).json();
        return { song: snap.presentationId, entry: snap.activeEntryIndex, slide: snap.currentSlideId };
      }, { message: "the trigger carries the mirrored occurrence" })
      .toEqual({ song: presIds[2], entry: 0, slide: slideIds[presIds[2]][1] });
    await page.unroute(`**/presentations/${presIds[2]}`);
    await page.unroute(`**/presentations/${presIds[4]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("Edit: a Follow ON open of the song that ends up shown never replaces it with older content", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: false });
  try {
    await openOperator(page);
    const follow = page.locator('[data-role="ableset-follow"]');
    await expect(follow, "Follow is off").toHaveAttribute("data-state", "off");
    await openTestLibrary(page);
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
    const song13 = presIds[12]; // edited here, used by no other test
    const firstSlide = slideIds[song13][0];
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 13" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${firstSlide}"]`);
    await send("POST", "/stage/state", { presentationId: song13, currentSlideId: firstSlide });
    await expect(
      page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 13" }).first(),
      "the stage frame reached the page",
    ).toHaveClass(/is-stage-active/);

    // The operator clicks song 5 (held, then failing) and turns Follow ON: Follow opens the stage
    // song 13 from "song 5 selected"; its answer is read from the server NOW and delivered later.
    const song5: Route[] = [];
    await page.route(`**/presentations/${presIds[4]}`, (route) => {
      song5.push(route);
    });
    let deliverOld!: () => void;
    const oldAnswerHeld = new Promise<void>((resolve) => (deliverOld = resolve));
    let oldAnswerRead = false;
    await page.route(`**/presentations/${song13}`, async (route) => {
      const response = await route.fetch();
      oldAnswerRead = true;
      await oldAnswerHeld;
      await route.fulfill({ response });
    });
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
    await expect.poll(() => song5.length).toBe(1);
    await follow.first().click();
    await expect(follow).toHaveAttribute("data-state", "on");
    await expect.poll(() => oldAnswerRead, { message: "the Follow open read song 13 before the edit" }).toBe(true);
    // Song 5 fails: the selection returns to song 13, which is shown.
    const song5Failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
    await song5[0].fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    await landed(song5Failed);
    await expect(page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 13" }).first()).toHaveClass(/is-active/);

    // The operator edits song 13's first slide; the save lands.
    const area = page.locator(`.operator__slides [data-slide-id="${firstSlide}"] textarea`).first();
    const saved = page.waitForResponse(
      (r) => r.url().includes(`/presentations/${song13}/slides/`) && r.request().method() !== "GET",
    );
    await area.click();
    await page.keyboard.press("Control+End");
    await page.keyboard.type(" edited");
    await page.locator('[data-role="context-title"]').click(); // blur → save
    await landed(saved);

    // The old (pre-edit) answer of the Follow open arrives: it must not replace the edited song.
    const oldDelivered = page.waitForResponse(`**/presentations/${song13}`);
    deliverOld();
    await landed(oldDelivered);
    await settle(page);
    await expect(area, "the edit stays").toHaveValue(/ edited$/);
    await page.unroute(`**/presentations/${song13}`);
    await page.unroute(`**/presentations/${presIds[4]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

// Guard (the page-load seed of Follow's stage shipped in 97d7bbbe without a failing test first).
test("Follow, API layout: right after the page loads, Follow ON opens the stage song", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  const previousLayout = (await (await fetch(new URL("/stage/layout", baseURL))).json()).code as string;
  try {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
    await send("POST", "/stage/layout", { code: "api" });
    // On an API layout the page's own snapshot carries no song; the stage is on song 2.
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await openOperator(page);
    const follow = page.locator('[data-role="ableset-follow"]');
    await expect(follow, "Follow is off").toHaveAttribute("data-state", "off");
    await openTestLibrary(page);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);
    // No stage frame since the page loaded: Follow ON must still know the stage song.
    await follow.first().click();
    await expect(follow).toHaveAttribute("data-state", "on");
    await expect(
      page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`),
      "Follow ON opens the stage song",
    ).toBeAttached();
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
    await send("POST", "/stage/layout", { code: previousLayout });
  }
});

test("a double-click whose first load fails still shows the song", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await openOperator(page);
  await openTestLibrary(page);
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

  // Song 5's first request fails — but only once the second (the double-click's) has been sent,
  // which is then answered.
  let requests = 0;
  let secondSent!: () => void;
  const secondSeen = new Promise<void>((resolve) => (secondSent = resolve));
  await page.route(`**/presentations/${presIds[4]}`, async (route) => {
    requests += 1;
    if (requests === 1) {
      await secondSeen;
      return route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    }
    secondSent();
    return route.continue();
  });
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().dblclick();
  await expect.poll(() => requests, { message: "both clicks load the song" }).toBe(2);
  await expect(
    page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[4]][0]}"]`),
    "the second load shows song 5",
  ).toBeAttached();
  await expect(page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first()).toHaveClass(/is-active/);
  await page.unroute(`**/presentations/${presIds[4]}`);
  expect(consoleMessages).toEqual([]);
});

test("a double-click whose second load fails fast while the first is slow still shows the song", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await openOperator(page);
  await openTestLibrary(page);
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

  // Song 5's first request is held; the second (the double-click's) fails at once.
  let releaseFirst!: () => void;
  const firstHeld = new Promise<void>((resolve) => (releaseFirst = resolve));
  let requests = 0;
  await page.route(`**/presentations/${presIds[4]}`, async (route) => {
    requests += 1;
    if (requests === 1) {
      await firstHeld;
      return route.continue();
    }
    return route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
  });
  const secondFailed = page.waitForResponse(
    (resp) => resp.url().endsWith(`/presentations/${presIds[4]}`) && resp.status() === 200,
  );
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().dblclick();
  await expect.poll(() => requests, { message: "both clicks load the song" }).toBe(2);
  await landed(secondFailed);
  await settle(page);
  const firstAnswered = page.waitForResponse(`**/presentations/${presIds[4]}`);
  releaseFirst();
  await landed(firstAnswered);
  await expect(
    page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[4]][0]}"]`),
    "the first load shows song 5",
  ).toBeAttached();
  await expect(page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first()).toHaveClass(/is-active/);
  await page.unroute(`**/presentations/${presIds[4]}`);
  expect(consoleMessages).toEqual([]);
});

test("Follow: opening the shown stage song from a playlist that lacks it still switches to its library", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const set = await send("POST", "/playlists", { name: "_E2E Without Song 2", showInDashboard: true });
  await send("PUT", `/playlists/${set.id}/entries`, {
    entries: [presIds[4], presIds[5]].map((id) => ({ type: "presentation", presentationId: id })),
  });
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await followReady(page);
    // Follow opens the stage song 2 from its library.
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`)).toBeAttached();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Stage Churn");
    // The operator opens a playlist without song 2 (song 2 stays shown) and clicks song 5 there (held).
    await page.locator(`[data-role="playlist-item"][data-playlist-id="${set.id}"]`).first().click();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Without Song 2");
    const releaseSong5 = await gateRoute(page, `**/presentations/${presIds[4]}`);
    const song5Requested = page.waitForRequest(`**/presentations/${presIds[4]}`);
    const song5Answered = page.waitForResponse(`**/presentations/${presIds[4]}`);
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
    await song5Requested;
    // AbleSet moves on within song 2: Follow brings the operator back to it — and to its library.
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][1] });
    await expect(page.locator('[data-role="context-title"]'), "Follow switches to song 2's library").toHaveText("_E2E Stage Churn");
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`)).toBeAttached();
    releaseSong5();
    await landed(song5Answered);
    await page.unroute(`**/presentations/${presIds[4]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("a failed re-click of the shown song in a playlist keeps the occurrence clicked (#496)", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const set = await send("POST", "/playlists", { name: "_E2E Repeat Set 4", showInDashboard: true });
  await send("PUT", `/playlists/${set.id}/entries`, {
    entries: [presIds[2], presIds[4], presIds[2]].map((id) => ({ type: "presentation", presentationId: id })),
  });
  await resetStage();
  await openOperator(page);
  await page.locator(`[data-role="playlist-item"][data-playlist-id="${set.id}"]`).first().click();
  await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Repeat Set 4");
  const rows = page.locator('[data-role="presentation-item"]');
  await expect(rows).toHaveCount(3);
  await rows.nth(0).click(); // song 3, occurrence 0
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);
  // The second song 3 (occurrence 2) is clicked; reloading the (already shown) song fails.
  await page.route(`**/presentations/${presIds[2]}`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
  );
  const failed = page.waitForResponse(`**/presentations/${presIds[2]}`);
  await rows.nth(2).click();
  await landed(failed);
  await settle(page);
  await page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][1]}"]`).click();
  await expect
    .poll(async () => {
      const snap = await (await fetch(new URL("/stage/snapshot", baseURL))).json();
      return { song: snap.presentationId, entry: snap.activeEntryIndex, slide: snap.currentSlideId };
    }, { message: "the trigger carries the clicked occurrence" })
    .toEqual({ song: presIds[2], entry: 2, slide: slideIds[presIds[2]][1] });
  await page.unroute(`**/presentations/${presIds[2]}`);
  expect(consoleMessages).toEqual([]);
});

// Guard (the re-point without a drop shipped in 90a42d34 without a failing test first).
test("Follow: a stage occurrence that arrives for the shown song while a clicked song fails is kept (#496)", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const set = await send("POST", "/playlists", { name: "_E2E Repeat Set 5", showInDashboard: true });
  await send("PUT", `/playlists/${set.id}/entries`, {
    entries: [presIds[2], presIds[4], presIds[2]].map((id) => ({ type: "presentation", presentationId: id })),
  });
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await followReady(page);
    await page.locator(`[data-role="playlist-item"][data-playlist-id="${set.id}"]`).first().click();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Repeat Set 5");
    const rows = page.locator('[data-role="presentation-item"]');
    await expect(rows).toHaveCount(3);
    await rows.nth(0).click(); // song 3, occurrence 0
    await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

    // Song 5 is clicked (held, then failing); the stage plays song 3's SECOND occurrence meanwhile (held).
    const song5: Route[] = [];
    await page.route(`**/presentations/${presIds[4]}`, (route) => {
      song5.push(route);
    });
    await rows.nth(1).click();
    await expect.poll(() => song5.length).toBe(1);
    const releaseSong3 = await gateRoute(page, `**/presentations/${presIds[2]}`);
    const song3Requested = page.waitForRequest(`**/presentations/${presIds[2]}`);
    const song3Answered = page.waitForResponse(`**/presentations/${presIds[2]}`);
    await send("POST", "/stage/state", {
      presentationId: presIds[2],
      currentSlideId: slideIds[presIds[2]][0],
      playlistId: set.id,
      entryIndex: 2,
    });
    await song3Requested;
    const song5Failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
    await song5[0].fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    await landed(song5Failed);
    releaseSong3();
    await landed(song3Answered);
    await settle(page);
    await page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][1]}"]`).click();
    await expect
      .poll(async () => {
        const snap = await (await fetch(new URL("/stage/snapshot", baseURL))).json();
        return { song: snap.presentationId, entry: snap.activeEntryIndex, slide: snap.currentSlideId };
      }, { message: "the trigger carries the stage's occurrence" })
      .toEqual({ song: presIds[2], entry: 2, slide: slideIds[presIds[2]][1] });
    await page.unroute(`**/presentations/${presIds[2]}`);
    await page.unroute(`**/presentations/${presIds[4]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});

test("a slow session song that fails after the operator opened another song leaves that song selected", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await openOperator(page);
  await openTestLibrary(page);
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

  // After the reload the session song 3 loads slowly and then fails; a click on it fails at once.
  const song3: Route[] = [];
  await page.route(`**/presentations/${presIds[2]}`, (route) => {
    song3.push(route);
    if (song3.length > 1) return route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
  });
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect.poll(() => song3.length, { message: "the session restore asks for song 3" }).toBe(1);
  await openTestLibrary(page);
  // The operator opens song 5 (shown), then clicks song 3 (fails at once).
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[4]][0]}"]`);
  const clickFailed = page.waitForResponse(`**/presentations/${presIds[2]}`);
  await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
  await landed(clickFailed);
  // The slow session restore fails last.
  const restoreFailed = page.waitForResponse(`**/presentations/${presIds[2]}`);
  await song3[0].fulfill({ status: 200, contentType: "application/json", body: "not-json" });
  await landed(restoreFailed);
  await settle(page);
  await expect(
    page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first(),
    "the shown song 5 is selected",
  ).toHaveClass(/is-active/);
  await page.unroute(`**/presentations/${presIds[2]}`);
  expect(consoleMessages).toEqual([]);
});

test("a re-click that fails while an earlier load of the same song hangs puts the selection back", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await openOperator(page);
  await openTestLibrary(page);
  const song3Row = page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first();
  await song3Row.click();
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);

  // Song 5's first request never answers; a later click of it fails at once.
  let requests = 0;
  await page.route(`**/presentations/${presIds[4]}`, async (route) => {
    requests += 1;
    if (requests > 1) return route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
  });
  const song5Row = page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first();
  await song5Row.click();
  await expect.poll(() => requests).toBe(1);
  await page.waitForTimeout(5_500); // the hung load no longer counts (`STALE_LOAD_MS` in song_load.rs)
  const failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
  await song5Row.click();
  await landed(failed);
  await expect(song3Row, "back on the song still shown").toHaveClass(/is-active/);
  expect(consoleMessages).toEqual([]);
});

// Guard (recording the re-click's occurrence shipped in 8decdedf without a failing test first).
test("a failed re-click's occurrence survives a later failed click (#496)", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const set = await send("POST", "/playlists", { name: "_E2E Repeat Set 6", showInDashboard: true });
  await send("PUT", `/playlists/${set.id}/entries`, {
    entries: [presIds[2], presIds[4], presIds[2]].map((id) => ({ type: "presentation", presentationId: id })),
  });
  await resetStage();
  await openOperator(page);
  await page.locator(`[data-role="playlist-item"][data-playlist-id="${set.id}"]`).first().click();
  await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Repeat Set 6");
  const rows = page.locator('[data-role="presentation-item"]');
  await expect(rows).toHaveCount(3);
  await rows.nth(0).click(); // song 3, occurrence 0
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);
  // Re-clicking song 3's second occurrence fails, then clicking song 5 fails too.
  await page.route(`**/presentations/${presIds[2]}`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
  );
  await page.route(`**/presentations/${presIds[4]}`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
  );
  const reclickFailed = page.waitForResponse(`**/presentations/${presIds[2]}`);
  await rows.nth(2).click();
  await landed(reclickFailed);
  await settle(page);
  const song5Failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
  await rows.nth(1).click();
  await landed(song5Failed);
  await expect(rows.nth(2)).toHaveClass(/is-active/);
  await page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[2]][1]}"]`).click();
  await expect
    .poll(async () => {
      const snap = await (await fetch(new URL("/stage/snapshot", baseURL))).json();
      return { song: snap.presentationId, entry: snap.activeEntryIndex, slide: snap.currentSlideId };
    }, { message: "the trigger carries the re-clicked occurrence" })
    .toEqual({ song: presIds[2], entry: 2, slide: slideIds[presIds[2]][1] });
  await page.unroute(`**/presentations/${presIds[2]}`);
  await page.unroute(`**/presentations/${presIds[4]}`);
  expect(consoleMessages).toEqual([]);
});

test("a click failing soon after an earlier load of the same song hung still puts the selection back", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  await resetStage();
  await openOperator(page);
  await openTestLibrary(page);
  const song3Row = page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first();
  await song3Row.click();
  await page.waitForSelector(`.operator__slides [data-slide-id="${slideIds[presIds[2]][0]}"]`);
  // Song 5's first request never answers; the second click, right after, fails at once.
  let requests = 0;
  await page.route(`**/presentations/${presIds[4]}`, async (route) => {
    requests += 1;
    if (requests > 1) return route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
  });
  const song5Row = page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first();
  await song5Row.click();
  await expect.poll(() => requests).toBe(1);
  const failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
  await song5Row.click();
  await landed(failed);
  // Once the hung load stops counting (`STALE_LOAD_MS`), the failed click gives up after all.
  await expect(song3Row, "back on the song still shown").toHaveClass(/is-active/, { timeout: 10_000 });
  expect(consoleMessages).toEqual([]);
});

test("Follow: a stage frame on the shown song keeps a pending open of it, so its library step still runs", async ({ page }) => {
  const consoleMessages: string[] = [];
  attachConsoleErrorCollector(page, consoleMessages);
  const set = await send("POST", "/playlists", { name: "_E2E Without Song 2b", showInDashboard: true });
  await send("PUT", `/playlists/${set.id}/entries`, {
    entries: [presIds[4], presIds[5]].map((id) => ({ type: "presentation", presentationId: id })),
  });
  await resetStage();
  await send("POST", "/integrations/ableset/follow", { enabled: true });
  try {
    await openOperator(page);
    await followReady(page);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][0]}"]`)).toBeAttached();
    await page.locator(`[data-role="playlist-item"][data-playlist-id="${set.id}"]`).first().click();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("_E2E Without Song 2b");
    // Song 5 is clicked (held, then failing); a stage frame on song 2 starts a Follow open (held).
    const song5: Route[] = [];
    await page.route(`**/presentations/${presIds[4]}`, (route) => {
      song5.push(route);
    });
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
    await expect.poll(() => song5.length).toBe(1);
    const releaseSong2 = await gateRoute(page, `**/presentations/${presIds[1]}`);
    const song2Requested = page.waitForRequest(`**/presentations/${presIds[1]}`);
    const song2Answered = page.waitForResponse(`**/presentations/${presIds[1]}`);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][1] });
    await song2Requested;
    // Song 5 fails (back to song 2) ... then AbleSet moves on within song 2 before the open's answer.
    const song5Failed = page.waitForResponse(`**/presentations/${presIds[4]}`);
    await song5[0].fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    await landed(song5Failed);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][2] });
    await expect(page.locator(`.operator__slides [data-slide-id="${slideIds[presIds[1]][2]}"]`)).toHaveClass(/is-active/);
    releaseSong2();
    await landed(song2Answered);
    await expect(page.locator('[data-role="context-title"]'), "Follow switches to song 2's library").toHaveText("_E2E Stage Churn");
    await page.unroute(`**/presentations/${presIds[1]}`);
    await page.unroute(`**/presentations/${presIds[4]}`);
    expect(consoleMessages).toEqual([]);
  } finally {
    await send("POST", "/integrations/ableset/follow", { enabled: false });
  }
});
