import { test, expect, type Browser, type BrowserContextOptions, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import path from "node:path";
import {
  REPO_ROOT,
  PHONE_390,
  TABLET_1180,
  assertNoHorizontalOverflow,
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";
import { startFakeSoniox, type FakeSoniox } from "./fake-soniox";

// #852 C3-9 — the guest's live English transcript, end to end, against a
// scripted fake Soniox (tests/e2e/fake-soniox.ts; the real service is never
// reached). The test server dials the fake through PRESENTER_LISTEN_SONIOX_URL
// with the key `e2e-test-key-not-real`. A real browser switches the paid
// transcription on in Settings → Počúvanie → "Prepis hosťa" (switch, guest mic
// select, daily cap; the status endpoint reflects it), presses "Spustiť teraz",
// and the real VBAN sender (scripts/dev/listen/vban_send.py) plays a voiced tone
// as the guest: on /ui/interpreter (tablet, touch) the live tail shows the
// fixture's words and the closed sentence appears; the fake saw the Bearer key,
// the measured config and NO `api_key`. An outage of the fake shows the C-S7
// banner within 3 s and the words come back after it; scrolling the column up
// shows "▼ Naživo · N nové", a tap returns to the bottom. The fake must never
// receive a WebSocket ping. The no-key warning is pinned in
// settings-listen.spec.ts (its server has no key).

const KEY = "e2e-test-key-not-real";
const GUEST_STREAM = "E2E_GUEST";
const GUEST = {
  id: "guest",
  name: "Hosť (test)",
  streamName: GUEST_STREAM,
  senderIp: null,
  channel: "left",
  language: "en",
  role: "guest",
};
const FIXTURE = path.join(REPO_ROOT, "tests", "e2e", "fixtures", "listen", "soniox-basic.jsonl");
const FIRST_SENTENCE = "Turn with me to the book of James.";
const OUTAGE_BANNER = "Služba prepisu nedostupná — skúšam znova";

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let fake: FakeSoniox;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  fake = await startFakeSoniox(FIXTURE);
  process.env.PRESENTER_LISTEN_SONIOX_URL = fake.url;
  process.env.PRESENTER_LISTEN_SONIOX_API_KEY = KEY;
  try {
    serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
  } finally {
    delete process.env.PRESENTER_LISTEN_SONIOX_URL;
    delete process.env.PRESENTER_LISTEN_SONIOX_API_KEY;
  }
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
  await fake?.stop();
});

test.beforeEach(async ({ request }) => {
  expect((await request.put(url("/listen/session"), { data: { mode: "auto" } })).ok()).toBe(true);
  const off = { enabled: false, guestMic: null, dailyCapMinutes: 240 };
  expect((await request.put(url("/listen/transcription"), { data: off })).ok()).toBe(true);
  expect((await request.put(url("/listen/mics"), { data: [GUEST] })).ok()).toBe(true);
});

function url(p: string): string {
  return new URL(p, baseURL).toString();
}

type Transcription = {
  keyConfigured: boolean;
  model: string;
  setting: { enabled: boolean; guestMic: string | null; dailyCapMinutes: number };
  mode: string;
  mics: { micId: string; state: string; reason: string | null }[];
};

async function transcription(page: Page): Promise<Transcription> {
  const reply = await page.request.get(url("/listen/status"));
  expect(reply.ok()).toBe(true);
  return (await reply.json()).transcription;
}

/** The VBAN socket's port, once it is bound (the mic list was just saved). */
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

async function openPage(
  browser: Browser,
  profile: BrowserContextOptions,
  route: string,
  errors: string[],
): Promise<Page> {
  const context = await browser.newContext({ ...profile, baseURL });
  const page = await context.newPage();
  attachConsoleErrorCollector(page, errors);
  await page.goto(route);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  return page;
}

/** A voiced 1 kHz tone as the guest; `seconds` only bounds a leaked process. */
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

test("the owner switches the guest transcription on in Settings", async ({ browser }) => {
  const errors: string[] = [];
  const page = await openPage(browser, { viewport: { width: 1400, height: 900 } }, "/ui/settings", errors);
  const section = page.locator('[data-role="listen-transcription"]');
  await expect(section).toBeVisible();
  const toggle = section.locator('[data-role="listen-transcription-toggle"]');
  await expect(toggle).toBeEnabled();
  await expect(toggle).not.toBeChecked();
  await expect(section.locator('[data-role="listen-transcription-line"]')).toContainText(
    "Prepis je vypnutý",
  );
  await expect(section.locator('[data-role="listen-transcription-key"]')).toHaveCount(0);

  await toggle.check();
  await expect.poll(async () => (await transcription(page)).setting.enabled).toBe(true);
  await section.locator('[data-role="listen-guest-mic"]').selectOption({ label: "Hosť (test)" });
  await expect.poll(async () => (await transcription(page)).setting.guestMic).toBe("guest");
  const cap = section.locator('[data-role="listen-transcription-cap"]');
  await cap.fill("30");
  await cap.press("Enter");
  await expect.poll(async () => (await transcription(page)).setting).toEqual({
    enabled: true,
    guestMic: "guest",
    dailyCapMinutes: 30,
  });
  const status = await transcription(page);
  expect(status.keyConfigured).toBe(true);
  expect(status.model).toBe("stt-rt-v6");

  // auto runs at every service (#852 T1): armed, it connects on the first
  // voiced frame; the three choices show today's mode
  const line = section.locator('[data-role="listen-transcription-line"]');
  await expect(section.locator('[data-role="listen-transcription-state"]')).toHaveAttribute(
    "data-state",
    "armed",
  );
  await expect(line).toContainText("/ 30 min dnes");
  const auto = section.locator('[data-role="listen-session-auto"]');
  await expect(auto).toHaveAttribute("aria-pressed", "true");
  await section.locator('[data-role="listen-session-off"]').click();
  await expect(section.locator('[data-role="listen-session-off"]')).toHaveAttribute("data-state", "on");
  await expect(auto).toHaveAttribute("data-state", "off");
  expect((await transcription(page)).mode).toBe("off");
  await expect(line).toContainText("Prepis je dnes zastavený");
  await auto.click();
  await expect.poll(async () => (await transcription(page)).mode).toBe("auto");

  // a cap out of range is not sent: the field goes back to what is stored
  await cap.fill("5");
  await cap.press("Enter");
  await expect(cap).toHaveValue("30");
  expect((await transcription(page)).setting.dailyCapMinutes).toBe(30);

  // the same section fits a 360-390 px phone
  const phone = await openPage(browser, PHONE_390, "/ui/settings", errors);
  await expect(phone.locator('[data-role="listen-transcription"]')).toBeVisible();
  await expect(phone.locator('[data-role="listen-transcription-toggle"]')).toBeChecked();
  const fit = await phone.evaluate(async () => {
    await document.fonts.ready;
    const layout = document.querySelector(".settings-layout");
    if (!layout) throw new Error("no .settings-layout");
    return { scrollWidth: layout.scrollWidth, clientWidth: layout.clientWidth };
  });
  expect(fit.scrollWidth).toBeLessThanOrEqual(fit.clientWidth);

  await page.context().close();
  await phone.context().close();
  expect(errors).toEqual([]);
});

test("the interpreter reads the guest live, through an outage, and can look back", async ({ browser }) => {
  const errors: string[] = [];
  const setting = { enabled: true, guestMic: "guest", dailyCapMinutes: 30 };
  const owner = await openPage(browser, { viewport: { width: 1400, height: 900 } }, "/ui/settings", errors);
  const switchedOff = { ...setting, enabled: false };
  expect((await owner.request.put(url("/listen/transcription"), { data: switchedOff })).ok()).toBe(true);
  const tablet = await openPage(browser, TABLET_1180, "/ui/interpreter", errors);
  const grid = tablet.locator('[data-role="interpreter-grid"]');
  // nothing runs while the switch is off: the tips keep the whole page
  await expect(grid).toHaveAttribute("data-transcript", "absent");
  expect((await owner.request.put(url("/listen/transcription"), { data: setting })).ok()).toBe(true);
  // auto runs at every service (#852 T1): the session is armed, the column is there
  await expect(grid).toHaveAttribute("data-transcript", "present");

  await owner.locator('[data-role="listen-session-on"]').click();
  await expect.poll(async () => (await transcription(owner)).mode).toBe("on");
  await expect(grid).toHaveAttribute("data-transcript", "present");
  const column = tablet.locator('[data-role="transcript"]');
  const sentences = column.locator('[data-role="transcript-sentence"]');
  const sender = startSender(await vbanPort(owner), 170);
  try {
    // the live words first, then the closed sentence
    await expect(column.locator('[data-role="transcript-tail"] [data-partial]')).not.toBeEmpty();
    await expect(sentences.filter({ hasText: FIRST_SENTENCE }).first()).toBeVisible();
    await expect(owner.locator('[data-role="listen-transcription-state"]')).toHaveAttribute(
      "data-state",
      "streaming",
    );

    // what Soniox was sent: the key in the header only, the measured config
    expect(fake.authorizations[0]).toBe(`Bearer ${KEY}`);
    expect(fake.userAgents[0]).toMatch(/^presenter-listen\//);
    const config = fake.configs[0];
    expect(config).toMatchObject({
      model: "stt-rt-v6",
      audio_format: "pcm_s16le",
      sample_rate: 16000,
      num_channels: 1,
      language_hints: ["sk", "en"],
      enable_language_identification: true,
    });
    expect(config).not.toHaveProperty("api_key");
    const terms = (config.context as { terms: string[] }).terms;
    expect(terms).toEqual(expect.arrayContaining(["Habakkuk", "James"]));

    // an outage: the banner within 3 s (C-S7), the words again after it
    await fake.kill();
    const banner = column.locator('[data-role="transcript-banner"]');
    // C-S7: the banner within 3 s — a delivery bound, read without back-off
    await expect
      .poll(async () => ((await banner.count()) > 0 ? banner.textContent() : null), {
        timeout: 3_000,
        intervals: [25],
      })
      .toBe(OUTAGE_BANNER);
    await fake.restart();
    const before = await sentences.count();
    // The session retries 1, 5, 20 s after the drop (listen-asr.md): the fake is
    // back before the 2nd retry, so recovery takes ≤ ~6 s, ≤ 26 s on a slow
    // runner that missed it — hence 40 s, not the config's expect timeout.
    await expect(banner).toHaveCount(0, { timeout: 40_000 });
    await expect.poll(async () => sentences.count(), { timeout: 40_000 }).toBeGreaterThan(before);

    // enough text to scroll: look back, new sentences are counted, a tap returns
    const list = column.locator('[data-role="transcript-list"]');
    // ~1.5 sentences per second of audio; the column holds ~17 one-line ones
    await expect
      .poll(async () => list.evaluate((el) => el.scrollHeight - el.clientHeight), { timeout: 30_000 })
      .toBeGreaterThan(200);
    await list.evaluate((el) => {
      el.scrollTop = 0;
    });
    const live = column.locator('[data-role="transcript-live"]');
    await expect(live).toBeVisible();
    await expect(live).toHaveText(/^▼ Naživo · [1-9]\d* nové$/);
    await live.tap();
    await expect(live).toHaveCount(0);
    await expect
      .poll(async () => list.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight))
      .toBeLessThanOrEqual(24);
    await assertNoHorizontalOverflow(tablet);

    // a phone: the transcript above the tips, nothing past the right edge
    const phone = await openPage(browser, PHONE_390, "/ui/interpreter", errors);
    await expect(phone.locator('[data-role="interpreter-grid"]')).toHaveAttribute(
      "data-transcript",
      "present",
    );
    await expect(phone.locator('[data-role="transcript-sentence"]').first()).toBeVisible();
    await assertNoHorizontalOverflow(phone);
    await phone.context().close();
  } finally {
    await stopSender(sender);
  }
  await owner.locator('[data-role="listen-session-off"]').click();
  await expect.poll(async () => (await transcription(owner)).mode).toBe("off");
  expect(fake.pings).toBe(0);
  await owner.context().close();
  await tablet.context().close();
  expect(errors).toEqual([]);
});
