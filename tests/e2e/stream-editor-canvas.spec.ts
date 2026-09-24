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

// ---- #787: no stuck drags, click != move, Escape cancels, output switch ----

/** Set + save the selected element's frame and wait for the saved def (so the
 *  overlay re-render from the save's refetch has settled before any click). */
async function saveFrame(
  page: Page,
  sceneId: string,
  id: string,
  x: number,
  y: number,
  w: number,
  h: number,
) {
  await setFrame(page, x, y, w, h);
  await page.locator(sel.save).click();
  await waitForSavedFrame(page, sceneId, id, x, y);
}

/** The frame position the fields currently show (the live draft). */
async function shownXY(page: Page): Promise<[number, number]> {
  return [
    Number(await page.locator(sel.frameX).inputValue()),
    Number(await page.locator(sel.frameY).inputValue()),
  ];
}

function listRow(page: Page, id: string) {
  return page.locator(`[data-role="stream-element"][data-element-id="${id}"]`);
}

/** A scene with one colour element at 10/10 20×20, saved + selected. */
async function sceneWithOneElement(page: Page, name: string) {
  const scene = await addScene(page, name, "base");
  await openPanel(page, scene);
  const el = await addElement(page, scene, "color"); // auto-selected
  await page.waitForSelector('[data-role="stream-prop-form"]', { timeout: 10_000 });
  await saveFrame(page, scene, el, 10, 10, 20, 20);
  await expect(overlayEl(page, el)).toHaveAttribute("data-selected", "true");
  return { scene, el };
}

async function centerOf(page: Page, id: string): Promise<[number, number]> {
  const b = (await overlayEl(page, id).boundingBox())!;
  return [b.x + b.width / 2, b.y + b.height / 2];
}

test("#787 a click with hand jitter keeps the frame; Escape mid-drag restores the start frame", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);
  const { el } = await sceneWithOneElement(page, "SC_787_Click");

  // A real-user click: press, a 2 px wobble, release. Below the drag threshold
  // → the element is selected but its frame does NOT change.
  const [cx, cy] = await centerOf(page, el);
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + 2, cy + 1);
  await page.mouse.up();
  await expect(overlayEl(page, el)).toHaveAttribute("data-selected", "true");
  expect(await shownXY(page)).toEqual([10, 10]);

  // Start a real drag, then press Escape while the button is still down: the
  // frame snaps back to where the drag started and the drag is over (further
  // moves with the button held do nothing).
  const canvas = (await page.locator(sel.overlay).boundingBox())!;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + canvas.width * 0.2, cy + canvas.height * 0.2, { steps: 8 });
  await expect.poll(async () => (await shownXY(page))[0]).toBeGreaterThan(12);
  await page.keyboard.press("Escape");
  await expect.poll(async () => shownXY(page)).toEqual([10, 10]);
  await page.mouse.move(cx + canvas.width * 0.3, cy + canvas.height * 0.3, { steps: 6 });
  await page.mouse.up();
  expect(await shownXY(page)).toEqual([10, 10]);
  // Escape during a drag cancels the drag only — the element stays selected.
  await expect(overlayEl(page, el)).toHaveAttribute("data-selected", "true");

  expect(errors, "console clean").toEqual([]);
});

test("#787 a drag ends when capture is lost + the button is released outside, and on window blur", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);
  const { el } = await sceneWithOneElement(page, "SC_787_Stuck");

  // Remember the pointer id of the last pointerdown (read it, don't assume 1).
  await page.evaluate(() => {
    document.addEventListener(
      "pointerdown",
      (e) => ((window as unknown as { __pid: number }).__pid = e.pointerId),
      true,
    );
  });

  const canvas = (await page.locator(sel.overlay).boundingBox())!;
  let [cx, cy] = await centerOf(page, el);

  // Drag, then lose pointer capture (what an iframe / alt-tab / OS gesture does)
  // and release the button OUTSIDE the overlay.
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + canvas.width * 0.1, cy, { steps: 6 });
  await expect.poll(async () => (await shownXY(page))[0]).toBeGreaterThan(11);
  await page.evaluate((overlaySel) => {
    const ov = document.querySelector(overlaySel) as HTMLElement;
    ov.releasePointerCapture((window as unknown as { __pid: number }).__pid);
  }, sel.overlay);
  await page.mouse.move(canvas.x + canvas.width / 2, canvas.y - 30, { steps: 4 });
  await page.mouse.up();
  const afterRelease = await shownXY(page);

  // Plain mouse moves back over the canvas (no button) must NOT move it.
  await page.mouse.move(canvas.x + canvas.width * 0.7, canvas.y + canvas.height * 0.7, {
    steps: 10,
  });
  await page.mouse.move(canvas.x + canvas.width * 0.2, canvas.y + canvas.height * 0.6, {
    steps: 10,
  });
  expect(await shownXY(page)).toEqual(afterRelease);

  // Window blur mid-drag (alt-tab) ends the drag too: moves with the button
  // still held after the blur do nothing.
  [cx, cy] = await centerOf(page, el);
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx, cy + canvas.height * 0.1, { steps: 6 });
  await expect
    .poll(async () => (await shownXY(page))[1])
    .toBeGreaterThan(afterRelease[1] + 1);
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  const afterBlur = await shownXY(page);
  await page.mouse.move(cx + canvas.width * 0.2, cy + canvas.height * 0.2, { steps: 8 });
  await page.mouse.up();
  expect(await shownXY(page)).toEqual(afterBlur);

  expect(errors, "console clean").toEqual([]);
});

test("#787 Escape with no drag deselects; a click on empty canvas deselects", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);
  const { el } = await sceneWithOneElement(page, "SC_787_Deselect");

  // Click the element (focuses the overlay), then Escape → nothing selected.
  const [cx, cy] = await centerOf(page, el);
  await page.mouse.click(cx, cy);
  await expect(overlayEl(page, el)).toHaveAttribute("data-selected", "true");
  await page.keyboard.press("Escape");
  await expect(overlayEl(page, el)).toHaveAttribute("data-selected", "false");
  await expect(listRow(page, el)).toHaveAttribute("data-selected", "false");
  await expect(page.locator('[data-role="stream-prop-form"]')).toHaveCount(0);

  // Select again, then click an EMPTY spot of the canvas → deselected.
  await page.mouse.click(cx, cy);
  await expect(overlayEl(page, el)).toHaveAttribute("data-selected", "true");
  const canvas = (await page.locator(sel.overlay).boundingBox())!;
  await page.mouse.click(canvas.x + canvas.width * 0.8, canvas.y + canvas.height * 0.8);
  await expect(overlayEl(page, el)).toHaveAttribute("data-selected", "false");
  await expect(listRow(page, el)).toHaveAttribute("data-selected", "false");

  expect(errors, "console clean").toEqual([]);
});

test("#787 a full-canvas element underneath does not steal clicks meant for the element above", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);

  const scene = await addScene(page, "SC_787_Z", "base");
  await openPanel(page, scene);
  // Background first (lowest z), then a small element above it.
  const bg = await addElement(page, scene, "color");
  await page.waitForSelector('[data-role="stream-prop-form"]', { timeout: 10_000 });
  await saveFrame(page, scene, bg, 0, 0, 100, 100);
  const top = await addElement(page, scene, "color");
  await saveFrame(page, scene, top, 40, 40, 20, 20);

  const canvas = (await page.locator(sel.overlay).boundingBox())!;
  const [tx, ty] = await centerOf(page, top);

  // Select the background by clicking beside the small element…
  await page.mouse.click(canvas.x + canvas.width * 0.1, canvas.y + canvas.height * 0.1);
  await expect(overlayEl(page, bg)).toHaveAttribute("data-selected", "true");
  // …then a click on the small element selects IT, even with the full-canvas
  // background selected underneath.
  await page.mouse.click(tx, ty);
  await expect(overlayEl(page, top)).toHaveAttribute("data-selected", "true");
  await expect(listRow(page, top)).toHaveAttribute("data-selected", "true");
  await expect(overlayEl(page, bg)).toHaveAttribute("data-selected", "false");
  expect(await shownXY(page)).toEqual([40, 40]);

  // Neither click moved anything.
  const saved = (await getScene(page, scene)).elements;
  const frameOf = (id: string) =>
    (saved.find((e) => String(e.id) === id)!.props.frame as { xPct: number; yPct: number });
  expect([frameOf(bg).xPct, frameOf(bg).yPct]).toEqual([0, 0]);
  expect([frameOf(top).xPct, frameOf(top).yPct]).toEqual([40, 40]);

  expect(errors, "console clean").toEqual([]);
});

test("#787 switching output shows loading, never the previous output's scenes", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await openEditor(page);
  const oldScene = await addScene(page, "SC_787_OldOutput", "base");

  // The timer output's scene id (read via the API request context, which the
  // page route below does not intercept).
  const timerRes = await page.request.get(
    new URL("/stream/api/outputs/timer/def", baseURL).toString(),
  );
  expect(timerRes.ok()).toBeTruthy();
  const timerScene = String(((await timerRes.json()) as OutputDef).scenes[0].id);

  // Hold the page's timer-def response until the test releases it, so the
  // in-between state is observable deterministically.
  let release: () => void = () => {};
  const gate = new Promise<void>((resolve) => (release = resolve));
  await page.route("**/stream/api/outputs/timer/def", async (route) => {
    await gate;
    await route.continue();
  });

  await page.locator('[data-role="stream-output-select"]').selectOption("timer");

  // While the new def is loading: a loading state, and NOT the old scenes.
  await expect(page.locator('[data-role="stream-loading"]')).toBeVisible({ timeout: 10_000 });
  await expect(page.locator(`${sel.scene}[data-scene-id="${oldScene}"]`)).toHaveCount(0);

  release();

  // The timer output's own scene appears; the old output's scene stays gone.
  await expect(page.locator(`${sel.scene}[data-scene-id="${timerScene}"]`)).toHaveCount(1, {
    timeout: 15_000,
  });
  await expect(page.locator(`${sel.scene}[data-scene-id="${oldScene}"]`)).toHaveCount(0);
  await expect(page.locator('[data-role="stream-loading"]')).toHaveCount(0);

  await page.unroute("**/stream/api/outputs/timer/def");
  expect(errors, "console clean").toEqual([]);
});
