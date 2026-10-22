/**
 * #827: the editor's „Výstup" select must always show the output being edited.
 *
 * The bug (PP, 2026-10-09): opening `/ui/stream` on a non-first output showed
 * that output's scenes while the select showed the FIRST output. The select's
 * value was applied once, before `GET /stream/api/outputs` had filled its
 * options, so the browser fell back to the first option. The operator then
 * edited the wrong output, believing it was another.
 *
 *  - A non-first output opened via `?output=` or localStorage, with the outputs
 *    list delayed (`page.route` gate): the select shows that output, and the
 *    scene columns are that output's.
 *  - A remembered output that no longer exists: the editor switches to the
 *    first listed output and records it (URL + localStorage), so the select
 *    and the editor agree.
 *  - #871: an output switch drops the previous output's plates and plate on
 *    air at once, while the new output's plate list is still on its way.
 *  - #871: a plate text whose save failed on the network (kept in the field
 *    for another try) is saved when the editor switches output, with the
 *    role it had, not dropped with a "deleted" toast.
 *  - #871: a save still on its way when the editor switches output goes
 *    once, with the role it had; if it then fails, the toast says to type
 *    the change again (its field is gone).
 *  - #871: a Zobraziť waiting for its save is dropped by an output switch,
 *    never put on air on the output the editor shows now.
 *  - #871: a save left waiting by an output switch (its row is gone) goes at
 *    once when the page hides, beside the save on its way; also after the
 *    editor switched back (the plate's new row has no drafts), once, when
 *    the tab closes (hidden, then `pagehide`).
 *  - #871: a row rebuilt by an output switch there and back while its save
 *    is on its way stands on the newest texts, not the list's older ones: a
 *    role typed there keeps the newer name.
 *
 * Clean console is asserted last.
 */
import { test, expect, type Page } from "@playwright/test";
import {
  attachEditorConsoleCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

type OutputSummary = { slug: string; name: string };
type OutputDef = { scenes: Array<{ id: number; name: string }> };

const OUTPUT_STORAGE_KEY = "stream-editor-output";
const OUTPUTS_ROUTE = "**/stream/api/outputs";

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

const outputSelect = (page: Page) => page.locator('[data-role="stream-output-select"]');
const sceneCard = (page: Page, id: number) =>
  page.locator(`[data-role="stream-scene"][data-scene-id="${id}"]`);

async function listOutputs(page: Page): Promise<OutputSummary[]> {
  // `page.request` is not intercepted by `page.route`.
  const res = await page.request.get(`${baseURL}/stream/api/outputs`);
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as OutputSummary[];
}

type PlateTexts = { id: number; primaryText: string; secondaryText: string };

/** Plate `id` of the `stream` output as the server has it. */
async function savedPlate(page: Page, id: number): Promise<PlateTexts | undefined> {
  const res = await page.request.get(`${baseURL}/stream/api/outputs/stream/nameplates`);
  expect(res.ok()).toBeTruthy();
  return ((await res.json()) as PlateTexts[]).find((p) => p.id === id);
}

async function getDef(page: Page, slug: string): Promise<OutputDef> {
  const res = await page.request.get(`${baseURL}/stream/api/outputs/${slug}/def`);
  expect(res.ok(), `def ${slug} -> ${res.status()}`).toBeTruthy();
  return (await res.json()) as OutputDef;
}

/** Hold the page's outputs-list response until the returned release() runs. */
async function holdOutputsList(page: Page): Promise<() => Promise<void>> {
  let release: () => void = () => {};
  const gate = new Promise<void>((resolve) => (release = resolve));
  await page.route(OUTPUTS_ROUTE, async (route) => {
    await gate;
    await route.continue();
  });
  return async () => {
    release();
    // Wait until the late list has actually filled the select.
    await expect(outputSelect(page).locator("option")).not.toHaveCount(0, { timeout: 15_000 });
    await page.unroute(OUTPUTS_ROUTE);
  };
}

test("#827 a non-first output opened via ?output= or localStorage is the one the select shows", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);

  const outputs = await listOutputs(page);
  expect(outputs.length, "needs at least two outputs").toBeGreaterThan(1);
  const target = outputs[outputs.length - 1].slug;
  expect(target).not.toBe(outputs[0].slug);

  // A scene that exists ONLY on the target output, so the scene columns tell
  // which output the page is editing.
  const created = await page.request.post(`${baseURL}/stream/api/outputs/${target}/scenes`, {
    data: { name: `SC_827_${Date.now()}`, kind: "base" },
  });
  expect(created.ok(), `create scene -> ${created.status()}`).toBeTruthy();
  const targetScene = (await created.json()).id as number;
  const targetSceneIds = (await getDef(page, target)).scenes.map((s) => s.id);

  const assertEditing = async () => {
    await expect(outputSelect(page).locator("option")).toHaveCount(outputs.length);
    await expect(outputSelect(page)).toHaveValue(target);
    await expect(sceneCard(page, targetScene)).toBeVisible();
    const shown = await page
      .locator('[data-role="stream-scene"]')
      .evaluateAll((els) => els.map((el) => Number(el.getAttribute("data-scene-id"))));
    expect(shown.sort((a, b) => a - b)).toEqual([...targetSceneIds].sort((a, b) => a - b));
  };

  // ── ?output= with the outputs list arriving AFTER the def. ──
  let release = await holdOutputsList(page);
  await page.goto(`${baseURL}/ui/stream?output=${target}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(sceneCard(page, targetScene)).toBeVisible({ timeout: 15_000 });
  await release();
  await assertEditing();

  // ── localStorage only (no URL param), same late list. ──
  await page.evaluate(
    ([key, slug]) => window.localStorage.setItem(key, slug),
    [OUTPUT_STORAGE_KEY, target] as const,
  );
  release = await holdOutputsList(page);
  await page.goto(`${baseURL}/ui/stream`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(sceneCard(page, targetScene)).toBeVisible({ timeout: 15_000 });
  await release();
  await assertEditing();

  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#827 a remembered output that no longer exists falls back to the first listed output", async ({
  page,
}) => {
  const GONE = "gone-827";
  // The stale slug's own cold-load requests (def + plate list) are genuine 404s
  // of the behaviour under test; collect them apart, scoped by URL, and keep
  // every other console line in `errors`.
  const errors: string[] = [];
  const staleNotFound: string[] = [];
  page.on("console", (msg) => {
    const type = msg.type();
    if (type !== "error" && type !== "warning") return;
    const url = msg.location()?.url ?? "";
    if (msg.text().includes("Failed to load resource") && url.includes(`/stream/api/outputs/${GONE}/`)) {
      staleNotFound.push(`${msg.text()} ${url}`);
      return;
    }
    errors.push(`[${type}] ${msg.text()}`);
  });
  page.on("pageerror", (err) => errors.push(`[pageerror] ${err.message}`));

  const first = (await listOutputs(page))[0].slug;
  const firstScenes = (await getDef(page, first)).scenes;

  await page.goto(`${baseURL}/ui/stream?output=${GONE}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  // The editor moved to the first output and remembers that choice.
  await expect
    .poll(() => new URL(page.url()).searchParams.get("output"), { timeout: 15_000 })
    .toBe(first);
  await expect(outputSelect(page)).toHaveValue(first);
  expect(await page.evaluate((key) => window.localStorage.getItem(key), OUTPUT_STORAGE_KEY)).toBe(
    first,
  );
  await expect(page.locator('[data-role="stream-editor"]')).toBeVisible({ timeout: 15_000 });
  if (firstScenes.length > 0) {
    await expect(sceneCard(page, firstScenes[0].id)).toBeVisible();
  }

  // Exactly the two output-scoped cold-load reads 404: the def and the plate
  // list (`nameplates/active` is in-memory state and answers 200 null).
  await expect
    .poll(() => staleNotFound.length, { message: staleNotFound.join(" | ") })
    .toBe(2);
  expect(
    staleNotFound.every((line) => /status of 404\b/.test(line)),
    `only 404s for the stale output: ${staleNotFound.join(" | ")}`,
  ).toBeTruthy();
  // Re-checked last, so a late third refusal still fails the test.
  expect(staleNotFound, "exactly the def + plate-list refusals").toHaveLength(2);
  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#871 an output switch drops the previous output's plates at once, before the new list lands", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const other = (await listOutputs(page)).find((o) => o.slug !== "stream")?.slug as string;
  expect(other, "a second output exists").toBeTruthy();

  // A plate of `stream`, on air.
  const created = await page.request.post(`${baseURL}/stream/api/outputs/stream/nameplates`, {
    data: { primaryText: `SW_871_${Date.now() % 100_000}`, secondaryText: "hosť" },
  });
  expect(created.ok()).toBeTruthy();
  const plateId = (await created.json()).id as number;
  const shown = await page.request.put(`${baseURL}/stream/api/outputs/stream/nameplates/active`, {
    data: { source: "person", id: plateId },
  });
  expect(shown.status()).toBe(200);

  await page.goto(`${baseURL}/ui/stream?output=stream&tab=nameplates`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  const plateRow = page.locator(`[data-role="stream-nameplate"][data-nameplate-id="${plateId}"]`);
  await expect(plateRow).toHaveAttribute("data-active", "true", { timeout: 15_000 });

  // Hold the other output's plate list: while it is on its way, nothing of
  // `stream` (its rows, its plate on air) may stay on screen.
  const listUrl = `${baseURL}/stream/api/outputs/${other}/nameplates`;
  const route = `**/stream/api/outputs/${other}/nameplates`;
  let release: () => void = () => {};
  const gate = new Promise<void>((resolve) => (release = resolve));
  await page.route(route, async (held) => {
    await gate;
    await held.continue();
  });
  const answered = page.waitForResponse(
    (res) => res.url() === listUrl && res.request().method() === "GET",
  );
  await outputSelect(page).selectOption(other);
  await expect(plateRow).toHaveCount(0);
  await expect(page.locator('[data-role="stream-nameplate"][data-active="true"]')).toHaveCount(0);
  release();
  await (await answered).finished();
  await page.unroute(route);

  // Leave `stream` as found.
  const hidden = await page.request.put(`${baseURL}/stream/api/outputs/stream/nameplates/active`, {
    data: { source: null },
  });
  expect(hidden.status()).toBe(200);
  const deleted = await page.request.delete(`${baseURL}/stream/api/nameplates/${plateId}`);
  expect(deleted.ok()).toBeTruthy();
  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#871 a plate text whose save failed is saved, not dropped, when the editor switches output", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const other = (await listOutputs(page)).find((o) => o.slug !== "stream")?.slug as string;
  expect(other, "a second output exists").toBeTruthy();
  const name = `SW_871_FAIL_${Date.now() % 100_000}`;
  const created = await page.request.post(`${baseURL}/stream/api/outputs/stream/nameplates`, {
    data: { primaryText: name, secondaryText: "hosť" },
  });
  expect(created.ok()).toBeTruthy();
  const plateId = (await created.json()).id as number;

  // The plate's first PATCH fails at the network (the Wi-Fi asleep), later
  // ones pass.
  let patches = 0;
  const patchRoute = `**/stream/api/nameplates/${plateId}`;
  await page.route(patchRoute, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    patches += 1;
    if (patches === 1) {
      await route.abort();
      return;
    }
    await route.continue();
  });

  await page.goto(`${baseURL}/ui/stream?output=stream&tab=nameplates`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  const plateRow = page.locator(`[data-role="stream-nameplate"][data-nameplate-id="${plateId}"]`);
  const plateName = plateRow.locator('[data-role="stream-nameplate-name"]');
  await expect(plateName).toHaveValue(name, { timeout: 15_000 });

  // The save fails: the typed text stays in the field for another try.
  await plateName.fill(`${name} nová`);
  await plateName.blur();
  await expect(page.locator('[data-role="toast"]')).toContainText("Text ostal v poli");
  await expect(plateName).toHaveValue(`${name} nová`);

  // Another output is picked before that try: the row leaves the list, and
  // the plate (still there) gets the text instead of a "deleted" toast.
  await outputSelect(page).selectOption(other);
  await expect(plateRow).toHaveCount(0);
  // The role (not typed) keeps its saved text: the row's saved texts outlive
  // the list that was cleared by the switch.
  await expect
    .poll(() => savedPlate(page, plateId))
    .toMatchObject({ primaryText: `${name} nová`, secondaryText: "hosť" });
  expect(patches).toBe(2);
  // Read ONCE, now that the switch has decided: a retrying negative check
  // keeps retrying until a wrong toast is replaced or hidden.
  const toastText = await page.locator('[data-role="toast"]').textContent();
  expect(toastText ?? "", "no deleted-plate toast").not.toContain("zmazaná");
  await page.unroute(patchRoute);

  const deleted = await page.request.delete(`${baseURL}/stream/api/nameplates/${plateId}`);
  expect(deleted.ok()).toBeTruthy();
  // Chrome logs the one aborted fetch itself; nothing else may be logged.
  const aborted = /Failed to load resource: net::ERR_FAILED$/;
  expect(errors.filter((line) => aborted.test(line))).toHaveLength(1);
  expect(
    errors.filter((line) => !aborted.test(line)),
    `editor console: ${errors.join(" | ")}`,
  ).toEqual([]);
});

type PatchBody = { primaryText: string; secondaryText: string };

async function createStreamPlate(
  page: Page,
  primaryText: string,
  secondaryText: string,
): Promise<number> {
  const created = await page.request.post(`${baseURL}/stream/api/outputs/stream/nameplates`, {
    data: { primaryText, secondaryText },
  });
  expect(created.ok()).toBeTruthy();
  return (await created.json()).id as number;
}

/** Open the editor's Menovky tab on `stream`, type `typed` into plate
 *  `plateId` (shown as `name`) and leave the field, with the save's PATCH held
 *  at the network, then switch the editor to `other` while it is on its way.
 *  Returns every PATCH body of the plate, and `finishFirst` to let the held
 *  PATCH through or fail it; it returns once the page has dealt with the
 *  answer (the list reload that follows the plate's last save). */
async function switchWhileSaving(
  page: Page,
  plateId: number,
  name: string,
  typed: string,
  other: string,
): Promise<{ bodies: PatchBody[]; finishFirst: (how: "continue" | "abort") => Promise<void> }> {
  const bodies: PatchBody[] = [];
  let finish: (how: "continue" | "abort") => void = () => {};
  const first = new Promise<"continue" | "abort">((resolve) => (finish = resolve));
  await page.route(`**/stream/api/nameplates/${plateId}`, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    const n = bodies.push(route.request().postDataJSON() as PatchBody);
    if (n === 1 && (await first) === "abort") {
      await route.abort();
      return;
    }
    await route.continue();
  });

  await page.goto(`${baseURL}/ui/stream?output=stream&tab=nameplates`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  const plateRow = page.locator(`[data-role="stream-nameplate"][data-nameplate-id="${plateId}"]`);
  const plateName = plateRow.locator('[data-role="stream-nameplate-name"]');
  await expect(plateName).toHaveValue(name, { timeout: 15_000 });
  await plateName.fill(typed);
  await plateName.blur();
  await expect.poll(() => bodies.length).toBe(1);

  // The editor switches output while that save is on its way: the row leaves
  // the list, and the new output's list read is answered.
  const otherList = `${baseURL}/stream/api/outputs/${other}/nameplates`;
  const listed = page.waitForResponse(
    (res) => res.url() === otherList && res.request().method() === "GET",
  );
  await outputSelect(page).selectOption(other);
  await expect(plateRow).toHaveCount(0);
  await listed;

  return {
    bodies,
    finishFirst: async (how) => {
      // Once nothing of the plate is left to send, the page reloads the
      // list: a second save of the plate would have gone out before it.
      const reloaded = page.waitForRequest(
        (req) => req.url() === otherList && req.method() === "GET",
      );
      finish(how);
      await reloaded;
    },
  };
}

test("#871 a save on its way when the editor switches output goes once, with the role it had", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const other = (await listOutputs(page)).find((o) => o.slug !== "stream")?.slug as string;
  expect(other, "a second output exists").toBeTruthy();
  const name = `SW_871_FLY_${Date.now() % 100_000}`;
  const plateId = await createStreamPlate(page, name, "hosť");

  const { bodies, finishFirst } = await switchWhileSaving(
    page,
    plateId,
    name,
    `${name} nová`,
    other,
  );
  await finishFirst("continue");

  // One save, with the role the plate had: the row leaving the list while
  // that save was on its way sent nothing more (no role wiped to "").
  expect(bodies).toEqual([{ primaryText: `${name} nová`, secondaryText: "hosť" }]);
  await expect
    .poll(() => savedPlate(page, plateId))
    .toMatchObject({ primaryText: `${name} nová`, secondaryText: "hosť" });
  await page.unrouteAll({ behavior: "wait" });

  const deleted = await page.request.delete(`${baseURL}/stream/api/nameplates/${plateId}`);
  expect(deleted.ok()).toBeTruthy();
  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#871 a save that fails after the editor switched output says to type the change again", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const other = (await listOutputs(page)).find((o) => o.slug !== "stream")?.slug as string;
  expect(other, "a second output exists").toBeTruthy();
  const name = `SW_871_LOST_${Date.now() % 100_000}`;
  const plateId = await createStreamPlate(page, name, "hosť");

  const { bodies, finishFirst } = await switchWhileSaving(
    page,
    plateId,
    name,
    `${name} nová`,
    other,
  );
  await finishFirst("abort");

  // The field that held the text is gone with its row, so the toast says to
  // type the change again, never that the text stayed in the field.
  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toContainText("Zmena menovky sa neuložila — skúste ju zadať znova");
  const toastText = (await toast.textContent()) ?? "";
  expect(toastText).not.toContain("Text ostal v poli");
  expect(bodies).toHaveLength(1);
  expect(await savedPlate(page, plateId)).toMatchObject({ primaryText: name, secondaryText: "hosť" });
  await page.unrouteAll({ behavior: "wait" });

  const deleted = await page.request.delete(`${baseURL}/stream/api/nameplates/${plateId}`);
  expect(deleted.ok()).toBeTruthy();
  // Chrome logs the one aborted fetch itself; nothing else may be logged.
  const aborted = /Failed to load resource: net::ERR_FAILED$/;
  expect(errors.filter((line) => aborted.test(line))).toHaveLength(1);
  expect(
    errors.filter((line) => !aborted.test(line)),
    `editor console: ${errors.join(" | ")}`,
  ).toEqual([]);
});

/** Make the page hidden (the laptop lid closes, another app or tab) or
 *  visible again, as the browser does: `visibilityState` + a bubbling
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

/** Close the tab as the browser does: the page goes hidden, then
 *  `pagehide`, in ONE task, so no answer is handled between the two. */
async function closePage(page: Page): Promise<void> {
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
    window.dispatchEvent(new Event("pagehide"));
  });
}

/** Record every PATCH of plate `id` and hold the FIRST one at the network
 *  until `releaseFirst()`; later ones go through at once. */
async function holdFirstPatch(
  page: Page,
  id: number,
): Promise<{ bodies: PatchBody[]; releaseFirst: () => void }> {
  const bodies: PatchBody[] = [];
  let releaseFirst = () => {};
  const gate = new Promise<void>((resolve) => (releaseFirst = resolve));
  await page.route(`**/stream/api/nameplates/${id}`, async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.continue();
      return;
    }
    const n = bodies.push(route.request().postDataJSON() as PatchBody);
    if (n === 1) await gate;
    await route.continue();
  });
  return { bodies, releaseFirst };
}

/** Resolves once the page has HANDLED a saved answer of plate `id` while
 *  another save of the plate is still on its way (the page's log line). A
 *  `waitForResponse` only says the network delivered the answer; the page
 *  handles it two awaits later, so on a CPU-starved runner the held save's
 *  answer could be handled first. Set it up before the save is answered. */
function savedWhileAnotherIsOut(page: Page, id: number) {
  return page.waitForEvent("console", {
    predicate: (msg) =>
      msg.text() === `nameplates: plate ${id} saved; a save of the plate is still on its way`,
    timeout: 10_000,
  });
}

test("#871 a Zobraziť waiting for its save is dropped when the editor switches output, never shown on the new one", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const other = (await listOutputs(page)).find((o) => o.slug !== "stream")?.slug as string;
  expect(other, "a second output exists").toBeTruthy();
  const otherActive = `${baseURL}/stream/api/outputs/${other}/nameplates/active`;
  const otherList = `${baseURL}/stream/api/outputs/${other}/nameplates`;
  const cleared = await page.request.put(otherActive, { data: { source: null } });
  expect(cleared.status()).toBe(200);
  const name = `SW_871_SHOW_${Date.now() % 100_000}`;
  const plateId = await createStreamPlate(page, name, "hosť");
  const { bodies, releaseFirst } = await holdFirstPatch(page, plateId);
  // Every show / hide the page asks for on the other output.
  const otherPuts: string[] = [];
  page.on("request", (req) => {
    if (req.method() === "PUT" && req.url() === otherActive) otherPuts.push(req.postData() ?? "");
  });

  await page.goto(`${baseURL}/ui/stream?output=stream&tab=nameplates`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  const plateRow = page.locator(`[data-role="stream-nameplate"][data-nameplate-id="${plateId}"]`);
  const plateName = plateRow.locator('[data-role="stream-nameplate-name"]');
  await expect(plateName).toHaveValue(name, { timeout: 15_000 });

  // The new name is saved (that PATCH held at the network), and Zobraziť is
  // pressed meanwhile: it waits for the save.
  await plateName.fill(`${name} nová`);
  await plateName.blur();
  await expect.poll(() => bodies.length).toBe(1);
  const waitsForSave = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${plateId} goes on air once its save is through`,
    timeout: 10_000,
  });
  await plateRow.locator('[data-role="stream-nameplate-show"]').click();
  await waitsForSave;

  // The editor switches output before the save is back.
  const listed = page.waitForResponse(
    (res) => res.url() === otherList && res.request().method() === "GET",
  );
  await outputSelect(page).selectOption(other);
  await expect(plateRow).toHaveCount(0);
  await listed;

  // The save is back. Once the plate is idle the page reloads the list; a
  // show of the plate would be asked for before that reload, on the output
  // the editor shows NOW (the server does not check which output owns it).
  const decided = new Promise<"show" | "reload">((resolve) => {
    page.on("request", (req) => {
      if (req.method() === "PUT" && req.url() === otherActive) resolve("show");
      if (req.method() === "GET" && req.url() === otherList) resolve("reload");
    });
  });
  releaseFirst();
  expect(await decided, "the waiting Zobraziť is dropped").toBe("reload");
  await expect
    .poll(() => savedPlate(page, plateId))
    .toMatchObject({ primaryText: `${name} nová`, secondaryText: "hosť" });
  // Read once, now that the page has dealt with the answer.
  const active = await page.request.get(otherActive);
  expect(await active.json()).toBeNull();
  expect(otherPuts).toEqual([]);
  expect(bodies).toHaveLength(1);
  await page.unrouteAll({ behavior: "wait" });

  const deleted = await page.request.delete(`${baseURL}/stream/api/nameplates/${plateId}`);
  expect(deleted.ok()).toBeTruthy();
  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#871 a save left waiting when the editor switches output goes at once when the page hides", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const other = (await listOutputs(page)).find((o) => o.slug !== "stream")?.slug as string;
  expect(other, "a second output exists").toBeTruthy();
  const name = `SW_871_HIDE_${Date.now() % 100_000}`;
  const plateId = await createStreamPlate(page, name, "hosť");
  const { bodies, releaseFirst } = await holdFirstPatch(page, plateId);
  const newest = { primaryText: `${name} nová`, secondaryText: "pastor" };

  await page.goto(`${baseURL}/ui/stream?output=stream&tab=nameplates`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  const plateRow = page.locator(`[data-role="stream-nameplate"][data-nameplate-id="${plateId}"]`);
  const plateName = plateRow.locator('[data-role="stream-nameplate-name"]');
  const plateRole = plateRow.locator('[data-role="stream-nameplate-role"]');
  await expect(plateName).toHaveValue(name, { timeout: 15_000 });

  // The new name is saved (that PATCH held at the network); the role is
  // typed.
  await plateName.fill(newest.primaryText);
  await plateName.blur();
  await expect.poll(() => bodies.length).toBe(1);
  await plateRole.fill(newest.secondaryText);

  // The editor switches output: the row leaves and hands both texts to the
  // store, where they wait for the save on its way.
  const waits = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${plateId} save waits for the one on its way`,
    timeout: 10_000,
  });
  await outputSelect(page).selectOption(other);
  await expect(plateRow).toHaveCount(0);
  await waits;
  expect(bodies).toHaveLength(1);

  // The page hides (the laptop lid closes, the tab is closed): no row is
  // left to send the texts, so the list sends them now, beside the save on
  // its way.
  const besideHandled = savedWhileAnotherIsOut(page, plateId);
  await setPageHidden(page, true);
  await expect.poll(() => bodies.length).toBe(2);
  expect(bodies[1]).toEqual(newest);
  await besideHandled;

  // The held save reaches the server last, so the page sends the newest
  // texts once more, and the server ends on them.
  releaseFirst();
  await expect.poll(() => bodies.length).toBe(3);
  expect(bodies[2]).toEqual(newest);
  await expect.poll(() => savedPlate(page, plateId)).toMatchObject(newest);
  await setPageHidden(page, false);
  await page.unrouteAll({ behavior: "wait" });
  expect(bodies).toHaveLength(3);

  const deleted = await page.request.delete(`${baseURL}/stream/api/nameplates/${plateId}`);
  expect(deleted.ok()).toBeTruthy();
  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#871 a save left waiting by an output switch goes when the tab closes, also once the editor is back on its output", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const other = (await listOutputs(page)).find((o) => o.slug !== "stream")?.slug as string;
  expect(other, "a second output exists").toBeTruthy();
  const name = `SW_871_BACK_${Date.now() % 100_000}`;
  const plateId = await createStreamPlate(page, name, "hosť");
  const { bodies, releaseFirst } = await holdFirstPatch(page, plateId);
  const newest = { primaryText: `${name} nová`, secondaryText: "pastor" };
  const logs: string[] = [];
  page.on("console", (msg) => logs.push(msg.text()));

  await page.goto(`${baseURL}/ui/stream?output=stream&tab=nameplates`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  const plateRow = page.locator(`[data-role="stream-nameplate"][data-nameplate-id="${plateId}"]`);
  const plateName = plateRow.locator('[data-role="stream-nameplate-name"]');
  const plateRole = plateRow.locator('[data-role="stream-nameplate-role"]');
  await expect(plateName).toHaveValue(name, { timeout: 15_000 });

  // The new name is saved (that PATCH held at the network); the role is
  // typed.
  await plateName.fill(newest.primaryText);
  await plateName.blur();
  await expect.poll(() => bodies.length).toBe(1);
  await plateRole.fill(newest.secondaryText);

  // The editor switches output: the row leaves and hands both texts to the
  // store, where they wait for the save on its way.
  const waits = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${plateId} save waits for the one on its way`,
    timeout: 10_000,
  });
  await outputSelect(page).selectOption(other);
  await expect(plateRow).toHaveCount(0);
  await waits;

  // ... and back before that save is answered: the plate is listed again,
  // in a NEW row with no drafts, while the old row's texts still wait in
  // the store. The row stands on those waiting texts, not on the list's
  // older ones (the server's).
  await outputSelect(page).selectOption("stream");
  await expect(plateName).toHaveValue(newest.primaryText, { timeout: 15_000 });
  await expect(plateRole).toHaveValue(newest.secondaryText);
  expect(bodies).toHaveLength(1);

  // The tab is closed: the waiting texts go now, beside the save on its way,
  // once (the page hiding and `pagehide` both ask). The list skips a listed
  // plate, so its row has to send them: its log line tells that send from
  // the held save giving up after 15 s, which sends the same texts. (Page
  // logs are read from `logs`: on code that sends nothing, the test fails on
  // that count, not on a `waitForEvent` timing out.)
  const sentBeside = `nameplates: plate ${plateId} save sent now beside the one on its way`;
  await closePage(page);
  await expect.poll(() => bodies.length).toBe(2);
  expect(bodies[1]).toEqual(newest);
  await expect.poll(() => logs.filter((line) => line.startsWith(sentBeside)).length).toBe(1);
  const stillOut = `nameplates: plate ${plateId} saved; a save of the plate is still on its way`;
  await expect.poll(() => logs.includes(stillOut)).toBe(true);

  // The held save reaches the server last, so the page sends the newest
  // texts once more, and the server ends on them.
  const idle = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${plateId} saved; nothing left to send`,
    timeout: 20_000,
  });
  releaseFirst();
  await idle;
  expect(bodies).toEqual([
    { primaryText: newest.primaryText, secondaryText: "hosť" },
    newest,
    newest,
  ]);
  expect(await savedPlate(page, plateId)).toMatchObject(newest);
  await expect(plateRole).toHaveValue(newest.secondaryText);
  // Sent once, and no save of the plate failed (the held one never ran into
  // its 15 s timeout).
  expect(logs.filter((line) => line.startsWith(sentBeside))).toHaveLength(1);
  expect(logs.filter((line) => line.startsWith(`nameplates: plate ${plateId} save not done`))).toEqual([]);
  await setPageHidden(page, false);
  await page.unrouteAll({ behavior: "wait" });

  const deleted = await page.request.delete(`${baseURL}/stream/api/nameplates/${plateId}`);
  expect(deleted.ok()).toBeTruthy();
  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#871 a row rebuilt while its save is on its way stands on the newest texts: a role typed there keeps the newer name", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const other = (await listOutputs(page)).find((o) => o.slug !== "stream")?.slug as string;
  expect(other, "a second output exists").toBeTruthy();
  const name = `SW_871_BASE_${Date.now() % 100_000}`;
  const plateId = await createStreamPlate(page, name, "hosť");
  const { bodies, releaseFirst } = await holdFirstPatch(page, plateId);
  const newer = `${name} nová`;

  await page.goto(`${baseURL}/ui/stream?output=stream&tab=nameplates`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  const plateRow = page.locator(`[data-role="stream-nameplate"][data-nameplate-id="${plateId}"]`);
  const plateName = plateRow.locator('[data-role="stream-nameplate-name"]');
  const plateRole = plateRow.locator('[data-role="stream-nameplate-role"]');
  await expect(plateName).toHaveValue(name, { timeout: 15_000 });

  // The new name is saved; that PATCH is held at the network.
  await plateName.fill(newer);
  await plateName.blur();
  await expect.poll(() => bodies.length).toBe(1);

  // The editor switches output and back before that save is answered: the
  // plate's row is rebuilt, while the list still has the server's OLDER
  // name.
  await outputSelect(page).selectOption(other);
  await expect(plateRow).toHaveCount(0);
  await outputSelect(page).selectOption("stream");
  await expect(plateRole).toHaveValue("hosť", { timeout: 15_000 });

  // Only the role is typed in the rebuilt row, then left: its save waits
  // for the held one.
  const waits = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${plateId} save waits for the one on its way`,
    timeout: 10_000,
  });
  await plateRole.fill("kazateľ");
  await plateRole.blur();
  await waits;
  expect(bodies).toHaveLength(1);

  // The held save is answered, the waiting one goes next: the server ends on
  // BOTH the newer name and the new role (a row standing on the list sent
  // the OLDER name back with the role).
  const idle = page.waitForEvent("console", {
    predicate: (msg) => msg.text() === `nameplates: plate ${plateId} saved; nothing left to send`,
    timeout: 20_000,
  });
  releaseFirst();
  await idle;
  expect(await savedPlate(page, plateId)).toMatchObject({
    primaryText: newer,
    secondaryText: "kazateľ",
  });
  expect(bodies).toEqual([
    { primaryText: newer, secondaryText: "hosť" },
    { primaryText: newer, secondaryText: "kazateľ" },
  ]);
  await expect(plateName).toHaveValue(newer);
  await expect(plateRole).toHaveValue("kazateľ");
  await page.unrouteAll({ behavior: "wait" });

  const deleted = await page.request.delete(`${baseURL}/stream/api/nameplates/${plateId}`);
  expect(deleted.ok()).toBeTruthy();
  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});
