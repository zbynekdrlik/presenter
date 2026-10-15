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
// A real browser adds a microphone in the shared inline editor (the three selects
// through their real `change` events), gets the server's Slovak 422 rule inline
// naming the id the editor showed, saves (the id comes from the role), edits a
// mic in place (the other mic comes back unchanged: the whole-list PUT) and
// removes one; the card fits a 360 px phone, also with the editor open. This
// file's server records into its own temp folder (the recording test, C1b-7,
// writes there).

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let recordings: string | undefined;
const STREAM = "E2E_INTERP";

test.describe.configure({ timeout: 120_000 });

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
  if (recordings) rmSync(recordings, { recursive: true, force: true });
});

test.beforeEach(async ({ request }) => {
  expect((await request.put(url("/listen/recorder"), { data: { enabled: false } })).ok()).toBe(true);
  expect((await request.put(url("/listen/mics"), { data: [] })).ok()).toBe(true);
});

function url(p: string): string {
  return new URL(p, baseURL).toString();
}

type Mic = {
  id: string;
  name: string;
  streamName: string;
  senderIp: string | null;
  channel: string;
  language: string;
  role: string;
};

const GUEST: Mic = {
  id: "guest",
  name: "Hosť",
  streamName: "GUEST_EN",
  senderIp: null,
  channel: "left",
  language: "en",
  role: "guest",
};
const INTERP: Mic = {
  id: "interp",
  name: "Tlmočník",
  streamName: "INTERP_SK",
  senderIp: "192.0.2.10",
  channel: "right",
  language: "sk",
  role: "interpreter",
};

async function seed(page: Page, mics: Mic[]): Promise<void> {
  expect((await page.request.put(url("/listen/mics"), { data: mics })).ok()).toBe(true);
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

// The page's scroll container must not scroll sideways (phone widths).
async function assertFitsPhone(page: Page): Promise<void> {
  const fit = await page.evaluate(async () => {
    await document.fonts.ready;
    const layout = document.querySelector(".settings-layout");
    if (!layout) throw new Error("no .settings-layout");
    return { scrollWidth: layout.scrollWidth, clientWidth: layout.clientWidth };
  });
  expect(fit.scrollWidth).toBeLessThanOrEqual(fit.clientWidth);
}

// Exactly the real 422 lines of the refused saves are allowed; nothing else.
function assertOnly422(errors: string[], refusals: number): void {
  const refused = /Failed to load resource: the server responded with a status of 422\b/;
  expect(errors.filter((line) => refused.test(line))).toHaveLength(refusals);
  expect(errors.filter((line) => !refused.test(line))).toEqual([]);
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
  const idLine = editor.locator('[data-role="listen-id"]');
  await expect(editor.locator('[data-role="listen-name"]')).toBeFocused();
  await expect(idLine).toHaveText("Identifikátor: guest (podľa roly, nemení sa)");
  await editor.locator('[data-role="listen-name"]').fill("Hosť E2E");
  await editor.locator('[data-role="listen-stream"]').fill("HOSŤ_EN"); // not ASCII: the server refuses
  await editor.locator('[data-role="listen-submit"]').click();
  const message = card.locator('[data-role="listen-form-status"]');
  await expect(message).toHaveAttribute("data-state", "error");
  // the refusal names the id the editor showed
  await expect(message).toContainText("Mikrofón „guest“: názov VBAN streamu môže mať 1–16 znakov ASCII");
  expect(await storedMics(page)).toEqual([]);

  // corrected, and the three selects moved off their defaults through real
  // `change` events: an interpreter's id is "interp" (spec §5.6)
  await editor.locator('[data-role="listen-name"]').fill("Tlmočník E2E");
  await editor.locator('[data-role="listen-stream"]').fill(STREAM);
  await editor.locator('[data-role="listen-role"]').selectOption("interpreter");
  await editor.locator('[data-role="listen-language"]').selectOption("sk");
  await editor.locator('[data-role="listen-channel"]').selectOption("mix");
  await expect(idLine).toHaveText("Identifikátor: interp (podľa roly, nemení sa)");
  await editor.locator('[data-role="listen-submit"]').click();
  const row = card.locator('[data-role="listen-mic-list"] li[data-id="interp"]');
  await expect(row.locator(".settings__host-label")).toHaveText("Tlmočník E2E");
  await expect(row.locator('[data-role="listen-config"]')).toHaveText(
    "slovenčina · tlmočník · kanál zmes · id interp",
  );
  await expect(card.locator('[data-role="listen-count"]')).toHaveText("1");
  expect(await storedMics(page)).toEqual([
    { id: "interp", name: "Tlmočník E2E", streamName: STREAM, senderIp: null, channel: "mix", language: "sk", role: "interpreter" },
  ]);

  // removed: the confirm names the mic, the list is empty again
  page.once("dialog", async (dialog) => {
    expect(dialog.message()).toBe("Odstrániť mikrofón „Tlmočník E2E“? Presenter ho prestane počúvať.");
    await dialog.accept();
  });
  await row.locator('[data-role="listen-delete"]').click();
  await expect(card.locator('[data-role="listen-empty"]')).toBeVisible();
  expect(await storedMics(page)).toEqual([]);

  assertOnly422(errors, 1);
});

test("a second guest shows the id it will get and a refusal names that id", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await seed(page, [GUEST]);
  await openSettings(page);
  const card = page.locator('[data-role="listen-card"]');
  await expect(card.locator('li[data-id="guest"]')).toBeVisible();

  await card.locator('[data-role="listen-add"]').click();
  const editor = card.locator('[data-role="listen-new-item"] [data-role="listen-editor"]');
  const idLine = editor.locator('[data-role="listen-id"]');
  // a guest already exists: the next guest id, from the listed ids
  await expect(idLine).toHaveText("Identifikátor: guest-2 (podľa roly, nemení sa)");
  await editor.locator('[data-role="listen-name"]').fill("Druhý hosť");
  await editor.locator('[data-role="listen-stream"]').fill("HOSŤ_EN"); // not ASCII: the server refuses
  await editor.locator('[data-role="listen-submit"]').click();
  const message = card.locator('[data-role="listen-form-status"]');
  await expect(message).toHaveAttribute("data-state", "error");
  await expect(message).toContainText("Mikrofón „guest-2“: názov VBAN streamu môže mať 1–16 znakov ASCII");
  expect(await storedMics(page)).toEqual([GUEST]);

  // the line follows the role select and back
  await editor.locator('[data-role="listen-role"]').selectOption("interpreter");
  await expect(idLine).toHaveText("Identifikátor: interp (podľa roly, nemení sa)");
  await editor.locator('[data-role="listen-role"]').selectOption("guest");
  await expect(idLine).toHaveText("Identifikátor: guest-2 (podľa roly, nemení sa)");

  await editor.locator('[data-role="listen-stream"]').fill("E2E_GUEST2");
  await editor.locator('[data-role="listen-submit"]').click();
  const row = card.locator('li[data-id="guest-2"]');
  await expect(row.locator(".settings__host-label")).toHaveText("Druhý hosť");
  await expect(row.locator('[data-role="listen-config"]')).toHaveText(
    "angličtina · hosť · kanál ľavý (1) · id guest-2",
  );
  await expect(card.locator('[data-role="listen-count"]')).toHaveText("2");
  expect(await storedMics(page)).toEqual([
    GUEST,
    { id: "guest-2", name: "Druhý hosť", streamName: "E2E_GUEST2", senderIp: null, channel: "left", language: "en", role: "guest" },
  ]);

  assertOnly422(errors, 1);
});

test("a microphone is edited in place, the other stays, and the card fits a 360 px phone", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await seed(page, [GUEST, INTERP]);
  await page.setViewportSize({ width: 360, height: 800 });
  await openSettings(page);
  const card = page.locator('[data-role="listen-card"]');
  const row = () => card.locator('li[data-id="interp"]');
  await expect(row().locator('[data-role="listen-config"]')).toHaveText(
    "slovenčina · tlmočník · kanál pravý (2) · id interp",
  );
  await expect(row().locator(".settings__host-addr")).toContainText("INTERP_SK · od 192.0.2.10");
  await expect(card.locator('[data-role="listen-count"]')).toHaveText("2");

  await row().locator('[data-role="listen-edit"]').click();
  const editor = row().locator('[data-role="listen-editor"]');
  await expect(editor.locator('[data-role="listen-sender-ip"]')).toHaveValue("192.0.2.10");
  await expect(editor.locator('[data-role="listen-channel"]')).toHaveValue("right");
  await expect(editor.locator('[data-role="listen-language"]')).toHaveValue("sk");
  await expect(editor.locator('[data-role="listen-role"]')).toHaveValue("interpreter");
  // an edited mic keeps its id: the line shows it
  await expect(editor.locator('[data-role="listen-id"]')).toHaveText("Identifikátor: interp (nemení sa)");
  await editor.locator('[data-role="listen-name"]').fill("Tlmočník booth");
  await editor.locator('[data-role="listen-sender-ip"]').fill("");
  await editor.locator('[data-role="listen-channel"]').selectOption("mix");
  await editor.locator('[data-role="listen-role"]').selectOption("preacher");
  // still the same id after the role changed
  await expect(editor.locator('[data-role="listen-id"]')).toHaveText("Identifikátor: interp (nemení sa)");
  // the open editor (three selects, the id line) fits the phone before it is saved
  await assertFitsPhone(page);
  await editor.locator('[data-role="listen-submit"]').click();
  await expect(row().locator(".settings__host-label")).toHaveText("Tlmočník booth");
  await expect(row().locator('[data-role="listen-config"]')).toHaveText(
    "slovenčina · kazateľ · kanál zmes · id interp",
  );
  // the whole list was written back: the other mic is exactly what it was, the
  // edited one changed in place (its id kept, the selects stored)
  expect(await storedMics(page)).toEqual([
    GUEST,
    { id: "interp", name: "Tlmočník booth", streamName: "INTERP_SK", senderIp: null, channel: "mix", language: "sk", role: "preacher" },
  ]);

  await assertFitsPhone(page);
  expect(errors).toEqual([]);
});
