import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #546 — the settings page must SAY why a mapped NDI source is not showing video.
//
// At PP an operator mapped `cgpp → RESOLUME-PP (cg-obs)`, activated it, and the stage
// stayed blank with no explanation anywhere in the UI. The sending machine simply was
// not broadcasting that name. Hours went into the (genuinely broken) encoder chain
// before the log revealed it. The server knew all along; nothing told the human.
//
// This spec runs on the DEFAULT lane, which has NO NDI SDK — so it can only prove the
// blind-server path. That path matters on its own (the server must never claim "not
// found on the network" when it cannot SEE the network), and it proves the endpoint,
// the poll and the render are wired. The real PP reproduction — a mapped name that is
// genuinely absent while a different name IS on the air — lives in
// `ndi-source-status.spec.ts`, on the self-hosted lane that has libndi.

test.describe.configure({ timeout: 180_000 });

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

async function createSource(request: any, label: string, ndiName: string) {
  const res = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label, ndiName } },
  );
  expect(res.status()).toBe(200);
  return await res.json();
}

test("status endpoint joins the rows, the network and the pipelines", async ({
  request,
}) => {
  const source = await createSource(request, "cgpp", "RESOLUME-PP (cg-obs)");

  const res = await request.get(
    new URL("/integrations/video-sources/status", baseURL).toString(),
  );
  expect(res.status()).toBe(200);
  const body = await res.json();

  // No SDK on this lane.
  expect(body.ndiAvailable).toBe(false);
  expect(body.discovered).toEqual([]);

  const entry = body.sources.find((s: any) => s.id === source.id);
  expect(entry, "the created source must appear in the status snapshot").toBeTruthy();
  // The honest answer for a server that cannot see the network. NOT "not-found" —
  // that would send the operator to check a sending machine that is perfectly fine.
  expect(entry.state).toBe("unknown");
  expect(entry.ndiName).toBe("RESOLUME-PP (cg-obs)");
});

test("the settings card shows the source's status badge", async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  const source = await createSource(request, "cam-status", "CAM-STATUS (usb)");

  await page.goto(new URL("/ui/settings", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 60_000 });

  const badge = page.locator(
    `[data-role="video-source-status"][data-source-id="${source.id}"]`,
  );
  await expect(badge).toHaveText("NDI unavailable", { timeout: 30_000 });
  await expect(badge).toHaveAttribute("data-state", "unknown");

  // Nothing can be discovered without the SDK, so the "on the network now" line must
  // not be there claiming an empty network.
  await expect(page.locator('[data-role="ndi-discovered"]')).toHaveCount(0);

  expect(errors, `console errors: ${errors.join(" | ")}`).toEqual([]);
});
