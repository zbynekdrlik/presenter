/**
 * #850 B1d — the operator's live socket (spec §6.7): what the hub published
 * while the socket was down is re-read on reconnect (the hub never replays),
 * and a socket that goes silent (a locked iPhone keeps a dead one "open") is
 * replaced within seconds. The lost frames are reproduced deterministically
 * with `routeWebSocket` (`.claude/rules/stage-live-sync.md` technique).
 */
import { test, expect, type Page, type WebSocketRoute } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test.beforeEach(async ({ request }) => {
  await request.put(new URL("/bible/preferences", baseURL).toString(), {
    data: { mainTranslation: "slk-seb", characterLimit: 320 },
  });
  await request.post(new URL("/bible/clear", baseURL).toString(), { data: {} });
});

type Routed = { routes: WebSocketRoute[]; deaf: Set<WebSocketRoute>; frames: number };

/** Relay /live/ws, except for the routes marked deaf (they hear nothing more). */
async function routeLive(page: Page): Promise<Routed> {
  const routed: Routed = { routes: [], deaf: new Set(), frames: 0 };
  await page.routeWebSocket(/\/live\/ws/, (ws: WebSocketRoute) => {
    routed.routes.push(ws);
    const server = ws.connectToServer();
    server.onMessage((message) => {
      if (!routed.deaf.has(ws)) {
        routed.frames += 1;
        ws.send(message);
      }
    });
  });
  return routed;
}

async function openOperatorBible(page: Page): Promise<void> {
  await page.goto("/ui/operator/bible");
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { __presenterLiveConnected: () => boolean }).__presenterLiveConnected(),
      ),
    )
    .toBe(true);
}

test("a show missed while the socket was down is on the strip and in the header after the reconnect", async ({
  browser,
  request,
}) => {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  try {
    const routed = await routeLive(page);
    await openOperatorBible(page);
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");

    const dying = routed.routes[routed.routes.length - 1];
    routed.deaf.add(dying);
    const shown = await request.post(new URL("/bible/trigger-slide", baseURL).toString(), {
      data: { mainText: "Viera je podstatou toho, v čo dúfame", mainReference: "Hebrejom 11:1 (SEB)" },
    });
    expect(shown.ok()).toBeTruthy();
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");

    const before = routed.routes.length;
    await dying.close({ code: 4000, reason: "reset" });
    await expect.poll(() => routed.routes.length, { timeout: 20_000 }).toBeGreaterThan(before);
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:1");
    await expect(page.locator('[data-role="bible-preview-ref"]')).toHaveText("Hebrejom 11:1 (SEB)");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a socket that goes silent is replaced within seconds, and live events flow again", async ({
  browser,
  request,
}) => {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  try {
    const routed = await routeLive(page);
    await openOperatorBible(page);
    const silent = routed.routes[routed.routes.length - 1];
    const before = routed.routes.length;
    routed.deaf.add(silent); // no frames, not even the 1.5 s heartbeat
    await expect.poll(() => routed.routes.length, { timeout: 12_000 }).toBeGreaterThan(before);

    await request.post(new URL("/bible/trigger-slide", baseURL).toString(), {
      data: { mainText: "Lebo Boh tak miloval svet", mainReference: "Ján 3:16 (SEB)" },
    });
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Ján 3:16");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

/** The socket has delivered a frame (a socket without one does not count as a
 *  connect), then stops hearing: what is published next is missed. */
async function goDeaf(routed: Routed): Promise<WebSocketRoute> {
  await expect.poll(() => routed.frames, { timeout: 10_000 }).toBeGreaterThan(0);
  const dying = routed.routes[routed.routes.length - 1];
  routed.deaf.add(dying);
  return dying;
}

async function reconnect(routed: Routed, dying: WebSocketRoute): Promise<void> {
  const before = routed.routes.length;
  await dying.close({ code: 4000, reason: "reset" });
  await expect.poll(() => routed.routes.length, { timeout: 20_000 }).toBeGreaterThan(before);
}

test("a failed first load of the lists recovers with the first frame", async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  try {
    let failed = 0;
    await page.route(/\/libraries$/, async (route) => {
      if (failed === 0 && route.request().method() === "GET") {
        failed += 1;
        await route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
      } else {
        await route.continue();
      }
    });
    await page.goto("/ui/operator");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(page.locator('[data-role="library-list"] [data-library-id]').first()).toBeVisible({
      timeout: 15_000,
    });
    expect(failed).toBe(1);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a presentation added while the socket was down is in the Prepared tab after the reconnect", async ({
  browser,
  request,
}) => {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  try {
    const routed = await routeLive(page);
    await openOperatorBible(page);
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();
    const dying = await goDeaf(routed);
    const created = await request.post(new URL("/bible/presentations", baseURL).toString(), {
      data: { name: "Zmeškaná prezentácia" },
    });
    const id = (await created.json()).id as string;
    const card = page.locator(`[data-role="presentation-card"][data-presentation-id="${id}"]`);
    await expect(card).toHaveCount(0);
    await reconnect(routed, dying);
    await expect(card).toBeVisible({ timeout: 15_000 });
    await request.delete(new URL(`/bible/presentations/${id}`, baseURL).toString());
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("the tablet after a reconnect: an edit it missed is shown, a clear it missed unmarks the card", async ({
  browser,
  request,
}) => {
  const context = await browser.newContext({ viewport: { width: 900, height: 700 }, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const url = (path: string) => new URL(path, baseURL).toString();
  const created = await request.post(url("/bible/presentations"), {
    data: { name: "Tablet zmeškané" },
  });
  const id = (await created.json()).id as string;
  try {
    const appended = await request.post(url(`/bible/presentations/${id}/append`), {
      data: {
        slides: [
          { bibleMain: "Pôvodný text", bibleTranslation: "", bibleMainReference: "Ján 3:16 (SEB)" },
        ],
      },
    });
    const slide = (await appended.json()).slides[0] as { id: string };
    const routed = await routeLive(page);
    await page.goto("/ui/tablet");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await page.locator(`[data-role="presentation-button"][data-presentation-id="${id}"]`).click();
    const card = page.locator(`[data-role="tablet-slide"][data-slide-id="${slide.id}"]`);
    await expect(card).toContainText("Pôvodný text");
    await card.click();
    await expect(card).toHaveClass(/is-active/);

    const dying = await goDeaf(routed);
    const edited = await request.patch(url(`/bible/presentations/${id}/slides/${slide.id}`), {
      data: {
        bibleMain: "Zmeškaná úprava",
        bibleTranslation: "",
        bibleMainReference: "Ján 3:16 (SEB)",
        bibleTranslationReference: "",
      },
    });
    expect(edited.ok()).toBeTruthy();
    await request.post(url("/bible/clear"), { data: {} });
    await expect(card).toHaveClass(/is-active/);

    await reconnect(routed, dying);
    await expect(card).toContainText("Zmeškaná úprava", { timeout: 15_000 });
    await expect(card).not.toHaveClass(/is-active/);
    expect(errors).toEqual([]);
  } finally {
    await request.delete(url(`/bible/presentations/${id}`));
    await context.close();
  }
});

test("the stream editor after a reconnect shows a plate it missed", async ({ browser, request }) => {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  try {
    const routed = await routeLive(page);
    await page.goto("/ui/stream?output=stream&tab=nameplates");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(page.locator('[data-role="stream-nameplates"]')).toBeVisible();
    const dying = await goDeaf(routed);
    const created = await request.post(
      new URL("/stream/api/outputs/stream/nameplates", baseURL).toString(),
      { data: { primaryText: "Zmeškaná menovka", secondaryText: "" } },
    );
    expect(created.ok()).toBeTruthy();
    const plateId = (await created.json()).id as number;
    const row = page.locator(`[data-role="stream-nameplate"][data-nameplate-id="${plateId}"]`);
    await expect(row).toHaveCount(0);
    await reconnect(routed, dying);
    await expect(row).toBeVisible({ timeout: 15_000 });
    await request.delete(new URL(`/stream/api/nameplates/${plateId}`, baseURL).toString());
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});
