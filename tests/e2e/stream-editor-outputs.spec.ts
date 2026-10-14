/**
 * #842: create and rename stream outputs from the editor, and show the
 * selected output's OBS URL.
 *
 * Owner (2026-10-11): "vies dororbit dalsi stream vystup kde si spravim
 * dizajn". The operator types only a NAME; the slug (the `/stream/{slug}`
 * address of the OBS source and the Companion `output` option) is derived from
 * it, shown before saving, and never changes on a rename.
 *
 *  - Create: the derived address is previewed, the new output is selected
 *    (select, `?output=`, OBS URL) and the server has it with that name. The
 *    same name again is refused inline before any request.
 *  - Rename: the form starts from the current name, the select shows the new
 *    name, and the slug and OBS URL stay.
 *  - Phone width: the open form causes no horizontal scroll.
 *
 * #855: delete an output from the bar. The inline confirmation names the
 * output and its OBS address; „Zrušiť" / Escape keep it; „Zmazať" removes it
 * from the bar and the server and moves the editor to the first remaining
 * output. The seeded `stream` output (Companion's default) has its button
 * disabled with the reason in the tooltip. A delete the server answers with a
 * real 404 (another operator was first) is a Slovak error toast plus a list
 * re-sync.
 *
 * Clean console is asserted last in every test.
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
const urlField = (page: Page) => page.locator('[data-role="stream-output-url"]');
const outputForm = (page: Page) => page.locator('[data-role="stream-output-form"]');
const nameField = (page: Page) => page.locator('[data-role="stream-output-name"]');
const slugPreview = (page: Page) => page.locator('[data-role="stream-output-slug"]');
const saveButton = (page: Page) => page.locator('[data-role="stream-output-save"]');
const formError = (page: Page) => page.locator('[data-role="stream-output-error"]');
const deleteButton = (page: Page) => page.locator('[data-role="stream-output-delete"]');
const deleteConfirm = (page: Page) => page.locator('[data-role="stream-output-delete-confirm"]');
const deleteYes = (page: Page) => page.locator('[data-role="stream-output-delete-confirm-yes"]');
const deleteCancel = (page: Page) =>
  page.locator('[data-role="stream-output-delete-confirm-cancel"]');
const toast = (page: Page) => page.locator('[data-role="toast"]');
const sceneCard = (page: Page, id: number) =>
  page.locator(`[data-role="stream-scene"][data-scene-id="${id}"]`);

async function listOutputs(page: Page): Promise<OutputSummary[]> {
  const res = await page.request.get(`${baseURL}/stream/api/outputs`);
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as OutputSummary[];
}

async function createOutput(page: Page, slug: string, name: string): Promise<void> {
  const res = await page.request.post(`${baseURL}/stream/api/outputs`, { data: { slug, name } });
  expect(res.ok(), `create output ${slug} -> ${res.status()}`).toBeTruthy();
}

/** HTTP status of an output's def: 200 while it exists, 404 once deleted. */
async function defStatus(page: Page, slug: string): Promise<number> {
  return (await page.request.get(`${baseURL}/stream/api/outputs/${slug}/def`)).status();
}

/** The slugs the bar's select lists, in the order the operator sees them. */
async function barSlugs(page: Page): Promise<string[]> {
  return outputSelect(page)
    .locator("option")
    .evaluateAll((options) => options.map((option) => (option as HTMLOptionElement).value));
}

async function openEditor(page: Page, output?: string): Promise<void> {
  const query = output ? `?output=${output}` : "";
  await page.goto(`${baseURL}/ui/stream${query}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(outputSelect(page).locator("option")).not.toHaveCount(0, { timeout: 15_000 });
  if (output) {
    await expect(outputSelect(page)).toHaveValue(output);
  }
}

test("#842 a new output is created from its name, selected, and its OBS URL shown", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const n = Date.now() % 100_000;
  const name = `Výstup E2E ${n}`;
  const slug = `vystup-e2e-${n}`;

  await openEditor(page);
  const current = await outputSelect(page).inputValue();
  await expect(urlField(page)).toHaveValue(`${baseURL}/stream/${current}`);
  const before = (await listOutputs(page)).length;

  await page.locator('[data-role="stream-output-new"]').click();
  await expect(outputForm(page)).toHaveAttribute("data-mode", "create");
  await nameField(page).fill(name);
  await expect(slugPreview(page)).toHaveText(`Adresa: /stream/${slug}`);
  await saveButton(page).click();

  await expect(outputForm(page)).toHaveCount(0);
  await expect(outputSelect(page)).toHaveValue(slug, { timeout: 15_000 });
  await expect(outputSelect(page).locator(`option[value="${slug}"]`)).toHaveText(name);
  await expect(urlField(page)).toHaveValue(`${baseURL}/stream/${slug}`);
  await expect.poll(() => new URL(page.url()).searchParams.get("output")).toBe(slug);
  const outputs = await listOutputs(page);
  expect(outputs).toHaveLength(before + 1);
  expect(outputs.find((o) => o.slug === slug)?.name).toBe(name);

  // The same name again: refused inline, nothing is created.
  await page.locator('[data-role="stream-output-new"]').click();
  await nameField(page).fill(name);
  await saveButton(page).click();
  await expect(formError(page)).toHaveText(`Výstup s adresou /stream/${slug} už existuje.`);
  expect(await listOutputs(page)).toHaveLength(before + 1);
  await page.locator('[data-role="stream-output-cancel"]').click();
  await expect(outputForm(page)).toHaveCount(0);

  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#842 renaming an output changes its name only; the slug and OBS URL stay", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const n = Date.now() % 100_000;
  const slug = `ren-842-${n}`;
  const created = await page.request.post(`${baseURL}/stream/api/outputs`, {
    data: { slug, name: `Pôvodný ${n}` },
  });
  expect(created.ok(), `create output -> ${created.status()}`).toBeTruthy();

  await openEditor(page, slug);
  await page.locator('[data-role="stream-output-rename"]').click();
  await expect(outputForm(page)).toHaveAttribute("data-mode", "rename");
  await expect(nameField(page)).toHaveValue(`Pôvodný ${n}`);
  await expect(slugPreview(page)).toHaveCount(0);
  await nameField(page).fill(`Nový názov ${n}`);
  await saveButton(page).click();

  await expect(outputForm(page)).toHaveCount(0);
  await expect(outputSelect(page).locator(`option[value="${slug}"]`)).toHaveText(
    `Nový názov ${n}`,
    { timeout: 15_000 },
  );
  await expect(outputSelect(page)).toHaveValue(slug);
  await expect(urlField(page)).toHaveValue(`${baseURL}/stream/${slug}`);
  const renamed = (await listOutputs(page)).find((o) => o.slug === slug);
  expect(renamed?.name).toBe(`Nový názov ${n}`);

  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#842 the output bar with its form open fits a phone without horizontal scroll", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await page.setViewportSize({ width: 360, height: 740 });

  await openEditor(page);
  await page.locator('[data-role="stream-output-new"]').click();
  await nameField(page).fill("Veľmi dlhý názov nového výstupu pre test");
  await expect(slugPreview(page)).toBeVisible();
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow, "horizontal overflow in px").toBeLessThanOrEqual(0);

  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#842 the rename form closes when the operator switches output, so it never renames another one", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const n = Date.now() % 100_000;
  const first = `sw-a-842-${n}`;
  const second = `sw-b-842-${n}`;
  for (const [slug, name] of [
    [first, `Prvý ${n}`],
    [second, `Druhý ${n}`],
  ]) {
    const res = await page.request.post(`${baseURL}/stream/api/outputs`, { data: { slug, name } });
    expect(res.ok(), `create ${slug} -> ${res.status()}`).toBeTruthy();
  }

  await openEditor(page, first);
  await page.locator('[data-role="stream-output-rename"]').click();
  await expect(nameField(page)).toHaveValue(`Prvý ${n}`);
  await outputSelect(page).selectOption(second);
  await expect(outputSelect(page)).toHaveValue(second);
  await expect(outputForm(page)).toHaveCount(0);

  const outputs = await listOutputs(page);
  expect(outputs.find((o) => o.slug === first)?.name).toBe(`Prvý ${n}`);
  expect(outputs.find((o) => o.slug === second)?.name).toBe(`Druhý ${n}`);

  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#842 a click on the OBS URL selects all of it, and an empty rename is refused inline", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);

  // The second click lands on the already focused field: no focus event, and
  // the click alone would collapse the selection to a caret.
  await urlField(page).click();
  await urlField(page).click();
  const selection = await urlField(page).evaluate((el) => {
    const input = el as HTMLInputElement;
    return { start: input.selectionStart, end: input.selectionEnd, length: input.value.length };
  });
  expect(selection.length).toBeGreaterThan(0);
  expect(selection).toEqual({ start: 0, end: selection.length, length: selection.length });

  await page.locator('[data-role="stream-output-rename"]').click();
  await nameField(page).fill("   ");
  await saveButton(page).click();
  await expect(formError(page)).toHaveText("Zadaj názov výstupu.");

  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#855 an output is deleted from the bar after a confirmation, and the editor moves to the first remaining one", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const n = Date.now() % 100_000;
  const slug = `del-855-${n}`;
  const name = `Na zmazanie ${n}`;
  await createOutput(page, slug, name);
  const scene = await page.request.post(`${baseURL}/stream/api/outputs/${slug}/scenes`, {
    data: { name: `Scéna ${n}`, kind: "base" },
  });
  expect(scene.ok(), `create scene -> ${scene.status()}`).toBeTruthy();
  const sceneId = (await scene.json()).id as number;

  await openEditor(page, slug);
  await expect(sceneCard(page, sceneId)).toBeVisible({ timeout: 15_000 });
  await expect(deleteButton(page)).toBeEnabled();
  await expect(deleteConfirm(page)).toHaveCount(0);

  // The confirmation names the output, its OBS address and what goes with it,
  // and nothing is deleted before it is answered.
  await deleteButton(page).click();
  await expect(deleteConfirm(page)).toBeVisible();
  await expect(deleteConfirm(page).locator('[data-role="stream-output-delete-name"]')).toHaveText(
    `Zmazať výstup „${name}“?`,
  );
  await expect(deleteConfirm(page).locator('[data-role="stream-output-delete-url"]')).toHaveText(
    `${baseURL}/stream/${slug}`,
  );
  await expect(
    deleteConfirm(page).locator('[data-role="stream-output-delete-effects"]'),
  ).toContainText("scény, prvky a menovky");
  expect(await defStatus(page, slug)).toBe(200);

  // „Zrušiť" keeps the output.
  await deleteCancel(page).click();
  await expect(deleteConfirm(page)).toHaveCount(0);
  expect(await defStatus(page, slug)).toBe(200);
  await expect(outputSelect(page)).toHaveValue(slug);

  // Escape cancels too.
  await deleteButton(page).click();
  await deleteCancel(page).focus();
  await page.keyboard.press("Escape");
  await expect(deleteConfirm(page)).toHaveCount(0);
  expect(await defStatus(page, slug)).toBe(200);
  await expect(outputSelect(page)).toHaveValue(slug);

  // „Zmazať": gone from the bar and the server, the editor is on the first
  // remaining output.
  const first = (await barSlugs(page)).find((candidate) => candidate !== slug);
  expect(first, "another output exists").toBeTruthy();
  const firstDef = await page.request.get(`${baseURL}/stream/api/outputs/${first}/def`);
  const firstScenes = ((await firstDef.json()) as { scenes: Array<{ id: number }> }).scenes;
  await deleteButton(page).click();
  await deleteYes(page).click();

  await expect(deleteConfirm(page)).toHaveCount(0);
  await expect(outputSelect(page).locator(`option[value="${slug}"]`)).toHaveCount(0, {
    timeout: 15_000,
  });
  await expect(outputSelect(page)).toHaveValue(first as string);
  await expect(urlField(page)).toHaveValue(`${baseURL}/stream/${first}`);
  await expect.poll(() => new URL(page.url()).searchParams.get("output")).toBe(first);
  await expect(toast(page)).toHaveText(`Výstup ${name} bol zmazaný.`);
  await expect(toast(page)).toHaveAttribute("data-state", "success");
  await expect(sceneCard(page, sceneId)).toHaveCount(0);
  if (firstScenes.length > 0) {
    await expect(sceneCard(page, firstScenes[0].id)).toBeVisible({ timeout: 15_000 });
  }
  expect(await defStatus(page, slug), "the deleted output's def is gone").toBe(404);
  expect((await listOutputs(page)).some((output) => output.slug === slug)).toBe(false);

  // The deleted output does not come back after a reload.
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(outputSelect(page).locator("option")).not.toHaveCount(0, { timeout: 15_000 });
  await expect(outputSelect(page)).toHaveValue(first as string);
  await expect(outputSelect(page).locator(`option[value="${slug}"]`)).toHaveCount(0);

  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#855 the seeded stream output cannot be deleted, and the button follows the selected output", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const n = Date.now() % 100_000;
  const slug = `keep-855-${n}`;
  await createOutput(page, slug, `Ostáva ${n}`);

  await openEditor(page, "stream");
  await expect(deleteButton(page)).toBeDisabled();
  await expect(deleteButton(page)).toHaveAttribute(
    "title",
    "Predvolený výstup pre Companion – nedá sa zmazať",
  );
  // A click that gets through anyway opens nothing.
  await deleteButton(page).click({ force: true });
  await expect(deleteConfirm(page)).toHaveCount(0);

  await outputSelect(page).selectOption(slug);
  await expect(outputSelect(page)).toHaveValue(slug);
  await expect(deleteButton(page)).toBeEnabled();
  await outputSelect(page).selectOption("stream");
  await expect(outputSelect(page)).toHaveValue("stream");
  await expect(deleteButton(page)).toBeDisabled();

  expect(await defStatus(page, "stream"), "the stream output still exists").toBe(200);
  expect(await defStatus(page, slug), "the other output was not touched").toBe(200);

  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#855 a delete of an output another operator already removed is an error toast and re-syncs the bar", async ({
  page,
}) => {
  // The refused DELETE is a REAL 404 (the output was removed behind the
  // editor's back), and Chrome logs it as a console error by itself. Collect
  // exactly that line apart, scoped by its URL; every other line stays an error.
  const errors: string[] = [];
  const refusals: string[] = [];
  const n = Date.now() % 100_000;
  const slug = `race-855-${n}`;
  const deleteUrl = `${baseURL}/stream/api/outputs/${slug}`;
  page.on("console", (msg) => {
    const type = msg.type();
    if (type !== "error" && type !== "warning") return;
    const url = msg.location()?.url ?? "";
    if (msg.text().includes("Failed to load resource") && url === deleteUrl) {
      refusals.push(`${msg.text()} ${url}`);
      return;
    }
    errors.push(`[${type}] ${msg.text()}`);
  });
  page.on("pageerror", (err) => errors.push(`[pageerror] ${err.message}`));
  await createOutput(page, slug, `Súbeh ${n}`);

  await openEditor(page, slug);
  const first = (await barSlugs(page)).find((candidate) => candidate !== slug);
  expect(first, "another output exists").toBeTruthy();
  const removed = await page.request.delete(deleteUrl);
  expect(removed.status(), "the output is removed behind the editor's back").toBe(204);

  await deleteButton(page).click();
  await deleteYes(page).click();
  await expect(toast(page)).toHaveText("Výstup už neexistuje.");
  await expect(toast(page)).toHaveAttribute("data-state", "error");
  await expect(deleteConfirm(page)).toHaveCount(0);

  // The list re-syncs: the vanished output leaves the bar, the editor moves on.
  await expect(outputSelect(page).locator(`option[value="${slug}"]`)).toHaveCount(0, {
    timeout: 15_000,
  });
  await expect(outputSelect(page)).toHaveValue(first as string);

  await expect.poll(() => refusals.length, { message: refusals.join(" | ") }).toBe(1);
  expect(
    refusals.every((line) => /status of 404\b/.test(line)),
    `only a 404 for the DELETE: ${refusals.join(" | ")}`,
  ).toBeTruthy();
  // Re-checked last, so a late second refusal still fails the test.
  expect(refusals, "exactly the one refused DELETE").toHaveLength(1);
  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#855 the delete confirmation fits a phone without horizontal scroll", async ({ page }) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const n = Date.now() % 100_000;
  const slug = `phone-855-${n}`;
  await createOutput(page, slug, `Veľmi dlhý názov výstupu určeného na zmazanie ${n}`);
  await page.setViewportSize({ width: 360, height: 740 });

  await openEditor(page, slug);
  await deleteButton(page).click();
  await expect(deleteConfirm(page)).toBeVisible();
  await expect(deleteYes(page)).toBeVisible();
  await expect(deleteCancel(page)).toBeVisible();
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow, "horizontal overflow in px").toBeLessThanOrEqual(0);

  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});
