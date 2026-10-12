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

async function listOutputs(page: Page): Promise<OutputSummary[]> {
  const res = await page.request.get(`${baseURL}/stream/api/outputs`);
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as OutputSummary[];
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
