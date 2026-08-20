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

type ElementDef = {
  id: number;
  z_order: number;
  props: Record<string, unknown> & { kind: string };
};

type SceneDef = {
  id: number;
  name: string;
  kind: string;
  position: number;
  is_active: boolean;
  elements: ElementDef[];
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

// --- #714: element CRUD + property panel -----------------------------------

async function getScene(page: Page, sceneId: string): Promise<SceneDef> {
  const def = await getDef(page);
  const scene = def.scenes.find((s) => String(s.id) === sceneId);
  expect(scene, `scene ${sceneId} present`).toBeTruthy();
  return scene as SceneDef;
}

/** Open a scene's element panel via its "Upraviť prvky" button. */
async function openPanel(page: Page, sceneId: string) {
  await cardById(page, sceneId).locator('[data-role="stream-scene-edit"]').click();
  await page.waitForSelector('[data-role="stream-element-panel"]', { timeout: 15_000 });
}

/** Add an element of `kind` to the currently-open panel; return its new id. */
async function addElement(
  page: Page,
  sceneId: string,
  kind: "image" | "countdown" | "lyrics" | "verse",
): Promise<string> {
  const before = (await getScene(page, sceneId)).elements.map((e) => e.id);
  await page.locator(`[data-role="stream-add-element-${kind}"]`).click();
  let newId = "";
  await expect
    .poll(async () => {
      const after = (await getScene(page, sceneId)).elements;
      const added = after.find((e) => !before.includes(e.id));
      if (added) {
        newId = String(added.id);
        return true;
      }
      return false;
    })
    .toBe(true);
  return newId;
}

async function setColorInput(page: Page, selector: string, value: string) {
  await page.locator(selector).evaluate((el, v) => {
    (el as HTMLInputElement).value = v;
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }, value);
}

test("element CRUD, property edit, inline 422 and z-order", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await openEditor(page);

  const scene = await addScene(page, "SE_ElemScene", "base");
  await openPanel(page, scene);

  // Add one element of each kind.
  const img = await addElement(page, scene, "image");
  const cd = await addElement(page, scene, "countdown");
  const lyr = await addElement(page, scene, "lyrics");
  const verse = await addElement(page, scene, "verse");
  {
    const kinds = (await getScene(page, scene)).elements.map((e) => e.props.kind);
    expect(kinds).toEqual(["image", "countdown", "lyrics", "verse"]);
  }

  // --- Edit the countdown's frame + font size + color + shadow + alignment ---
  await page
    .locator(`[data-role="stream-element"][data-element-id="${cd}"] [data-role="stream-element-select"]`)
    .click();
  await page.waitForSelector('[data-role="stream-prop-form"]', { timeout: 10_000 });
  await page.locator('[data-role="stream-frame-x"]').fill("12.5");
  const tsCd = '[data-role="stream-ts-countdown"] ';
  await page.locator(`${tsCd}[data-role="stream-ts-size"]`).fill("9.5");
  await setColorInput(page, `${tsCd}[data-role="stream-ts-color"]`, "#ff0000");
  await page.locator(`${tsCd}[data-role="stream-ts-align-left"]`).click();
  await page.locator(`${tsCd}[data-role="stream-ts-shadow-enable"]`).check();
  await page.locator(`${tsCd}[data-role="stream-ts-shadow-blur"]`).fill("6");
  await page.locator('[data-role="stream-prop-save"]').click();

  await expect
    .poll(async () => {
      const el = (await getScene(page, scene)).elements.find((e) => String(e.id) === cd);
      const p = el?.props as Record<string, any> | undefined;
      return (
        p &&
        p.frame.x_pct === 12.5 &&
        p.style.size_pct === 9.5 &&
        p.style.color === "#ff0000" &&
        p.style.align === "left" &&
        p.style.shadow &&
        p.style.shadow.blur_px === 6
      );
    })
    .toBeTruthy();

  // --- Invalid value (pct > 100) → inline 422, def unchanged ---
  await page.locator('[data-role="stream-frame-x"]').fill("150");
  await page.locator('[data-role="stream-prop-save"]').click();
  await expect(page.locator('[data-role="stream-prop-error"]')).toBeVisible({ timeout: 10_000 });
  {
    const el = (await getScene(page, scene)).elements.find((e) => String(e.id) === cd);
    expect((el?.props as any).frame.x_pct, "def unchanged after 422").toBe(12.5);
  }

  // --- z-order: move the last element (verse) up one; def order reflects it ---
  const orderBefore = (await getScene(page, scene)).elements.map((e) => String(e.id));
  expect(orderBefore).toEqual([img, cd, lyr, verse]);
  await page
    .locator(`[data-role="stream-element"][data-element-id="${verse}"] [data-role="stream-element-up"]`)
    .click();
  await expect
    .poll(async () => (await getScene(page, scene)).elements.map((e) => String(e.id)))
    .toEqual([img, cd, verse, lyr]);

  // --- Delete the image element via the native confirm ---
  page.once("dialog", (dialog) => dialog.accept());
  await page
    .locator(`[data-role="stream-element"][data-element-id="${img}"] [data-role="stream-element-delete"]`)
    .click();
  await expect
    .poll(async () => (await getScene(page, scene)).elements.some((e) => String(e.id) === img))
    .toBe(false);

  expect(errors, "browser console must be clean").toEqual([]);
});

test("a config change reflects live in a second editor context", async ({
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

  // Seed a base scene both contexts can open.
  const createRes = await pageA.request.post(
    new URL("/stream/api/outputs/stream/scenes", baseURL).toString(),
    { data: { name: "SE_LiveElems", kind: "base" }, timeout: 30_000 },
  );
  expect(createRes.ok()).toBeTruthy();
  const sceneId = String(((await createRes.json()) as SceneDef).id);

  await openEditor(pageA);
  await openEditor(pageB);
  await openPanel(pageA, sceneId);
  await openPanel(pageB, sceneId);

  await expect(
    pageB.locator('[data-role="stream-element-list"] [data-role="stream-element"]'),
  ).toHaveCount(0);

  // A adds an element → its PATCH bumps config_revision → B refetches the def
  // (StreamConfigChanged) and its list reflects the new element without reload.
  await addElement(pageA, sceneId, "lyrics");
  await expect(
    pageB.locator('[data-role="stream-element-list"] [data-role="stream-element"]'),
  ).toHaveCount(1, { timeout: 15_000 });

  expect(errorsA, "context A console clean").toEqual([]);
  expect(errorsB, "context B console clean").toEqual([]);

  await ctxA.close();
  await ctxB.close();
});
