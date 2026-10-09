/**
 * Uploaded font faces whose OS/2 weight is mislabelled (#830).
 *
 * Nexa on SNV/PP ships Light, Heavy, Black and XBold files that all declare
 * the default OS/2 weight 400; the real style is only in the name table. The
 * editor offered just "400" and "700", and fonts.css served colliding
 * `@font-face` rules. This spec builds a family the same way: four faces derived
 * in memory from the OFL fixture (Regular, Heavy, Black, Black Italic), every
 * one at OS/2 400, Heavy uploaded BEFORE Black. It uploads them through the
 * editor's Písma tab, then checks:
 *
 *  - the server gives each face its own weight (Heavy 800 below Black 900);
 *  - the weight picker lists every face as "<style name> <weight>";
 *  - Italic is offered only where the family has an italic face;
 *  - picking Black + Italic renders the OUTPUT text at font-weight 900,
 *    font-style italic, with one `@font-face` per weight/style loaded.
 *
 * A clean console on both pages is the last assertion.
 */
import fs from "fs";
import path from "path";
import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  attachEditorConsoleCollector,
  deriveTestConfig,
  refreshDevData,
  REPO_ROOT,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";
import { withItalicFlag, withNames } from "./font-surgery";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

const SLUG = "stream"; // the migration-seeded default output.
const FAMILY = "Facet830";
const TYPO_FAMILY = 16;
const TYPO_SUBFAMILY = 17;

type ElementDef = { id: number; props: Record<string, any> & { kind: string } };
type SceneDef = { id: number; name: string; kind: string; elements: ElementDef[] };
type OutputDef = { scenes: SceneDef[] };
type ListedFont = { family: string; weight: number; italic: boolean; styleName?: string };

const sel = {
  editor: '[data-role="stream-editor"]',
  fontsTab: '[data-role="stream-editor-tab"][data-tab="fonts"]',
  scenesTab: '[data-role="stream-editor-tab"][data-tab="scenes"]',
  fontUpload: '[data-role="stream-font-upload"]',
  fontUploadBtn: '[data-role="stream-font-upload-btn"]',
  fontItem: '[data-role="stream-font-item"]',
  addName: '[data-role="stream-add-name"]',
  addKind: '[data-role="stream-add-kind"]',
  addSubmit: '[data-role="stream-add-submit"]',
  scene: '[data-role="stream-scene"]',
  save: '[data-role="stream-prop-save"]',
  propForm: '[data-role="stream-prop-form"]',
  canvas: '[data-role="stream-canvas"]',
};

const fixture = fs.readFileSync(
  path.join(REPO_ROOT, "tests", "e2e", "fixtures", "fonts", "Gruppo-Regular.ttf"),
);

/** A face of FAMILY named `style` in name 17; its OS/2 weight stays 400. */
function face(style: string): Buffer {
  return withNames(fixture, { [TYPO_FAMILY]: FAMILY, [TYPO_SUBFAMILY]: style });
}

/** Heavy comes BEFORE Black: Black's upload must move the stored Heavy. */
const FACES = [
  { name: "Facet830-Regular.ttf", buffer: face("Regular") },
  { name: "Facet830-Heavy.ttf", buffer: face("Heavy") },
  { name: "Facet830-Black.ttf", buffer: face("Black") },
  { name: "Facet830-BlackItalic.ttf", buffer: withItalicFlag(face("Black Italic")) },
];

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

/** Upload every face through the Písma tab's panel, then back to Scény. */
async function uploadFaces(page: Page) {
  await page.locator(sel.fontsTab).click();
  await page.setInputFiles(
    sel.fontUpload,
    FACES.map((f) => ({ name: f.name, mimeType: "font/ttf", buffer: f.buffer })),
  );
  await page.locator(sel.fontUploadBtn).click();
  await expect(page.locator(`${sel.fontItem}[data-font-family="${FAMILY}"]`)).toHaveCount(
    FACES.length,
    { timeout: 30_000 },
  );
  await page.locator(sel.scenesTab).click();
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

/** Add an element of `kind`; returns its id once the page selected AND seeded it. */
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

async function save(page: Page) {
  await page.locator(sel.save).click();
  await expect(page.locator(sel.save)).toHaveAttribute("data-dirty", "false", { timeout: 15_000 });
}

test("#830 every face of a mislabelled family is offered by name; Black Italic renders on the output", async ({
  page,
}) => {
  const editorErrors: string[] = [];
  attachEditorConsoleCollector(page, editorErrors);
  await openEditor(page);
  await uploadFaces(page);

  // Server: one distinct weight/style per face, each named by its style.
  const listed = (await (await page.request.get(`${baseURL}/stream/api/fonts`)).json()) as ListedFont[];
  const faces = listed
    .filter((f) => f.family === FAMILY)
    .map((f) => `${f.weight}/${f.italic ? "italic" : "normal"}/${f.styleName ?? ""}`)
    .sort();
  expect(faces).toEqual([
    "400/normal/Regular",
    "800/normal/Heavy",
    "900/italic/Black Italic",
    "900/normal/Black",
  ]);

  const scene = await addScene(page, `SC_830_${Date.now()}`);
  await openPanel(page, scene);
  const el = await addElement(page, scene, "countdown");

  const group = page.locator('[data-role="stream-ts-countdown"]');
  await group.locator('[data-role="stream-ts-font"]').selectOption(FAMILY);
  const weight = group.locator('[data-role="stream-ts-weight"]');
  const italic = group.locator('[data-role="stream-ts-italic"]');

  // Every face by style name and weight.
  await expect(weight.locator('option[value="400"]')).toHaveText("Regular 400");
  await expect(weight.locator('option[value="800"]')).toHaveText("Heavy 800");
  await expect(weight.locator('option[value="900"]')).toHaveText("Black 900");
  // A new countdown sits at 700: no face there, so no italic face either.
  await expect(weight).toHaveValue("700");
  await expect(italic).toBeDisabled();

  await weight.selectOption("900");
  await expect(italic).toBeEnabled();
  await italic.check();
  await save(page);
  await expect
    .poll(async () => {
      const style = (await getElement(page, scene, el)).props.style;
      return [style?.fontFamily, style?.weight, style?.italic ?? null];
    })
    .toEqual([FAMILY, 900, true]);

  const act = await page.request.put(`${baseURL}/stream/api/outputs/${SLUG}/active-scene`, {
    data: { sceneId: Number(scene) },
  });
  expect(act.ok(), `activate -> ${act.status()}`).toBeTruthy();

  const outputErrors: string[] = [];
  const output = await page.context().newPage();
  attachConsoleErrorCollector(output, outputErrors);
  await output.goto(`${baseURL}/stream/${SLUG}`);
  await output.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(output.locator(`${sel.canvas}[data-fonts-ready="true"]`)).toHaveCount(1, {
    timeout: 15_000,
  });
  const countdown = output.locator(
    `[data-role="stream-element-countdown"][data-element-id="${el}"]`,
  );
  await expect(countdown).toHaveCount(1, { timeout: 15_000 });
  const computed = await countdown.evaluate((node) => {
    const style = getComputedStyle(node);
    return { family: style.fontFamily, weight: style.fontWeight, style: style.fontStyle };
  });
  expect(computed.family).toContain(FAMILY);
  expect(computed.weight).toBe("900");
  expect(computed.style).toBe("italic");

  // fonts.css registers one face per weight/style for the family: no collision.
  const registered = await output.evaluate((fam) => {
    const out: string[] = [];
    document.fonts.forEach((f) => {
      if (f.family.replace(/"/g, "") === fam) out.push(`${f.weight}/${f.style}`);
    });
    return out.sort();
  }, FAMILY);
  expect(registered).toEqual(["400/normal", "800/normal", "900/italic", "900/normal"]);

  // And the browser loads the Black Italic face itself.
  await expect
    .poll(
      () =>
        output.evaluate(async (fam) => {
          try {
            await document.fonts.load(`italic 900 1em "${fam}"`);
          } catch {
            // ignored — `check` below is the assertion
          }
          return document.fonts.check(`italic 900 1em "${fam}"`);
        }, FAMILY),
      { message: "the Black Italic face is loaded", timeout: 10_000 },
    )
    .toBe(true);
  await output.close();

  expect(outputErrors, `output console: ${outputErrors.join(" | ")}`).toEqual([]);
  expect(editorErrors, `editor console: ${editorErrors.join(" | ")}`).toEqual([]);
});
