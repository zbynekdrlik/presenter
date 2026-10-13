/**
 * #853 (prod SNV, 2026-10-13): while the stage changes slides every few seconds (AbleSet driving the
 * lyrics during a rehearsal), an operator working in the operator page must keep their place — no
 * reload, no scroll jump in the song list or the playlist, and, in Edit mode, no yank of the slide
 * list away from the slide being edited, even when that song is the one on stage.
 */
import { test, expect, type Page } from "@playwright/test";
import {
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

async function post(path: string, body: unknown): Promise<any> {
  const resp = await fetch(new URL(path, baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await resp.text();
  return text ? JSON.parse(text) : null;
}

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);

  const lib = await post("/libraries", { name: "_E2E Stage Churn" });
  libId = lib.id;
  for (let p = 0; p < 24; p++) {
    const slides = Array.from({ length: 16 }, (_, i) => ({ main: `Song ${p + 1} slide ${i + 1}\nSecond line` }));
    const data = await post(`/libraries/${libId}/presentations`, { name: `Churn Song ${String(p + 1).padStart(2, "0")}`, slides });
    presIds.push(data.presentation.id);
    slideIds[data.presentation.id] = data.presentation.slides.map((s: { id: string }) => s.id);
  }
  const pl = await post("/playlists", { name: "_E2E Churn Set", showInDashboard: true });
  playlistId = pl.id;
  const resp = await fetch(new URL(`/playlists/${playlistId}/entries`, baseURL).toString(), {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ entries: presIds.map((id) => ({ type: "presentation", presentationId: id })) }),
  });
  expect(resp.ok, `playlist entries ${resp.status}`).toBeTruthy();
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

async function churnStage(livePres: string, count: number) {
  for (let i = 0; i < count; i++) {
    await post("/stage/state", { presentationId: livePres, currentSlideId: slideIds[livePres][i % 16] });
    await new Promise((r) => setTimeout(r, 700));
  }
}

async function scrollTops(page: Page) {
  return page.evaluate(() => ({
    list: document.querySelector(".operator__presentation-list")?.scrollTop ?? -1,
    slides: document.querySelector(".operator__slides")?.scrollTop ?? -1,
    marker: (window as any).__churnMarker === 1,
  }));
}

for (const source of ["library", "playlist"] as const) {
  test(`stage churn keeps the operator's place (${source})`, async ({ page }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    });
    await page.goto(new URL("/ui/operator", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    if (source === "library") {
      await page.locator('[data-role="library-more"]').click();
      await page.locator(`[data-role="library-row"][data-library-id="${libId}"] .operator__list-button`).click();
    } else {
      await page.locator(`[data-role="playlist-item"][data-playlist-id="${playlistId}"]`).first().click();
    }
    await page.waitForSelector('[data-role="presentation-item"]');
    // The operator opens song 20 (not the live one) and scrolls both lists.
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 20" }).first().click();
    await page.waitForSelector(".operator__slides [data-slide-id]");
    await page.evaluate(() => {
      (window as any).__churnMarker = 1;
      const list = document.querySelector(".operator__presentation-list") as HTMLElement;
      const slides = document.querySelector(".operator__slides") as HTMLElement;
      list.scrollTop = 300;
      slides.scrollTop = 400;
    });
    const before = await scrollTops(page);

    // AbleSet-like churn on song 2 (live), not the open one.
    await churnStage(presIds[1], 8);
    await page.waitForTimeout(500);
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
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    });
    await page.goto(new URL("/ui/operator", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await page.locator('[data-role="library-more"]').click();
    await page.locator(`[data-role="library-row"][data-library-id="${libId}"] .operator__list-button`).click();
    await page.waitForSelector('[data-role="presentation-item"]');
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
    // The operator edits song 3; AbleSet drives song 3 ("live") or song 2 ("other").
    const edited = presIds[2];
    const live = which === "live" ? presIds[2] : presIds[1];
    await page.locator('[data-role="presentation-item"]', { hasText: "Churn Song 03" }).first().click();
    await page.waitForSelector(".operator__slides [data-slide-id] textarea");
    const target = slideIds[edited][13];
    const area = page.locator(`.operator__slides [data-slide-id="${target}"] textarea`).first();
    await area.scrollIntoViewIfNeeded();
    await area.click();
    await page.keyboard.type(" typing");
    await page.evaluate(() => { (window as any).__churnMarker = 1; });
    const before = await scrollTops(page);
    await churnStage(live, 6);
    await page.waitForTimeout(500);
    const after = await scrollTops(page);
    const editor = await page.evaluate((sid) => {
      const ta = document.querySelector(`.operator__slides [data-slide-id="${sid}"] textarea`) as HTMLTextAreaElement | null;
      return { exists: !!ta, focused: !!ta && document.activeElement === ta, value: ta ? ta.value.slice(-12) : "" };
    }, target);
    console.log(`[edit ${which}] before ${JSON.stringify(before)} after ${JSON.stringify(after)} editor ${JSON.stringify(editor)}`);
    expect(after.marker, "the page must not reload").toBe(true);
    expect(editor.focused, "the slide being edited keeps focus").toBe(true);
    expect(after.slides, "the slide list must not move away from the edited slide").toBe(before.slides);
    expect(consoleMessages).toEqual([]);
  });
}
