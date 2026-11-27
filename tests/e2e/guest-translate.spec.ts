import { test, expect, type Browser, type BrowserContextOptions, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
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

// #999 L3 — a guest on a phone opens the guest hostname's root (`localhost`
// here: PRESENTER_GUEST_HOST; the normal tests use 127.0.0.1), picks English and taps Play: the
// translated speech plays (the AudioWorklet reports played samples) and the
// text appears, against a scripted fake Gemini Live (tests/e2e/fake-gemini.ts;
// Google is never reached). The preacher is a VBAN mic (`guest`) the real
// sender (scripts/dev/listen/vban_send.py) plays a voiced tone into; the
// server translates it because PRESENTER_LISTEN_GUEST_MIC names it. The guest
// hostname serves nothing else; the operator's QR page is on the normal host.

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

const GUEST_HOST = "localhost";

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let guestURL: string;
let fake: FakeGemini;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  guestURL = `http://${GUEST_HOST}:${new URL(baseURL).port}/`;
  await refreshDevData(config.dbUrl);
  fake = await startFakeGemini();
  process.env.PRESENTER_LISTEN_GEMINI_URL = fake.url;
  process.env.PRESENTER_LISTEN_GEMINI_API_KEY = KEY;
  process.env.PRESENTER_LISTEN_GUEST_MIC = PREACHER.id;
  process.env.PRESENTER_GUEST_HOST = GUEST_HOST;
  try {
    serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
  } finally {
    delete process.env.PRESENTER_LISTEN_GEMINI_URL;
    delete process.env.PRESENTER_LISTEN_GEMINI_API_KEY;
    delete process.env.PRESENTER_LISTEN_GUEST_MIC;
    delete process.env.PRESENTER_GUEST_HOST;
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
    // the worklet really played the speech (samples counted by the player)
    await expect.poll(() => bodyNumber(page, "data-guest-played")).toBeGreaterThan(4_800);
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

test("the guest hostname serves only the guest page; the operator's QR page names it", async ({ browser, request }) => {
  const errors: string[] = [];
  for (const path of ["ui/operator", "ui/settings", "listen/status", "listen/mics", "ui/guest-qr"]) {
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
