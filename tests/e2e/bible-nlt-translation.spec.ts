/**
 * #826 — Bible: the English Living translation (NLT, Tyndale) is selectable.
 *
 * `eng-nlt` is served on demand from Tyndale's NLT API, never stored. This
 * spec points the test server at a tiny mock of that API
 * (`PRESENTER_NLT_API_URL`) that answers in the API's real markup — chapter
 * and section headings, a footnote marker + body — with SYNTHETIC words, so
 * no NLT text lands in the repo and nothing egresses to api.nlt.to.
 *
 * Like an operator: SEB as main, NLT as secondary, load 1 Ján 1:1-3. The
 * slide must show the parsed NLT lines (no heading, no footnote) and the
 * English reference `1 John 1:1-3 (NLT)` (#824 composer fix), and the server
 * must have asked the API for `1Jn.1.1-3` with the anonymous key.
 */

import { test, expect, type Page } from "@playwright/test";
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
      res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      res.end(mockNltPage(url.searchParams.get("ref") ?? ""));
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

function collectConsoleMessages(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    if (msg.text().includes("crbug.com/981419")) return;
    messages.push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (err) => {
    messages.push(`[pageerror] ${err.message}`);
  });
  return messages;
}

test("NLT as secondary translation: parsed NLT lines and the English '(NLT)' reference", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);

  await page.goto(new URL("/ui/operator/bible", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  const main = page.locator('[data-role="main-translation"]');
  const secondary = page.locator('[data-role="secondary-translation"]');
  // The NLT is offered as main AND as secondary translation.
  await expect(main.locator('option[value="eng-nlt"]')).toHaveText(
    "New Living Translation (en)",
    { timeout: 15_000 },
  );
  await expect(secondary.locator('option[value="eng-nlt"]')).toHaveCount(1);

  await main.selectOption("slk-seb");
  await secondary.selectOption("eng-nlt");
  await expect(secondary).toHaveValue("eng-nlt");
  await expect(page.locator('[data-role="book-list"]')).toHaveAttribute(
    "data-books-translation",
    "slk-seb",
  );

  await page.locator('[data-role="book-item"][data-book-code="1JN"]').click();
  const chapter = page.locator('[data-role="chapter-input"]');
  await chapter.fill("1");
  await chapter.press("Tab");
  const verseStart = page.locator('[data-role="verse-start"]');
  await verseStart.fill("1");
  await verseStart.press("Tab");
  const verseEnd = page.locator('[data-role="verse-end"]');
  await verseEnd.fill("3");
  await verseEnd.press("Tab");
  await page.locator('[data-role="load-button"]').click();

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
