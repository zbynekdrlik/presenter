/**
 * Stream editor TEXT OPTIONS, end to end through the real editor UI and the
 * real OBS output page.
 *
 *  - #831 "VEĽKÉ PÍSMENÁ": ticking it on the nameplate (lower third) NAME line
 *    saves `uppercase: true` on that TextStyle only, and the output renders the
 *    line with `text-transform: uppercase` while the stored text keeps its case.
 *
 * Every test asserts a clean browser console last.
 */
import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  attachEditorConsoleCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

const SLUG = "stream"; // the migration-seeded default output.

type ElementDef = { id: number; props: Record<string, any> & { kind: string } };
type SceneDef = { id: number; name: string; kind: string; elements: ElementDef[] };
type OutputDef = { scenes: SceneDef[] };

const sel = {
  editor: '[data-role="stream-editor"]',
  addName: '[data-role="stream-add-name"]',
  addKind: '[data-role="stream-add-kind"]',
  addSubmit: '[data-role="stream-add-submit"]',
  scene: '[data-role="stream-scene"]',
  save: '[data-role="stream-prop-save"]',
  propForm: '[data-role="stream-prop-form"]',
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

async function openEditor(page: Page) {
  await page.goto(`${baseURL}/ui/stream`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector(sel.editor, { timeout: 30_000 });
}

async function getDef(page: Page): Promise<OutputDef> {
  const res = await page.request.get(`${baseURL}/stream/api/outputs/${SLUG}/def`, {
    timeout: 30_000,
  });
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as OutputDef;
}

async function getElement(page: Page, sceneId: string, id: string): Promise<ElementDef> {
  const scene = (await getDef(page)).scenes.find((s) => String(s.id) === sceneId);
  const el = scene?.elements.find((e) => String(e.id) === id);
  expect(el, `element ${id} present in scene ${sceneId}`).toBeTruthy();
  return el as ElementDef;
}

async function addScene(page: Page, name: string): Promise<string> {
  await page.locator(sel.addName).fill(name);
  await page.locator(sel.addKind).selectOption("base");
  await page.locator(sel.addSubmit).click();
  const card = page.locator(sel.scene).filter({ has: page.getByText(name, { exact: true }) });
  await expect(card).toHaveCount(1, { timeout: 15_000 });
  const id = await card.getAttribute("data-scene-id");
  expect(id, "new scene has a data-scene-id").toBeTruthy();
  return id as string;
}

async function openPanel(page: Page, sceneId: string) {
  await page
    .locator(`${sel.scene}[data-scene-id="${sceneId}"] [data-role="stream-scene-edit"]`)
    .click();
  await page.waitForSelector('[data-role="stream-element-panel"]', { timeout: 15_000 });
}

/** Add an element of `kind` through the panel; returns its id once the page
 *  selected it AND seeded its draft (the canvas outline reads the draft). */
async function addElement(page: Page, sceneId: string, kind: string): Promise<string> {
  const before = ((await getDef(page)).scenes.find((s) => String(s.id) === sceneId)?.elements ?? []).map(
    (e) => e.id,
  );
  await page.locator(`[data-role="stream-add-element-${kind}"]`).click();
  let newId = "";
  await expect
    .poll(async () => {
      const scene = (await getDef(page)).scenes.find((s) => String(s.id) === sceneId);
      const added = scene?.elements.find((e) => !before.includes(e.id));
      if (added) newId = String(added.id);
      return newId !== "";
    })
    .toBe(true);
  await expect(
    page.locator(`[data-role="stream-overlay-element"][data-element-id="${newId}"]`),
  ).toHaveAttribute("data-selected", "true", { timeout: 15_000 });
  await page.waitForSelector(sel.propForm, { timeout: 10_000 });
  return newId;
}

/** Save the selected element and wait until the page's own def agrees. */
async function save(page: Page) {
  await page.locator(sel.save).click();
  await expect(page.locator(sel.save)).toHaveAttribute("data-dirty", "false", { timeout: 15_000 });
}

test("#831 VEĽKÉ PÍSMENÁ on the nameplate name line renders capitals on the output", async ({
  page,
}) => {
  const editorErrors: string[] = [];
  attachEditorConsoleCollector(page, editorErrors);
  await openEditor(page);

  const scene = await addScene(page, "SC_831_Upper");
  await openPanel(page, scene);
  const el = await addElement(page, scene, "lower_third");

  // Tick capitals on the NAME line only; the role line stays as typed.
  const primaryUpper = page.locator(
    '[data-role="stream-ts-primary"] [data-role="stream-ts-uppercase"]',
  );
  const secondaryUpper = page.locator(
    '[data-role="stream-ts-secondary"] [data-role="stream-ts-uppercase"]',
  );
  await expect(primaryUpper).not.toBeChecked();
  await primaryUpper.check();
  await expect(secondaryUpper).not.toBeChecked();
  await save(page);

  await expect
    .poll(async () => {
      const props = (await getElement(page, scene, el)).props;
      return [props.primary_style?.uppercase ?? null, props.secondary_style?.uppercase ?? null];
    })
    .toEqual([true, null]);

  // Put a person plate on air in that scene, then read the OBS output page.
  const act = await page.request.put(`${baseURL}/stream/api/outputs/${SLUG}/active-scene`, {
    data: { sceneId: Number(scene) },
  });
  expect(act.ok(), `activate -> ${act.status()}`).toBeTruthy();
  const plateResp = await page.request.post(`${baseURL}/stream/api/outputs/${SLUG}/nameplates`, {
    data: { primaryText: "Ján Novák", secondaryText: "pastor" },
  });
  expect(plateResp.ok(), `create plate -> ${plateResp.status()}`).toBeTruthy();
  const plateId = (await plateResp.json()).id as number;
  const show = await page.request.put(`${baseURL}/stream/api/outputs/${SLUG}/nameplates/active`, {
    data: { source: "person", id: plateId },
  });
  expect(show.ok(), `show plate -> ${show.status()}`).toBeTruthy();

  const outputErrors: string[] = [];
  const output = await page.context().newPage();
  attachConsoleErrorCollector(output, outputErrors);
  await output.goto(`${baseURL}/stream/${SLUG}`);
  await output.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  const plate = output.locator(
    `[data-role="stream-element-lower-third"][data-element-id="${el}"] [data-role="stream-lower-third-plate"]`,
  );
  await expect(plate).toHaveCount(1, { timeout: 15_000 });
  const name = plate.locator('[data-role="stream-lower-third-primary"]');
  const role = plate.locator('[data-role="stream-lower-third-secondary"]');

  // Rendered in capitals by CSS; the stored text keeps its own case.
  expect(await name.evaluate((n) => getComputedStyle(n).textTransform)).toBe("uppercase");
  expect(await role.evaluate((n) => getComputedStyle(n).textTransform)).toBe("none");
  await expect(name).toHaveText("Ján Novák");
  expect(await name.evaluate((n) => (n as HTMLElement).innerText)).toBe("JÁN NOVÁK");

  const hide = await page.request.put(`${baseURL}/stream/api/outputs/${SLUG}/nameplates/active`, {
    data: { source: null },
  });
  expect(hide.ok()).toBeTruthy();
  await output.close();

  expect(outputErrors, `output console: ${outputErrors.join(" | ")}`).toEqual([]);
  expect(editorErrors, `editor console: ${editorErrors.join(" | ")}`).toEqual([]);
});
