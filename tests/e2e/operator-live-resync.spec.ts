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

type Routed = { routes: WebSocketRoute[]; deaf: Set<WebSocketRoute> };

/** Relay /live/ws, except for the routes marked deaf (they hear nothing more). */
async function routeLive(page: Page): Promise<Routed> {
  const routed: Routed = { routes: [], deaf: new Set() };
  await page.routeWebSocket(/\/live\/ws/, (ws: WebSocketRoute) => {
    routed.routes.push(ws);
    const server = ws.connectToServer();
    server.onMessage((message) => {
      if (!routed.deaf.has(ws)) ws.send(message);
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
