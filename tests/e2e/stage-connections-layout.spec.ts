import { test, expect, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// #797 — /stage/connections must report the layout each display SHOWS.
//
// The StagePresence frame is sent on socket open, before the page's layout
// resync, so the server used to keep the `worship-snv` default forever. The
// stage page now reports its displayed layout (body[data-layout-code]) on
// every heartbeat ACK, for EVERY layout, including those that mount no NDI
// <video> (so no diag frames exist). This drives the real path end to end:
// the operator switches the server layout, the /stage page renders it, and
// the connection's top-level layoutCode on /stage/connections follows.
// ─────────────────────────────────────────────────────────────────────────

test.describe.configure({ timeout: 120_000 });

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

type StageConnection = { layoutCode: string; status: string };

async function connectedLayouts(page: Page): Promise<string> {
  const res = await page.request.get(
    new URL("/stage/connections", baseURL).toString(),
  );
  const conns = (await res.json()) as StageConnection[];
  const live = conns.filter((c) => c.status === "connected");
  return live.map((c) => c.layoutCode).join(",");
}

async function selectLayout(page: Page, code: string): Promise<void> {
  const res = await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code } },
  );
  expect(res.ok(), `select layout ${code}`).toBe(true);
}

test("/stage/connections follows the displayed non-NDI layout (#797)", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  page.on("pageerror", (err) =>
    consoleMessages.push(`[pageerror] ${err.message}`),
  );

  // No NDI source: none of these layouts mounts an NDI <video>, so only the
  // heartbeat ACK can carry the displayed layout.
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
    { failOnStatusCode: false },
  );
  await selectLayout(page, "preach");

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="preach"]', {
    timeout: 10_000,
  });

  // The presence frame registered the pre-resync default; the next heartbeat
  // ACK (1.5 s cadence) must replace it with the displayed layout.
  await expect
    .poll(() => connectedLayouts(page), {
      timeout: 15_000,
      intervals: [500, 1000, 1000],
    })
    .toBe("preach");

  // A live switch to another non-NDI layout is followed as well.
  await selectLayout(page, "bible");
  await page.waitForSelector('body[data-layout-code="bible"]', {
    timeout: 10_000,
  });
  await expect
    .poll(() => connectedLayouts(page), {
      timeout: 15_000,
      intervals: [500, 1000, 1000],
    })
    .toBe("bible");

  expect(consoleMessages).toEqual([]);
});
