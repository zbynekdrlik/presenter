import path from "node:path";
import { test, expect, type Page } from "@playwright/test";
import {
  attachEditorConsoleCollector,
  deriveTestConfig,
  refreshDevData,
  REPO_ROOT,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #715: in-editor live preview iframe (forced selected scene) + asset upload /
// picker for image elements. The output page (`/stream/{slug}`) and the asset
// REST surface are PARALLEL lanes (#709 / #708); these tests therefore run in
// integrated CI (where those routes exist), not in an isolated worktree.
//
// `page.on("console")` (attachEditorConsoleCollector) captures console messages
// from child frames too, so the zero-console assertions cover the preview
// iframe as required by the stream-graphics UI skill. It ignores only the
// expected `/stream/assets/<id>` 404 of a not-yet-picked placeholder image
// asset (authoring transient); every other iframe error still fails.

let serverHandle: ServerHandle | undefined;
let baseURL: string;

// A 1x1 PNG fixture — a valid magic-byte + mime image for the #708 upload
// validation (contents are irrelevant to the picker/assign/reference flow).
const PNG_FIXTURE = path.join(REPO_ROOT, "tests", "e2e", "fixtures", "stream-asset-1x1.png");

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
  await page.waitForSelector('[data-role="stream-editor"]', { timeout: 30_000 });
}

async function createScene(page: Page, name: string): Promise<number> {
  const res = await page.request.post(
    new URL("/stream/api/outputs/stream/scenes", baseURL).toString(),
    { data: { name, kind: "base" }, timeout: 30_000 },
  );
  expect(res.ok()).toBeTruthy();
  return ((await res.json()) as { id: number }).id;
}

async function createImageElement(page: Page, sceneId: number, assetId: number): Promise<number> {
  const res = await page.request.post(
    new URL(`/stream/api/scenes/${sceneId}/elements`, baseURL).toString(),
    {
      data: {
        kind: "image",
        asset_id: assetId,
        fit: "contain",
        frame: { xPct: 10, yPct: 10, wPct: 80, hPct: 60 },
        opacity: 1.0,
      },
      timeout: 30_000,
    },
  );
  expect(res.ok(), "seed image element").toBeTruthy();
  return ((await res.json()) as { id: number }).id;
}

async function openPanel(page: Page, sceneId: number) {
  await page
    .locator(`[data-role="stream-scene"][data-scene-id="${sceneId}"] [data-role="stream-scene-edit"]`)
    .click();
  await page.waitForSelector('[data-role="stream-element-panel"]', { timeout: 15_000 });
}

test("preview iframe forces the selected scene and toggles live", async ({ page }) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);

  const sceneId = await createScene(page, "SP_PreviewScene");
  await openPanel(page, sceneId);

  const frame = page.locator('[data-role="stream-preview-frame"]');
  await expect(frame).toHaveCount(1);

  // Forced to the selected scene by default.
  await expect
    .poll(async () => (await frame.getAttribute("src")) ?? "")
    .toContain(`preview=1&scene=${sceneId}`);

  // "Naživo" toggle drops the forced scene param.
  await page.locator('[data-role="stream-preview-live-toggle"]').click();
  await expect
    .poll(async () => (await frame.getAttribute("src")) ?? "")
    .not.toContain(`scene=${sceneId}`);
  await expect
    .poll(async () => (await frame.getAttribute("src")) ?? "")
    .toContain("preview=1");

  expect(errors, "editor + iframe console must be clean").toEqual([]);
});

test("upload → picker → assign → visible in preview; guarded delete 409", async ({ page }) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);

  const sceneId = await createScene(page, "SP_AssetScene");
  const elementId = await createImageElement(page, sceneId, 1);
  await openPanel(page, sceneId);

  // Select the image element → property form + asset picker button.
  await page
    .locator(`[data-role="stream-element"][data-element-id="${elementId}"] [data-role="stream-element-select"]`)
    .click();
  await page.waitForSelector('[data-role="stream-prop-form"]', { timeout: 10_000 });

  // Open the picker and upload a PNG through the editor.
  await page.locator('[data-role="stream-asset-pick"]').click();
  await page.waitForSelector('[data-role="stream-asset-picker"]', { timeout: 10_000 });
  await page.locator('[data-role="stream-asset-upload"]').setInputFiles(PNG_FIXTURE);
  await page.locator('[data-role="stream-asset-upload-btn"]').click();

  // The uploaded asset appears in the picker.
  const item = page.locator('[data-role="stream-asset-item"]');
  await expect(item).toHaveCount(1, { timeout: 15_000 });
  const assetId = await item.first().getAttribute("data-asset-id");
  expect(assetId, "uploaded asset has an id").toBeTruthy();

  // Assign it to the image element and save.
  await item.first().locator('[data-role="stream-asset-select"]').click();
  await expect(page.locator('[data-role="stream-asset-picker"]')).toHaveCount(0);
  await page.locator('[data-role="stream-image-asset-id"]').fill(String(assetId));
  await page.locator('[data-role="stream-prop-save"]').click();

  // Def reflects the assigned asset_id.
  await expect
    .poll(async () => {
      const def = await page.request
        .get(new URL("/stream/api/outputs/stream/def", baseURL).toString())
        .then((r) => r.json());
      const el = def.scenes
        .flatMap((s: any) => s.elements)
        .find((e: any) => String(e.id) === String(elementId));
      return el?.props?.asset_id;
    })
    .toBe(Number(assetId));

  // Visible in the forced-scene preview (output page renders image elements as
  // <img src="/stream/assets/{asset_id}"> per #709's contract).
  await expect(
    page
      .frameLocator('[data-role="stream-preview-frame"]')
      .locator(`img[src*="/stream/assets/${assetId}"]`),
  ).toBeVisible({ timeout: 20_000 });

  // Deleting the referenced asset is refused 409 with a message naming scenes.
  await page.locator('[data-role="stream-asset-pick"]').click();
  await page.waitForSelector('[data-role="stream-asset-picker"]', { timeout: 10_000 });
  await page
    .locator(`[data-role="stream-asset-item"][data-asset-id="${assetId}"] [data-role="stream-asset-delete"]`)
    .click();
  await expect(page.locator('[data-role="stream-asset-error"]')).toBeVisible({ timeout: 10_000 });
  // Still present (delete was refused).
  await expect(
    page.locator(`[data-role="stream-asset-item"][data-asset-id="${assetId}"]`),
  ).toHaveCount(1);

  expect(errors, "editor + iframe console must be clean").toEqual([]);
});
