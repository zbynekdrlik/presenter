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

/** AbleSet-like churn: `count` stage slide changes of `pres` from slide `from`, 0.7 s apart. */
async function churnStage(pres: string, count: number, from = 0) {
  for (let i = from; i < from + count; i++) {
    await send("POST", "/stage/state", { presentationId: pres, currentSlideId: slideIds[pres][i % 16] });
    await new Promise((r) => setTimeout(r, 700));
  }
}

/**
 * Counts the `stage` frames the operator page's own live socket receives (not the preview iframe's),
 * one per stage change: the always-on camera-crew snapshot each change also sends is skipped.
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

/** Holds every matching GET for `ms` before letting it through (a slow network). */
async function delayRoute(page: Page, url: string, ms: number) {
  await page.route(url, async (route) => {
    await new Promise((r) => setTimeout(r, ms));
    await route.continue();
  });
}

/** Holds every matching GET until the returned `release` is called. */
async function gateRoute(page: Page, url: string): Promise<() => void> {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  await page.route(url, async (route) => {
    await gate;
    await route.continue();
  });
  return release;
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
    await delayRoute(page, `**/presentations/${presIds[1]}`, 2_000);
    const songRequested = page.waitForRequest(`**/presentations/${presIds[1]}`);
    const songAnswered = page.waitForResponse(`**/presentations/${presIds[1]}`);
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    await songRequested; // Follow is loading song 2 ...
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
    await songAnswered; // ... and it lands while in Edit mode
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
    await delayRoute(page, "**/libraries/summary", 3_000);
    const libraryRequested = page.waitForRequest("**/libraries/summary");
    const libraryAnswered = page.waitForResponse("**/libraries/summary");
    await send("POST", "/stage/state", { presentationId: presIds[1], currentSlideId: slideIds[presIds[1]][0] });
    // Click while Follow waits for that library.
    await libraryRequested;
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 05" }).first().click();
    await libraryAnswered; // the slow library lands after the click
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
    await failedAnswer;
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
    await songAnswered;
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
    await clickAnswered;
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
    await answered;
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
    await libraryAnswered;
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
