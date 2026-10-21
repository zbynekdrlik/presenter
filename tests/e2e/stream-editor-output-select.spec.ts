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
 *    for another try) is saved when the editor switches output, not dropped
 *    with a "deleted" toast.
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
  const savedName = async () => {
    const res = await page.request.get(`${baseURL}/stream/api/outputs/stream/nameplates`);
    expect(res.ok()).toBeTruthy();
    const plates = (await res.json()) as Array<{ id: number; primaryText: string }>;
    return plates.find((p) => p.id === plateId)?.primaryText;
  };

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
  await expect.poll(savedName).toBe(`${name} nová`);
  expect(patches).toBe(2);
  await expect(page.locator('[data-role="toast"]')).not.toContainText("zmazaná");
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
