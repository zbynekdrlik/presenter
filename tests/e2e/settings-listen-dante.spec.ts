import { test, expect, type Page } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #933 — Settings → "Počúvanie" on a box that names a Dante interface: Presenter's
// own Dante device has two FIXED mono RX channels, "mic 1" and "mic 2", whatever
// the saved mic list holds (it is empty here). The card shows them read-only —
// name, channel, state, id — and saving a VBAN mic in the editor never touches
// them. This server names an interface that does not exist on the runner
// (PRESENTER_LISTEN_DANTE_IFACE), so dlink refuses to start and the device line
// and both mics say why and that it retries: the start path is the real one,
// only the network interface is missing.

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let recordings: string | undefined;
const IFACE = "e2e-no-dante";

test.describe.configure({ timeout: 120_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  recordings = mkdtempSync(path.join(os.tmpdir(), "presenter-recordings-"));
  process.env.PRESENTER_LISTEN_RECORDINGS_DIR = recordings;
  process.env.PRESENTER_LISTEN_DANTE_IFACE = IFACE;
  try {
    serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
  } finally {
    delete process.env.PRESENTER_LISTEN_RECORDINGS_DIR;
    delete process.env.PRESENTER_LISTEN_DANTE_IFACE;
  }
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
  if (recordings) rmSync(recordings, { recursive: true, force: true });
});

test.beforeEach(async ({ request }) => {
  expect((await request.put(url("/listen/mics"), { data: [] })).ok()).toBe(true);
});

function url(p: string): string {
  return new URL(p, baseURL).toString();
}

async function micIds(page: Page): Promise<string[]> {
  const reply = await page.request.get(url("/listen/status"));
  expect(reply.ok()).toBe(true);
  const status = await reply.json();
  return status.mics.map((mic: { id: string }) => mic.id);
}

test("the two fixed Dante mics show read-only beside the saved VBAN mics", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.goto(url("/ui/settings"));
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  const card = page.locator('[data-role="listen-card"]');

  // nothing saved, yet both channels are there
  await expect(card.locator('[data-role="listen-empty"]')).toBeVisible();
  const rows = card.locator('[data-role="listen-dante-mics"] [data-role="listen-dante-mic"]');
  await expect(rows).toHaveCount(2);
  await expect(rows.locator(".settings__host-label")).toHaveText(["Dante mic 1", "Dante mic 2"]);
  await expect(rows.locator('[data-role="listen-dante-channel"]')).toHaveText([
    "kanál Dante „mic 1“",
    "kanál Dante „mic 2“",
  ]);
  await expect(rows.locator('[data-role="listen-config"]')).toHaveText([
    "neprepisovať · moderátor · id dante-mic-1",
    "neprepisovať · moderátor · id dante-mic-2",
  ]);
  for (const index of [0, 1]) {
    await expect(rows.nth(index).locator('[data-role="listen-state"]')).toHaveAttribute("data-state", "waiting");
  }
  await expect(rows.locator('[data-role="listen-health"]').first()).toHaveText("Zatiaľ žiadny zvuk z Dante.");
  // read-only: no edit or delete on a Dante mic
  await expect(rows.locator("button")).toHaveCount(0);

  // the interface is missing on this runner: the real start is refused and retried
  const device = card.locator('[data-role="listen-dante"]');
  await expect(device).toContainText("zariadenie presenter-listen nebeží — skúšam znova.");
  await expect(rows.first().locator('[data-role="listen-error"]')).toContainText("zariadenie Dante sa nedá spustiť");
  const status = await (await page.request.get(url("/listen/status"))).json();
  expect(status.dante).toMatchObject({ state: "retrying", device: "presenter-listen", interface: IFACE });
  expect(status.mics.map((mic: { dante: { channel: string } }) => mic.dante.channel)).toEqual(["mic 1", "mic 2"]);

  // a VBAN mic saved in the editor comes first and the fixed pair stays
  await card.locator('[data-role="listen-add"]').click();
  const editor = card.locator('[data-role="listen-new-item"] [data-role="listen-editor"]');
  await editor.locator('[data-role="listen-name"]').fill("Hosť");
  await editor.locator('[data-role="listen-stream"]').fill("GUEST_EN");
  await editor.locator('[data-role="listen-submit"]').click();
  await expect(card.locator('[data-role="listen-mic-list"] li[data-id="guest"]')).toBeVisible();
  await expect(card.locator('[data-role="listen-count"]')).toHaveText("1");
  await expect.poll(() => micIds(page)).toEqual(["guest", "dante-mic-1", "dante-mic-2"]);
  await expect(rows).toHaveCount(2);
  const stored = await (await page.request.get(url("/listen/mics"))).json();
  expect(stored.map((mic: { id: string }) => mic.id)).toEqual(["guest"]);

  expect(errors).toEqual([]);
});
