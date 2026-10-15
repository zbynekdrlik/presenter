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

// #852 C1b — Settings → "Počúvanie": the listening microphones card.
// A real browser adds a microphone in the shared inline editor, gets the server's
// Slovak 422 rule inline, saves (the id comes from the role), edits a mic in place
// and removes one; the card fits a 360 px phone. This file's server records into
// its own temp folder (the recording test, C1b-7, writes there).

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let recordings: string;
const STREAM = "E2E_GUEST";

test.describe.configure({ mode: "serial", timeout: 120_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  recordings = mkdtempSync(path.join(os.tmpdir(), "presenter-recordings-"));
  process.env.PRESENTER_LISTEN_RECORDINGS_DIR = recordings;
  try {
    serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
  } finally {
    delete process.env.PRESENTER_LISTEN_RECORDINGS_DIR;
  }
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
  rmSync(recordings, { recursive: true, force: true });
});

test.beforeEach(async ({ request }) => {
  expect((await request.put(url("/listen/recorder"), { data: { enabled: false } })).ok()).toBe(true);
  expect((await request.put(url("/listen/mics"), { data: [] })).ok()).toBe(true);
});

function url(p: string): string {
  return new URL(p, baseURL).toString();
}

async function storedMics(page: Page): Promise<unknown[]> {
  const reply = await page.request.get(url("/listen/mics"));
  expect(reply.ok()).toBe(true);
  return reply.json();
}

async function openSettings(page: Page): Promise<void> {
  await page.goto(url("/ui/settings"));
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
}

test("a microphone is added, refused inline, saved and removed", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await openSettings(page);
  const card = page.locator('[data-role="listen-card"]');
  await expect(card.locator('[data-role="listen-empty"]')).toBeVisible();

  // "+ Pridať mikrofón" opens the shared inline editor at the top of the list
  await card.locator('[data-role="listen-add"]').click();
  const editor = card.locator('[data-role="listen-new-item"] [data-role="listen-editor"]');
  await expect(editor.locator('[data-role="listen-name"]')).toBeFocused();
  await editor.locator('[data-role="listen-name"]').fill("Hosť E2E");
  await editor.locator('[data-role="listen-stream"]').fill("HOSŤ_EN"); // not ASCII: the server refuses
  await editor.locator('[data-role="listen-submit"]').click();
  const message = card.locator('[data-role="listen-form-status"]');
  await expect(message).toHaveAttribute("data-state", "error");
  await expect(message).toContainText("názov VBAN streamu môže mať 1–16 znakov ASCII");
  expect(await storedMics(page)).toEqual([]);

  // corrected: saved; a guest's id is "guest" (spec §5.6)
  await editor.locator('[data-role="listen-stream"]').fill(STREAM);
  await editor.locator('[data-role="listen-language"]').selectOption("en");
  await editor.locator('[data-role="listen-role"]').selectOption("guest");
  await editor.locator('[data-role="listen-channel"]').selectOption("left");
  await editor.locator('[data-role="listen-submit"]').click();
  const row = card.locator('[data-role="listen-mic-list"] li[data-id="guest"]');
  await expect(row.locator(".settings__host-label")).toHaveText("Hosť E2E");
  await expect(row.locator('[data-role="listen-config"]')).toHaveText(
    "angličtina · hosť · kanál ľavý (1) · id guest",
  );
  await expect(card.locator('[data-role="listen-count"]')).toHaveText("1");
  expect(await storedMics(page)).toEqual([
    { id: "guest", name: "Hosť E2E", streamName: STREAM, senderIp: null, channel: "left", language: "en", role: "guest" },
  ]);

  // removed: the confirm names the mic, the list is empty again
  page.once("dialog", async (dialog) => {
    expect(dialog.message()).toBe("Odstrániť mikrofón „Hosť E2E“? Presenter ho prestane počúvať.");
    await dialog.accept();
  });
  await row.locator('[data-role="listen-delete"]').click();
  await expect(card.locator('[data-role="listen-empty"]')).toBeVisible();
  expect(await storedMics(page)).toEqual([]);

  // exactly the one real 422 of the refused save; nothing else
  const refused = /Failed to load resource: the server responded with a status of 422\b/;
  expect(errors.filter((line) => refused.test(line))).toHaveLength(1);
  expect(errors.filter((line) => !refused.test(line))).toEqual([]);
});

test("a microphone is edited in place and the card fits a 360 px phone", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const seeded = await page.request.put(url("/listen/mics"), {
    data: [
      { id: "interp", name: "Tlmočník", streamName: "INTERP_SK", senderIp: "192.0.2.10", channel: "right", language: "sk", role: "interpreter" },
    ],
  });
  expect(seeded.ok()).toBe(true);
  await page.setViewportSize({ width: 360, height: 800 });
  await openSettings(page);
  const card = page.locator('[data-role="listen-card"]');
  const row = () => card.locator('li[data-id="interp"]');
  await expect(row().locator('[data-role="listen-config"]')).toHaveText(
    "slovenčina · tlmočník · kanál pravý (2) · id interp",
  );
  await expect(row().locator(".settings__host-addr")).toContainText("INTERP_SK · od 192.0.2.10");

  await row().locator('[data-role="listen-edit"]').click();
  const editor = row().locator('[data-role="listen-editor"]');
  await expect(editor.locator('[data-role="listen-sender-ip"]')).toHaveValue("192.0.2.10");
  await expect(editor.locator('[data-role="listen-channel"]')).toHaveValue("right");
  await expect(editor.locator('[data-role="listen-language"]')).toHaveValue("sk");
  await editor.locator('[data-role="listen-name"]').fill("Tlmočník booth");
  await editor.locator('[data-role="listen-sender-ip"]').fill("");
  await editor.locator('[data-role="listen-submit"]').click();
  await expect(row().locator(".settings__host-label")).toHaveText("Tlmočník booth");
  expect(await storedMics(page)).toEqual([
    { id: "interp", name: "Tlmočník booth", streamName: "INTERP_SK", senderIp: null, channel: "right", language: "sk", role: "interpreter" },
  ]);

  const fit = await page.evaluate(async () => {
    await document.fonts.ready;
    const layout = document.querySelector(".settings-layout");
    if (!layout) throw new Error("no .settings-layout");
    return { scrollWidth: layout.scrollWidth, clientWidth: layout.clientWidth };
  });
  expect(fit.scrollWidth).toBeLessThanOrEqual(fit.clientWidth);
  expect(errors).toEqual([]);
});
