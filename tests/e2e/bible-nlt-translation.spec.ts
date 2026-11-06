/**
 * #826 — Bible: the New Living Translation (NLT, Tyndale) is selectable.
 *
 * `eng-nlt` is served on demand from Tyndale's NLT API, never stored. This
 * spec points the test server at a tiny mock of that API
 * (`PRESENTER_NLT_API_URL`) that answers in the API's real markup — chapter
 * and section headings, a footnote marker + body — with SYNTHETIC words, so
 * no NLT text lands in the repo and nothing egresses to api.nlt.to. Refs in
 * `FAILING_REFS` answer HTTP 500, like an API outage.
 *
 * Like an operator:
 * - SEB main + NLT secondary, 1 Ján 1:1-3 → the parsed NLT lines (no heading,
 *   no footnote) and the English reference `1 John 1:1-3 (NLT)` (#824).
 * - SEB main + NLT secondary while the API fails → the SEB slides still load,
 *   without the English line, and a toast says "NLT nedostupné".
 * - NLT is never offered as main (#850 B2c); a browser that stored it keeps
 *   its chip, and while the API fails the console's toast carries the
 *   server's "NLT nedostupné" message (not just "Bad Gateway").
 */

import { test, expect, type ConsoleMessage, type Page } from "@playwright/test";
import http from "http";
import type { AddressInfo } from "net";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let mockNlt: http.Server | undefined;
let baseURL = "";
/** Query strings of every request the mock NLT API received. */
const nltRequests: URLSearchParams[] = [];
/** Refs the mock answers with HTTP 500 (an NLT API outage). */
const FAILING_REFS = new Set(["1Jn.1.4-6", "1Jn.1.7-9"]);

/**
 * A mock API page for `ref=<Book>.<chapter>.<start>[-<end>]`: the live
 * markup shape with synthetic verse text (`Mock NLT verse N continues.`).
 */
function mockNltPage(reference: string): string {
  const [, chapterPart = "1", rangePart = "1"] = reference.split(".");
  const [startPart, endPart = startPart] = rangePart.split("-");
  const chapter = Number(chapterPart);
  const start = Number(startPart);
  const end = Number(endPart);
  let html =
    '<div id="bibletext" class=" NLT NLT BibleText section"><section>' +
    `<h2 class="bk_ch_vs_header">Mock ${reference}, NLT</h2>`;
  for (let verse = start; verse <= end; verse += 1) {
    const headings =
      verse === 1
        ? `<h2 class="chapter-number"><span class="cw">1 John</span> ` +
          `<span class="cw_ch">${chapter}</span></h2>\n` +
          '<h3 class="subhead">Mock Heading</h3>\n'
        : "";
    html +=
      `<verse_export orig="joh1_${chapter}_${verse}" bk="joh1" ch="${chapter}" vn="${verse}">\n` +
      `${headings}<p class="body"><span class="vn">${verse}</span>Mock NLT verse ${verse}` +
      '<a class="a-tn">*</a><span class="tn">' +
      `<span class="tn-ref">${chapter}:${verse}</span> Mock <em>footnote</em>.</span>` +
      " continues.</p>\n</verse_export>";
  }
  return `${html}</section></div>`;
}

test.beforeAll(async ({}, testInfo) => {
  const mock = http.createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    if (req.method === "GET" && url.pathname === "/api/passages") {
      nltRequests.push(url.searchParams);
      const reference = url.searchParams.get("ref") ?? "";
      if (FAILING_REFS.has(reference)) {
        res.writeHead(500);
        res.end("mock NLT outage");
        return;
      }
      res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      res.end(mockNltPage(reference));
      return;
    }
    res.writeHead(404);
    res.end("not found");
  });
  mockNlt = mock;
  await new Promise<void>((resolve) => {
    mock.listen(0, "127.0.0.1", () => resolve());
  });
  const mockPort = (mock.address() as AddressInfo).port;

  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  const previous = process.env.PRESENTER_NLT_API_URL;
  process.env.PRESENTER_NLT_API_URL = `http://127.0.0.1:${mockPort}`;
  try {
    server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
  } finally {
    if (previous === undefined) delete process.env.PRESENTER_NLT_API_URL;
    else process.env.PRESENTER_NLT_API_URL = previous;
  }
  // One slide for the whole passage, so every line is on the first card.
  const prefs = await fetch(new URL("/bible/preferences", baseURL), {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ characterLimit: 2000 }),
  });
  expect(prefs.ok).toBe(true);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
  const mock = mockNlt;
  if (mock) {
    await new Promise<void>((resolve) => mock.close(() => resolve()));
  }
  mockNlt = undefined;
});

/**
 * Console errors/warnings of `page`. `allowed` matches the expected lines a
 * failure test provokes on purpose (Chrome logs every non-2xx fetch — one per
 * failed load, so the button load and the debounced auto-load each add one).
 */
function collectConsoleMessages(
  page: Page,
  allowed: (msg: ConsoleMessage) => boolean = () => false,
): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    if (msg.text().includes("crbug.com/981419")) return;
    if (allowed(msg)) return;
    messages.push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (err) => {
    messages.push(`[pageerror] ${err.message}`);
  });
  return messages;
}

async function openBible(page: Page): Promise<void> {
  await page.goto(new URL("/ui/operator/bible", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await expect(translationButton(page, "secondary", "eng-nlt")).toHaveCount(1, {
    timeout: 15_000,
  });
}

/** A button of the verse console's translation bar (#850 B2c). */
function translationButton(page: Page, kind: "main" | "secondary", code: string) {
  return page.locator(
    `[data-role="console-translation"][data-kind="${kind}"][data-translation="${code}"]`,
  );
}

/** Pick the translations, then look 1 John/Ján 1:<start>-<end> up in the console. */
async function loadFirstJohn(
  page: Page,
  translations: { main: string; secondary: string },
  verses: { start: number; end: number },
): Promise<void> {
  await translationButton(page, "main", translations.main).click();
  await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
    "data-books-translation",
    translations.main,
  );
  const secondary = translationButton(page, "secondary", translations.secondary);
  await secondary.click();
  await expect(secondary).toHaveAttribute("aria-pressed", "true");
  await page
    .locator('[data-role="console-field"]')
    .fill(`1jn 1:${verses.start}-${verses.end}`);
}

test("NLT as secondary translation: parsed NLT lines and the English '(NLT)' reference", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  await openBible(page);

  // The NLT is offered as the line below, never as the main translation
  // (remote, #826; owner decision 2026-10-13).
  await expect(translationButton(page, "secondary", "eng-nlt")).toHaveText("NLT");
  await expect(translationButton(page, "main", "eng-nlt")).toHaveCount(0);

  await loadFirstJohn(
    page,
    { main: "slk-seb", secondary: "eng-nlt" },
    { start: 1, end: 3 },
  );

  const card = page.locator('[data-role="slide-card"]').first();
  await expect(
    card.locator(".operator__slide-reference--secondary"),
  ).toHaveText("1 John 1:1-3 (NLT)", { timeout: 15_000 });
  await expect(card.locator(".operator__slide-reference").first()).toHaveText(
    /\(SEB\)$/,
  );
  const translation = card.locator(".operator__slide-text--translation");
  await expect(translation).toContainText("1. Mock NLT verse 1 continues.");
  await expect(translation).toContainText("3. Mock NLT verse 3 continues.");
  await expect(translation).not.toContainText("footnote");
  await expect(translation).not.toContainText("Mock Heading");

  // The server fetched the passage from the (mock) NLT API, anonymous key.
  const asked = nltRequests.some(
    (query) =>
      query.get("ref") === "1Jn.1.1-3" &&
      query.get("version") === "NLT" &&
      query.get("key") === "TEST",
  );
  expect(asked).toBe(true);

  expect(consoleMessages).toEqual([]);
});

test("NLT API down: the SEB slides still load and a toast says the NLT is unavailable", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  await openBible(page);

  await loadFirstJohn(
    page,
    { main: "slk-seb", secondary: "eng-nlt" },
    { start: 4, end: 6 },
  );

  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toContainText("NLT nedostupné", { timeout: 15_000 });
  // The degraded-load warning, not the failed-load toast.
  await expect(toast).toContainText("sekundárny preklad vynechaný");
  await expect(toast).not.toContainText("Verše sa nenačítali");
  const card = page.locator('[data-role="slide-card"]').first();
  await expect(card.locator(".operator__slide-reference").first()).toHaveText(
    /1:4-6 \(SEB\)$/,
  );
  await expect(card.locator(".operator__slide-text--main")).toContainText(
    "4. ",
  );
  await expect(card.locator(".operator__slide-text--translation")).toHaveCount(
    0,
  );
  await expect(
    card.locator(".operator__slide-reference--secondary"),
  ).toHaveCount(0);
  expect(nltRequests.some((query) => query.get("ref") === "1Jn.1.4-6")).toBe(
    true,
  );

  // The load itself succeeded (200 + warning), so the console stays clean.
  expect(consoleMessages).toEqual([]);
});

test("a stored NLT main with the API down: the preview's toast carries the server's reason", async ({
  page,
}) => {
  // NLT is never offered as main (#850 B2c), but a browser that stored it
  // before keeps its chip (#701). The 502 of /bible/resolve is the point of
  // this test; Chrome logs exactly that one failed load.
  const failedLoads: string[] = [];
  const consoleMessages = collectConsoleMessages(page, (msg) => {
    const is502 =
      msg.type() === "error" &&
      msg.text().includes("status of 502") &&
      msg.location().url.endsWith("/bible/resolve");
    if (is502) failedLoads.push(msg.text());
    return is502;
  });
  await page.addInitScript(() => {
    localStorage.setItem("presenter:bibleMainTranslation", JSON.stringify("eng-nlt"));
  });
  await openBible(page);
  await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
    "data-books-translation",
    "eng-nlt",
  );
  await expect(translationButton(page, "main", "eng-nlt")).toHaveAttribute("aria-pressed", "true");

  const answered = page.waitForResponse((res) => new URL(res.url()).pathname === "/bible/resolve");
  await page.locator('[data-role="console-field"]').fill("1jn 1:7-9");
  expect((await answered).status()).toBe(502);
  await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
    "data-preview-state",
    "error",
  );
  await expect(page.locator('[data-role="toast"]')).toContainText(
    "Náhľad sa nenačítal: HTTP 502: NLT nedostupné",
    { timeout: 15_000 },
  );
  expect(nltRequests.some((query) => query.get("ref") === "1Jn.1.7-9")).toBe(true);
  expect(failedLoads).toHaveLength(1);

  expect(consoleMessages).toEqual([]);
});
