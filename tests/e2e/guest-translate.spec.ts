import { test, expect, firefox, type Browser, type BrowserContextOptions, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { createServer, type AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  REPO_ROOT,
  PHONE_390,
  assertNoHorizontalOverflow,
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";
import { startFakeGemini, type FakeGemini } from "./fake-gemini";

// #999 L3 — a guest on a phone opens the root of the guest-only listener
// (PRESENTER_GUEST_PORT, a second port of the test server), picks English and taps Play: the
// translated speech plays (the AudioWorklet reports played samples) and the
// text appears, against a scripted fake Gemini Live (tests/e2e/fake-gemini.ts;
// Google is never reached). The preacher is a VBAN mic (`guest`) the real
// sender (scripts/dev/listen/vban_send.py) plays a voiced tone into; the
// server translates it because PRESENTER_LISTEN_GUEST_MIC names it. The guest
// listener serves nothing else; the operator's QR page is on the main port and
// prints https://<PRESENTER_GUEST_HOST>/.

const KEY = "e2e-test-key-not-real";
const STREAM = "E2E_PREACHER";
const PREACHER = {
  id: "guest",
  name: "Kazateľ (test)",
  streamName: STREAM,
  senderIp: null,
  channel: "left",
  language: "off",
  role: "guest",
};

const GUEST_HOST = "listen.example.org";

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let guestURL: string;
let fake: FakeGemini;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  guestURL = `http://127.0.0.1:${await freePort()}/`;
  await refreshDevData(config.dbUrl);
  fake = await startFakeGemini();
  process.env.PRESENTER_LISTEN_GEMINI_URL = fake.url;
  process.env.PRESENTER_LISTEN_GEMINI_API_KEY = KEY;
  process.env.PRESENTER_LISTEN_GUEST_MIC = PREACHER.id;
  process.env.PRESENTER_GUEST_HOST = GUEST_HOST;
  process.env.PRESENTER_GUEST_PORT = new URL(guestURL).port;
  try {
    serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
  } finally {
    delete process.env.PRESENTER_LISTEN_GEMINI_URL;
    delete process.env.PRESENTER_LISTEN_GEMINI_API_KEY;
    delete process.env.PRESENTER_LISTEN_GUEST_MIC;
    delete process.env.PRESENTER_GUEST_HOST;
    delete process.env.PRESENTER_GUEST_PORT;
  }
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
  await fake?.stop();
});

test.beforeEach(async ({ request }) => {
  expect((await request.put(url("/listen/mics"), { data: [PREACHER] })).ok()).toBe(true);
});

/** A port nothing listens on right now (the guest listener's). */
async function freePort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", () => resolve()));
  const { port } = server.address() as AddressInfo;
  await new Promise<void>((resolve) => server.close(() => resolve()));
  return port;
}

function url(p: string): string {
  return new URL(p, baseURL).toString();
}

async function vbanPort(page: Page): Promise<number> {
  let port = 0;
  await expect
    .poll(async () => {
      const reply = await page.request.get(url("/listen/status"));
      const localAddr: string = (await reply.json()).socket.localAddr ?? "";
      port = Number(localAddr.split(":").pop());
      return port;
    })
    .toBeGreaterThan(0);
  return port;
}

async function openPage(browser: Browser, profile: BrowserContextOptions, route: string, errors: string[]): Promise<Page> {
  const context = await browser.newContext({ ...profile, baseURL });
  const page = await context.newPage();
  attachConsoleErrorCollector(page, errors);
  await page.goto(route);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  return page;
}

function startSender(port: number, seconds: number): ChildProcess {
  return spawn(
    "python3",
    [
      path.join(REPO_ROOT, "scripts", "dev", "listen", "vban_send.py"),
      "--to",
      `127.0.0.1:${port}`,
      "--stream",
      STREAM,
      "--seconds",
      String(seconds),
      "--amplitude",
      "0.5",
    ],
    { stdio: "inherit" },
  );
}

async function stopSender(sender: ChildProcess): Promise<void> {
  if (sender.exitCode !== null || sender.signalCode !== null) return;
  sender.kill("SIGTERM");
  await once(sender, "exit");
}

async function bodyNumber(page: Page, name: string): Promise<number> {
  return Number((await page.locator("body").getAttribute(name)) ?? "0");
}

test("a guest picks a language on a phone and hears the translation with its text", async ({ browser }) => {
  const errors: string[] = [];
  const page = await openPage(browser, PHONE_390, guestURL, errors);
  const languages = page.locator('[data-role="guest-language"]');
  await expect(languages.filter({ hasText: "Deutsch" })).toBeVisible();
  await expect(languages.filter({ hasText: "Українська" })).toBeVisible();
  await expect(languages.filter({ hasText: "Magyar" })).toBeVisible();
  await expect(page.locator('[data-role="guest-version"]')).toContainText(/v\d+\.\d+\.\d+/);
  await assertNoHorizontalOverflow(page);

  await languages.filter({ hasText: "English" }).tap();
  await expect(page).toHaveURL(/[?&]lang=en/);
  await expect(page.locator('[data-role="guest-current-language"]')).toHaveText("English");
  const state = page.locator('[data-role="guest-state"]');
  await expect(state).toHaveAttribute("data-state", "idle");
  await page.locator('[data-role="guest-play"]').tap();
  await expect(page.locator('[data-role="guest-play"]')).toHaveAttribute("data-playing", "true");
  // connected, the preacher quiet: nothing is sent
  await expect(state).toHaveAttribute("data-state", "waiting");
  expect(fake.audioMessages).toBe(0);

  const sender = startSender(await vbanPort(page), 60);
  try {
    await expect(state).toHaveAttribute("data-state", "live");
    await expect.poll(() => bodyNumber(page, "data-guest-chunks")).toBeGreaterThan(5);
    const transcript = page.locator('[data-role="guest-transcript"]');
    await expect(transcript).toContainText("Word1 Word2");
    // the Slovak original shows only when asked for
    const original = page.locator('[data-role="guest-original"]').filter({ hasText: "slovo5" });
    await expect(original).toBeHidden();
    await page.locator('[data-role="guest-original-toggle"]').check();
    await expect(original).toBeVisible();
    await assertNoHorizontalOverflow(page);
  } finally {
    await stopSender(sender);
  }

  // what Gemini was sent: the key in the header, the language, audio only while voiced
  expect(fake.keys[0]).toBe(KEY);
  const setup = fake.setups[0].setup as Record<string, Record<string, Record<string, unknown>>>;
  expect(setup.generationConfig.translationConfig).toEqual({ targetLanguageCode: "en" });
  expect(fake.audioMessages).toBeGreaterThan(5);

  // Stop: the stream ends and the play button returns
  await page.locator('[data-role="guest-play"]').tap();
  await expect(page.locator('[data-role="guest-play"]')).toHaveAttribute("data-playing", "false");
  await expect(state).toHaveAttribute("data-state", "idle");
  await page.locator('[data-role="guest-change-language"]').tap();
  await expect(languages.filter({ hasText: "English" })).toHaveAttribute("data-active", "true");

  await page.context().close();
  expect(errors).toEqual([]);
});

/** A private PulseAudio with one null sink: the runner has no sound server, and a
 * browser without one never renders audio (Firefox's AudioContext never resumes).
 * A missing `pulseaudio` fails the test — never a skip. */
async function startNullSink(): Promise<{ server: string; stop: () => Promise<void> }> {
  const dir = mkdtempSync(path.join(tmpdir(), "e2e-pulse-"));
  const socket = path.join(dir, "native");
  const pulse = spawn(
    "pulseaudio",
    [
      "-n",
      "--daemonize=no",
      "--exit-idle-time=-1",
      "--use-pid-file=no",
      "-L",
      `module-native-protocol-unix socket=${socket} auth-anonymous=1`,
      "-L",
      "module-null-sink sink_name=e2e",
    ],
    { stdio: "inherit", env: { ...process.env, XDG_RUNTIME_DIR: dir, HOME: dir } },
  );
  await expect.poll(() => existsSync(socket) || pulse.exitCode !== null, { timeout: 10_000 }).toBe(true);
  expect(pulse.exitCode, "pulseaudio started").toBeNull();
  return {
    server: `unix:${socket}`,
    stop: async () => {
      if (pulse.exitCode === null) {
        pulse.kill("SIGTERM");
        await once(pulse, "exit");
      }
      rmSync(dir, { recursive: true, force: true });
    },
  };
}

// The speech itself is proven in Firefox through a null-sink PulseAudio: Chromium
// on the runner (dev2) never loads an AudioWorklet module — addModule stays pending
// without fetching the file, headless or headed, with or without a sound server
// (27.11.2026) — while Firefox loads it in ~150 ms. Every other step runs in Chromium.
test("the speech really plays through the AudioWorklet (Firefox)", async () => {
  const errors: string[] = [];
  const sink = await startNullSink();
  const browser = await firefox.launch({ env: { ...process.env, PULSE_SERVER: sink.server } });
  try {
    const context = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true });
    const page = await context.newPage();
    attachConsoleErrorCollector(page, errors);
    await page.goto(`${guestURL}?lang=de`);
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await page.locator('[data-role="guest-play"]').tap();
    const state = page.locator('[data-role="guest-state"]');
    await expect(state).toHaveAttribute("data-state", "waiting");
    const sender = startSender(await vbanPort(page), 60);
    try {
      await expect(state).toHaveAttribute("data-state", "live");
      // the worklet played the speech (samples it rendered), not only received it
      await expect.poll(() => bodyNumber(page, "data-guest-played")).toBeGreaterThan(4_800);
      expect(await bodyNumber(page, "data-guest-chunks")).toBeGreaterThan(2);
    } finally {
      await stopSender(sender);
    }
    await context.close();
  } finally {
    await browser.close();
    await sink.stop();
  }
  expect(errors).toEqual([]);
});

test("the guest listener serves only the guest page; the operator's QR page names it", async ({ browser, request }) => {
  const errors: string[] = [];
  for (const path of ["ui/operator", "ui/settings", "settings", "listen/status", "listen/mics", "ui/guest-qr"]) {
    expect((await request.get(guestURL + path)).status(), path).toBe(404);
  }
  expect((await request.get(guestURL + "manifest.json")).ok()).toBe(true);
  // the normal host's root is still the home page, never the guest page
  const guestRoot = await (await request.get(guestURL)).text();
  const normalRoot = await (await request.get(url("/"))).text();
  expect(guestRoot).toContain("data-wasm-ready");
  expect(normalRoot).not.toEqual(guestRoot);
  expect((await request.get(url("/manifest.json"))).status()).toBe(404);

  const page = await openPage(browser, { viewport: { width: 1280, height: 800 } }, "/ui/guest-qr", errors);
  await expect(page.locator('[data-role="guest-qr-code"] svg')).toBeVisible();
  await expect(page.locator('[data-role="guest-qr-url"]')).toHaveText(`https://${GUEST_HOST}/`);
  await page.context().close();
  expect(errors).toEqual([]);
});
