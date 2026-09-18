/**
 * Uploaded web fonts for stream graphics (#778, epic #718).
 *
 * Acceptance path: upload the committed OFL fixture (family "Gruppo") THROUGH
 * the editor UI, pick it on a countdown element's text style, activate the
 * scene, and confirm the OUTPUT page renders in that face — its computed
 * `font-family` starts with the family AND `document.fonts.check()` is true —
 * with a clean console. Also: an unknown family is still rejected 422.
 */
import {
  test,
  expect,
  type APIRequestContext,
  type Page,
} from "@playwright/test";
import path from "path";
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

let serverHandle: ServerHandle | undefined;
let baseURL: string;

const SLUG = "stream"; // the migration-seeded default output.
const FONT_FIXTURE = path.join(
  REPO_ROOT,
  "tests",
  "e2e",
  "fixtures",
  "fonts",
  "Gruppo-Regular.ttf",
);
const FAMILY = "Gruppo"; // the fixture's internal name-table family.

const sel = {
  editorBody: '[data-role="stream-editor"]',
  fontUpload: '[data-role="stream-font-upload"]',
  fontUploadBtn: '[data-role="stream-font-upload-btn"]',
  fontItem: '[data-role="stream-font-item"]',
  addName: '[data-role="stream-add-name"]',
  addKind: '[data-role="stream-add-kind"]',
  addSubmit: '[data-role="stream-add-submit"]',
  scene: '[data-role="stream-scene"]',
  sceneEdit: '[data-role="stream-scene-edit"]',
  addCountdown: '[data-role="stream-add-element-countdown"]',
  tsFont: '[data-role="stream-ts-font"]',
  propSave: '[data-role="stream-prop-save"]',
  canvas: '[data-role="stream-canvas"]',
  countdown: '[data-role="stream-element-countdown"]',
};

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(
    config.port,
    config.dbUrl,
    config.oscPort,
  );
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

async function openEditor(page: Page): Promise<void> {
  await page.goto(`${baseURL}/ui/stream`);
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector(sel.editorBody, { timeout: 30_000 });
}

/** Upload the fixture font through the editor's font panel UI. */
async function uploadFontViaUi(page: Page): Promise<void> {
  await page.setInputFiles(sel.fontUpload, FONT_FIXTURE);
  await page.locator(sel.fontUploadBtn).click();
  // The face appears in the panel list once the upload + reload complete.
  await expect(
    page.locator(`${sel.fontItem}[data-font-family="${FAMILY}"]`),
  ).toHaveCount(1, { timeout: 20_000 });
}

async function addBaseScene(page: Page, name: string): Promise<string> {
  await page.locator(sel.addName).fill(name);
  await page.locator(sel.addKind).selectOption("base");
  await page.locator(sel.addSubmit).click();
  const card = page
    .locator(sel.scene)
    .filter({ has: page.getByText(name, { exact: true }) });
  await expect(card).toHaveCount(1, { timeout: 15_000 });
  const id = await card.getAttribute("data-scene-id");
  expect(id, `scene ${name} has a data-scene-id`).toBeTruthy();
  return id as string;
}

async function activateBase(
  request: APIRequestContext,
  sceneId: number,
): Promise<void> {
  const resp = await request.put(
    `${baseURL}/stream/api/outputs/${SLUG}/active-scene`,
    { data: { sceneId } },
  );
  expect(resp.ok(), `activate -> ${resp.status()}`).toBeTruthy();
}

async function getDef(request: APIRequestContext): Promise<any> {
  const resp = await request.get(`${baseURL}/stream/api/outputs/${SLUG}/def`);
  expect(resp.ok(), `def -> ${resp.status()}`).toBeTruthy();
  return resp.json();
}

test("upload a font via the editor, pick it, output renders in that face", async ({
  page,
}) => {
  const editorErrors: string[] = [];
  attachEditorConsoleCollector(page, editorErrors);

  await openEditor(page);
  await uploadFontViaUi(page);

  // Author a base scene + a countdown element, then PICK the uploaded family.
  const sceneName = `FontScene_${Date.now()}`;
  const sceneIdStr = await addBaseScene(page, sceneName);
  await page
    .locator(`${sel.scene}[data-scene-id="${sceneIdStr}"] ${sel.sceneEdit}`)
    .click();
  await page.locator(sel.addCountdown).click();
  // The property form's font <select> now includes the uploaded family.
  const fontSelect = page.locator(sel.tsFont);
  await expect(fontSelect).toBeVisible({ timeout: 15_000 });
  await expect(fontSelect.locator(`option[value="${FAMILY}"]`)).toHaveCount(1, {
    timeout: 15_000,
  });
  await fontSelect.selectOption(FAMILY);
  await page.locator(sel.propSave).click();

  // The saved def must carry the picked family (proves pick + save worked).
  await expect
    .poll(
      async () => {
        const def = await getDef(page.request);
        const styles: string[] = [];
        for (const scene of def.scenes ?? []) {
          for (const el of scene.elements ?? []) {
            const ff = el?.props?.style?.fontFamily;
            if (ff) styles.push(ff);
          }
        }
        return styles;
      },
      { timeout: 15_000 },
    )
    .toContain(FAMILY);

  expect(editorErrors, `editor console: ${editorErrors.join(" | ")}`).toEqual(
    [],
  );

  // Activate the scene, then open the OUTPUT page in a clean tab.
  const def = await getDef(page.request);
  const sceneId = def.scenes.find((s: any) => s.name === sceneName)
    ?.id as number;
  expect(sceneId, "scene id resolved").toBeTruthy();
  await activateBase(page.request, sceneId);

  const outputErrors: string[] = [];
  const output = await page.context().newPage();
  attachConsoleErrorCollector(output, outputErrors);
  await output.goto(`${baseURL}/stream/${SLUG}`);
  await output.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await output.waitForSelector(sel.canvas, { timeout: 10_000 });
  await output.waitForSelector(sel.countdown, { timeout: 15_000 });

  // Text is revealed once the fonts settle (data-fonts-ready flips true).
  await expect(
    output.locator(`${sel.canvas}[data-fonts-ready="true"]`),
  ).toHaveCount(1, { timeout: 15_000 });

  // The countdown element's computed font-family resolves to the uploaded face.
  const fontFamily = await output
    .locator(sel.countdown)
    .evaluate((el) => window.getComputedStyle(el).fontFamily);
  expect(fontFamily).toContain(FAMILY);

  // And the browser has actually loaded that face (poll: the face finishes
  // loading asynchronously after the stylesheet is parsed).
  await expect
    .poll(
      () =>
        output.evaluate(
          (fam) => document.fonts.check(`400 1em "${fam}"`),
          FAMILY,
        ),
      {
        message: "document.fonts.check for the uploaded family",
        timeout: 10_000,
      },
    )
    .toBe(true);

  expect(outputErrors, `output console: ${outputErrors.join(" | ")}`).toEqual(
    [],
  );
  await output.close();
});

test("an unknown font family is still rejected 422", async ({ page }) => {
  // Seed a base scene, then a countdown element with a family that is neither a
  // built-in nor uploaded → core validation refuses with 422.
  const sceneResp = await page.request.post(
    `${baseURL}/stream/api/outputs/${SLUG}/scenes`,
    { data: { name: `Unknown_${Date.now()}`, kind: "base" } },
  );
  expect(sceneResp.ok()).toBeTruthy();
  const sceneId = (await sceneResp.json()).id as number;

  const resp = await page.request.post(
    `${baseURL}/stream/api/scenes/${sceneId}/elements`,
    {
      data: {
        kind: "countdown",
        timer_id: 1,
        style: {
          fontFamily: "Definitely Not A Real Font 778",
          sizePct: 10,
          color: "#ffffff",
          weight: 400,
          align: "center",
          lineHeight: 1.2,
        },
        frame: { xPct: 10, yPct: 10, wPct: 50, hPct: 20 },
      },
    },
  );
  expect(resp.status(), "unknown family -> 422").toBe(422);
});
