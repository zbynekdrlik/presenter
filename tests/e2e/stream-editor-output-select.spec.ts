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
  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});
