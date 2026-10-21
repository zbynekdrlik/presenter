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
 *  - a reorder keeps the focus where it was: ↑ / ↓ on the drag handle step on,
 *    and a reorder from elsewhere while typing keeps the focus, the caret and
 *    the text, and saves nothing early;
 *  - leaving the page (the window loses the focus, the page goes hidden, or
 *    the window blur and then the page hidden) saves the half-typed text,
 *    once, also where `hasFocus()` still says true when the page decides;
 *  - every save is a keepalive request: a reload right after typing keeps
 *    the text, and a save waiting in the queue goes at once when the page
 *    hides;
 *  - a save that fails on the network keeps the typed text and goes again on
 *    the next leave or when the page is back in view; a 404 says the plate
 *    was deleted;
 *  - a failed save marks its row "neuložené" until it is saved and goes again
 *    by itself (2 s, 5 s, 15 s) and as soon as the network is back;
 *    Zobraziť on such a row saves first and puts the saved text on air (a
 *    test that acts on a failed row does it from a MutationObserver on the
 *    row's `data-unsaved`, in the same pass as the mark: never a race with
 *    the page's own retry timer);
 *  - a save sent beside another as the page hid keeps the next save waiting;
 *    an older save answered after the newer one makes the page send the
 *    newer texts once more; a 404 drops the save waiting behind it;
 *  - a name cleared to retype it does not hold back a role typed in the same
 *    row when the page hides, and that role goes with the NEWEST name;
 *  - one controller per list owns the saves: a commit while a save is on its
 *    way goes after it, once, with the newest texts, and Zobraziť meanwhile
 *    (or while an older answer is being repaired) waits for it; typed,
 *    changed and typed back is saved once more and never left marked; an
 *    empty name on leaving the field is refused before anything is sent; a
 *    save that never answers is given up after 15 s and goes again;
 *    Zobraziť with no blur (iOS) saves the typed text first;
 *  - a name cleared to retype it is kept on alt-tab / a hidden page, and
 *    still refused when the field is left;
 *  - a plate deleted elsewhere while half-typed: a toast says it was not saved;
 *  - two saves of one plate go one at a time (the text saved last wins);
 *  - a plate deleted elsewhere mid-drag takes its drop line with it;
 *  - the Companion slot labels (#870) follow a rename and a reorder made here;
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
import { createCompanionSocket, enableCompanionSocket } from "./companion-socket";
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
let wsURL: string;

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
  // The real Companion socket, for the Stream Deck label checks (#870).
  wsURL = await enableCompanionSocket(baseURL, config.port + 100);
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

/** Reorder the `stream` plates from outside (another operator, Companion). */
async function putOrder(request: APIRequestContext, ids: number[]): Promise<void> {
  const resp = await request.put(`${baseURL}/stream/api/outputs/${SLUG}/nameplates/order`, {
    data: { ids },
  });
  expect(resp.status(), `reorder nameplates ${JSON.stringify(ids)}`).toBe(204);
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
  // Both ways to move a plate: the ↑ ↓ keys and dragging by the handle (its icon).
  await expect(view(page).locator('[data-role="operator-nameplates-help"]')).toContainText(
    "Šípkami ↑ ↓ alebo potiahnutím za úchyt ⠿ presuniete menovku na iné tlačidlo.",
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

test("#871 ↑ / ↓ on the drag handle keep its focus, so each press moves the plate one more place", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a, b, c] = await seedPlates(request, [
    ["Kláves A", "1"],
    ["Kláves B", "2"],
    ["Kláves C", "3"],
  ]);
  await openOperator(page, "/ui/operator/menovky");
  await expect.poll(() => rowOrder(page)).toEqual([a, b, c]);

  // Focus C's handle ONCE. The keys go to whatever holds the focus
  // (`page.keyboard`, never `locator.press`, which focuses the handle again
  // before every key and would hide a lost focus).
  const handle = row(page, c).locator(DRAG);
  await handle.focus();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowUp");
  await expectOrder(page, request, [c, a, b]);
  await expect(handle).toBeFocused();

  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expectOrder(page, request, [a, b, c]);
  await expect(handle).toBeFocused();

  expect(consoleErrors).toEqual([]);
});

test("#871 a reorder from elsewhere while typing keeps the focus, the caret and the text, and saves nothing early", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a, b, c] = await seedPlates(request, [
    ["Pís A", "1"],
    ["Pís B", "2"],
    ["Pís C", "3"],
  ]);
  const patches: string[] = [];
  page.on("request", (req) => {
    if (req.method() === "PATCH" && req.url().endsWith(`/stream/api/nameplates/${b}`)) {
      patches.push(req.postData() ?? "");
    }
  });
  await openOperator(page, "/ui/operator/menovky");
  await expect.poll(() => rowOrder(page)).toEqual([a, b, c]);

  // Half-typed in B's name, the caret put back after "Pís" (index 3).
  const bName = row(page, b).locator(NAME);
  await bName.fill("Pís Bé");
  await bName.evaluate((el) => (el as HTMLInputElement).setSelectionRange(3, 3));

  // Another operator moves B to the top: the page moves B's row.
  await putOrder(request, [b, a, c]);
  await expect.poll(() => rowOrder(page)).toEqual([b, a, c]);
  await expect(row(page, b).locator(BUTTON)).toHaveText("2");

  // Still typing in B: the focus, the caret and the text are kept, and the
  // move saved nothing (a blur from the move is not "left the field").
  await expect(bName).toBeFocused();
  await expect(bName).toHaveValue("Pís Bé");
  expect(
    await bName.evaluate((el) => {
      const input = el as HTMLInputElement;
      return [input.selectionStart, input.selectionEnd];
    }),
  ).toEqual([3, 3]);
  await page.keyboard.type("!");
  await expect(bName).toHaveValue("Pís! Bé");
  expect(patches, "nothing is saved while the operator types").toEqual([]);

  // Leaving the field saves the whole text, once.
  await bName.blur();
  await expect
    .poll(async () => (await listPlates(request)).find((p) => p.id === b)?.primaryText)
    .toBe("Pís! Bé");
  await expect.poll(() => patches.length).toBe(1);
  expect((JSON.parse(patches[0]) as { primaryText: string }).primaryText).toBe("Pís! Bé");
  await expect(bName).toHaveValue("Pís! Bé");

  expect(consoleErrors).toEqual([]);
});

test("#871 leaving the page (another window, tab or app, a hidden phone page) saves the half-typed text, once", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a, b, c] = await seedPlates(request, [
    ["Preč A", "1"],
    ["Preč B", "2"],
    ["Preč C", "3"],
  ]);
  const patches = new Map<number, string[]>([
    [a, []],
    [b, []],
    [c, []],
  ]);
  page.on("request", (req) => {
    if (req.method() !== "PATCH") return;
    const id = Number(req.url().split("/stream/api/nameplates/")[1]);
    patches.get(id)?.push((JSON.parse(req.postData() ?? "{}") as { primaryText: string }).primaryText);
  });
  const logs: string[] = [];
  page.on("console", (msg) => logs.push(msg.text()));
  await openOperator(page, "/ui/operator/menovky");
  await expect.poll(() => rowOrder(page)).toEqual([a, b, c]);
  const savedName = async (id: number) =>
    (await listPlates(request)).find((p) => p.id === id)?.primaryText;

  // A: the WINDOW loses the focus (alt-tab, OBS, Companion, another browser
  // tab): the field gets `blur` but stays the document's active element.
  const aName = row(page, a).locator(NAME);
  await aName.fill("Preč A okno");
  await aName.evaluate((el) => {
    document.hasFocus = () => false;
    el.dispatchEvent(new FocusEvent("blur"));
  });
  expect(await aName.evaluate((el) => document.activeElement === el)).toBe(true);
  await expect.poll(() => savedName(a)).toBe("Preč A okno");
  expect(patches.get(a)).toEqual(["Preč A okno"]);
  await page.evaluate(() => {
    delete (document as { hasFocus?: unknown }).hasFocus;
  });

  // B: the page goes hidden (a phone locked or switched to another app, where
  // the tab may be killed), with no blur at all.
  const bName = row(page, b).locator(NAME);
  await bName.fill("Preč B skryté");
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
  });
  await expect.poll(() => savedName(b)).toBe("Preč B skryté");
  expect(patches.get(b)).toEqual(["Preč B skryté"]);
  await page.evaluate(() => {
    delete (document as { visibilityState?: unknown }).visibilityState;
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
  });

  // C: both, in the order a phone gives them: the window blur first, whose
  // save decides in a task after the blur and is then on its way (held here
  // at the network), THEN the page goes hidden: still ONE save. Two separate
  // evaluates: inside one, no task runs between the two dispatches, so the
  // hidden page would save first and the blur's task would find it sent.
  const cName = row(page, c).locator(NAME);
  await cName.fill("Preč C oboje");
  let releaseC = () => {};
  const cHeld = new Promise<void>((resolve) => (releaseC = resolve));
  await page.route(`**/stream/api/nameplates/${c}`, async (route) => {
    if (route.request().method() === "PATCH") await cHeld;
    await route.continue();
  });
  await cName.evaluate((el) => {
    document.hasFocus = () => false;
    el.dispatchEvent(new FocusEvent("blur"));
  });
  await expect.poll(() => patches.get(c)?.length).toBe(1);
  const hiddenDecided = page.waitForEvent("console", {
    predicate: (msg) =>
      msg.text().startsWith(`nameplates: plate ${c} save of these texts is on its way`),
    timeout: 10_000,
  });
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
  });
  // The hidden page logs its decision once it is made (that save was on its
  // way already), never a "saved" line ahead of it.
  expect((await hiddenDecided).text()).toBe(
    `nameplates: plate ${c} save of these texts is on its way (the page went hidden)`,
  );
  releaseC();
  await expect.poll(() => savedName(c)).toBe("Preč C oboje");
  await page.unrouteAll({ behavior: "wait" });
  await page.evaluate(() => {
    delete (document as { hasFocus?: unknown }).hasFocus;
    delete (document as { visibilityState?: unknown }).visibilityState;
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
  });
  // Back in the page: leaving the field now has nothing left to save.
  await cName.blur();
  await expect(cName).toHaveValue("Preč C oboje");
  expect(patches.get(c)).toEqual(["Preč C oboje"]);
  expect(patches.get(a)).toEqual(["Preč A okno"]);
  expect(patches.get(b)).toEqual(["Preč B skryté"]);
  expect(logs.filter((line) => line.includes("saved as the page went hidden"))).toEqual([]);

  expect(consoleErrors).toEqual([]);
});

/** Make the page hidden (a locked phone, another app or tab) or visible
 *  again, as the browser does: `visibilityState` + a bubbling
 *  `visibilitychange`, so a window listener hears it. */
async function setPageHidden(page: Page, hidden: boolean): Promise<void> {
  await page.evaluate((hide) => {
    if (hide) {
      Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    } else {
      delete (document as { visibilityState?: unknown }).visibilityState;
    }
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
  }, hidden);
}

/** The window loses the focus (alt-tab, OBS, Companion): `field` gets a blur
 *  and stays the page's active element. `pageReportsFocus` = what
 *  `document.hasFocus()` still says when the page decides (Chrome: false;
 *  Safari / Firefox may still say true). */
async function windowBlur(field: Locator, pageReportsFocus: boolean): Promise<void> {
  await field.evaluate((el, focused) => {
    if (!focused) document.hasFocus = () => false;
    el.dispatchEvent(new FocusEvent("blur"));
  }, pageReportsFocus);
}

async function restoreWindowFocus(page: Page): Promise<void> {
  await page.evaluate(() => {
    delete (document as { hasFocus?: unknown }).hasFocus;
  });
}

/** The server's primary text of plate `id`. */
async function savedText(request: APIRequestContext, id: number): Promise<string | undefined> {
  return (await listPlates(request)).find((p) => p.id === id)?.primaryText;
}

/** The console lines Chrome logs itself for a fetch that failed on purpose
 *  (`pattern`), split from every other line. */
function splitConsole(lines: string[], pattern: RegExp): { expected: string[]; other: string[] } {
  return {
    expected: lines.filter((line) => pattern.test(line)),
    other: lines.filter((line) => !pattern.test(line)),
  };
}

const NETWORK_FAILED = /^\[error\] Failed to load resource: net::ERR_FAILED$/;
const DELETED_TOAST = "Menovka bola medzičasom zmazaná — zmena sa neuložila.";
const UNSAVED = '[data-role="stream-nameplate-unsaved"]';

type PatchBody = { primaryText: string; secondaryText: string };

/** Hold every PATCH of plate `id` at the network, in the order they come (a
 *  slow phone network), until the test lets it go: `held[n].release()`;
 *  `held[n].answered` resolves once its answer was handed to the page. */
async function holdPatches(
  page: Page,
  id: number,
): Promise<{ bodies: PatchBody[]; held: Array<{ release: () => void; answered: Promise<void> }> }> {
  const bodies: PatchBody[] = [];
  const held: Array<{ release: () => void; answered: Promise<void> }> = [];
  await page.route(`**/stream/api/nameplates/${id}`, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    bodies.push(route.request().postDataJSON() as PatchBody);
    let release = () => {};
    let answer = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    held.push({ release, answered: new Promise<void>((resolve) => (answer = resolve)) });
    await gate;
    const response = await route.fetch();
    await route.fulfill({ response });
    answer();
  });
  return { bodies, held };
}

/** Fail the first `failures` PATCHes of plate `id` at the network (the
 *  phone's Wi-Fi asleep) and let the later ones through. Returns the body of
 *  every PATCH, in order. */
async function failFirstPatches(page: Page, id: number, failures: number): Promise<PatchBody[]> {
  const patches: PatchBody[] = [];
  await page.route(`**/stream/api/nameplates/${id}`, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    patches.push(route.request().postDataJSON() as PatchBody);
    if (patches.length <= failures) {
      await route.abort();
      return;
    }
    await route.continue();
  });
  return patches;
}

test("#871 every save is a keepalive request, so a reload right after typing keeps the text", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a, b] = await seedPlates(request, [
    ["Kľúč A", "1"],
    ["Kľúč B", "2"],
  ]);
  // The keepalive flag of every nameplate PATCH the page hands to fetch
  // (wasm-bindgen calls the page's `fetch`, so this wrapper sees the Request).
  await page.addInitScript(() => {
    const flags: boolean[] = [];
    Object.assign(window, { __plateKeepalive: flags });
    const original = window.fetch.bind(window);
    window.fetch = (input, init) => {
      if (
        input instanceof Request &&
        input.method === "PATCH" &&
        input.url.includes("/stream/api/nameplates/")
      ) {
        flags.push(input.keepalive);
      }
      return original(input, init);
    };
  });
  const keepaliveFlags = () =>
    page.evaluate(() => (window as unknown as { __plateKeepalive: boolean[] }).__plateKeepalive);
  await openOperator(page, "/ui/operator/menovky");
  await expect.poll(() => rowOrder(page)).toEqual([a, b]);

  // A save is a keepalive request: Chrome cancels a plain fetch when the page
  // unloads, keepalive lets it finish.
  const aName = row(page, a).locator(NAME);
  await aName.fill("Kľúč A uložená");
  await aName.blur();
  await expect.poll(() => savedText(request, a)).toBe("Kľúč A uložená");
  expect(await keepaliveFlags()).toEqual([true]);

  // Half-typed in B, then the page is reloaded (a closed tab is the same):
  // the save made as the page goes away still reaches the server. The new
  // page's list read waits until the server has that save: it could arrive
  // first, and the page would then show the old text with no refresh to
  // follow (its socket may connect after the save's change event).
  const bName = row(page, b).locator(NAME);
  await bName.fill("Kľúč B pred obnovením");
  let serverHasIt = () => {};
  const saved = new Promise<void>((resolve) => (serverHasIt = resolve));
  const listRoute = `**/stream/api/outputs/${SLUG}/nameplates`;
  await page.route(listRoute, async (route) => {
    if (route.request().method() === "GET") await saved;
    // A read of the page being unloaded is gone by now; nothing to answer.
    await route.continue().catch(() => {});
  });
  await page.reload();
  await expect.poll(() => savedText(request, b)).toBe("Kľúč B pred obnovením");
  serverHasIt();
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(row(page, b).locator(NAME)).toHaveValue("Kľúč B pred obnovením");
  await expect(row(page, a).locator(NAME)).toHaveValue("Kľúč A uložená");
  await page.unroute(listRoute);

  expect(consoleErrors).toEqual([]);
});

test("#871 a save waiting for the one on its way goes out at once when the page hides", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [anna] = await seedPlates(request, [["Fronta Anna", "kazateľka"]]);
  await openOperator(page, "/ui/operator/menovky");
  const annaName = row(page, anna).locator(NAME);
  const annaRole = row(page, anna).locator(ROLE);
  await expect(annaName).toHaveValue("Fronta Anna");

  // Every PATCH of Anna is held at the network until the test lets it go,
  // in the order they came (a slow phone network).
  type Body = { primaryText: string; secondaryText: string };
  const bodies: Body[] = [];
  const held: Array<{ release: () => void; answered: Promise<void> }> = [];
  await page.route(`**/stream/api/nameplates/${anna}`, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    bodies.push(route.request().postDataJSON() as Body);
    let release = () => {};
    let answer = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    held.push({ release, answered: new Promise<void>((resolve) => (answer = resolve)) });
    await gate;
    const response = await route.fetch();
    await route.fulfill({ response });
    answer();
  });

  // Two saves within one round trip: the name, Tab to the role (save 1, held),
  // the role, leave it (save 2 waits for save 1).
  const waits = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${anna} save waits for the one on its way`,
    timeout: 10_000,
  });
  await annaName.fill("Fronta Anna Nová");
  await page.keyboard.press("Tab");
  await expect(annaRole).toBeFocused();
  await annaRole.fill("pastorka");
  await annaRole.blur();
  await waits;
  expect(bodies).toHaveLength(1);

  // The phone locks before save 1 is back: save 2 must not wait for a page
  // that may be frozen or killed. It goes out now.
  await setPageHidden(page, true);
  await expect.poll(() => bodies.length).toBe(2);
  expect(bodies).toEqual([
    { primaryText: "Fronta Anna Nová", secondaryText: "kazateľka" },
    { primaryText: "Fronta Anna Nová", secondaryText: "pastorka" },
  ]);
  held[0].release();
  await held[0].answered;
  held[1].release();
  await held[1].answered;
  await expect
    .poll(async () => (await listPlates(request)).find((p) => p.id === anna))
    .toMatchObject({ primaryText: "Fronta Anna Nová", secondaryText: "pastorka" });
  await setPageHidden(page, false);
  await expect(annaRole).toHaveValue("pastorka");
  await page.unrouteAll({ behavior: "wait" });
  expect(bodies).toHaveLength(2);

  expect(consoleErrors).toEqual([]);
});

test("#871 a save that fails on the network keeps the typed text, says so, and goes again on the next leave", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a] = await seedPlates(request, [["Sieť A", "1"]]);
  // The first PATCH fails at the network (the Wi-Fi asleep), later ones pass.
  const patches: string[] = [];
  await page.route(`**/stream/api/nameplates/${a}`, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    patches.push((route.request().postDataJSON() as { primaryText: string }).primaryText);
    if (patches.length === 1) {
      await route.abort();
      return;
    }
    await route.continue();
  });
  await openOperator(page, "/ui/operator/menovky");
  const aName = row(page, a).locator(NAME);
  await expect(aName).toHaveValue("Sieť A");
  const toast = page.locator('[data-role="toast"]');

  await aName.fill("Sieť A nová");
  await aName.blur();
  await expect(toast).toContainText(/^Uloženie zlyhalo: .*Text ostal v poli/);
  await expect(toast).toHaveAttribute("data-visible", "true");
  // The typed text stays in the field; the server still has the old one.
  await expect(aName).toHaveValue("Sieť A nová");
  expect(await savedText(request, a)).toBe("Sieť A");

  // Leaving the field again sends it again, and it is saved.
  await aName.focus();
  await aName.blur();
  await expect.poll(() => savedText(request, a)).toBe("Sieť A nová");
  await expect(aName).toHaveValue("Sieť A nová");
  expect(patches).toEqual(["Sieť A nová", "Sieť A nová"]);
  await page.unrouteAll({ behavior: "wait" });

  // Chrome logs the one aborted fetch itself; nothing else may be logged.
  const lines = splitConsole(consoleErrors, NETWORK_FAILED);
  expect(lines.expected).toHaveLength(1);
  expect(lines.other).toEqual([]);
});

test("#871 a save that fails while the page is hidden keeps the text and goes again once the page is back in view", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [c] = await seedPlates(request, [["Zámok C", "3"]]);
  const patches: string[] = [];
  await page.route(`**/stream/api/nameplates/${c}`, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    patches.push((route.request().postDataJSON() as { primaryText: string }).primaryText);
    if (patches.length === 1) {
      await route.abort();
      return;
    }
    await route.continue();
  });
  await openOperator(page, "/ui/operator/menovky");
  const cName = row(page, c).locator(NAME);
  await expect(cName).toHaveValue("Zámok C");
  const toast = page.locator('[data-role="toast"]');

  // Typed, then the phone locks: the save goes, and fails (Wi-Fi asleep).
  await cName.fill("Zámok C nová");
  await setPageHidden(page, true);
  await expect(toast).toContainText("Text ostal v poli");
  await expect(cName).toHaveValue("Zámok C nová");
  expect(await savedText(request, c)).toBe("Zámok C");

  // Unlocked: the page is back in view and sends it again.
  await setPageHidden(page, false);
  await expect.poll(() => savedText(request, c)).toBe("Zámok C nová");
  await expect(cName).toHaveValue("Zámok C nová");
  expect(patches).toEqual(["Zámok C nová", "Zámok C nová"]);
  await page.unrouteAll({ behavior: "wait" });

  const lines = splitConsole(consoleErrors, NETWORK_FAILED);
  expect(lines.expected).toHaveLength(1);
  expect(lines.other).toEqual([]);
});

test("#871 a save of a plate deleted meanwhile (404) says the plate was deleted", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a, b] = await seedPlates(request, [
    ["Neskoro A", "1"],
    ["Neskoro B", "2"],
  ]);
  await openOperator(page, "/ui/operator/menovky");
  await expect.poll(() => rowOrder(page)).toEqual([a, b]);
  // The page's list reloads are held, so B's row is still on screen when
  // its save reaches the server after another operator deleted B.
  const heldLists: Array<() => void> = [];
  let holdLists = true;
  await page.route(`**/stream/api/outputs/${SLUG}/nameplates`, async (route) => {
    if (route.request().method() === "GET" && holdLists) {
      await new Promise<void>((resolve) => heldLists.push(resolve));
    }
    await route.continue();
  });
  const bName = row(page, b).locator(NAME);
  await bName.fill("Neskoro B rozpísaná");
  await deletePlate(request, b);
  // The page heard the delete and asked for the list (held).
  await expect.poll(() => heldLists.length).toBeGreaterThan(0);
  await bName.blur();
  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toHaveText(DELETED_TOAST);
  await expect(toast).toHaveAttribute("data-visible", "true");

  // The list comes back without B.
  holdLists = false;
  for (const release of heldLists) release();
  await expect.poll(() => rowOrder(page)).toEqual([a]);
  await page.unrouteAll({ behavior: "wait" });

  const lines = splitConsole(
    consoleErrors,
    /^\[error\] Failed to load resource: the server responded with a status of 404\b/,
  );
  expect(lines.expected).toHaveLength(1);
  expect(lines.other).toEqual([]);
});

test("#871 leaving the window saves even where the page still reports the focus when the save decides", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a] = await seedPlates(request, [["Okno A", "1"]]);
  const patches: string[] = [];
  page.on("request", (req) => {
    if (req.method() === "PATCH" && req.url().endsWith(`/stream/api/nameplates/${a}`)) {
      patches.push((JSON.parse(req.postData() ?? "{}") as { primaryText: string }).primaryText);
    }
  });
  await openOperator(page, "/ui/operator/menovky");
  const aName = row(page, a).locator(NAME);
  await expect(aName).toHaveValue("Okno A");

  // Safari / Firefox timing: the window loses the focus and the field gets
  // its blur, but `document.hasFocus()` still says true when the page
  // decides (Chrome already says false). No rows were moving: this is a
  // window left, so it saves.
  await aName.fill("Okno A nová");
  await windowBlur(aName, true);
  expect(await aName.evaluate((el) => document.activeElement === el && document.hasFocus())).toBe(
    true,
  );
  await expect.poll(() => savedText(request, a)).toBe("Okno A nová");
  expect(patches).toEqual(["Okno A nová"]);
  await expect(aName).toHaveValue("Okno A nová");

  expect(consoleErrors).toEqual([]);
});

test("#871 a name cleared to retype it is kept when the window loses the focus or the page hides; leaving the field still refuses it", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a] = await seedPlates(request, [["Prázdne A", "1"]]);
  const patches: string[] = [];
  page.on("request", (req) => {
    if (req.method() === "PATCH" && req.url().endsWith(`/stream/api/nameplates/${a}`)) {
      patches.push((JSON.parse(req.postData() ?? "{}") as { primaryText: string }).primaryText);
    }
  });
  await openOperator(page, "/ui/operator/menovky");
  const aName = row(page, a).locator(NAME);
  const toast = page.locator('[data-role="toast"]');
  await expect(aName).toHaveValue("Prázdne A");
  const keptEmpty = () =>
    page.waitForEvent("console", {
      predicate: (msg) => msg.text().startsWith(`nameplates: plate ${a} name is empty; kept as typed`),
      timeout: 10_000,
    });

  // The operator clears the name to retype it, then alt-tabs (OBS): no
  // "Meno nesmie byť prázdne.", the field stays empty, nothing is sent.
  await aName.fill("");
  const keptOnBlur = keptEmpty();
  await windowBlur(aName, false);
  await keptOnBlur;
  await expect(aName).toHaveValue("");
  expect(await toast.getAttribute("data-visible")).toBe("false");
  await restoreWindowFocus(page);

  // ... or the phone locks: the same.
  const keptOnHide = keptEmpty();
  await setPageHidden(page, true);
  await keptOnHide;
  await expect(aName).toHaveValue("");
  expect(await toast.getAttribute("data-visible")).toBe("false");
  await setPageHidden(page, false);

  // Back, the new name typed and Enter: saved.
  await aName.fill("Prázdne A nová");
  await aName.press("Enter");
  await expect.poll(() => savedText(request, a)).toBe("Prázdne A nová");

  // Leaving the field (a click elsewhere) with the name empty is refused,
  // as before: the toast, and the field shows the saved name again.
  await aName.fill("");
  await aName.blur();
  await expect(toast).toHaveText("Meno nesmie byť prázdne.");
  await expect(aName).toHaveValue("Prázdne A nová");
  expect(patches).toEqual(["Prázdne A nová"]);

  expect(consoleErrors).toEqual([]);
});

test("#871 a plate deleted elsewhere while its text is half-typed: a toast says the change was not saved", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a, b] = await seedPlates(request, [
    ["Zmaz A", "1"],
    ["Zmaz B", "2"],
  ]);
  const patches: string[] = [];
  page.on("request", (req) => {
    if (req.method() === "PATCH") patches.push(req.url());
  });
  await openOperator(page, "/ui/operator/menovky");
  await expect.poll(() => rowOrder(page)).toEqual([a, b]);

  // Typing in B (not saved yet) when another operator deletes B.
  const bName = row(page, b).locator(NAME);
  await bName.fill("Zmaz B rozpísaná");
  await deletePlate(request, b);
  await expect(row(page, b)).toHaveCount(0);
  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toHaveText("Menovka bola medzičasom zmazaná — zmena sa neuložila.");
  await expect(toast).toHaveAttribute("data-visible", "true");
  expect(patches, "nothing is saved for a deleted plate").toEqual([]);
  await expect.poll(() => rowOrder(page)).toEqual([a]);

  expect(consoleErrors).toEqual([]);
});

test("#871 two saves of one plate go one at a time, so the text saved last is the one kept", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [anna] = await seedPlates(request, [["Ulož Anna", "kazateľka"]]);
  await openOperator(page, "/ui/operator/menovky");
  const annaName = row(page, anna).locator(NAME);
  const annaRole = row(page, anna).locator(ROLE);
  await expect(annaName).toHaveValue("Ulož Anna");

  // The first save is held at the network until the page says the second save
  // WAITS for it (its own log line), or until a second PATCH arrives, whichever
  // comes first. A second PATCH that arrives first is let through FIRST, as a
  // server applying the two out of order would, and fails the test below. No
  // timer: the page's own signal releases the first save on any runner.
  type Body = { primaryText: string; secondaryText: string };
  const bodies: Body[] = [];
  let queuedSeen = () => {};
  const queued = new Promise<"queued">((resolve) => (queuedSeen = () => resolve("queued")));
  page.on("console", (msg) => {
    if (msg.text() === `nameplates: plate ${anna} save waits for the one on its way`) {
      queuedSeen();
    }
  });
  let secondSeen = () => {};
  const second = new Promise<"second">((resolve) => (secondSeen = () => resolve("second")));
  let secondApplied = () => {};
  const secondDone = new Promise<void>((resolve) => (secondApplied = resolve));
  let firstReleasedBy: "queued" | "second" | undefined;
  await page.route(`**/stream/api/nameplates/${anna}`, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    const n = bodies.push(route.request().postDataJSON() as Body);
    if (n === 1) {
      firstReleasedBy = await Promise.race([queued, second]);
      if (firstReleasedBy === "second") await secondDone;
      await route.continue();
      return;
    }
    if (n === 2) secondSeen();
    const response = await route.fetch();
    secondApplied();
    await route.fulfill({ response });
  });

  // The name, Tab to the role (save 1), the role, leave it (save 2): both
  // within one round trip.
  await annaName.fill("Ulož Anna Nová");
  await page.keyboard.press("Tab");
  await expect(annaRole).toBeFocused();
  await annaRole.fill("pastorka");
  await annaRole.blur();

  await expect.poll(() => bodies.length).toBe(2);
  expect(firstReleasedBy, "the second save waited for the first").toBe("queued");
  expect(bodies).toEqual([
    { primaryText: "Ulož Anna Nová", secondaryText: "kazateľka" },
    { primaryText: "Ulož Anna Nová", secondaryText: "pastorka" },
  ]);
  await expect
    .poll(async () => (await listPlates(request)).find((p) => p.id === anna))
    .toMatchObject({ primaryText: "Ulož Anna Nová", secondaryText: "pastorka" });
  await expect(annaName).toHaveValue("Ulož Anna Nová");
  await expect(annaRole).toHaveValue("pastorka");
  await page.unrouteAll({ behavior: "wait" });

  expect(consoleErrors).toEqual([]);
});

test("#871 a failed save marks its row unsaved and goes again by itself until it is saved", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a] = await seedPlates(request, [["Znova A", "1"]]);
  // The first two PATCHes fail at the network (the Wi-Fi still asleep), the
  // third goes through.
  const patches = await failFirstPatches(page, a, 2);
  await openOperator(page, "/ui/operator/menovky");
  const aRow = row(page, a);
  const aName = aRow.locator(NAME);
  await expect(aName).toHaveValue("Znova A");
  await expect(aRow).toHaveAttribute("data-unsaved", "false");
  const toast = page.locator('[data-role="toast"]');

  await aName.fill("Znova A nová");
  await aName.blur();
  // The row itself says the text is not saved, for as long as it is not (the
  // toast is gone after 3.5 s), and the toast says it goes again by itself.
  await expect(aRow).toHaveAttribute("data-unsaved", "true");
  await expect(aRow.locator(UNSAVED)).toHaveText("neuložené");
  await expect(toast).toContainText("uloží sa znova sám");
  await expect(aName).toHaveValue("Znova A nová");

  // No click, no leave, no page change: the page tries again after a short
  // pause (2 s, then 5 s), and the third try is saved.
  await expect.poll(() => savedText(request, a), { timeout: 20_000 }).toBe("Znova A nová");
  await expect(aRow).toHaveAttribute("data-unsaved", "false");
  await expect(aRow.locator(UNSAVED)).toHaveCount(0);
  await expect(aName).toHaveValue("Znova A nová");
  expect(patches.map((p) => p.primaryText)).toEqual(["Znova A nová", "Znova A nová", "Znova A nová"]);
  await page.unrouteAll({ behavior: "wait" });

  const lines = splitConsole(consoleErrors, NETWORK_FAILED);
  expect(lines.expected).toHaveLength(2);
  expect(lines.other).toEqual([]);
});

test("#871 a failed save goes again as soon as the network is back", async ({ page, request }) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a] = await seedPlates(request, [["Sieť späť A", "1"]]);
  const patches = await failFirstPatches(page, a, 1);
  await openOperator(page, "/ui/operator/menovky");
  const aName = row(page, a).locator(NAME);
  await expect(aName).toHaveValue("Sieť späť A");
  const toast = page.locator('[data-role="toast"]');

  // The phone's Wi-Fi is back and the browser says so, in the same pass as
  // the row's unsaved mark (so the page's own 2 s retry can never go first):
  // the page sends the text at once.
  await onUnsavedMark(page, a, "online");
  const resent = page.waitForEvent("console", {
    predicate: (msg) =>
      msg.text().startsWith(`nameplates: plate ${a} failed save`) &&
      msg.text().endsWith("(the network is back)"),
    timeout: 10_000,
  });
  await aName.fill("Sieť späť A nová");
  await aName.blur();
  expect((await resent).text()).toBe(
    `nameplates: plate ${a} failed save sent again (the network is back)`,
  );
  await expect(toast).toContainText("Text ostal v poli");
  expect(await unsavedMarkSeen(page, a)).toBe(true);
  await expect.poll(() => savedText(request, a)).toBe("Sieť späť A nová");
  await expect(row(page, a)).toHaveAttribute("data-unsaved", "false");
  expect(patches.map((p) => p.primaryText)).toEqual(["Sieť späť A nová", "Sieť späť A nová"]);
  await page.unrouteAll({ behavior: "wait" });

  const lines = splitConsole(consoleErrors, NETWORK_FAILED);
  expect(lines.expected).toHaveLength(1);
  expect(lines.other).toEqual([]);
});

test("#871 Zobraziť on a row whose save failed saves the text first and puts the saved text on air", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  await setActive(request, { source: null });
  const [a] = await seedPlates(request, [["Vzduch A", "1"]]);
  const patches = await failFirstPatches(page, a, 1);
  await openOperator(page, "/ui/operator/menovky");
  const aRow = row(page, a);
  const aName = aRow.locator(NAME);
  await expect(aName).toHaveValue("Vzduch A");

  // Zobraziť is pressed in the same pass as the row's unsaved mark, so the
  // row is still failed (the page's own 2 s retry cannot go first). The
  // server puts on air the text it HAS: shown at once, the old name would go
  // on air while the field shows the new one. So the page saves the typed
  // text first and shows the plate once that is saved.
  await onUnsavedMark(page, a, "show");
  await aName.fill("Vzduch A nová");
  await aName.blur();
  await expect(page.locator('[data-role="toast"]')).toContainText("Text ostal v poli");
  await expect.poll(() => unsavedMarkSeen(page, a)).toBe(true);
  await expect.poll(async () => (await activePlate(request))?.primary).toBe("Vzduch A nová");
  await expect(aRow).toHaveAttribute("data-active", "true");
  await expect(aRow).toHaveAttribute("data-unsaved", "false");
  expect(await savedText(request, a)).toBe("Vzduch A nová");
  expect(patches.map((p) => p.primaryText)).toEqual(["Vzduch A nová", "Vzduch A nová"]);
  await setActive(request, { source: null });
  await page.unrouteAll({ behavior: "wait" });

  const lines = splitConsole(consoleErrors, NETWORK_FAILED);
  expect(lines.expected).toHaveLength(1);
  expect(lines.other).toEqual([]);
});

test("#871 a save sent at once as the page hid keeps the next save of the plate waiting until it is back", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [anna] = await seedPlates(request, [["Rad Anna", "kazateľka"]]);
  await openOperator(page, "/ui/operator/menovky");
  const annaName = row(page, anna).locator(NAME);
  const annaRole = row(page, anna).locator(ROLE);
  await expect(annaName).toHaveValue("Rad Anna");
  const { bodies, held } = await holdPatches(page, anna);
  const waitsLine = `nameplates: plate ${anna} save waits for the one on its way`;

  // Save 1 (the name, Tab) is held; save 2 (the role) waits for it; the phone
  // locks, so save 2 goes at once beside save 1. The name ends in a space,
  // which the save trims: the field showing the trimmed name later proves the
  // page has dealt with save 1's answer.
  const waits = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === waitsLine,
    timeout: 10_000,
  });
  await annaName.fill("Rad Anna Nová ");
  await page.keyboard.press("Tab");
  await expect(annaRole).toBeFocused();
  await annaRole.fill("pastorka");
  await annaRole.blur();
  await waits;
  await setPageHidden(page, true);
  await expect.poll(() => bodies.length).toBe(2);
  await setPageHidden(page, false);

  // Save 1 is back; save 2 is still on its way.
  held[0].release();
  await held[0].answered;
  await expect(annaName).toHaveValue("Rad Anna Nová");

  // The next save of Anna waits for save 2: sent beside it, a server applying
  // the two out of order would keep the older role.
  const decided = page.waitForEvent("console", {
    predicate: (msg) =>
      msg.text() === waitsLine || msg.text() === `nameplates: plate ${anna} save sent`,
    timeout: 10_000,
  });
  await annaRole.fill("pastorka 2");
  await annaRole.blur();
  expect((await decided).text()).toBe(waitsLine);
  expect(bodies).toHaveLength(2);

  // Save 2 is back: save 3 goes, and the server ends on the newest role.
  held[1].release();
  await held[1].answered;
  await expect.poll(() => bodies.length).toBe(3);
  held[2].release();
  await held[2].answered;
  await expect
    .poll(async () => (await listPlates(request)).find((p) => p.id === anna))
    .toMatchObject({ primaryText: "Rad Anna Nová", secondaryText: "pastorka 2" });
  expect(bodies).toEqual([
    { primaryText: "Rad Anna Nová", secondaryText: "kazateľka" },
    { primaryText: "Rad Anna Nová", secondaryText: "pastorka" },
    { primaryText: "Rad Anna Nová", secondaryText: "pastorka 2" },
  ]);
  await expect(annaRole).toHaveValue("pastorka 2");
  await page.unrouteAll({ behavior: "wait" });

  expect(consoleErrors).toEqual([]);
});

test("#871 when an older save of a plate is answered after the newer one, the newer texts are sent once more", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [anna] = await seedPlates(request, [["Skôr Anna", "kazateľka"]]);
  await openOperator(page, "/ui/operator/menovky");
  const annaName = row(page, anna).locator(NAME);
  const annaRole = row(page, anna).locator(ROLE);
  await expect(annaName).toHaveValue("Skôr Anna");
  const { bodies, held } = await holdPatches(page, anna);
  const savedPlate = async () => (await listPlates(request)).find((p) => p.id === anna);

  // Save 1 (the name, Tab) is held, save 2 (the role) waits, and the phone
  // locks: save 2 goes at once beside save 1.
  const waits = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${anna} save waits for the one on its way`,
    timeout: 10_000,
  });
  await annaName.fill("Skôr Anna Nová");
  await page.keyboard.press("Tab");
  await expect(annaRole).toBeFocused();
  await annaRole.fill("pastorka");
  await annaRole.blur();
  await waits;
  await setPageHidden(page, true);
  await expect.poll(() => bodies.length).toBe(2);
  await setPageHidden(page, false);

  // The newer save reaches the server first, the older one after it: the
  // server now holds the OLDER role ("sent last" is not "applied last").
  held[1].release();
  await held[1].answered;
  await expect.poll(async () => (await savedPlate())?.secondaryText).toBe("pastorka");
  held[0].release();
  await held[0].answered;
  await expect.poll(async () => (await savedPlate())?.secondaryText).toBe("kazateľka");

  // The page heard the older answer last, so it sends the newest texts once
  // more, and the server ends on them.
  await expect.poll(() => bodies.length).toBe(3);
  expect(bodies[2]).toEqual({ primaryText: "Skôr Anna Nová", secondaryText: "pastorka" });
  held[2].release();
  await held[2].answered;
  await expect
    .poll(savedPlate)
    .toMatchObject({ primaryText: "Skôr Anna Nová", secondaryText: "pastorka" });
  await expect(annaRole).toHaveValue("pastorka");
  await page.unrouteAll({ behavior: "wait" });
  expect(bodies).toHaveLength(3);

  expect(consoleErrors).toEqual([]);
});

test("#871 a save of a deleted plate (404) drops the save waiting behind it: one toast, no second request", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a, b] = await seedPlates(request, [
    ["Zhoda A", "1"],
    ["Zhoda B", "2"],
  ]);
  await openOperator(page, "/ui/operator/menovky");
  await expect.poll(() => rowOrder(page)).toEqual([a, b]);
  // The page's list reloads are held, so B's row stays on screen.
  const heldLists: Array<() => void> = [];
  let holdLists = true;
  await page.route(`**/stream/api/outputs/${SLUG}/nameplates`, async (route) => {
    if (route.request().method() === "GET" && holdLists) {
      await new Promise<void>((resolve) => heldLists.push(resolve));
    }
    await route.continue();
  });
  const { bodies, held } = await holdPatches(page, b);
  const bName = row(page, b).locator(NAME);
  const bRole = row(page, b).locator(ROLE);

  // Save 1 (the name, Tab) is on its way; save 2 (the role) waits for it.
  const waits = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${b} save waits for the one on its way`,
    timeout: 10_000,
  });
  await bName.fill("Zhoda B nová");
  await page.keyboard.press("Tab");
  await expect(bRole).toBeFocused();
  await bRole.fill("nová rola");
  await bRole.blur();
  await waits;

  // Another operator deletes B meanwhile; the page hears it (list read held).
  await deletePlate(request, b);
  await expect.poll(() => heldLists.length).toBeGreaterThan(0);
  const listReads = heldLists.length;
  held[0].release();
  await held[0].answered;
  await expect(page.locator('[data-role="toast"]')).toHaveText(DELETED_TOAST);

  // Once the 404 is dealt with, the page either sends the waiting save or,
  // with nothing of B left to send, reloads the list. The waiting save must
  // not go: it could only be refused again (a second 404 and toast).
  await expect.poll(() => heldLists.length > listReads || bodies.length > 1).toBe(true);
  expect(bodies).toHaveLength(1);

  // The list comes back without B.
  holdLists = false;
  for (const release of heldLists) release();
  await expect.poll(() => rowOrder(page)).toEqual([a]);
  await page.unrouteAll({ behavior: "wait" });

  const lines = splitConsole(
    consoleErrors,
    /^\[error\] Failed to load resource: the server responded with a status of 404\b/,
  );
  expect(lines.expected).toHaveLength(1);
  expect(lines.other).toEqual([]);
});

test("#871 a name cleared to retype it does not hold back a role typed in the same row when the page hides", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a] = await seedPlates(request, [["Rola A", "1"]]);
  const { bodies: patches, failFirst } = await failFirstPatchOnCue(page, a);
  await openOperator(page, "/ui/operator/menovky");
  const aName = row(page, a).locator(NAME);
  const aRole = row(page, a).locator(ROLE);
  await expect(aName).toHaveValue("Rola A");

  // The role is typed and left; its save is held at the network.
  await aRole.fill("nová rola");
  await aRole.blur();
  await expect.poll(() => patches.length).toBe(1);

  // The name is cleared to retype it. Then the role's save fails (Wi-Fi
  // asleep) and, in the same pass as the row's unsaved mark (so the page's
  // own 2 s retry cannot go first), the phone locks: the name stays as typed
  // (empty, never saved), and the role goes now, with the newest name.
  await aName.fill("");
  await onUnsavedMark(page, a, "hide");
  const decided = page.waitForEvent("console", {
    predicate: (msg) => msg.text().startsWith(`nameplates: plate ${a} name is empty; kept as typed`),
    timeout: 10_000,
  });
  failFirst();
  expect((await decided).text()).toBe(
    `nameplates: plate ${a} name is empty; kept as typed, the role sent with the newest name (the page went hidden)`,
  );
  await expect(page.locator('[data-role="toast"]')).toContainText("Text ostal v poli");
  await expect
    .poll(async () => (await listPlates(request)).find((p) => p.id === a))
    .toMatchObject({ primaryText: "Rola A", secondaryText: "nová rola" });
  await setPageHidden(page, false);
  await expect(aName).toHaveValue("");
  await expect(aRole).toHaveValue("nová rola");
  expect(patches).toEqual([
    { primaryText: "Rola A", secondaryText: "nová rola" },
    { primaryText: "Rola A", secondaryText: "nová rola" },
  ]);
  await page.unrouteAll({ behavior: "wait" });

  const lines = splitConsole(consoleErrors, NETWORK_FAILED);
  expect(lines.expected).toHaveLength(1);
  expect(lines.other).toEqual([]);
});

/** When row `id` gets its unsaved mark, do `action` at once, inside the
 *  MutationObserver callback (the same microtask pass that set the mark), so
 *  the page's own 2 s retry timer can never act first: the window `online`
 *  event, the page going hidden (`visibilityState` + a bubbling
 *  `visibilitychange`), or a click on the row's Zobraziť. Install it BEFORE
 *  the action that fails the save; `unsavedMarkSeen` tells whether it fired. */
async function onUnsavedMark(page: Page, id: number, action: "online" | "hide" | "show"): Promise<void> {
  await page.evaluate(
    ({ id, action }) => {
      const row = document.querySelector(
        `[data-role="operator-nameplates"] [data-role="stream-nameplate"][data-nameplate-id="${id}"]`,
      );
      if (!row) throw new Error(`row ${id} is not on the page`);
      const seen = window as unknown as { __unsavedMarkSeen?: number[] };
      const observer = new MutationObserver(() => {
        if (row.getAttribute("data-unsaved") !== "true") return;
        observer.disconnect();
        seen.__unsavedMarkSeen = [...(seen.__unsavedMarkSeen ?? []), id];
        if (action === "online") {
          window.dispatchEvent(new Event("online"));
        } else if (action === "hide") {
          Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
          document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
        } else {
          (row.querySelector('[data-role="stream-nameplate-show"]') as HTMLElement).click();
        }
      });
      observer.observe(row, { attributes: true, attributeFilter: ["data-unsaved"] });
    },
    { id, action },
  );
}

/** Whether the unsaved mark of row `id` was seen by [`onUnsavedMark`]. */
async function unsavedMarkSeen(page: Page, id: number): Promise<boolean> {
  return page.evaluate(
    (id) =>
      ((window as unknown as { __unsavedMarkSeen?: number[] }).__unsavedMarkSeen ?? []).includes(id),
    id,
  );
}

/** Hold the first PATCH of plate `id` at the network until `failFirst()`
 *  makes it fail (the Wi-Fi asleep); later PATCHes go through. Returns the
 *  body of every PATCH, in order. */
async function failFirstPatchOnCue(
  page: Page,
  id: number,
): Promise<{ bodies: PatchBody[]; failFirst: () => void }> {
  const bodies: PatchBody[] = [];
  let failFirst = () => {};
  const failed = new Promise<void>((resolve) => (failFirst = resolve));
  await page.route(`**/stream/api/nameplates/${id}`, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    const n = bodies.push(route.request().postDataJSON() as PatchBody);
    if (n === 1) {
      await failed;
      await route.abort();
      return;
    }
    await route.continue();
  });
  return { bodies, failFirst };
}

test("#871 a commit while a save is on its way is sent after it, once, with the newest texts, and Zobraziť meanwhile waits for it", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  await setActive(request, { source: null });
  const [a] = await seedPlates(request, [["Poradie A", "1"]]);
  await openOperator(page, "/ui/operator/menovky");
  const aRow = row(page, a);
  const aName = aRow.locator(NAME);
  await expect(aName).toHaveValue("Poradie A");
  const { bodies, held } = await holdPatches(page, a);
  const logs: string[] = [];
  page.on("console", (msg) => logs.push(msg.text()));
  const waitsLine = `nameplates: plate ${a} save waits for the one on its way`;

  // Save 1 is held; two more commits wait behind it (the newest replaces).
  await aName.fill("Poradie A1");
  await aName.blur();
  await expect.poll(() => bodies.length).toBe(1);
  await aName.fill("Poradie A2");
  await aName.blur();
  await aName.fill("Poradie A3");
  await aName.blur();
  await expect.poll(() => logs.filter((line) => line === waitsLine).length).toBe(2);

  // Zobraziť while save 1 is on its way: the server would put "A1" on air.
  const waitsForSave = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${a} goes on air once its save is through`,
    timeout: 10_000,
  });
  await aRow.locator('[data-role="stream-nameplate-show"]').click();
  await waitsForSave;
  expect(await activePlate(request)).toBeNull();

  // Save 1 is back: the newest texts go next, once.
  held[0].release();
  await held[0].answered;
  await expect.poll(() => bodies.length).toBe(2);
  expect(bodies[1]).toEqual({ primaryText: "Poradie A3", secondaryText: "1" });
  expect(await activePlate(request)).toBeNull();

  // Save 2 is back: the plate goes on air with the newest text.
  held[1].release();
  await held[1].answered;
  await expect.poll(async () => (await activePlate(request))?.primary).toBe("Poradie A3");
  await expect(aRow).toHaveAttribute("data-active", "true");
  await expect(aName).toHaveValue("Poradie A3");
  expect(bodies).toHaveLength(2);
  await setActive(request, { source: null });
  await page.unrouteAll({ behavior: "wait" });

  expect(consoleErrors).toEqual([]);
});

test("#871 Zobraziť while the page repairs an older save answered last waits for the repair, so the newest role goes on air", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  await setActive(request, { source: null });
  const [anna] = await seedPlates(request, [["Oprava Anna", "kazateľka"]]);
  await openOperator(page, "/ui/operator/menovky");
  const annaRow = row(page, anna);
  const annaName = annaRow.locator(NAME);
  const annaRole = annaRow.locator(ROLE);
  await expect(annaName).toHaveValue("Oprava Anna");
  const { bodies, held } = await holdPatches(page, anna);

  // Save 1 (the name, Tab) is held, save 2 (the role) waits, and the phone
  // locks: save 2 goes at once beside save 1.
  const waits = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${anna} save waits for the one on its way`,
    timeout: 10_000,
  });
  await annaName.fill("Oprava Anna Nová");
  await page.keyboard.press("Tab");
  await expect(annaRole).toBeFocused();
  await annaRole.fill("pastorka");
  await annaRole.blur();
  await waits;
  await setPageHidden(page, true);
  await expect.poll(() => bodies.length).toBe(2);
  await setPageHidden(page, false);

  // The newer save is answered first, the older one last: the server holds
  // the OLDER role now, and the page sends the newest texts once more.
  held[1].release();
  await held[1].answered;
  held[0].release();
  await held[0].answered;
  await expect.poll(() => bodies.length).toBe(3);

  // Zobraziť while that repair is on its way must wait for it: shown now, the
  // older role would go on air.
  await annaRow.locator('[data-role="stream-nameplate-show"]').click();
  held[2].release();
  await held[2].answered;
  await expect
    .poll(async () => {
      const active = await activePlate(request);
      return active && { primary: active.primary, secondary: active.secondary };
    })
    .toEqual({ primary: "Oprava Anna Nová", secondary: "pastorka" });
  await expect(annaRow).toHaveAttribute("data-active", "true");
  expect(bodies).toHaveLength(3);
  await setActive(request, { source: null });
  await page.unrouteAll({ behavior: "wait" });

  expect(consoleErrors).toEqual([]);
});

test("#871 a text typed, changed and typed back while its save is on its way is saved once more and never left marked unsaved", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a] = await seedPlates(request, [["Späť A", "1"]]);
  await openOperator(page, "/ui/operator/menovky");
  const aRow = row(page, a);
  const aName = aRow.locator(NAME);
  await expect(aName).toHaveValue("Späť A");
  const { bodies, failFirst } = await failFirstPatchOnCue(page, a);
  const decisions: string[] = [];
  let decide: (how: "idle" | "third") => void = () => {};
  const decided = new Promise<"idle" | "third">((resolve) => (decide = resolve));
  page.on("console", (msg) => {
    const text = msg.text();
    if (
      text === `nameplates: plate ${a} save waits for the one on its way` ||
      text.startsWith(`nameplates: plate ${a} save of these texts is on its way`)
    ) {
      decisions.push(text);
    }
    if (text === `nameplates: plate ${a} saved; nothing left to send`) decide("idle");
  });
  let patchCount = 0;
  page.on("request", (req) => {
    if (req.method() === "PATCH" && req.url().endsWith(`/stream/api/nameplates/${a}`)) {
      patchCount += 1;
      if (patchCount === 3) decide("third");
    }
  });

  // A (save 1, held), B, and back to A: the text of the save on its way.
  await aName.fill("Späť A nová");
  await aName.blur();
  await expect.poll(() => bodies.length).toBe(1);
  await aName.fill("Späť A iná");
  await aName.blur();
  await aName.fill("Späť A nová");
  await aName.blur();
  await expect.poll(() => decisions.length).toBe(2);

  // Save 1 fails; the network is back in the same pass as the unsaved mark.
  // The page then sends A once more and is done: either it says so, or a
  // third PATCH comes first (A sent again after it was saved).
  await onUnsavedMark(page, a, "online");
  failFirst();
  expect(await decided, "nothing more is sent once A is saved").toBe("idle");
  expect(await unsavedMarkSeen(page, a)).toBe(true);
  expect(bodies).toEqual([
    { primaryText: "Späť A nová", secondaryText: "1" },
    { primaryText: "Späť A nová", secondaryText: "1" },
  ]);
  // Read once, now that the page has dealt with the answer.
  expect(await aRow.getAttribute("data-unsaved")).toBe("false");
  expect(await savedText(request, a)).toBe("Späť A nová");
  await expect(aName).toHaveValue("Späť A nová");
  await page.unrouteAll({ behavior: "wait" });

  const lines = splitConsole(consoleErrors, NETWORK_FAILED);
  expect(lines.expected).toHaveLength(1);
  expect(lines.other).toEqual([]);
});

test("#871 an empty name on leaving the field is refused before anything is sent: the role's save on its way keeps its text and goes again when it fails", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [b] = await seedPlates(request, [["Prázdna B", "1"]]);
  await openOperator(page, "/ui/operator/menovky");
  const bRow = row(page, b);
  const bName = bRow.locator(NAME);
  const bRole = bRow.locator(ROLE);
  const toast = page.locator('[data-role="toast"]');
  await expect(bName).toHaveValue("Prázdna B");
  const { bodies, failFirst } = await failFirstPatchOnCue(page, b);

  // The role is saved (held at the network) ...
  await bRole.fill("nová rola");
  await bRole.blur();
  await expect.poll(() => bodies.length).toBe(1);
  // ... and the name is cleared and left: refused, the old name stays.
  await bName.fill("");
  await bName.blur();
  await expect(toast).toHaveText("Meno nesmie byť prázdne.");
  await expect(bName).toHaveValue("Prázdna B");

  // The role's save fails: its text stays typed, the row is marked, and
  // (the network back at once) it goes again and is saved.
  await onUnsavedMark(page, b, "online");
  failFirst();
  await expect(toast).toContainText("Text ostal v poli");
  await expect.poll(() => unsavedMarkSeen(page, b), { timeout: 5_000 }).toBe(true);
  await expect(bRole).toHaveValue("nová rola");
  await expect
    .poll(async () => (await listPlates(request)).find((p) => p.id === b))
    .toMatchObject({ primaryText: "Prázdna B", secondaryText: "nová rola" });
  await expect(bRow).toHaveAttribute("data-unsaved", "false");
  await expect(bName).toHaveValue("Prázdna B");
  await expect(bRole).toHaveValue("nová rola");
  expect(bodies).toEqual([
    { primaryText: "Prázdna B", secondaryText: "nová rola" },
    { primaryText: "Prázdna B", secondaryText: "nová rola" },
  ]);
  await page.unrouteAll({ behavior: "wait" });

  const lines = splitConsole(consoleErrors, NETWORK_FAILED);
  expect(lines.expected).toHaveLength(1);
  expect(lines.other).toEqual([]);
});

test("#871 a name cleared while its new text is still saving: the role goes with that NEWEST name when the page hides", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a] = await seedPlates(request, [["Najnovšie A", "1"]]);
  await openOperator(page, "/ui/operator/menovky");
  const aName = row(page, a).locator(NAME);
  const aRole = row(page, a).locator(ROLE);
  await expect(aName).toHaveValue("Najnovšie A");
  const { bodies, held } = await holdPatches(page, a);

  // The new name is saved (held), the role is typed and left (waits), and
  // the name is cleared to retype it.
  const waits = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${a} save waits for the one on its way`,
    timeout: 10_000,
  });
  await aName.fill("Najnovšie A nová");
  await page.keyboard.press("Tab");
  await expect(aRole).toBeFocused();
  await aRole.fill("nová rola");
  await aName.fill("");
  await waits;
  expect(bodies).toHaveLength(1);

  // The phone locks: the role goes now, with the newest name, not the one
  // the server had before.
  const decided = page.waitForEvent("console", {
    predicate: (msg) => msg.text().startsWith(`nameplates: plate ${a} name is empty; kept as typed`),
    timeout: 10_000,
  });
  await setPageHidden(page, true);
  await decided;
  await expect.poll(() => bodies.length).toBe(2);
  expect(bodies[1]).toEqual({ primaryText: "Najnovšie A nová", secondaryText: "nová rola" });

  held[0].release();
  await held[0].answered;
  held[1].release();
  await held[1].answered;
  await expect
    .poll(async () => (await listPlates(request)).find((p) => p.id === a))
    .toMatchObject({ primaryText: "Najnovšie A nová", secondaryText: "nová rola" });
  await setPageHidden(page, false);
  await expect(aRole).toHaveValue("nová rola");
  await expect(aName).toHaveValue("");
  expect(bodies).toHaveLength(2);
  await page.unrouteAll({ behavior: "wait" });

  expect(consoleErrors).toEqual([]);
});

test("#871 a save that never answers is given up after 15 s: the row is marked unsaved and the text goes again", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const [a] = await seedPlates(request, [["Ticho A", "1"]]);
  // The first PATCH never gets an answer (a dead network that never errors).
  const bodies: PatchBody[] = [];
  let endFirst = () => {};
  const firstEnds = new Promise<void>((resolve) => (endFirst = resolve));
  await page.route(`**/stream/api/nameplates/${a}`, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    const n = bodies.push(route.request().postDataJSON() as PatchBody);
    if (n === 1) {
      await firstEnds;
      // The page gave up on it long ago; nothing to answer.
      await route.abort().catch(() => {});
      return;
    }
    await route.continue();
  });
  await openOperator(page, "/ui/operator/menovky");
  const aRow = row(page, a);
  const aName = aRow.locator(NAME);
  await expect(aName).toHaveValue("Ticho A");

  // When the page gives the save up, the network is back in the same pass.
  await onUnsavedMark(page, a, "online");
  await aName.fill("Ticho A nová");
  await aName.blur();
  await expect.poll(() => bodies.length).toBe(1);
  // After 15 s the page gives the save up (an aborted fetch): the row is
  // marked, and the text goes again and is saved.
  await expect.poll(() => unsavedMarkSeen(page, a), { timeout: 25_000 }).toBe(true);
  await expect.poll(() => savedText(request, a)).toBe("Ticho A nová");
  await expect(aRow).toHaveAttribute("data-unsaved", "false");
  expect(bodies.map((body) => body.primaryText)).toEqual(["Ticho A nová", "Ticho A nová"]);
  endFirst();
  await page.unrouteAll({ behavior: "wait" });

  // An aborted fetch logs nothing in Chrome.
  expect(consoleErrors).toEqual([]);
});

test("#871 Zobraziť tapped while the name field keeps the focus (iOS gives no blur) saves the typed text first and puts it on air", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  await setActive(request, { source: null });
  const [a] = await seedPlates(request, [["Dotyk A", "1"]]);
  const patches: string[] = [];
  page.on("request", (req) => {
    if (req.method() === "PATCH" && req.url().endsWith(`/stream/api/nameplates/${a}`)) {
      patches.push((JSON.parse(req.postData() ?? "{}") as { primaryText: string }).primaryText);
    }
  });
  await openOperator(page, "/ui/operator/menovky");
  const aRow = row(page, a);
  const aName = aRow.locator(NAME);
  await expect(aName).toHaveValue("Dotyk A");

  // Typed, then Zobraziť tapped with no blur of the field (a synthetic click
  // moves no focus, as iOS Safari's tap on a button does not).
  await aName.fill("Dotyk A nová");
  await aRow.locator('[data-role="stream-nameplate-show"]').dispatchEvent("click");
  await expect(aName).toBeFocused();
  await expect.poll(async () => (await activePlate(request))?.primary).toBe("Dotyk A nová");
  await expect(aRow).toHaveAttribute("data-active", "true");
  expect(await savedText(request, a)).toBe("Dotyk A nová");
  await expect(aName).toHaveValue("Dotyk A nová");

  // Leaving the field later has nothing left to save.
  await aName.blur();
  await expect(aName).toHaveValue("Dotyk A nová");
  expect(patches).toEqual(["Dotyk A nová"]);
  await setActive(request, { source: null });

  expect(consoleErrors).toEqual([]);
});

test("#871 a plate deleted elsewhere while it is dragged leaves no drop line behind", async ({
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
  await resetPlates(request);
  const [a, b, c] = await seedPlates(request, [
    ["Zmiz A", "1"],
    ["Zmiz B", "2"],
    ["Zmiz C", "3"],
  ]);
  await openOperator(page, "/ui/operator/menovky");
  await expect.poll(() => rowOrder(page)).toEqual([a, b, c]);

  // C is held above A (the line on A) when another operator deletes C.
  const first = await box(row(page, a));
  await mouseDrag(page, c, first.top - 4, async () => {
    await expect(row(page, a)).toHaveAttribute("data-drop", "before");
    await deletePlate(request, c);
    await expect(row(page, c)).toHaveCount(0);
    await expect(view(page).locator("[data-drop]")).toHaveCount(0);
    await expect(view(page).locator('[data-dragging="true"]')).toHaveCount(0);
  });
  // The release moves nothing, and the next drag works as usual.
  await expect.poll(() => rowOrder(page)).toEqual([a, b]);
  await mouseDrag(page, b, first.top - 4);
  await expectOrder(page, request, [b, a]);
  expect(orderPuts.map((body) => (JSON.parse(body) as { ids: number[] }).ids)).toEqual([[b, a]]);

  expect(consoleErrors).toEqual([]);
});

test("#871 the Stream Deck labels follow a rename and a reorder made in the Menovky view", async ({
  page,
  request,
}) => {
  const consoleErrors = collectConsole(page);
  await resetPlates(request);
  const sock = createCompanionSocket(wsURL, "operator-nameplates-spec");
  try {
    await sock.handshake();
    const [anna, boris] = await seedPlates(request, [
      ["Slot Anna", "kazateľka"],
      ["Slot Boris", "hosť"],
    ]);
    // #870: slot n = the n-th person plate; the label variables are pushed live.
    await expect.poll(() => sock.vars.get("nameplate_slot_1_name")).toBe("Slot Anna");
    await expect.poll(() => sock.vars.get("nameplate_slot_2_name")).toBe("Slot Boris");

    await openOperator(page, "/ui/operator/menovky");
    await expect(row(page, anna).locator(BUTTON)).toHaveText("2");

    // A rename in the view → slot 1's label.
    const annaName = row(page, anna).locator(NAME);
    await annaName.fill("Slot Anna Nová");
    await annaName.blur();
    await expect.poll(() => sock.vars.get("nameplate_slot_1_name")).toBe("Slot Anna Nová");
    expect(sock.vars.get("nameplate_slot_1_id")).toBe(String(anna));

    // A reorder in the view → the slots follow the list.
    await row(page, anna).locator('[data-role="stream-nameplate-down"]').click();
    await expect.poll(() => sock.vars.get("nameplate_slot_1_name")).toBe("Slot Boris");
    await expect.poll(() => sock.vars.get("nameplate_slot_2_name")).toBe("Slot Anna Nová");
    expect(sock.vars.get("nameplate_slot_1_id")).toBe(String(boris));
    expect(sock.vars.get("nameplate_slot_2_id")).toBe(String(anna));
    await expect(row(page, boris).locator(BUTTON)).toHaveText("2");
    await expect(row(page, anna).locator(BUTTON)).toHaveText("3");

    expect(sock.errors).toHaveLength(0);
  } finally {
    sock.socket.close();
  }

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
