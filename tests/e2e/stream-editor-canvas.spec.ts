import { test, expect, type Page } from "@playwright/test";
import {
  attachEditorConsoleCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// Canvas direct-manipulation for the stream editor (#777): the preview iframe
// renders the operator's UNSAVED draft (pushed via postMessage), and an
// interaction overlay drags/resizes the selected element with real pointer
// events. The default output `stream` is seeded by the migration.

let serverHandle: ServerHandle | undefined;
let baseURL: string;

type ElementDef = {
  id: number;
  zOrder: number;
  props: Record<string, unknown> & { kind: string };
};
type SceneDef = {
  id: number;
  name: string;
  kind: string;
  position: number;
  isActive: boolean;
  elements: ElementDef[];
};
type OutputDef = { configRevision: number; scenes: SceneDef[] };

const sel = {
  editor: '[data-role="stream-editor"]',
  addName: '[data-role="stream-add-name"]',
  addKind: '[data-role="stream-add-kind"]',
  addSubmit: '[data-role="stream-add-submit"]',
  scene: '[data-role="stream-scene"]',
  overlay: '[data-role="stream-canvas-overlay"]',
  frameX: '[data-role="stream-frame-x"]',
  frameY: '[data-role="stream-frame-y"]',
  frameW: '[data-role="stream-frame-w"]',
  save: '[data-role="stream-prop-save"]',
  previewFrame: '[data-role="stream-preview-frame"]',
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
  const res = await page.request.get(
    new URL("/stream/api/outputs/stream/def", baseURL).toString(),
    { timeout: 30_000 },
  );
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as OutputDef;
}

async function getScene(page: Page, sceneId: string): Promise<SceneDef> {
  const def = await getDef(page);
  const scene = def.scenes.find((s) => String(s.id) === sceneId);
  expect(scene, `scene ${sceneId} present`).toBeTruthy();
  return scene as SceneDef;
}

async function addScene(page: Page, name: string, kind: "base" | "overlay"): Promise<string> {
  await page.locator(sel.addName).fill(name);
  await page.locator(sel.addKind).selectOption(kind);
  await page.locator(sel.addSubmit).click();
  const card = page
    .locator(sel.scene)
    .filter({ has: page.getByText(name, { exact: true }) });
  await expect(card).toHaveCount(1, { timeout: 15_000 });
  const id = await card.getAttribute("data-scene-id");
  expect(id).toBeTruthy();
  return id as string;
}

async function openPanel(page: Page, sceneId: string) {
  await page
    .locator(`${sel.scene}[data-scene-id="${sceneId}"] [data-role="stream-scene-edit"]`)
    .click();
  await page.waitForSelector('[data-role="stream-element-panel"]', { timeout: 15_000 });
}

async function addElement(
  page: Page,
  sceneId: string,
  kind: "image" | "color" | "countdown" | "lyrics" | "verse",
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

/** Set the selected element's frame via the numeric fields (commit on input),
 * then blur so the fields leave "editing" mode and thereafter display the live
 * committed draft (which a canvas drag updates). */
async function setFrame(page: Page, x: number, y: number, w: number, h: number) {
  await page.locator(sel.frameX).fill(String(x));
  await page.locator(sel.frameY).fill(String(y));
  await page.locator(sel.frameW).fill(String(w));
  await page.locator('[data-role="stream-frame-h"]').fill(String(h));
  await page.locator('[data-role="stream-frame-h"]').blur();
}

/** Wait until the SAVED def carries the given frame for `id` — a save triggers a
 *  def refetch that re-renders the canvas overlay; clicking before it settles
 *  races that re-render (the selection can be re-seeded under the click). */
async function waitForSavedFrame(page: Page, sceneId: string, id: string, x: number, y: number) {
  await expect
    .poll(
      async () => {
        const scene = await getScene(page, sceneId);
        const el = scene.elements.find((e) => String(e.id) === id);
        const frame = (el?.props as { frame?: { xPct?: number; yPct?: number } } | undefined)?.frame;
        return frame ? [frame.xPct, frame.yPct] : null;
      },
      { timeout: 10_000 },
    )
    .toEqual([x, y]);
}

function overlayEl(page: Page, id: string) {
  return page.locator(`[data-role="stream-overlay-element"][data-element-id="${id}"]`);
}

function iframeColor(page: Page, id: string) {
  return page
    .frameLocator(sel.previewFrame)
    .locator(`[data-role="stream-element-color"][data-element-id="${id}"]`);
}

/** Drag from a start point by (dx,dy) px using real pointer events, in steps. */
async function dragBy(page: Page, sx: number, sy: number, dx: number, dy: number) {
  await page.mouse.move(sx, sy);
  await page.mouse.down();
  await page.mouse.move(sx + dx / 2, sy + dy / 2, { steps: 6 });
  await page.mouse.move(sx + dx, sy + dy, { steps: 6 });
  await page.mouse.up();
}

test("drag moves the element live in the iframe before save; fields update; save persists", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);

  const scene = await addScene(page, "SC_Drag", "base");
  await openPanel(page, scene);
  const c1 = await addElement(page, scene, "color"); // auto-selected
  await page.waitForSelector('[data-role="stream-prop-form"]', { timeout: 10_000 });

  // A small, well-inside rectangle with room to move.
  await setFrame(page, 10, 10, 20, 20);

  // The overlay outline exists and is selected; the iframe rendered the element.
  await expect(overlayEl(page, c1)).toHaveAttribute("data-selected", "true");
  await expect(iframeColor(page, c1)).toBeVisible({ timeout: 15_000 });

  const canvas = await page.locator(sel.overlay).boundingBox();
  expect(canvas).toBeTruthy();
  const beforeBox = await iframeColor(page, c1).boundingBox();
  expect(beforeBox).toBeTruthy();

  // Drag the outline body right + down by ~30% of the canvas.
  const outline = await overlayEl(page, c1).boundingBox();
  expect(outline).toBeTruthy();
  const cx = outline!.x + outline!.width / 2;
  const cy = outline!.y + outline!.height / 2;
  await dragBy(page, cx, cy, canvas!.width * 0.3, canvas!.height * 0.3);

  // BEFORE any save: the iframe element moved right+down, and the numeric fields
  // reflect the new position.
  await expect
    .poll(async () => (await iframeColor(page, c1).boundingBox())?.x ?? 0)
    .toBeGreaterThan(beforeBox!.x + 5);
  const shownX = Number(await page.locator(sel.frameX).inputValue());
  const shownY = Number(await page.locator(sel.frameY).inputValue());
  expect(shownX).toBeGreaterThan(10);
  expect(shownY).toBeGreaterThan(10);

  // Nothing persisted yet — the def still has the pre-drag frame.
  {
    const el = (await getScene(page, scene)).elements.find((e) => String(e.id) === c1);
    expect((el!.props as any).frame.xPct).toBe(10);
  }

  // Save, reload → the persisted frame equals what was shown (tol 0.2%).
  await page.locator(sel.save).click();
  await expect
    .poll(async () => {
      const el = (await getScene(page, scene)).elements.find((e) => String(e.id) === c1);
      return (el?.props as any)?.frame.xPct;
    })
    .toBeGreaterThan(10);
  const saved = (await getScene(page, scene)).elements.find((e) => String(e.id) === c1)!;
  expect(Math.abs((saved.props as any).frame.xPct - shownX)).toBeLessThanOrEqual(0.2);
  expect(Math.abs((saved.props as any).frame.yPct - shownY)).toBeLessThanOrEqual(0.2);

  expect(errors, "console clean").toEqual([]);
});

test("SE handle resizes; dragging past the edge clamps inside the canvas", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);

  const scene = await addScene(page, "SC_Resize", "base");
  await openPanel(page, scene);
  const c1 = await addElement(page, scene, "color");
  await page.waitForSelector('[data-role="stream-prop-form"]', { timeout: 10_000 });
  await setFrame(page, 20, 20, 20, 20);
  await expect(overlayEl(page, c1)).toHaveAttribute("data-selected", "true");

  const canvas = (await page.locator(sel.overlay).boundingBox())!;
  const widthBefore = Number(await page.locator(sel.frameW).inputValue());

  // Drag the SE handle down-right → width + height grow.
  const handle = overlayEl(page, c1).locator('[data-role="stream-overlay-handle"][data-handle="se"]');
  const hb = (await handle.boundingBox())!;
  await dragBy(page, hb.x + hb.width / 2, hb.y + hb.height / 2, canvas.width * 0.2, canvas.height * 0.2);
  await expect
    .poll(async () => Number(await page.locator(sel.frameW).inputValue()))
    .toBeGreaterThan(widthBefore + 5);

  // Drag the BODY far past the right edge → x clamps so x + w stays <= 100.
  const outline = (await overlayEl(page, c1).boundingBox())!;
  await dragBy(
    page,
    outline.x + outline.width / 2,
    outline.y + outline.height / 2,
    canvas.width * 2, // way past the edge
    0,
  );
  await page.waitForTimeout(150);
  const x = Number(await page.locator(sel.frameX).inputValue());
  const w = Number(await page.locator(sel.frameW).inputValue());
  expect(x + w).toBeLessThanOrEqual(100.01);

  expect(errors, "console clean").toEqual([]);
});

test("click-to-select syncs the list; overlay works on an overlay scene + last element", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);

  // An OVERLAY scene with two elements: first + last.
  const scene = await addScene(page, "SC_Overlay", "overlay");
  await openPanel(page, scene);
  const first = await addElement(page, scene, "color");
  await page.waitForSelector('[data-role="stream-prop-form"]', { timeout: 10_000 });
  await setFrame(page, 5, 5, 20, 20);
  await page.locator(sel.save).click();
  await waitForSavedFrame(page, scene, first, 5, 5);

  const last = await addElement(page, scene, "color"); // auto-selected (last)
  await setFrame(page, 60, 60, 20, 20);
  await page.locator(sel.save).click();
  await waitForSavedFrame(page, scene, last, 60, 60);

  // Click the FIRST element's outline on the canvas → the list selects it.
  const fb = (await overlayEl(page, first).boundingBox())!;
  await page.mouse.click(fb.x + fb.width / 2, fb.y + fb.height / 2);
  await expect(
    page.locator(`[data-role="stream-element"][data-element-id="${first}"]`),
  ).toHaveAttribute("data-selected", "true", { timeout: 10_000 });
  await expect(
    page.locator(`[data-role="stream-element"][data-element-id="${last}"]`),
  ).toHaveAttribute("data-selected", "false");
  await expect(overlayEl(page, first)).toHaveAttribute("data-selected", "true");

  // The first element is draggable on the canvas (overlay scene).
  const canvas = (await page.locator(sel.overlay).boundingBox())!;
  const outline = (await overlayEl(page, first).boundingBox())!;
  await dragBy(
    page,
    outline.x + outline.width / 2,
    outline.y + outline.height / 2,
    canvas.width * 0.2,
    0,
  );
  await expect
    .poll(async () => Number(await page.locator(sel.frameX).inputValue()))
    .toBeGreaterThan(5);

  expect(errors, "console clean").toEqual([]);
});
