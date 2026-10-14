/**
 * #852 šepkár tips — the interpreter tablet (`/ui/interpreter`) and the
 * šepkár phone page (`/ui/sepkar`), spec §15 / T-S1–T-S3. One server for the
 * whole file. Every test starts from an empty interpreter list (dismiss-all)
 * and uses its own sender names: the in-memory rate limit (5 / min per IP +
 * name, 30 / min overall) spans the file; the interpreter tests send 10 tips
 * and the šepkár tests 10 more, so 20 in all — under the overall limit even
 * when every test lands inside one minute. Only one name ever sends 5 (the
 * test that provokes the refusal of its 6th).
 *
 * The šepkár page lists ALL of today's tips (dismissed ones too), and
 * dismiss-all only hides: every word a šepkár test looks for in that list is
 * therefore unique across the whole file.
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
  assertMinFontSize,
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  assertVersionLabel,
  attachConsoleErrorCollector,
  deriveTestConfig,
  PHONE_390,
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

async function openSepkar(
  browser: Browser,
  profile: BrowserContextOptions = PHONE_390,
): Promise<Opened> {
  return open(browser, profile, "/ui/sepkar", "sepkar-ws");
}

/** Fill the šepkár form: a field the tip does not name is emptied; the name is left alone unless given. */
async function fillTip(
  page: Page,
  tip: { name?: string; source?: string; target?: string; note?: string },
): Promise<void> {
  if (tip.name !== undefined) await page.locator('[data-role="sepkar-name"]').fill(tip.name);
  await page.locator('[data-role="sepkar-source"]').fill(tip.source ?? "");
  await page.locator('[data-role="sepkar-target"]').fill(tip.target ?? "");
  await page.locator('[data-role="sepkar-note"]').fill(tip.note ?? "");
}

/** A row of the šepkár page's "Dnes" list. */
const row = (page: Page, text: string) =>
  page.locator('[data-role="sepkar-row"]').filter({ hasText: text });

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

test("a tip sent from the phone shows on the tablet within 1 s, newest on top (T-S1)", async ({
  browser,
}) => {
  const interp = await openInterpreter(browser);
  const sepkar = await openSepkar(browser);
  const phone = sepkar.page;
  const tablet = interp.page;

  await fillTip(phone, { name: "Peter", source: "atonement", target: "zmierenie" });
  const clickedAt = Date.now();
  await phone.locator('[data-role="sepkar-send"]').click();
  const first = card(tablet, "atonement");
  await expect(first).toBeVisible({ timeout: 1_000 });
  expect(Date.now() - clickedAt, "click → card on the interpreter").toBeLessThan(1_000);
  await expect(first.locator('[data-role="interpreter-tip-pair"]')).toHaveText(
    "atonement → zmierenie",
  );
  await expect(first.locator('[data-role="interpreter-tip-meta"]')).toHaveText(
    /^Peter · \d{2}:\d{2}$/,
  );
  await expect(first).toHaveAttribute("data-new", "true");

  // The phone confirms, empties the tip fields, keeps the name and is ready for the next word.
  await expect(phone.locator('[data-role="sepkar-notice"]')).toHaveText(/^Odoslané \d{2}:\d{2}$/);
  await expect(phone.locator('[data-role="sepkar-source"]')).toHaveValue("");
  await expect(phone.locator('[data-role="sepkar-target"]')).toHaveValue("");
  await expect(phone.locator('[data-role="sepkar-note"]')).toHaveValue("");
  await expect(phone.locator('[data-role="sepkar-name"]')).toHaveValue("Peter");
  await expect(phone.locator('[data-role="sepkar-source"]')).toBeFocused();
  await expect(row(phone, "atonement")).toHaveAttribute("data-state", "on-screen");

  // A second šepkár's note lands ABOVE the first; Enter in Poznámka sends.
  await fillTip(phone, { name: "Jana", note: "„sanctification“ = posvätenie" });
  await phone.locator('[data-role="sepkar-note"]').press("Enter");
  await expect(cards(tablet)).toHaveCount(2);
  await expect(cards(tablet).first()).toContainText("sanctification");
  await expect(cards(tablet).nth(1)).toContainText("atonement");

  // The same pair again (other case, stray blanks) is not shown twice.
  await fillTip(phone, { source: " Atonement ", target: "ZMIERENIE" });
  await phone.locator('[data-role="sepkar-send"]').click();
  await expect(phone.locator('[data-role="sepkar-notice"]')).toHaveText("Tento tip už tlmočník má");
  await expect(cards(tablet)).toHaveCount(2);
  await expect(row(phone, "atonement")).toHaveCount(1);

  // The name is remembered on this device.
  await phone.reload();
  await phone.waitForSelector('body[data-wasm-ready="true"]');
  await expect(phone.locator('[data-role="sepkar-name"]')).toHaveValue("Jana");

  expect(interp.errors).toEqual([]);
  expect(sepkar.errors).toEqual([]);
  await interp.context.close();
  await sepkar.context.close();
});

test("the šepkár list follows the interpreter: skryté after ✕, na obrazovke after Späť", async ({
  browser,
}) => {
  const interp = await openInterpreter(browser);
  const sepkar = await openSepkar(browser);
  await fillTip(sepkar.page, { name: "Marek", source: "forgiveness", target: "odpustenie" });
  await sepkar.page.locator('[data-role="sepkar-send"]').click();
  const tip = card(interp.page, "forgiveness");
  await expect(tip).toBeVisible();
  await expect(row(sepkar.page, "forgiveness")).toHaveAttribute("data-state", "on-screen");

  await tip.locator('[data-role="interpreter-tip-dismiss"]').click();
  await expect(row(sepkar.page, "forgiveness")).toHaveAttribute("data-state", "dismissed");
  await expect(row(sepkar.page, "forgiveness")).toContainText("skryté");

  await interp.page.locator('[data-role="interpreter-undo-button"]').click();
  await expect(tip).toBeVisible();
  await expect(row(sepkar.page, "forgiveness")).toHaveAttribute("data-state", "on-screen");
  await expect(row(sepkar.page, "forgiveness")).toContainText("na obrazovke");
  expect(interp.errors).toEqual([]);
  expect(sepkar.errors).toEqual([]);
  await interp.context.close();
  await sepkar.context.close();
});

test("server refusals show inline: a 201-char note → 422, the 6th tip in a minute → 429", async ({
  browser,
}) => {
  const sepkar = await openSepkar(browser);
  const { page } = sepkar;
  await fillTip(page, { name: "Rýchly", note: "x".repeat(201) });
  await expect(page.locator('[data-role="sepkar-note-counter"]')).toHaveText("201 / 200");
  await expect(page.locator('[data-role="sepkar-note-counter"]')).toHaveAttribute(
    "data-over",
    "true",
  );
  await page.locator('[data-role="sepkar-send"]').click();
  await expect(page.locator('[data-role="sepkar-error"]')).toHaveText(
    "Poznámka môže mať najviac 200 znakov.",
  );
  await expect(page.locator('[data-role="sepkar-note"]')).toHaveValue("x".repeat(201));

  // The refused tip used no slot: five valid ones still go through.
  for (let n = 1; n <= 5; n += 1) {
    await fillTip(page, { note: `rýchly tip ${n}` });
    await page.locator('[data-role="sepkar-send"]').click();
    await expect(page.locator('[data-role="sepkar-note"]')).toHaveValue("");
  }
  await expect(page.locator('[data-role="sepkar-error"]')).toHaveCount(0);
  await fillTip(page, { note: "rýchly tip 6" });
  await page.locator('[data-role="sepkar-send"]').click();
  await expect(page.locator('[data-role="sepkar-error"]')).toHaveText(
    /^Pomaly — najviac 5 tipov za minútu\. Skús znova o \d+ s\.$/,
  );
  await expect(page.locator('[data-role="sepkar-note"]')).toHaveValue("rýchly tip 6");

  // Chrome logs every real non-2xx fetch itself: exactly one 422 and one 429 are
  // the behavior under test; every other console line must still be absent.
  const refused = (status: number) =>
    sepkar.errors.filter((line) =>
      new RegExp(`Failed to load resource: the server responded with a status of ${status}\\b`).test(
        line,
      ),
    );
  expect(refused(422)).toHaveLength(1);
  expect(refused(429)).toHaveLength(1);
  expect(
    sepkar.errors.filter(
      (line) =>
        !/Failed to load resource: the server responded with a status of (422|429)\b/.test(line),
    ),
  ).toEqual([]);
  await sepkar.context.close();
});

test("the šepkár page fits a phone (T-S3) and puts the list beside the form on a laptop", async ({
  browser,
}) => {
  const sepkar = await openSepkar(browser);
  const { page } = sepkar;
  await assertNoHorizontalOverflow(page);
  await assertMinFontSize(page.locator(".sepkar__input"), 16);
  await assertMinTapTarget(page.locator('.sepkar__input, [data-role="sepkar-send"]'), 44);
  const hints: Array<[string, string]> = [
    ["sepkar-name", "next"],
    ["sepkar-source", "next"],
    ["sepkar-target", "send"],
    ["sepkar-note", "send"],
  ];
  for (const [role, hint] of hints) {
    await expect(page.locator(`[data-role="${role}"]`)).toHaveAttribute("enterkeyhint", hint);
  }

  // A long unspaced word wraps, and HTML-looking text renders literally.
  await apiTip(page, { senderName: "Dlhé", note: "dlheslovo".repeat(21) });
  await apiTip(page, { senderName: "Dlhé", note: "<b>tučné</b>" });
  await expect(row(page, "dlheslovodlheslovo")).toBeVisible();
  await expect(row(page, "<b>tučné</b>")).toBeVisible();
  await expect(page.locator('[data-role="sepkar-row"] b')).toHaveCount(0);
  await assertNoHorizontalOverflow(page);

  // On the phone the list sits BELOW the form.
  const phoneForm = await page.locator('[data-role="sepkar-form"]').boundingBox();
  const phoneToday = await page.locator('[data-role="sepkar-today"]').boundingBox();
  if (!phoneForm || !phoneToday) throw new Error("the form or the list has no box");
  expect(phoneToday.y, "the list sits below the form").toBeGreaterThanOrEqual(
    phoneForm.y + phoneForm.height - 1,
  );
  await assertVersionLabel(page, baseURL);

  // On a laptop it sits BESIDE it, from the same top edge.
  const laptop = await openSepkar(browser, DESKTOP_1280);
  await expect(row(laptop.page, "<b>tučné</b>")).toBeVisible();
  await assertNoHorizontalOverflow(laptop.page);
  const form = await laptop.page.locator('[data-role="sepkar-form"]').boundingBox();
  const today = await laptop.page.locator('[data-role="sepkar-today"]').boundingBox();
  if (!form || !today) throw new Error("the form or the list has no box");
  expect(today.x, "the list sits to the right of the form").toBeGreaterThanOrEqual(
    form.x + form.width - 1,
  );
  expect(Math.abs(today.y - form.y), "both start at the same top edge").toBeLessThanOrEqual(2);

  expect(sepkar.errors).toEqual([]);
  expect(laptop.errors).toEqual([]);
  await sepkar.context.close();
  await laptop.context.close();
});
