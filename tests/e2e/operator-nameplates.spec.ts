/**
 * #871: the operator's Menovky view, a tab next to Worship / Bible / Timers /
 * AI / Settings. It lists the `stream` output's plates in Stream Deck grid
 * order (#870: button 1 = the song plate, then the person plates in list
 * order), so the number on screen is the button that fires the plate.
 *
 *  - the Menovky tab opens the view (after AI, `/ui/operator/menovky`);
 *  - add two plates and rename one: the row text AND the REST list;
 *  - reorder: the button numbers 2 / 3 swap and the REST order follows;
 *  - half-typed text survives every list refresh until it is saved (blur or
 *    Enter), even a rename of the same plate from elsewhere;
 *  - drag to reorder by the handle: above the first, below the last, in the
 *    middle (full id set each time), one plate or none breaks nothing; two
 *    quick ↑ taps move a plate up two places;
 *  - Zobraziť → on-air highlight + `GET …/nameplates/active`; Skryť → cleared;
 *    the song row shows the live song and puts the song plate on air;
 *  - changes made through REST from outside (add, rename, show) show live;
 *  - delete asks first; the no-layer hint follows the `stream` def live;
 *  - from button 13 on a plate is outside the 4 × 3 grid (marker + note);
 *  - at 390 px the view fits: nothing past the right edge, 44 px tap targets,
 *    16 px inputs, picking the view closes the phone menu, and a finger drag
 *    reorders.
 *
 * The rows are the stream editor's shared plate list (same data-roles), scoped
 * here to `[data-role="operator-nameplates"]`. Clean console is the last
 * assertion of every test.
 */
import {
  test,
  expect,
  type APIRequestContext,
  type Locator,
  type Page,
} from "@playwright/test";
import {
  assertMinFontSize,
  assertMinTapTarget,
  deriveTestConfig,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  touchDrag,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

const SLUG = "stream";

type Plate = { id: number; primaryText: string; secondaryText: string };
type ActivePlate = {
  source: "person" | "song";
  nameplateId?: number;
  primary: string;
  secondary: string;
};

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

/** Errors + warnings, minus the known Chrome integrity notice other operator
 *  specs filter the same way (`crbug.com/981419`). */
function collectConsole(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    if (msg.text().includes("crbug.com/981419")) return;
    messages.push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (err) => messages.push(`[pageerror] ${err.message}`));
  return messages;
}

// ── Locators ─────────────────────────────────────────────────────────────────

const view = (page: Page) => page.locator('[data-role="operator-nameplates"]');
const rows = (page: Page) => view(page).locator('[data-role="stream-nameplate"]');
const row = (page: Page, id: number) =>
  view(page).locator(`[data-role="stream-nameplate"][data-nameplate-id="${id}"]`);
const songRow = (page: Page) => view(page).locator('[data-role="stream-nameplate-song"]');
const BUTTON = '[data-role="stream-nameplate-button"]';
const NAME = '[data-role="stream-nameplate-name"]';
const ROLE = '[data-role="stream-nameplate-role"]';
const DRAG = '[data-role="stream-nameplate-drag"]';
const noLayerHint = (page: Page) => page.locator('[data-role="operator-nameplates-no-layer"]');
const gridNote = (page: Page) => page.locator('[data-role="operator-nameplates-grid-note"]');

// ── REST helpers ─────────────────────────────────────────────────────────────

async function listPlates(request: APIRequestContext): Promise<Plate[]> {
  const resp = await request.get(`${baseURL}/stream/api/outputs/${SLUG}/nameplates`);
  expect(resp.ok(), `list nameplates -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()) as Plate[];
}

async function activePlate(request: APIRequestContext): Promise<ActivePlate | null> {
  const resp = await request.get(`${baseURL}/stream/api/outputs/${SLUG}/nameplates/active`);
  expect(resp.ok(), `active nameplate -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()) as ActivePlate | null;
}

async function createPlate(
  request: APIRequestContext,
  primaryText: string,
  secondaryText: string,
): Promise<number> {
  const resp = await request.post(`${baseURL}/stream/api/outputs/${SLUG}/nameplates`, {
    data: { primaryText, secondaryText },
  });
  expect(resp.ok(), `create nameplate -> ${resp.status()}`).toBeTruthy();
  return (await resp.json()).id as number;
}

async function patchPlate(
  request: APIRequestContext,
  id: number,
  primaryText: string,
  secondaryText: string,
): Promise<void> {
  const resp = await request.patch(`${baseURL}/stream/api/nameplates/${id}`, {
    data: { primaryText, secondaryText },
  });
  expect(resp.ok(), `patch nameplate -> ${resp.status()}`).toBeTruthy();
}

async function deletePlate(request: APIRequestContext, id: number): Promise<void> {
  const resp = await request.delete(`${baseURL}/stream/api/nameplates/${id}`);
  expect(resp.ok(), `delete nameplate -> ${resp.status()}`).toBeTruthy();
}

async function setActive(
  request: APIRequestContext,
  body: Record<string, unknown>,
): Promise<void> {
  const resp = await request.put(`${baseURL}/stream/api/outputs/${SLUG}/nameplates/active`, {
    data: body,
  });
  expect(resp.status(), `set active nameplate ${JSON.stringify(body)}`).toBe(200);
}

/** Delete every `stream` plate, so a test starts from a known list. */
async function resetPlates(request: APIRequestContext): Promise<void> {
  for (const plate of await listPlates(request)) {
    await deletePlate(request, plate.id);
  }
}

/** Create plates in this order (they land at the end of the list). */
async function seedPlates(
  request: APIRequestContext,
  plates: Array<[string, string]>,
): Promise<number[]> {
  const ids: number[] = [];
  for (const [name, role] of plates) {
    ids.push(await createPlate(request, name, role));
  }
  return ids;
}

/** The id of the plate named `name`, once the server lists it. */
async function waitForPlate(request: APIRequestContext, name: string): Promise<number> {
  let id = 0;
  await expect
    .poll(async () => {
      id = (await listPlates(request)).find((p) => p.primaryText === name)?.id ?? 0;
      return id;
    })
    .toBeGreaterThan(0);
  return id;
}

/** An overlay scene on `stream` with a lower third (no auto-hide): the layer a
 *  plate shows on, as the editor's "Vytvoriť menovkovú vrstvu" makes it. */
async function createLowerThirdLayer(request: APIRequestContext): Promise<void> {
  const scene = await request.post(`${baseURL}/stream/api/outputs/${SLUG}/scenes`, {
    data: { name: "Menovky", kind: "overlay" },
  });
  expect(scene.ok(), `create scene -> ${scene.status()}`).toBeTruthy();
  const sceneId = (await scene.json()).id as number;
  const style = (sizePct: number, weight: number) => ({
    fontFamily: "Arial",
    sizePct,
    color: "#ffffff",
    weight,
    align: "left",
    lineHeight: 1.2,
  });
  const element = await request.post(`${baseURL}/stream/api/scenes/${sceneId}/elements`, {
    data: {
      kind: "lower_third",
      frame: { xPct: 6, yPct: 74, wPct: 46, hPct: 14 },
      bar_color: "#0f172a",
      bar_opacity: 0.85,
      accent_color: "#38bdf8",
      accent_width_pct: 2.5,
      primary_style: style(5, 700),
      secondary_style: style(3.5, 400),
      padding_pct: 3,
      animation: "slide_left",
      in_ms: 200,
      out_ms: 150,
      auto_hide_s: 0,
    },
  });
  expect(element.ok(), `add lower third -> ${element.status()}`).toBeTruthy();
}

/** A live song: library `libraryName` + presentation `songTitle`, triggered. The
 *  song plate is the presentation NAME + the library name (#779). */
async function triggerSong(
  request: APIRequestContext,
  songTitle: string,
  libraryName: string,
): Promise<void> {
  const libResp = await request.post(`${baseURL}/libraries`, { data: { name: libraryName } });
  expect(libResp.ok()).toBeTruthy();
  const library = (await libResp.json()) as { id: string };
  const presResp = await request.post(`${baseURL}/libraries/${library.id}/presentations`, {
    data: { name: songTitle },
  });
  expect(presResp.ok()).toBeTruthy();
  const pres = (await presResp.json()) as {
    presentation: { id: string; slides: Array<{ id: string }> };
  };
  const presentationId = pres.presentation.id;
  const slideId = pres.presentation.slides[0].id;
  const patchResp = await request.patch(
    `${baseURL}/presentations/${presentationId}/slides/${slideId}`,
    { data: { main: "verš", translation: "", stage: "", group: "Verse 1" } },
  );
  expect(patchResp.ok()).toBeTruthy();
  const trigger = await request.post(`${baseURL}/stage/state`, {
    data: { presentationId, currentSlideId: slideId },
  });
  expect(trigger.status(), "trigger stage state").toBe(204);
}

// ── Page helpers ─────────────────────────────────────────────────────────────

async function openOperator(page: Page, path = "/ui/operator"): Promise<void> {
  await page.goto(`${baseURL}${path}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
}

/** Add a plate through the view's form. */
async function addThroughUi(page: Page, name: string, role: string): Promise<void> {
  await view(page).locator('[data-role="stream-nameplate-new-name"]').fill(name);
  await view(page).locator('[data-role="stream-nameplate-new-role"]').fill(role);
  await view(page).locator('[data-role="stream-nameplate-add-submit"]').click();
}

async function rowOrder(page: Page): Promise<number[]> {
  return rows(page).evaluateAll((els) => els.map((el) => Number(el.getAttribute("data-nameplate-id"))));
}

type Box = { top: number; bottom: number; centerX: number; centerY: number };

async function box(locator: Locator): Promise<Box> {
  const rect = await locator.boundingBox();
  expect(rect, "the element has a box").toBeTruthy();
  return {
    top: rect!.y,
    bottom: rect!.y + rect!.height,
    centerX: rect!.x + rect!.width / 2,
    centerY: rect!.y + rect!.height / 2,
  };
}

/** Drag plate `id` by its handle with the mouse to the height `toY` (real
 *  pointer events). `whileHeld` runs before the button is released. */
async function mouseDrag(
  page: Page,
  id: number,
  toY: number,
  whileHeld?: () => Promise<void>,
): Promise<void> {
  const handle = await box(row(page, id).locator(DRAG));
  await page.mouse.move(handle.centerX, handle.centerY);
  await page.mouse.down();
  await page.mouse.move(handle.centerX, toY, { steps: 12 });
  if (whileHeld) await whileHeld();
  await page.mouse.up();
}

/** The REST list, the rows on screen and their Stream Deck numbers all show
 *  `ids` in this order. */
async function expectOrder(page: Page, request: APIRequestContext, ids: number[]): Promise<void> {
  await expect.poll(async () => (await listPlates(request)).map((p) => p.id)).toEqual(ids);
  await expect.poll(() => rowOrder(page)).toEqual(ids);
  for (const [index, id] of ids.entries()) {
    await expect(row(page, id).locator(BUTTON)).toHaveText(String(index + 2));
  }
}

// ── Tests ────────────────────────────────────────────────────────────────────

test("#871 the Menovky tab lists the plates in Stream Deck order: add, rename, reorder, show, hide, delete, live", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await openOperator(page);

  // The tab sits after AI, before Settings, and opens the view.
  const tab = page.locator('[data-role="view-toggle"][data-view="menovky"]');
  await expect(tab).toHaveText("Menovky");
  expect(
    await page
      .locator('[data-role="view-toggle"]')
      .evaluateAll((els) => els.map((el) => el.getAttribute("data-view"))),
  ).toEqual(["worship", "bible", "timers", "ai", "menovky", "settings"]);
  await tab.click();
  await expect(page.locator("body")).toHaveAttribute("data-view", "menovky");
  await expect(tab).toHaveAttribute("data-active", "true");
  expect(new URL(page.url()).pathname).toBe("/ui/operator/menovky");
  await expect(view(page)).toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).toBeHidden();
  await expect(view(page).locator('[data-role="operator-nameplates-help"]')).toContainText(
    "tlačidlo na Stream Decku",
  );

  // Button 1 is the song. A fresh DB: no plates, and `stream` has no lower
  // third yet, so the hint says a plate would not show.
  await expect(songRow(page).locator(BUTTON)).toHaveText("1");
  expect(await listPlates(request)).toEqual([]);
  await expect(rows(page)).toHaveCount(0);
  await expect(view(page).locator(DRAG)).toHaveCount(0);
  await expect(noLayerHint(page)).toBeVisible();
  await expect(noLayerHint(page).locator("a")).toHaveAttribute(
    "href",
    "/ui/stream?output=stream&tab=nameplates",
  );

  // ── Add two people: the rows (texts + buttons 2, 3) and the REST list.
  await addThroughUi(page, "Op Anna", "kazateľka");
  const anna = await waitForPlate(request, "Op Anna");
  await expect(page.locator('[data-role="toast"]')).toHaveText("Menovka pridaná.");
  await addThroughUi(page, "Op Boris", "hosť");
  const boris = await waitForPlate(request, "Op Boris");
  await expect(row(page, anna).locator(NAME)).toHaveValue("Op Anna");
  await expect(row(page, anna).locator(ROLE)).toHaveValue("kazateľka");
  await expect(row(page, boris).locator(NAME)).toHaveValue("Op Boris");
  await expect(row(page, anna).locator(BUTTON)).toHaveText("2");
  await expect(row(page, boris).locator(BUTTON)).toHaveText("3");
  await expect(row(page, anna)).toHaveAttribute("data-beyond-grid", "false");
  await expect(view(page).locator('[data-role="stream-nameplate-new-name"]')).toHaveValue("");

  // ── Rename one (the field saves when it loses the focus).
  const annaName = row(page, anna).locator(NAME);
  await annaName.fill("Op Anna Nová");
  await annaName.blur();
  await expect
    .poll(async () => (await listPlates(request)).find((p) => p.id === anna))
    .toMatchObject({ primaryText: "Op Anna Nová", secondaryText: "kazateľka" });
  await expect(annaName).toHaveValue("Op Anna Nová");

  // ── Reorder: Boris up → he is button 2, Anna button 3, in the list too.
  await row(page, boris).locator('[data-role="stream-nameplate-up"]').click();
  await expect
    .poll(async () => (await listPlates(request)).map((p) => p.id))
    .toEqual([boris, anna]);
  await expect(row(page, boris).locator(BUTTON)).toHaveText("2");
  await expect(row(page, anna).locator(BUTTON)).toHaveText("3");
  await expect.poll(() => rowOrder(page)).toEqual([boris, anna]);

  // ── The layer appears from outside (the editor): the hint goes, live.
  await createLowerThirdLayer(request);
  await expect(noLayerHint(page)).toHaveCount(0);

  // ── Zobraziť → on air (highlight + REST); Skryť → off.
  await row(page, anna).locator('[data-role="stream-nameplate-show"]').click();
  await expect(row(page, anna)).toHaveAttribute("data-active", "true");
  await expect(row(page, boris)).toHaveAttribute("data-active", "false");
  await expect
    .poll(async () => (await activePlate(request))?.nameplateId)
    .toBe(anna);
  await row(page, anna).locator('[data-role="stream-nameplate-hide"]').click();
  await expect(row(page, anna)).toHaveAttribute("data-active", "false");
  await expect.poll(() => activePlate(request)).toBeNull();

  // ── The song row: the live song, and it puts the song plate on air.
  await triggerSong(request, "Ako Ťa mám rád", "Kapela XY");
  await expect(songRow(page).locator('[data-role="stream-nameplate-song-live"]')).toHaveText(
    "Ako Ťa mám rád · Kapela XY",
  );
  await songRow(page).locator('[data-role="stream-nameplate-song-show"]').click();
  await expect(songRow(page)).toHaveAttribute("data-active", "true");
  await expect
    .poll(async () => (await activePlate(request))?.source)
    .toBe("song");
  expect((await activePlate(request))?.primary).toBe("Ako Ťa mám rád");
  await songRow(page).locator('[data-role="stream-nameplate-hide"]').click();
  await expect(songRow(page)).toHaveAttribute("data-active", "false");
  await expect.poll(() => activePlate(request)).toBeNull();

  // ── Changes made from outside (another tab, Companion) show without a reload.
  const cyril = await createPlate(request, "Op Cyril", "spevák");
  await expect(row(page, cyril).locator(NAME)).toHaveValue("Op Cyril");
  await expect(row(page, cyril).locator(BUTTON)).toHaveText("4");
  await patchPlate(request, anna, "Op Anna Externá", "kazateľka");
  await expect(row(page, anna).locator(NAME)).toHaveValue("Op Anna Externá");
  await setActive(request, { source: "person", id: cyril });
  await expect(row(page, cyril)).toHaveAttribute("data-active", "true");
  await setActive(request, { source: null });
  await expect(row(page, cyril)).toHaveAttribute("data-active", "false");

  // ── Delete asks first, then the row and the plate are gone.
  const dialogs: string[] = [];
  page.once("dialog", (dialog) => {
    dialogs.push(dialog.message());
    void dialog.accept();
  });
  await row(page, cyril).locator('[data-role="stream-nameplate-delete"]').click();
  await expect(row(page, cyril)).toHaveCount(0);
  expect(dialogs).toEqual(["Zmazať túto menovku?"]);
  await expect
    .poll(async () => (await listPlates(request)).map((p) => p.id))
    .toEqual([boris, anna]);
  await expect.poll(() => rowOrder(page)).toEqual([boris, anna]);

  // The view survives a reload on its own URL.
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(page.locator("body")).toHaveAttribute("data-view", "menovky");
  await expect(row(page, boris).locator(BUTTON)).toHaveText("2");
  await expect(row(page, anna).locator(NAME)).toHaveValue("Op Anna Externá");

  expect(consoleErrors).toEqual([]);
});

test("#871 typing in a row survives every list refresh until it is saved", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [anna, boris] = await seedPlates(request, [
    ["Typ Anna", "kazateľka"],
    ["Typ Boris", "hosť"],
  ]);
  await openOperator(page, "/ui/operator/menovky");
  const annaName = row(page, anna).locator(NAME);
  await expect(annaName).toHaveValue("Typ Anna");

  // Half-typed, not saved (the field keeps the focus).
  await annaName.fill("Typ Anna rozpísaná");
  await expect(annaName).toBeFocused();

  // Three refreshes of the list from outside: a new plate, ANOTHER plate
  // renamed, and THIS plate renamed elsewhere (its role too).
  const cyril = await createPlate(request, "Typ Cyril", "spevák");
  await expect(row(page, cyril).locator(NAME)).toHaveValue("Typ Cyril");
  await patchPlate(request, boris, "Typ Boris Externý", "hosť");
  await expect(row(page, boris).locator(NAME)).toHaveValue("Typ Boris Externý");
  await patchPlate(request, anna, "Typ Anna Externá", "kazateľka nová");
  // The role field (nothing typed there) shows the refresh, so it has landed.
  await expect(row(page, anna).locator(ROLE)).toHaveValue("kazateľka nová");
  await expect(annaName).toHaveValue("Typ Anna rozpísaná");
  await expect(annaName).toBeFocused();

  // Leaving the field saves what was typed (with the role the list now has).
  await annaName.blur();
  await expect
    .poll(async () => (await listPlates(request)).find((p) => p.id === anna))
    .toMatchObject({ primaryText: "Typ Anna rozpísaná", secondaryText: "kazateľka nová" });
  await expect(annaName).toHaveValue("Typ Anna rozpísaná");

  // Enter saves too, and an external rename shows again once nothing is typed.
  await annaName.fill("Typ Anna Enter");
  await annaName.press("Enter");
  await expect
    .poll(async () => (await listPlates(request)).find((p) => p.id === anna)?.primaryText)
    .toBe("Typ Anna Enter");
  await patchPlate(request, anna, "Typ Anna Zvonka", "kazateľka nová");
  await expect(annaName).toHaveValue("Typ Anna Zvonka");

  expect(consoleErrors).toEqual([]);
});

test("#871 drag to reorder: above the first, below the last, in the middle; one plate or none breaks nothing", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await page.setViewportSize({ width: 1280, height: 1200 });
  const orderPuts: string[] = [];
  page.on("request", (req) => {
    if (req.method() === "PUT" && req.url().endsWith(`/stream/api/outputs/${SLUG}/nameplates/order`)) {
      orderPuts.push(req.postData() ?? "");
    }
  });

  // An empty list: no rows, no handles, nothing to drag.
  await resetPlates(request);
  await openOperator(page, "/ui/operator/menovky");
  await expect(view(page)).toBeVisible();
  await expect(rows(page)).toHaveCount(0);
  await expect(view(page).locator(DRAG)).toHaveCount(0);

  // One plate: a drag of it changes nothing and sends nothing.
  const [solo] = await seedPlates(request, [["Ťah Sólo", "hosť"]]);
  await expect(row(page, solo).locator(BUTTON)).toHaveText("2");
  const soloBox = await box(row(page, solo));
  await mouseDrag(page, solo, soloBox.bottom + 120);
  await expect(row(page, solo)).toHaveAttribute("data-dragging", "false");

  // Four plates: A B C D (buttons 2..5). The positive settle signal for "the
  // solo drag sent nothing": these creates have landed in the page first.
  await deletePlate(request, solo);
  const [a, b, c, d] = await seedPlates(request, [
    ["Ťah A", "1"],
    ["Ťah B", "2"],
    ["Ťah C", "3"],
    ["Ťah D", "4"],
  ]);
  await expect.poll(() => rowOrder(page)).toEqual([a, b, c, d]);
  expect(orderPuts, "a one-plate drag sends no reorder").toEqual([]);

  // C above the first (A): the drop line shows on A while C is held.
  const first = await box(row(page, a));
  await mouseDrag(page, c, first.top - 4, async () => {
    await expect(row(page, c)).toHaveAttribute("data-dragging", "true");
    await expect(row(page, a)).toHaveAttribute("data-drop", "before");
  });
  await expectOrder(page, request, [c, a, b, d]);
  await expect(row(page, c)).toHaveAttribute("data-dragging", "false");
  await expect(view(page).locator("[data-drop]")).toHaveCount(0);

  // C (now first) below the last (D).
  const last = await box(row(page, d));
  await mouseDrag(page, c, last.bottom + 4, async () => {
    await expect(row(page, d)).toHaveAttribute("data-drop", "after");
  });
  await expectOrder(page, request, [a, b, d, c]);

  // D (third) into the middle, between A and B.
  const gap = await box(row(page, a));
  await mouseDrag(page, d, gap.bottom + 4);
  await expectOrder(page, request, [a, d, b, c]);

  // Every drop sent the FULL id set.
  expect(orderPuts.map((body) => (JSON.parse(body) as { ids: number[] }).ids)).toEqual([
    [c, a, b, d],
    [a, b, d, c],
    [a, d, b, c],
  ]);

  expect(consoleErrors).toEqual([]);
});

test("#871 two quick ↑ taps move a plate up two places", async ({ page, request }) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a, b, c] = await seedPlates(request, [
    ["Rýchlo A", "1"],
    ["Rýchlo B", "2"],
    ["Rýchlo C", "3"],
  ]);
  await openOperator(page, "/ui/operator/menovky");
  await expect.poll(() => rowOrder(page)).toEqual([a, b, c]);

  const up = row(page, c).locator('[data-role="stream-nameplate-up"]');
  await up.click();
  await up.click();
  await expectOrder(page, request, [c, a, b]);

  expect(consoleErrors).toEqual([]);
});

test("#871 from button 13 on a plate is outside the 4×3 grid: marked, with a note", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);

  // Twelve people = buttons 2..13 (the song is button 1).
  let n = (await listPlates(request)).length;
  while (n < 12) {
    n += 1;
    await createPlate(request, `Grid ${n}`, "hosť");
  }
  const plates = await listPlates(request);
  await openOperator(page, "/ui/operator/menovky");
  await expect(view(page)).toBeVisible();
  await expect(rows(page)).toHaveCount(plates.length);

  const twelve = row(page, plates[10].id);
  const thirteen = row(page, plates[11].id);
  await expect(twelve.locator(BUTTON)).toHaveText("12");
  await expect(twelve).toHaveAttribute("data-beyond-grid", "false");
  await expect(thirteen.locator(BUTTON)).toHaveText("13");
  await expect(thirteen).toHaveAttribute("data-beyond-grid", "true");
  await expect(thirteen.locator(BUTTON)).toHaveAttribute(
    "title",
    "Tlačidlo 13: mimo mriežky 4 × 3 na Stream Decku",
  );
  await expect(gridNote(page)).toBeVisible();
  await expect(gridNote(page)).toContainText("12 tlačidiel");

  // Back to eleven people (from outside): every plate has a button, no note.
  for (const plate of plates.slice(11)) {
    await deletePlate(request, plate.id);
  }
  await expect(rows(page)).toHaveCount(11);
  await expect(gridNote(page)).toHaveCount(0);
  await expect(view(page).locator('[data-role="stream-nameplate"][data-beyond-grid="true"]')).toHaveCount(0);

  expect(consoleErrors).toEqual([]);
});

test("#871 on a 390 px phone the view fits, the menu closes on the pick and a finger drag reorders", async ({
  browser,
}) => {
  const context = await browser.newContext({ ...PHONE_390, baseURL });
  try {
    const page = await context.newPage();
    const consoleErrors = collectConsole(page);
    await resetPlates(page.request);
    const [first, second] = await seedPlates(page.request, [
      ["Telefón A", "hosť"],
      ["Telefón B", "hosť"],
    ]);
    await openOperator(page);

    // ≤768 px the views sit behind the menu button; picking one closes the menu.
    await page.locator('[data-role="mobile-menu-toggle"]').tap();
    await expect(page.locator("body")).toHaveClass(/operator--mobile-nav-open/);
    await page.locator('[data-role="view-toggle"][data-view="menovky"]').tap();
    await expect(page.locator("body")).toHaveAttribute("data-view", "menovky");
    await expect(page.locator("body")).not.toHaveClass(/operator--mobile-nav-open/);
    await expect(view(page)).toBeVisible();

    // A plate added by touch.
    await view(page).locator('[data-role="stream-nameplate-new-name"]').fill("Telefón C");
    await view(page).locator('[data-role="stream-nameplate-new-role"]').fill("hosť");
    await view(page).locator('[data-role="stream-nameplate-add-submit"]').tap();
    const third = await waitForPlate(page.request, "Telefón C");
    await expect(row(page, third).locator(NAME)).toHaveValue("Telefón C");
    await expect(row(page, third).locator(BUTTON)).toHaveText("4");

    // A finger drag of C's handle above B: C becomes button 3.
    await row(page, second).evaluate((el) => el.scrollIntoView({ block: "start" }));
    const handle = await box(row(page, third).locator(DRAG));
    const target = await box(row(page, second));
    await touchDrag(
      page,
      { x: handle.centerX, y: handle.centerY },
      { x: handle.centerX, y: target.top + 4 },
    );
    await expectOrder(page, page.request, [first, third, second]);

    // Nothing in the view reaches past the screen edges and the panel does not
    // scroll sideways. (The operator header above is out of this view's scope.)
    const fit = await page.evaluate(() => {
      const panel = document.querySelector('[data-view-panel="menovky"]') as HTMLElement;
      const width = window.innerWidth;
      const offenders = Array.from(panel.querySelectorAll<HTMLElement>("*"))
        .filter((el) => {
          const rect = el.getBoundingClientRect();
          return rect.width > 0 && rect.height > 0 && (rect.right > width + 1 || rect.left < -1);
        })
        .slice(0, 5)
        .map((el) => `${el.tagName.toLowerCase()}[data-role=${el.dataset.role ?? ""}]`);
      return { offenders, overflow: panel.scrollWidth - panel.clientWidth };
    });
    expect(fit.offenders, "elements past the screen edge").toEqual([]);
    expect(fit.overflow, "the view scrolls sideways").toBeLessThanOrEqual(0);

    // Touch sizes: every button (the drag handle too) ≥ 44 px, inputs 16 px.
    await assertMinTapTarget(view(page).locator("button"), 44);
    await assertMinFontSize(view(page).locator("input"), 16);

    expect(consoleErrors).toEqual([]);
  } finally {
    await context.close();
  }
});
