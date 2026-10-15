import { test, expect, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  REPO_ROOT,
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
// removes one; the card fits a 360 px phone, also with the editor open. Two more
// tests (C1b-7) cover the live part of the card: one plays a 1 kHz tone from the
// real VBAN sender scripts/dev/listen/vban_send.py into this server's loopback
// socket, watches the row's health, switches the recorder on for today and off
// again and reads the WAV back from this file's temp folder; the other fails the
// status poll twice and sees the card show nothing instead of stale numbers.

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let recordings: string | undefined;
const STREAM = "E2E_INTERP";
// The guest the real sender plays as (a stream of its own, like STREAM).
const GUEST_STREAM = "E2E_GUEST";

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

type RecorderMic = {
  id: string;
  state: string;
  file: string | null;
  bytes: number;
  droppedFrames: number;
  error: string | null;
};

type ListenStatus = {
  socket: { state: string; localAddr: string | null };
  recorder: { enabled: boolean; mics: RecorderMic[] };
};

async function listenStatus(page: Page): Promise<ListenStatus> {
  const reply = await page.request.get(url("/listen/status"));
  expect(reply.ok()).toBe(true);
  return reply.json();
}

/** The recordings folder of this file's server (set in `beforeAll`). */
function recordingsRoot(): string {
  if (!recordings) throw new Error("the recordings folder was not created");
  return recordings;
}

/** A 1 kHz half-scale tone, 48 kHz INT16, into the test server's VBAN socket. */
function startSender(port: number, seconds: number): ChildProcess {
  return spawn(
    "python3",
    [
      path.join(REPO_ROOT, "scripts", "dev", "listen", "vban_send.py"),
      "--to",
      `127.0.0.1:${port}`,
      "--stream",
      GUEST_STREAM,
      "--seconds",
      String(seconds),
    ],
    { stdio: "inherit" },
  );
}

async function stopSender(sender: ChildProcess): Promise<void> {
  if (sender.exitCode !== null || sender.signalCode !== null) return;
  sender.kill("SIGTERM");
  await once(sender, "exit");
}

/**
 * A recorded WAV after its header is checked against the file: PCM, mono, 16 kHz,
 * 16 bit, `length = 44 + data`, RIFF size = `36 + data`. Returns its audio.
 */
function readWav(file: string): Buffer {
  const wav = readFileSync(file);
  expect(wav.subarray(0, 4).toString("ascii")).toBe("RIFF");
  expect(wav.subarray(8, 16).toString("ascii")).toBe("WAVEfmt ");
  expect(wav.readUInt16LE(20)).toBe(1); // PCM
  expect(wav.readUInt16LE(22)).toBe(1); // mono
  expect(wav.readUInt32LE(24)).toBe(16_000);
  expect(wav.readUInt16LE(34)).toBe(16); // bits per sample
  expect(wav.subarray(36, 40).toString("ascii")).toBe("data");
  const dataBytes = wav.readUInt32LE(40);
  expect(wav.length).toBe(44 + dataBytes);
  expect(wav.readUInt32LE(4)).toBe(36 + dataBytes);
  return wav.subarray(44);
}

test("a received microphone shows its health and the recorder writes today's WAV", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await openSettings(page);
  const card = page.locator('[data-role="listen-card"]');
  const toggle = card.locator('[data-role="listen-recorder-toggle"]');
  const recorderState = card.locator('[data-role="listen-recorder-state"]');
  // no mic: no socket, and nothing to record — the switch says so
  await expect(card.locator('[data-role="listen-socket"]')).toContainText("nečinný");
  await expect(recorderState).toContainText("Najprv pridaj mikrofón");
  await expect(toggle).toBeDisabled();

  await seed(page, [{ ...GUEST, name: "Hosť E2E", streamName: GUEST_STREAM }]);
  const row = card.locator('li[data-id="guest"]');
  const state = row.locator('[data-role="listen-state"]');
  await expect(state).toHaveAttribute("data-state", "waiting", { timeout: 10_000 });
  await expect(card.locator('[data-role="listen-socket"]')).toContainText("Počúva na 127.0.0.1:", { timeout: 10_000 });
  await expect(toggle).toBeEnabled();
  const localAddr = (await listenStatus(page)).socket.localAddr ?? "";
  const port = Number(localAddr.split(":").pop());
  expect(port).toBeGreaterThan(0);

  const sender = startSender(port, 40);
  try {
    await expect(state).toHaveAttribute("data-state", "receiving", { timeout: 10_000 });
    await expect(row.locator('[data-role="listen-health"]')).toContainText("dBFS ● hlas", { timeout: 10_000 });
    await expect(row.locator('[data-role="listen-health"]')).toContainText("48000 Hz · 1 kan. · int16");

    // the recorder: on for today
    await toggle.check();
    await expect(recorderState).toContainText("vypne sa samo o polnoci", { timeout: 10_000 });
    await expect(row.locator('[data-role="listen-recording"]')).toContainText(/Nahráva: guest_\d{6}(-\d)?\.wav/, { timeout: 10_000 });
    expect((await listenStatus(page)).recorder.enabled).toBe(true);
    // more than half a second recorded before it is switched off
    await expect
      .poll(async () => (await listenStatus(page)).recorder.mics[0].bytes, { timeout: 10_000 })
      .toBeGreaterThan(16_000);

    // off again: the file is closed with its header
    await toggle.uncheck();
    await expect(recorderState).toContainText("Nahrávanie je vypnuté", { timeout: 10_000 });
    await expect.poll(async () => (await listenStatus(page)).recorder.mics[0].state, { timeout: 10_000 }).toBe("off");
  } finally {
    await stopSender(sender);
  }

  // The WAVs on disk: one day folder; one file per continuous stretch (a stall of
  // more than a second on a loaded runner starts a new one, as designed — never
  // loosen the 1 s rule for it), each with a header that matches its audio.
  const root = recordingsRoot();
  const days = readdirSync(root);
  expect(days).toHaveLength(1);
  expect(days[0]).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  const dayDir = path.join(root, days[0]);
  const files = readdirSync(dayDir);
  expect(files.length).toBeGreaterThanOrEqual(1);
  for (const name of files) expect(name).toMatch(/^guest_\d{6}(-\d)?\.wav$/);
  const audio = files.map((name) => readWav(path.join(dayDir, name)));
  expect(audio.reduce((sum, data) => sum + data.length, 0)).toBeGreaterThan(16_000);
  // the status names the last file and its audio bytes: the file's own data size
  const last = (await listenStatus(page)).recorder.mics[0];
  expect(last.error).toBeNull();
  const lastName = path.basename(last.file ?? "");
  expect(files).toContain(lastName);
  expect(audio[files.indexOf(lastName)].length).toBe(last.bytes);
  // real audio, not silence: the half-scale tone peaks near 16 384
  let peak = 0;
  for (const data of audio) {
    for (let at = 0; at + 1 < data.length; at += 2) peak = Math.max(peak, Math.abs(data.readInt16LE(at)));
  }
  expect(peak).toBeGreaterThan(8_000);
  await expect(row.locator('[data-role="listen-recording"]')).toContainText("Posledná nahrávka:", { timeout: 10_000 });
  expect(errors).toEqual([]);
});

test("a status poll that fails twice shows the card as unknown, not stale, and it recovers", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await seed(page, [GUEST]);
  await openSettings(page);
  const card = page.locator('[data-role="listen-card"]');
  const toggle = card.locator('[data-role="listen-recorder-toggle"]');
  const row = card.locator('li[data-id="guest"]');
  const state = row.locator('[data-role="listen-state"]');
  await expect(state).toHaveAttribute("data-state", "waiting", { timeout: 10_000 });
  await expect(card.locator('[data-role="listen-socket"]')).toContainText("Počúva na");
  await expect(toggle).toBeEnabled();

  // the status endpoint goes down: two failed polls in a row and the card shows
  // nothing it cannot vouch for (a real 503 for the page's own fetch)
  let refused = 0;
  await page.route("**/listen/status", async (route) => {
    refused += 1;
    await route.fulfill({ status: 503, contentType: "text/plain", body: "down" });
  });
  await expect(state).toHaveAttribute("data-state", "unknown", { timeout: 15_000 });
  await expect(state).toHaveText("—");
  await expect(row.locator('[data-role="listen-health"]')).toHaveText("");
  await expect(card.locator('[data-role="listen-socket"]')).toHaveText("");
  await expect(toggle).toBeDisabled();
  expect(refused).toBeGreaterThanOrEqual(2);

  // it answers again: the card is back with what the server says
  await page.unroute("**/listen/status");
  await expect(state).toHaveAttribute("data-state", "waiting", { timeout: 15_000 });
  await expect(card.locator('[data-role="listen-socket"]')).toContainText("Počúva na");
  await expect(toggle).toBeEnabled();

  // the console holds exactly the refused polls and the ONE warn at the second failure
  const refusedLine = /^\[error\] Failed to load resource: the server responded with a status of 503\b/;
  const staleWarn = /^\[warning\] listen status poll failed 2x in a row — showing it as unknown: HTTP 503: /;
  expect(errors.filter((line) => refusedLine.test(line))).toHaveLength(refused);
  expect(errors.filter((line) => staleWarn.test(line))).toHaveLength(1);
  expect(errors.filter((line) => !refusedLine.test(line) && !staleWarn.test(line))).toEqual([]);
});
