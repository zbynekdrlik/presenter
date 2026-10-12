/**
 * #843 — the stream PLAYBACK element: a Google Drive folder of photos and
 * videos played in a loop inside the element's frame.
 *
 * The test server's Drive hosts (`PRESENTER_DRIVE_URL` +
 * `PRESENTER_DRIVE_DOWNLOAD_URL`) point at a tiny mock started in this spec:
 * it serves the keyless folder listing (`/embeddedfolderview`, Drive's
 * `flip-entry` markup) and the download endpoint (`/download`, HEAD + GET) with
 * the committed fixtures `tests/e2e/fixtures/playback/` (two 64×64 PNGs and a
 * 1 s 64×64 H.264 MP4). Nothing reaches Google.
 *
 * - Editor (default chromium project): add a playback element, an invalid link
 *   shows an inline hint, the folder link + seconds / sound / fit / transition
 *   save, the status line reaches "2 fotky / 1 video, synchronizované HH:MM",
 *   and Obnoviť lists the folder again.
 * - Output (`@video-codec` → real Chrome with H.264 and a strict autoplay
 *   policy): the loop plays photo → photo → video (to its `ended`) → the first
 *   photo again.
 *
 * Every test asserts a clean browser console last.
 */

import { readFileSync } from "fs";
import http from "http";
import path from "path";
import type { AddressInfo } from "net";
import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  attachEditorConsoleCollector,
  deriveTestConfig,
  refreshDevData,
  REPO_ROOT,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

const FIXTURES = path.join(REPO_ROOT, "tests", "e2e", "fixtures", "playback");
const FOLDER_ID = "1E2EPlaybackFolderAAAAAAAAAAAAAA";
const RED = "1E2ERedPhotoAAAAAAAAAAAAAAAAAAA1";
const BLUE = "1E2EBluePhotoAAAAAAAAAAAAAAAAAA2";
const CLIP = "1E2EClipVideoAAAAAAAAAAAAAAAAAA3";
const SUBFOLDER = "1E2ESubfolderAAAAAAAAAAAAAAAAAA4";
const PDF = "1E2EProgramPdfAAAAAAAAAAAAAAAAA5";
const FOLDER_LINK = `https://drive.google.com/drive/folders/${FOLDER_ID}?usp=sharing`;
const LAST_MODIFIED = "Mon, 12 Oct 2026 08:00:00 GMT";

type MockFile = { mime: string; name: string; bytes: Buffer };
const FILES: Record<string, MockFile> = {
  [RED]: { mime: "image/png", name: "red.png", bytes: readFileSync(path.join(FIXTURES, "red.png")) },
  [BLUE]: { mime: "image/png", name: "blue.png", bytes: readFileSync(path.join(FIXTURES, "blue.png")) },
  [CLIP]: { mime: "video/mp4", name: "clip.mp4", bytes: readFileSync(path.join(FIXTURES, "clip.mp4")) },
};

let server: ServerHandle | undefined;
let mockDrive: http.Server | undefined;
let baseURL = "";
/** Folder listings the mock answered (Obnoviť must add one). */
let listingRequests = 0;

/** One entry of the listing, in Drive's `flip-entry` markup. */
function fileEntry(id: string, mime: string, name: string): string {
  return (
    `<div class="flip-entry" id="entry-${id}" tabindex="0" role="link"><div class="flip-entry-info">` +
    `<a href="https://drive.google.com/file/d/${id}/view?usp=drive_web" target="_blank">` +
    `<div class="flip-entry-list-icon"><img src="https://drive-thirdparty.googleusercontent.com/16/type/${mime}" alt=""/></div>` +
    `<div class="flip-entry-title">${name}</div></a></div>` +
    `<div class="flip-entry-last-modified"><div>Oct 12</div></div></div>`
  );
}

/** red, a subfolder (skipped), blue, a PDF (skipped), the clip. */
function listingPage(): string {
  const folder =
    `<div class="flip-entry" id="entry-${SUBFOLDER}" tabindex="0" role="link"><div class="flip-entry-info">` +
    `<a href="https://drive.google.com/drive/folders/${SUBFOLDER}" target="_blank">` +
    `<div class="flip-entry-list-icon"><div aria-label="Folder" class="icon-color-1"></div></div>` +
    `<div class="flip-entry-title">Archív</div></a></div></div>`;
  return (
    '<!DOCTYPE html><html><head><title>Playback E2E</title></head><body class="flip-embedded">' +
    '<div id="flip-contents" class="flip-contents flip-list-view"><div class="flip-entries">' +
    fileEntry(RED, "image/png", "red.png") +
    folder +
    fileEntry(BLUE, "image/png", "blue.png") +
    fileEntry(PDF, "application/pdf", "program.pdf") +
    fileEntry(CLIP, "video/mp4", "clip.mp4") +
    "</div></div></body></html>"
  );
}

function startMockDrive(): Promise<http.Server> {
  const mock = http.createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    const id = url.searchParams.get("id") ?? "";
    if (req.method === "GET" && url.pathname === "/embeddedfolderview" && id === FOLDER_ID) {
      listingRequests += 1;
      res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      res.end(listingPage());
      return;
    }
    const file = FILES[id];
    if (url.pathname === "/download" && file && (req.method === "GET" || req.method === "HEAD")) {
      res.writeHead(200, {
        "content-type": file.mime,
        "content-length": String(file.bytes.length),
        "last-modified": LAST_MODIFIED,
        "accept-ranges": "bytes",
      });
      res.end(req.method === "GET" ? file.bytes : undefined);
      return;
    }
    res.writeHead(404, { "content-type": "text/html" });
    res.end("<html>not found</html>");
  });
  return new Promise((resolve) => mock.listen(0, "127.0.0.1", () => resolve(mock)));
}

test.beforeAll(async ({}, testInfo) => {
  mockDrive = await startMockDrive();
  const mockUrl = `http://127.0.0.1:${(mockDrive.address() as AddressInfo).port}`;
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  const previous = {
    list: process.env.PRESENTER_DRIVE_URL,
    download: process.env.PRESENTER_DRIVE_DOWNLOAD_URL,
  };
  process.env.PRESENTER_DRIVE_URL = mockUrl;
  process.env.PRESENTER_DRIVE_DOWNLOAD_URL = mockUrl;
  try {
    server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
  } finally {
    if (previous.list === undefined) delete process.env.PRESENTER_DRIVE_URL;
    else process.env.PRESENTER_DRIVE_URL = previous.list;
    if (previous.download === undefined) delete process.env.PRESENTER_DRIVE_DOWNLOAD_URL;
    else process.env.PRESENTER_DRIVE_DOWNLOAD_URL = previous.download;
  }
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
  const mock = mockDrive;
  if (mock) {
    await new Promise<void>((resolve) => mock.close(() => resolve()));
  }
  mockDrive = undefined;
});

// ── Shared REST helpers ────────────────────────────────────────────────────────

type Playlist = {
  folderId: string;
  items: { fileId: string; kind: string; name: string; url: string }[];
  syncedAt: string | null;
  error: string | null;
  syncing: boolean;
};

async function playlist(request: APIRequestContext): Promise<Playlist> {
  const resp = await request.get(
    `${baseURL}/stream/api/playback?url=${encodeURIComponent(FOLDER_LINK)}`,
  );
  expect(resp.ok(), `playlist -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()) as Playlist;
}

type ElementDef = { id: number; props: Record<string, any> & { kind: string } };
type OutputDef = { scenes: { id: number; elements: ElementDef[] }[] };

async function getDef(request: APIRequestContext, slug: string): Promise<OutputDef> {
  const res = await request.get(`${baseURL}/stream/api/outputs/${slug}/def`);
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as OutputDef;
}

// ── Editor ────────────────────────────────────────────────────────────────────

const sel = {
  editor: '[data-role="stream-editor"]',
  addName: '[data-role="stream-add-name"]',
  addKind: '[data-role="stream-add-kind"]',
  addSubmit: '[data-role="stream-add-submit"]',
  scene: '[data-role="stream-scene"]',
  save: '[data-role="stream-prop-save"]',
  propForm: '[data-role="stream-prop-form"]',
  link: '[data-role="stream-playback-link"]',
  linkError: '[data-role="stream-playback-link-error"]',
  status: '[data-role="stream-playback-status"]',
  refresh: '[data-role="stream-playback-refresh"]',
};

async function openEditor(page: Page) {
  await page.goto(`${baseURL}/ui/stream`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector(sel.editor, { timeout: 30_000 });
}

async function addScene(page: Page, name: string): Promise<string> {
  await page.locator(sel.addName).fill(name);
  await page.locator(sel.addKind).selectOption("base");
  await page.locator(sel.addSubmit).click();
  const card = page.locator(sel.scene).filter({ has: page.getByText(name, { exact: true }) });
  await expect(card).toHaveCount(1, { timeout: 15_000 });
  const id = await card.getAttribute("data-scene-id");
  expect(id, "new scene has a data-scene-id").toBeTruthy();
  return id as string;
}

/** Add a playback element through the panel; returns its id once the page
 *  selected it and seeded its draft. */
async function addPlayback(page: Page, sceneId: string): Promise<string> {
  await page
    .locator(`${sel.scene}[data-scene-id="${sceneId}"] [data-role="stream-scene-edit"]`)
    .click();
  await page.waitForSelector('[data-role="stream-element-panel"]', { timeout: 15_000 });
  await page.locator('[data-role="stream-add-element-playback"]').click();
  let newId = "";
  await expect
    .poll(async () => {
      const scene = (await getDef(page.request, "stream")).scenes.find(
        (s) => String(s.id) === sceneId,
      );
      newId = scene?.elements[0] ? String(scene.elements[0].id) : "";
      return newId;
    })
    .not.toBe("");
  await expect(
    page.locator(`[data-role="stream-overlay-element"][data-element-id="${newId}"]`),
  ).toHaveAttribute("data-selected", "true", { timeout: 15_000 });
  await page.waitForSelector(sel.propForm, { timeout: 10_000 });
  return newId;
}

test("#843 editor: a playback element takes a Drive folder link and shows its sync status", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);
  const sceneId = await addScene(page, "SC_843_Playback");
  const elementId = await addPlayback(page, sceneId);

  // The row is labelled and the new element has no link yet: no status line.
  await expect(
    page.locator(`[data-role="stream-element"][data-element-id="${elementId}"]`),
  ).toHaveAttribute("data-kind", "playback");
  await expect(page.locator(sel.link)).toHaveValue("");
  await expect(page.locator(sel.status)).toHaveCount(0);

  // A link that is not a Drive folder is explained inline — nothing is sent.
  await page.locator(sel.link).fill("https://example.com/photos");
  await expect(page.locator(sel.linkError)).toContainText("Google Drive");
  await expect(page.locator(sel.status)).toHaveCount(0);
  await page
    .locator(sel.link)
    .fill(`https://drive.google.com/file/d/${RED}/view?usp=sharing`);
  await expect(page.locator(sel.linkError)).toContainText("jeden súbor");

  // The folder link: the hint goes away, the status line appears.
  await page.locator(sel.link).fill(FOLDER_LINK);
  await expect(page.locator(sel.linkError)).toHaveCount(0);
  await expect(page.locator(sel.status)).toBeVisible();

  await page.locator('[data-role="stream-playback-seconds"]').fill("7");
  await page.locator('[data-role="stream-playback-muted"]').uncheck();
  await page.locator('[data-role="stream-playback-fit"]').selectOption("cover");
  await page.locator('[data-role="stream-transition-cut"]').check();
  await page.locator(sel.save).click();
  await expect(page.locator(sel.save)).toHaveAttribute("data-dirty", "false", {
    timeout: 15_000,
  });

  // Saved as typed.
  const def = await getDef(page.request, "stream");
  const saved = def.scenes
    .find((s) => String(s.id) === sceneId)
    ?.elements.find((e) => String(e.id) === elementId);
  expect(saved?.props).toMatchObject({
    kind: "playback",
    source_url: FOLDER_LINK,
    image_seconds: 7,
    muted: false,
    fit: "cover",
    content_transition: { mode: "cut" },
  });

  // The save triggers a sync of the new folder; the status line follows it.
  const status = page.locator(sel.status);
  await expect(status).toHaveAttribute("data-state", "synced", { timeout: 60_000 });
  await expect(status).toContainText(/2 fotky \/ 1 video, synchronizované \d\d:\d\d/);
  const server = await playlist(page.request);
  expect(server.items.map((i) => i.name)).toEqual(["red.png", "blue.png", "clip.mp4"]);

  // Obnoviť lists the folder again and settles back on "synced".
  const before = listingRequests;
  await page.locator(sel.refresh).click();
  await expect.poll(() => listingRequests, { timeout: 30_000 }).toBeGreaterThan(before);
  await expect(status).toHaveAttribute("data-state", "synced", { timeout: 30_000 });

  expect(errors).toEqual([]);
});

// ── Output loop (real Chrome: H.264 + strict autoplay policy) ───────────────────

const LOOP_SLUG = "t843-loop";

/** Seed an own output whose base scene holds one playback element: photos 1 s,
 *  hard cuts, muted. Returns once the mirror holds all three items. */
async function seedLoopOutput(request: APIRequestContext): Promise<void> {
  const created = await request.post(`${baseURL}/stream/api/outputs`, {
    data: { slug: LOOP_SLUG, name: "Playback loop" },
  });
  expect(created.ok(), `create output -> ${created.status()}`).toBeTruthy();
  const scene = await request.post(`${baseURL}/stream/api/outputs/${LOOP_SLUG}/scenes`, {
    data: { name: "Loop", kind: "base" },
  });
  expect(scene.ok(), `create scene -> ${scene.status()}`).toBeTruthy();
  const sceneId = (await scene.json()).id as number;
  const element = await request.post(`${baseURL}/stream/api/scenes/${sceneId}/elements`, {
    data: {
      kind: "playback",
      source_url: FOLDER_LINK,
      fit: "contain",
      frame: { xPct: 10, yPct: 10, wPct: 40, hPct: 40 },
      opacity: 1,
      image_seconds: 1,
      muted: true,
      content_transition: { mode: "cut" },
    },
  });
  expect(element.ok(), `create element -> ${element.status()}`).toBeTruthy();
  const active = await request.put(`${baseURL}/stream/api/outputs/${LOOP_SLUG}/active-scene`, {
    data: { sceneId },
  });
  expect(active.ok(), `activate -> ${active.status()}`).toBeTruthy();
  const refresh = await request.post(`${baseURL}/stream/api/playback/refresh`, {
    data: { url: FOLDER_LINK },
  });
  expect(refresh.status()).toBe(202);
  await expect
    .poll(async () => (await playlist(request)).items.length, { timeout: 60_000 })
    .toBe(3);
}

/** Record, from the first byte of the page: every playback layer mounted (by
 *  file id, in order) and every media `ended` / `error`. */
async function installLoopRecorder(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const rec = { shown: [] as string[], ended: [] as string[], errors: [] as string[] };
    (window as unknown as { __playback: typeof rec }).__playback = rec;
    const LAYER = '[data-role="stream-playback-layer"]';
    new MutationObserver((records) => {
      for (const record of records) {
        record.addedNodes.forEach((node) => {
          if (!(node instanceof Element)) return;
          const layers = node.matches(LAYER) ? [node] : Array.from(node.querySelectorAll(LAYER));
          for (const layer of layers) rec.shown.push((layer as HTMLElement).dataset.fileId ?? "");
        });
      }
    }).observe(document, { childList: true, subtree: true });
    const media = (event: Event) => {
      const target = event.target as HTMLElement | null;
      return target?.dataset?.role === "stream-playback-media" ? target.dataset.fileId ?? "" : null;
    };
    document.addEventListener("ended", (event) => {
      const id = media(event);
      if (id !== null) rec.ended.push(id);
    }, true);
    document.addEventListener("error", (event) => {
      const id = media(event);
      if (id !== null) rec.errors.push(id);
    }, true);
  });
}

type LoopRecord = { shown: string[]; ended: string[]; errors: string[] };

async function loopRecord(page: Page): Promise<LoopRecord> {
  return page.evaluate(
    () => (window as unknown as { __playback: LoopRecord }).__playback,
  );
}

/** `true` when `shown` holds red → blue → clip → red back to back. */
function loopedOnce(shown: string[]): boolean {
  for (let i = 0; i + 3 < shown.length; i += 1) {
    if (shown[i] === RED && shown[i + 1] === BLUE && shown[i + 2] === CLIP && shown[i + 3] === RED) {
      return true;
    }
  }
  return false;
}

test("#843 output: the loop plays photo, photo, video to its end, then the first photo again @video-codec", async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await seedLoopOutput(page.request);
  await installLoopRecorder(page);

  await page.goto(`${baseURL}/stream/${LOOP_SLUG}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  const element = page.locator('[data-role="stream-element-playback"]');
  await expect(element).toHaveAttribute("data-item-count", "3", { timeout: 30_000 });

  // A photo really rendered (served from the mirror, decoded at 64 px).
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const img = document.querySelector<HTMLImageElement>(
            'img[data-role="stream-playback-media"]',
          );
          return img?.complete ? img.naturalWidth : 0;
        }),
      { timeout: 15_000 },
    )
    .toBe(64);

  // One full loop, the clip played to its `ended`.
  await expect
    .poll(async () => loopedOnce((await loopRecord(page)).shown), { timeout: 60_000 })
    .toBe(true);
  const record = await loopRecord(page);
  expect(record.ended).toContain(CLIP);
  expect(record.errors).toEqual([]);
  // Hard cuts: never more than one layer on screen.
  await expect(page.locator('[data-role="stream-playback-layer"]')).toHaveCount(1);

  expect(errors).toEqual([]);
});
