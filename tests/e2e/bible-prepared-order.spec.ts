// #874: the Bible Prepared tab — the operator's own order (drag by the handle,
// arrow keys on it) and the archive instead of deleting (archive → restore →
// delete for good). Every check reads the screen AND the REST list.
import {
  test,
  expect,
  type APIRequestContext,
  type Locator,
  type Page,
} from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ mode: "serial", timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

type Summary = { id: string; name: string };

const CARD = '[data-role="presentation-card"]';
const DRAG = '[data-role="presentation-drag"]';

function url(path: string): string {
  return new URL(path, baseURL).toString();
}

async function activeList(request: APIRequestContext): Promise<Summary[]> {
  const resp = await request.get(url("/bible/presentations"));
  expect(resp.ok()).toBeTruthy();
  return resp.json();
}

async function archivedList(request: APIRequestContext): Promise<Summary[]> {
  const resp = await request.get(url("/bible/presentations?archived=true"));
  expect(resp.ok()).toBeTruthy();
  return resp.json();
}

/** Archive every active presentation, so each test starts from an empty list. */
async function emptyActiveList(request: APIRequestContext): Promise<void> {
  for (const p of await activeList(request)) {
    const resp = await request.post(url(`/bible/presentations/${p.id}/archive`));
    expect(resp.status()).toBe(204);
  }
}

/** Create presentations; the newest goes on top, so create in reverse to get
 *  `names` top-down. Returns the ids in `names` order. */
async function createTopDown(request: APIRequestContext, names: string[]): Promise<string[]> {
  const ids: string[] = [];
  for (const name of [...names].reverse()) {
    const resp = await request.post(url("/bible/presentations"), { data: { name } });
    expect(resp.ok()).toBeTruthy();
    ids.unshift((await resp.json()).id);
  }
  return ids;
}

function collectConsole(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      messages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  page.on("pageerror", (err) => messages.push(`[pageerror] ${err.message}`));
  return messages;
}

async function openPrepared(page: Page): Promise<Locator> {
  await page.setViewportSize({ width: 1280, height: 1200 });
  await page.goto(url("/ui/operator"));
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.locator('[data-role="view-toggle"][data-view="bible"]').click();
  await expect(page.locator("body")).toHaveAttribute("data-view", "bible");
  const panel = page.locator('[data-view-panel="bible"]');
  await panel.locator('[data-role="bible-tab"][data-tab="prepared"]').click();
  return panel;
}

async function screenOrder(panel: Locator): Promise<string[]> {
  return panel
    .locator(`[data-role="presentations-list"] > ${CARD}`)
    .evaluateAll((cards) => cards.map((c) => c.getAttribute("data-presentation-id") ?? ""));
}

/** The REST list and the cards on screen both show `ids` in this order. */
async function expectOrder(
  panel: Locator,
  request: APIRequestContext,
  ids: string[],
): Promise<void> {
  await expect.poll(async () => (await activeList(request)).map((p) => p.id)).toEqual(ids);
  await expect.poll(() => screenOrder(panel)).toEqual(ids);
}

function card(panel: Locator, id: string): Locator {
  return panel.locator(`${CARD}[data-presentation-id="${id}"]`);
}

/** Drag card `id` by its handle with the mouse (real pointer events) to the
 *  height `toY`. */
async function mouseDrag(page: Page, panel: Locator, id: string, toY: number): Promise<void> {
  const handle = await card(panel, id).locator(DRAG).boundingBox();
  expect(handle).toBeTruthy();
  const x = handle!.x + handle!.width / 2;
  await page.mouse.move(x, handle!.y + handle!.height / 2);
  await page.mouse.down();
  await page.mouse.move(x, toY, { steps: 12 });
  await page.mouse.up();
}

async function edges(panel: Locator, id: string): Promise<{ top: number; bottom: number; middle: number }> {
  const box = await card(panel, id).boundingBox();
  expect(box).toBeTruthy();
  return { top: box!.y, bottom: box!.y + box!.height, middle: box!.y + box!.height / 2 };
}

test("an empty list shows the empty note and no drag handle", async ({ page, request }) => {
  const messages = collectConsole(page);
  await emptyActiveList(request);
  const panel = await openPrepared(page);
  await expect(panel.locator('[data-role="presentations-list"]')).toContainText(
    "Zatiaľ žiadne prezentácie.",
  );
  await expect(panel.locator(DRAG)).toHaveCount(0);
  expect(messages).toEqual([]);
});

test("a drag above the first, below the last and into the middle reorders and persists", async ({
  page,
  request,
}) => {
  const messages = collectConsole(page);
  await emptyActiveList(request);
  const [a, b, c, d] = await createTopDown(request, ["A 874", "B 874", "C 874", "D 874"]);
  const panel = await openPrepared(page);
  await expectOrder(panel, request, [a, b, c, d]);

  // C above the first.
  await mouseDrag(page, panel, c, (await edges(panel, a)).top + 2);
  await expectOrder(panel, request, [c, a, b, d]);

  // C below the last.
  await mouseDrag(page, panel, c, (await edges(panel, d)).bottom - 2);
  await expectOrder(panel, request, [a, b, d, c]);

  // A into the middle (between D and C).
  const between = ((await edges(panel, d)).middle + (await edges(panel, c)).middle) / 2;
  await mouseDrag(page, panel, a, between);
  await expectOrder(panel, request, [b, d, a, c]);

  // A drop on the card's own place sends nothing and keeps the order.
  let puts = 0;
  page.on("request", (r) => {
    if (r.method() === "PUT" && r.url().endsWith("/bible/presentations/order")) puts += 1;
  });
  await mouseDrag(page, panel, d, (await edges(panel, d)).middle);
  await expectOrder(panel, request, [b, d, a, c]);
  expect(puts).toBe(0);

  // The order survives a reload.
  await page.reload();
  const reloaded = await openPrepared(page);
  await expectOrder(reloaded, request, [b, d, a, c]);
  expect(messages).toEqual([]);
});

test("the arrow keys on a focused handle move the card step by step", async ({ page, request }) => {
  const messages = collectConsole(page);
  await emptyActiveList(request);
  const [a, b, c] = await createTopDown(request, ["A 874k", "B 874k", "C 874k"]);
  const panel = await openPrepared(page);
  await expectOrder(panel, request, [a, b, c]);

  await card(panel, c).locator(DRAG).focus();
  await page.keyboard.press("ArrowUp");
  await expectOrder(panel, request, [a, c, b]);
  await card(panel, c).locator(DRAG).focus();
  await page.keyboard.press("ArrowUp");
  await expectOrder(panel, request, [c, a, b]);
  await card(panel, a).locator(DRAG).focus();
  await page.keyboard.press("ArrowDown");
  await expectOrder(panel, request, [c, b, a]);
  expect(messages).toEqual([]);
});

test("archive instead of delete: archive, restore, delete for good", async ({ page, request }) => {
  const messages = collectConsole(page);
  await emptyActiveList(request);
  const [keep, sermon] = await createTopDown(request, ["Ostáva 874", "Kázeň 874"]);
  const panel = await openPrepared(page);
  await expectOrder(panel, request, [keep, sermon]);

  // The edit dialog archives; it has no delete any more.
  await card(panel, sermon).click();
  await expect(card(panel, sermon)).toHaveClass(/is-active/);
  await card(panel, sermon).locator('[data-role="presentation-edit"]').click();
  const modal = page.locator('[data-role="presentation-modal"]');
  await expect(modal).toBeVisible();
  await expect(modal.locator('[data-role="modal-delete"]')).toHaveCount(0);
  await modal.locator('[data-role="modal-archive"]').click();
  await expect(modal).toBeHidden();

  await expectOrder(panel, request, [keep]);
  await expect.poll(async () => (await archivedList(request)).map((p) => p.id)).toContain(sermon);
  const archive = panel.locator('[data-role="presentations-archive"]');
  await expect(archive.locator('[data-role="presentations-archive-toggle"]')).toContainText(
    /Archív \(\d+\)/,
  );
  await archive.locator('[data-role="presentations-archive-toggle"]').click();
  const archivedCard = archive.locator(
    `[data-role="archived-presentation"][data-presentation-id="${sermon}"]`,
  );
  await expect(archivedCard).toBeVisible();

  // Restore: back at the top of the active list.
  await archivedCard.locator('[data-role="archived-restore"]').click();
  await expectOrder(panel, request, [sermon, keep]);
  await expect(archivedCard).toHaveCount(0);
  expect((await archivedList(request)).map((p) => p.id)).not.toContain(sermon);

  // Archive again through the API (another device): the open page follows.
  expect((await request.post(url(`/bible/presentations/${sermon}/archive`))).status()).toBe(204);
  await expectOrder(panel, request, [keep]);
  await expect(archivedCard).toBeVisible();

  // Delete for good asks first; a cancel keeps it.
  page.once("dialog", (dialog) => dialog.dismiss());
  await archivedCard.locator('[data-role="archived-delete"]').click();
  await expect(archivedCard).toBeVisible();
  expect((await request.get(url(`/bible/presentations/${sermon}`))).status()).toBe(200);

  page.once("dialog", (dialog) => dialog.accept());
  await archivedCard.locator('[data-role="archived-delete"]').click();
  await expect(archivedCard).toHaveCount(0);
  await expect
    .poll(async () => (await request.get(url(`/bible/presentations/${sermon}`))).status())
    .toBe(404);
  await expectOrder(panel, request, [keep]);
  expect(messages).toEqual([]);
});
