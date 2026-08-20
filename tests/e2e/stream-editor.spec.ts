import { test, expect, type Browser, type Page } from "@playwright/test";
import {
  assertVersionLabel,
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// Operator editor page for stream graphics (#713, epic #718). The default
// output `stream` is seeded by the migration, so the page loads with zero
// scenes on a fresh DB. All scene names in this file are globally unique
// (case-insensitive uniqueness is enforced per output by the server).

let serverHandle: ServerHandle | undefined;
let baseURL: string;

const sel = {
  page: '[data-role="stream-editor-page"]',
  body: '[data-role="stream-editor"]',
  baseColumns: '[data-role="stream-base-columns"]',
  overlayRow: '[data-role="stream-overlay-row"]',
  addName: '[data-role="stream-add-name"]',
  addKind: '[data-role="stream-add-kind"]',
  addSubmit: '[data-role="stream-add-submit"]',
  scene: '[data-role="stream-scene"]',
  sceneName: '[data-role="stream-scene-name"]',
  toast: '[data-role="toast"]',
};

type SceneDef = {
  id: number;
  name: string;
  kind: string;
  position: number;
  is_active: boolean;
};

type OutputDef = {
  active_scene_id: number | null;
  scenes: SceneDef[];
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
  await page.waitForSelector(sel.body, { timeout: 30_000 });
}

async function getDef(page: Page): Promise<OutputDef> {
  const res = await page.request.get(
    new URL("/stream/api/outputs/stream/def", baseURL).toString(),
    { timeout: 30_000 },
  );
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as OutputDef;
}

async function scenePosition(page: Page, id: string): Promise<number> {
  const def = await getDef(page);
  const scene = def.scenes.find((s) => String(s.id) === id);
  expect(scene, `scene ${id} present in def`).toBeTruthy();
  return scene!.position;
}

/** Create a scene through the UI and return its server-assigned id (as string). */
async function addScene(
  page: Page,
  name: string,
  kind: "base" | "overlay",
): Promise<string> {
  await page.locator(sel.addName).fill(name);
  await page.locator(sel.addKind).selectOption(kind);
  await page.locator(sel.addSubmit).click();
  const card = page
    .locator(sel.scene)
    .filter({ has: page.getByText(name, { exact: true }) });
  await expect(card).toHaveCount(1, { timeout: 15_000 });
  const id = await card.getAttribute("data-scene-id");
  expect(id, `new scene ${name} has a data-scene-id`).toBeTruthy();
  return id as string;
}

function cardById(page: Page, id: string) {
  return page.locator(`${sel.scene}[data-scene-id="${id}"]`);
}

test("create, rename, reorder, activate, overlay-toggle and delete scenes", async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);

  await openEditor(page);

  // Version label present + correct (dashboard-version mandate).
  await assertVersionLabel(page, baseURL);

  // --- Create two base scenes and one overlay scene ---
  const alpha = await addScene(page, "SE_BaseAlpha", "base");
  const beta = await addScene(page, "SE_BaseBeta", "base");
  const overlay = await addScene(page, "SE_OverlayX", "overlay");

  // Base scenes render as columns, overlay in the overlay row.
  await expect(
    page.locator(`${sel.baseColumns} ${sel.scene}[data-kind="base"]`),
  ).toHaveCount(2);
  await expect(
    page.locator(`${sel.overlayRow} ${sel.scene}[data-kind="overlay"]`),
  ).toHaveCount(1);

  // --- Rename base alpha ---
  await cardById(page, alpha)
    .locator('[data-role="stream-scene-rename"]')
    .click();
  await cardById(page, alpha)
    .locator('[data-role="stream-scene-name-input"]')
    .fill("SE_BaseAlphaRenamed");
  await cardById(page, alpha)
    .locator('[data-role="stream-scene-rename-save"]')
    .click();
  await expect(cardById(page, alpha).locator(sel.sceneName)).toHaveText(
    "SE_BaseAlphaRenamed",
  );
  // Persisted in the def.
  await expect
    .poll(async () => {
      const def = await getDef(page);
      return def.scenes.find((s) => String(s.id) === alpha)?.name;
    })
    .toBe("SE_BaseAlphaRenamed");

  // --- Reorder: move beta up (it was created second → position after alpha) ---
  const alphaPosBefore = await scenePosition(page, alpha);
  const betaPosBefore = await scenePosition(page, beta);
  expect(betaPosBefore).toBeGreaterThan(alphaPosBefore);
  await cardById(page, beta).locator('[data-role="stream-scene-up"]').click();
  await expect
    .poll(async () => {
      const a = await scenePosition(page, alpha);
      const b = await scenePosition(page, beta);
      return b < a;
    })
    .toBe(true);

  // --- Activate a base scene → exclusive highlight + def reflects it ---
  await cardById(page, alpha).locator('[data-role="stream-activate"]').click();
  await expect(cardById(page, alpha)).toHaveAttribute("data-active", "true");
  await expect(cardById(page, beta)).toHaveAttribute("data-active", "false");
  await expect
    .poll(async () => (await getDef(page)).active_scene_id)
    .toBe(Number(alpha));

  // --- Activation persists across a reload ---
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(cardById(page, alpha)).toHaveAttribute("data-active", "true", {
    timeout: 15_000,
  });

  // --- Overlay toggles independently of the base ---
  await cardById(page, overlay)
    .locator('[data-role="stream-overlay-toggle"]')
    .click();
  await expect(cardById(page, overlay)).toHaveAttribute("data-active", "true");
  // Base is still active — overlay is independent.
  await expect(cardById(page, alpha)).toHaveAttribute("data-active", "true");
  await expect
    .poll(async () => {
      const def = await getDef(page);
      return def.scenes.find((s) => String(s.id) === overlay)?.is_active;
    })
    .toBe(true);

  // --- Delete the overlay with the native confirm dialog ---
  page.once("dialog", (dialog) => dialog.accept());
  await cardById(page, overlay)
    .locator('[data-role="stream-scene-delete"]')
    .click();
  await expect(cardById(page, overlay)).toHaveCount(0, { timeout: 15_000 });

  expect(errors, "browser console must be clean").toEqual([]);
});

test("a second operator context reflects activation live", async ({
  browser,
}: {
  browser: Browser;
}) => {
  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  const pageA = await ctxA.newPage();
  const pageB = await ctxB.newPage();
  const errorsA: string[] = [];
  const errorsB: string[] = [];
  attachConsoleErrorCollector(pageA, errorsA);
  attachConsoleErrorCollector(pageB, errorsB);

  // Seed a base scene via the API so both contexts load it identically.
  const createRes = await pageA.request.post(
    new URL("/stream/api/outputs/stream/scenes", baseURL).toString(),
    { data: { name: "SE_LiveBase", kind: "base" }, timeout: 30_000 },
  );
  expect(createRes.ok()).toBeTruthy();
  const liveId = String(((await createRes.json()) as SceneDef).id);

  await openEditor(pageA);
  await openEditor(pageB);

  // Both see the scene inactive to start.
  await expect(cardById(pageA, liveId)).toHaveAttribute("data-active", "false");
  await expect(cardById(pageB, liveId)).toHaveAttribute("data-active", "false");

  // Activate in context A → context B must reflect it live (StreamState WS).
  await pageA.locator(`${sel.scene}[data-scene-id="${liveId}"] [data-role="stream-activate"]`).click();
  await expect(cardById(pageB, liveId)).toHaveAttribute("data-active", "true", {
    timeout: 15_000,
  });

  expect(errorsA, "context A console clean").toEqual([]);
  expect(errorsB, "context B console clean").toEqual([]);

  await ctxA.close();
  await ctxB.close();
});
