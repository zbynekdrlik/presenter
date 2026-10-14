/**
 * #852 šepkár tips — the interpreter tablet (`/ui/interpreter`) and the
 * šepkár phone page (`/ui/sepkar`), spec §15 / T-S1–T-S3. One server for the
 * whole file. Every test starts from an empty interpreter list (dismiss-all)
 * and uses its own sender names: the in-memory rate limit (5 / min per IP +
 * name, 30 / min overall) spans the file; the interpreter tests alone send
 * 10 tips.
 */
import {
  test,
  expect,
  type Browser,
  type BrowserContext,
  type BrowserContextOptions,
  type Page,
  type WebSocketRoute,
} from "@playwright/test";
import {
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  assertVersionLabel,
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  TABLET_1180,
  touchDrag,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test.beforeEach(async ({ request }) => {
  const res = await request.post(new URL("/interpreter/tips/dismiss-all", baseURL).toString());
  expect(res.ok()).toBeTruthy();
});

type Opened = { context: BrowserContext; page: Page; errors: string[] };

async function open(
  browser: Browser,
  profile: BrowserContextOptions,
  path: string,
  wsRole: string,
): Promise<Opened> {
  const context = await browser.newContext({ ...profile, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.goto(path);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(page.locator(`[data-role="${wsRole}"]`)).toHaveAttribute("data-ws", "connected", {
    timeout: 15_000,
  });
  return { context, page, errors };
}

/** A desktop browser with a mouse and no touch (the mouse-swipe path). */
const DESKTOP_1280 = { viewport: { width: 1280, height: 800 } };

/** The inline style of a card that is dragged to the left. */
const DRAGGED_LEFT = /translateX\(-[0-9.]+px\)/;

async function openInterpreter(
  browser: Browser,
  profile: BrowserContextOptions = TABLET_1180,
): Promise<Opened> {
  const opened = await open(browser, profile, "/ui/interpreter", "interpreter-ws");
  await expect(opened.page.locator('[data-role="interpreter-tips"]')).toHaveAttribute(
    "data-ready",
    "true",
  );
  return opened;
}

const cards = (page: Page) => page.locator('[data-role="interpreter-tip"]');
const card = (page: Page, text: string) => cards(page).filter({ hasText: text });

async function apiTip(page: Page, body: Record<string, string>): Promise<number> {
  const res = await page.request.post(new URL("/interpreter/tips", baseURL).toString(), {
    data: body,
  });
  expect(res.status(), JSON.stringify(body)).toBe(201);
  return (await res.json()).id as number;
}

test("an empty tablet says so, the TV layout is not guessed, the version shows", async ({
  browser,
}) => {
  const interp = await openInterpreter(browser);
  const { page } = interp;
  await expect(page.locator('[data-role="interpreter-tips-empty"]')).toBeVisible();
  await expect(page.locator('[data-role="interpreter-grid"]')).toHaveAttribute(
    "data-transcript",
    "absent",
  );
  await expect(page.locator('[data-role="interpreter-tips-dismiss-all"]')).toBeDisabled();
  await assertVersionLabel(page, baseURL);

  await page.goto("/ui/interpreter?layout=tv");
  await page.waitForSelector('body[data-wasm-ready="true"]');
  await expect(page.locator('[data-role="interpreter"]')).toHaveAttribute("data-layout", "tv");
  await expect(page.locator('[data-role="interpreter-tv-pending"]')).toBeVisible();
  expect(interp.errors).toEqual([]);
  await interp.context.close();
});

test("✕ hides a tip at once, Späť brings it back, the offer closes after 5 s", async ({
  browser,
}) => {
  const interp = await openInterpreter(browser);
  const { page } = interp;
  await apiTip(page, { senderName: "Krížik", sourceTerm: "grace", targetTerm: "milosť" });
  const tip = card(page, "grace");
  await expect(tip).toBeVisible();
  await expect(tip.locator('[data-role="interpreter-tip-pair"]')).toHaveText("grace → milosť");
  await expect(tip.locator('[data-role="interpreter-tip-meta"]')).toHaveText(
    /^Krížik · \d{2}:\d{2}$/,
  );
  await expect(tip).toHaveAttribute("data-new", "true");

  await tip.locator('[data-role="interpreter-tip-dismiss"]').click();
  await expect(tip).toHaveCount(0);
  const undo = page.locator('[data-role="interpreter-undo"]');
  await expect(undo).toContainText("Skryté");
  await undo.locator('[data-role="interpreter-undo-button"]').click();
  await expect(tip).toBeVisible();
  await expect(undo).toHaveCount(0);

  await tip.locator('[data-role="interpreter-tip-dismiss"]').click();
  await expect(undo).toBeVisible();
  await expect(undo).toHaveCount(0, { timeout: 7_000 });
  await expect(tip).toHaveCount(0);
  const active = await (
    await page.request.get(new URL("/interpreter/tips", baseURL).toString())
  ).json();
  expect(active.tips).toEqual([]);
  expect(interp.errors).toEqual([]);
  await interp.context.close();
});

test("a left swipe dismisses a tip on the touch tablet, a short one snaps back", async ({
  browser,
}) => {
  const interp = await openInterpreter(browser);
  const { page } = interp;
  await apiTip(page, { senderName: "Ťahač", sourceTerm: "redemption", targetTerm: "vykúpenie" });
  const tip = card(page, "redemption");
  await expect(tip).toBeVisible();
  const box = await tip.boundingBox();
  if (!box) throw new Error("the tip card has no box");
  const start = { x: box.x + box.width / 2, y: box.y + box.height / 2 };

  // Mid-drag the card follows the finger to the left; the check runs while the
  // finger is still down, so it cannot pass on a card that never moved.
  await touchDrag(page, start, { x: start.x - 40, y: start.y }, 8, async () => {
    await expect(tip).toHaveAttribute("style", DRAGGED_LEFT);
    await expect(tip).toHaveAttribute("data-dragging", "true");
  });
  await expect(tip).toBeVisible();
  await expect(tip).toHaveAttribute("style", /translateX\(0px\)/);
  await expect(tip).toHaveAttribute("data-dragging", "false");

  await touchDrag(page, start, { x: start.x - 220, y: start.y });
  await expect(tip).toHaveCount(0);
  await expect(page.locator('[data-role="interpreter-undo"]')).toBeVisible();
  expect(interp.errors).toEqual([]);
  await interp.context.close();
});

test("a mouse drag swipes a tip away even past the card edge, a short one snaps back", async ({
  browser,
}) => {
  const interp = await openInterpreter(browser, DESKTOP_1280);
  const { page } = interp;
  await apiTip(page, { senderName: "Myšiak", sourceTerm: "mercy", targetTerm: "zľutovanie" });
  const tip = card(page, "mercy");
  await expect(tip).toBeVisible();
  const box = await tip.boundingBox();
  if (!box) throw new Error("the tip card has no box");
  const y = box.y + box.height / 2;
  // 100 px inside the card's left edge: a 110 px drag ends OUTSIDE the card,
  // so only pointer capture keeps the swipe alive (and the mouse on screen).
  const startX = box.x + 100;
  expect(startX - 110, "the drag must stay on screen").toBeGreaterThanOrEqual(0);
  expect(startX - 110, "the drag must end outside the card").toBeLessThan(box.x);

  await page.mouse.move(startX, y);
  await page.mouse.down();
  await page.mouse.move(startX - 40, y, { steps: 8 });
  await expect(tip).toHaveAttribute("style", DRAGGED_LEFT);
  await page.mouse.up();
  await expect(tip).toBeVisible();
  await expect(tip).toHaveAttribute("style", /translateX\(0px\)/);

  await page.mouse.move(startX, y);
  await page.mouse.down();
  await page.mouse.move(startX - 110, y, { steps: 8 });
  await page.mouse.up();
  await expect(tip).toHaveCount(0);
  await expect(page.locator('[data-role="interpreter-undo"]')).toBeVisible();
  expect(interp.errors).toEqual([]);
  await interp.context.close();
});

test("Späť pressed while the hide is still in flight restores the tip after it, and a list that arrives meanwhile does not bring it back", async ({
  browser,
}) => {
  const interp = await openInterpreter(browser);
  const { page } = interp;
  await apiTip(page, { senderName: "Pomalý", sourceTerm: "patience", targetTerm: "trpezlivosť" });
  const tip = card(page, "patience");
  await expect(tip).toBeVisible();

  // Hold the dismiss POST: the server still has the tip active meanwhile.
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/interpreter/tips/*/dismiss", async (route) => {
    await gate;
    await route.continue();
  });
  await tip.locator('[data-role="interpreter-tip-dismiss"]').click();
  await expect(tip).toHaveCount(0);

  // A list published now still carries the tip: it must stay hidden.
  await apiTip(page, { senderName: "Pomalý", note: "druhý tip počas čakania" });
  await expect(card(page, "druhý tip počas čakania")).toBeVisible();
  await expect(tip).toHaveCount(0);

  // Späť before the POST answered: the restore may only go out after it, or
  // it would be a no-op and the dismiss would then win.
  await page.locator('[data-role="interpreter-undo-button"]').click();
  release();
  await expect(tip).toBeVisible({ timeout: 10_000 });
  const active = (await (
    await page.request.get(new URL("/interpreter/tips", baseURL).toString())
  ).json()) as { tips: { sourceTerm: string | null }[] };
  expect(active.tips.map((entry) => entry.sourceTerm)).toContain("patience");
  expect(interp.errors).toEqual([]);
  await interp.context.close();
});

test("Skryť všetky asks first, then hides every tip", async ({ browser }) => {
  const interp = await openInterpreter(browser);
  const { page } = interp;
  await apiTip(page, { senderName: "Všetky", note: "prvý tip" });
  await apiTip(page, { senderName: "Všetky", note: "druhý tip" });
  await expect(cards(page)).toHaveCount(2);
  const button = page.locator('[data-role="interpreter-tips-dismiss-all"]');

  let asked = "";
  page.once("dialog", (dialog) => {
    asked = dialog.message();
    void dialog.dismiss();
  });
  await button.click();
  expect(asked).toBe("Skryť všetky tipy?");
  await expect(cards(page)).toHaveCount(2);

  page.once("dialog", (dialog) => void dialog.accept());
  await button.click();
  await expect(cards(page)).toHaveCount(0);
  await expect(page.locator('[data-role="interpreter-tips-empty"]')).toBeVisible();
  await expect(button).toBeDisabled();
  expect(interp.errors).toEqual([]);
  await interp.context.close();
});

test("the interpreter catches up after a reload and after a lost live frame", async ({
  browser,
}) => {
  const context = await browser.newContext({ ...TABLET_1180, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const routes: WebSocketRoute[] = [];
  let dying: WebSocketRoute | null = null;
  await page.routeWebSocket(/\/live\/ws/, (ws: WebSocketRoute) => {
    routes.push(ws);
    const server = ws.connectToServer();
    server.onMessage((message) => {
      // The dying socket delivers nothing more: what the hub publishes now is lost.
      if (ws === dying) return;
      ws.send(message);
    });
  });
  await page.goto("/ui/interpreter");
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(page.locator('[data-role="interpreter-ws"]')).toHaveAttribute(
    "data-ws",
    "connected",
  );

  await apiTip(page, { senderName: "Obnova", note: "pred obnovením stránky" });
  await expect(card(page, "pred obnovením stránky")).toBeVisible();
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]');
  await expect(page.locator('[data-role="interpreter-ws"]')).toHaveAttribute(
    "data-ws",
    "connected",
  );
  await expect(card(page, "pred obnovením stránky")).toBeVisible();

  dying = routes[routes.length - 1];
  await apiTip(page, { senderName: "Obnova", note: "počas výpadku spojenia" });
  const before = routes.length;
  await dying.close({ code: 4000, reason: "reset" });
  await expect.poll(() => routes.length, { timeout: 20_000 }).toBeGreaterThan(before);
  await expect(card(page, "počas výpadku spojenia")).toBeVisible({ timeout: 15_000 });
  expect(errors).toEqual([]);
  await context.close();
});

test("the tablet fits landscape and portrait with 44 px controls", async ({ browser }) => {
  const interp = await openInterpreter(browser);
  const { page } = interp;
  await apiTip(page, { senderName: "Šírka", note: "slovo".repeat(38) });
  await expect(card(page, "slovoslovo")).toBeVisible();
  await assertNoHorizontalOverflow(page);
  await assertMinTapTarget(
    page.locator('[data-role="interpreter-tip-dismiss"], [data-role="interpreter-tips-dismiss-all"]'),
    44,
  );
  await page.setViewportSize({ width: 820, height: 1180 });
  await assertNoHorizontalOverflow(page);
  expect(interp.errors).toEqual([]);
  await interp.context.close();
});
