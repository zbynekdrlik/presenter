/**
 * WASM Operator Slide Multi-Select + Copy/Cut/Paste (#554)
 *
 * The full spec matrix, as REAL interactions (checkbox clicks, Shift+click,
 * keyboard shortcuts, dispatched HTML5 drag chain) against a presentation
 * created fresh via the API so positions are deterministic. Never skips.
 */

import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

async function createPresentationWithSlides(
  request: any,
  count: number,
  libraryName: string,
): Promise<{ presentationId: string; slideIds: string[] }> {
  const libResp = await request.post(new URL("/libraries", baseURL).toString(), {
    data: { name: libraryName },
  });
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Multiselect test song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;

  let slideIds = presPayload.presentation.slides.map((s) => s.id);
  while (slideIds.length < count) {
    const insertResp = await request.post(
      new URL(`/presentations/${presentationId}/slides`, baseURL).toString(),
      { data: { position: null } },
    );
    expect(insertResp.ok()).toBeTruthy();
    const slides: Array<{ id: string }> = await insertResp.json();
    slideIds = slides.map((s) => s.id);
  }
  for (let i = 0; i < slideIds.length; i += 1) {
    const updateResp = await request.patch(
      new URL(
        `/presentations/${presentationId}/slides/${slideIds[i]}`,
        baseURL,
      ).toString(),
      { data: { main: `Slide ${i + 1}`, translation: "", stage: "" } },
    );
    expect(updateResp.ok()).toBeTruthy();
  }
  return { presentationId, slideIds };
}

async function openPresentationInEditMode(
  page: import("@playwright/test").Page,
  name: string,
) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  const searchInput = page.locator('[data-role="global-search-query"]');
  await searchInput.fill(name);
  const result = page
    .locator('[data-role="search-result-item"][data-kind="presentation"]')
    .first();
  await expect(result).toBeVisible({ timeout: 15_000 });
  await result.click();
  await page.waitForFunction(
    () =>
      (document
        .querySelector('[data-role="slides"]')
        ?.querySelectorAll("[data-slide-id]").length ?? 0) > 0,
    { timeout: 15_000 },
  );
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-mode") === "edit",
    { timeout: 5_000 },
  );
}

/** The main texts of the slide cards in DOM order (edit-mode textareas). */
async function mainTextsInOrder(page: import("@playwright/test").Page) {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll("[data-slide-id]")).map(
      (card) =>
        (card.querySelector('textarea[data-field="main"]') as HTMLTextAreaElement | null)
          ?.value ?? "",
    ),
  );
}

async function domSlideOrder(page: import("@playwright/test").Page) {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll("[data-slide-id]")).map((s) =>
      s.getAttribute("data-slide-id"),
    ),
  );
}

function checkboxFor(page: import("@playwright/test").Page, slideId: string) {
  return page.locator(
    `[data-slide-id="${slideId}"] [data-role="slide-select-checkbox"]`,
  );
}

function insertBar(page: import("@playwright/test").Page, gap: number) {
  return page.locator(`[data-role="slide-insert-bar"][data-insert-index="${gap}"]`);
}

/** Dispatch the real HTML5 drag chain from the clipboard block handle onto a gap bar. */
async function dragClipboardToGap(
  page: import("@playwright/test").Page,
  gap: number,
) {
  await page.evaluate((gapIndex) => {
    const handle = document.querySelector('[data-role="clipboard-drag"]');
    const bar = document.querySelector(
      `[data-role="slide-insert-bar"][data-insert-index="${gapIndex}"]`,
    );
    if (!handle || !bar) {
      throw new Error("clipboard drag handle or insertion bar not found");
    }
    if (handle.getAttribute("draggable") !== "true") {
      throw new Error('clipboard drag handle must be draggable="true"');
    }
    const dataTransfer = new DataTransfer();
    const opts = { bubbles: true, cancelable: true, dataTransfer };
    handle.dispatchEvent(new DragEvent("dragstart", opts));
    const dragoverWasPrevented = !bar.dispatchEvent(new DragEvent("dragover", opts));
    if (!dragoverWasPrevented) {
      throw new Error(
        "insertion-bar gating regression: dragover was not preventDefault()'ed",
      );
    }
    bar.dispatchEvent(new DragEvent("drop", opts));
    handle.dispatchEvent(new DragEvent("dragend", opts));
  }, gap);
}

test.describe("WASM Operator Slide Multi-Select (#554)", () => {
  test("checkbox multi-select shows the count and marks the cards", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel Count ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 4, lib);
    await openPresentationInEditMode(page, lib);

    await checkboxFor(page, slideIds[0]).click();
    await checkboxFor(page, slideIds[2]).click();

    await expect(
      page.locator('[data-role="slide-selection-count"]'),
    ).toHaveText("2 selected");
    await expect(page.locator(`[data-slide-id="${slideIds[0]}"]`)).toHaveClass(
      /operator__slide-card--selected/,
    );
    await expect(page.locator(`[data-slide-id="${slideIds[2]}"]`)).toHaveClass(
      /operator__slide-card--selected/,
    );
  });

  test("Shift+click selects the inclusive range from the anchor", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel Range ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 5, lib);
    await openPresentationInEditMode(page, lib);

    await checkboxFor(page, slideIds[0]).click();
    await checkboxFor(page, slideIds[3]).click({ modifiers: ["Shift"] });

    await expect(
      page.locator('[data-role="slide-selection-count"]'),
    ).toHaveText("4 selected");
  });

  test("copy + paste at start, a true middle gap, and the end", async ({
    page,
    request,
  }) => {
    const consoleIssues: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleIssues.push(`${msg.type()}: ${msg.text()}`);
      }
    });

    const lib = `E2E MSel CopyPaste ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 4, lib);
    await openPresentationInEditMode(page, lib);

    // Copy slide 4 (distinct text), paste ABOVE FIRST (gap 0).
    await checkboxFor(page, slideIds[3]).click();
    await page.locator('[data-role="slide-copy"]').click();
    await expect(insertBar(page, 0)).toBeVisible();
    await insertBar(page, 0).click();
    await expect
      .poll(() => mainTextsInOrder(page), { timeout: 10_000 })
      .toEqual(["Slide 4", "Slide 1", "Slide 2", "Slide 3", "Slide 4"]);

    // Paste again at a TRUE MIDDLE gap (gap 2 of the now-5-slide list).
    await insertBar(page, 2).click();
    await expect
      .poll(() => mainTextsInOrder(page), { timeout: 10_000 })
      .toEqual(["Slide 4", "Slide 1", "Slide 4", "Slide 2", "Slide 3", "Slide 4"]);

    // Paste at the END (trailing bar = index len).
    await insertBar(page, 6).click();
    await expect
      .poll(() => mainTextsInOrder(page), { timeout: 10_000 })
      .toEqual([
        "Slide 4",
        "Slide 1",
        "Slide 4",
        "Slide 2",
        "Slide 3",
        "Slide 4",
        "Slide 4",
      ]);

    expect(consoleIssues, `console issues: ${consoleIssues.join(" | ")}`).toEqual([]);
  });

  test("cut + paste moves the block, count unchanged, persists after reload", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel CutPaste ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 5, lib);
    await openPresentationInEditMode(page, lib);

    // Cut slides 1+2, paste at the end.
    await checkboxFor(page, slideIds[0]).click();
    await checkboxFor(page, slideIds[1]).click();
    await page.locator('[data-role="slide-cut"]').click();
    await expect(page.locator(`[data-slide-id="${slideIds[0]}"]`)).toHaveClass(
      /operator__slide-card--cut/,
    );
    await expect(page.locator(`[data-slide-id="${slideIds[1]}"]`)).toHaveClass(
      /operator__slide-card--cut/,
    );

    await insertBar(page, 5).click();
    await expect
      .poll(() => domSlideOrder(page), { timeout: 10_000 })
      .toEqual([slideIds[2], slideIds[3], slideIds[4], slideIds[0], slideIds[1]]);
    // Count unchanged — a cut MOVES, never duplicates or deletes.
    expect((await domSlideOrder(page)).length).toBe(5);

    // Persistence: reload + reopen shows the same order.
    await page.reload();
    await openPresentationInEditMode(page, lib);
    expect(await domSlideOrder(page)).toEqual([
      slideIds[2],
      slideIds[3],
      slideIds[4],
      slideIds[0],
      slideIds[1],
    ]);
  });

  test("cut then Escape abandons the cut without moving anything", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel CutEscape ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 3, lib);
    await openPresentationInEditMode(page, lib);

    await checkboxFor(page, slideIds[1]).click();
    await page.locator('[data-role="slide-cut"]').click();
    await expect(page.locator(`[data-slide-id="${slideIds[1]}"]`)).toHaveClass(
      /operator__slide-card--cut/,
    );

    await page.keyboard.press("Escape");
    await expect(page.locator(".operator__slide-card--cut")).toHaveCount(0);
    expect(await domSlideOrder(page)).toEqual(slideIds);
  });

  test("Ctrl+C / Ctrl+X / Ctrl+V shortcuts drive the clipboard", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel Shortcuts ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 4, lib);
    await openPresentationInEditMode(page, lib);

    // Ctrl+C right after a checkbox select (focus is ON the checkbox — a
    // checkbox must NOT suppress the shortcut).
    await checkboxFor(page, slideIds[3]).click();
    await page.keyboard.press("Control+c");
    await expect(insertBar(page, 0)).toBeVisible({ timeout: 5_000 });

    // Hover a bar to set the paste target, then Ctrl+V pastes there.
    await insertBar(page, 0).hover();
    const pasteResponse = page.waitForResponse(
      (r) => r.url().includes("/slides/paste") && r.request().method() === "POST",
    );
    await page.keyboard.press("Control+v");
    expect((await pasteResponse).status()).toBe(200);
    await expect
      .poll(() => mainTextsInOrder(page), { timeout: 10_000 })
      .toEqual(["Slide 4", "Slide 1", "Slide 2", "Slide 3", "Slide 4"]);

    // Now a cut via Ctrl+X + Ctrl+V: move the (new) first slide to the end.
    await page.keyboard.press("Escape"); // clear previous clipboard+selection
    await expect(page.locator('[data-role="slide-insert-bar"]')).toHaveCount(0);
    const order = await domSlideOrder(page);
    await checkboxFor(page, order[0]!).click();
    await page.keyboard.press("Control+x");
    await expect(page.locator(".operator__slide-card--cut")).toHaveCount(1);
    await insertBar(page, 5).hover();
    const reorderResponse = page.waitForResponse(
      (r) => r.url().includes("/slides/reorder") && r.request().method() === "POST",
    );
    await page.keyboard.press("Control+v");
    expect((await reorderResponse).status()).toBe(200);
    await expect
      .poll(() => domSlideOrder(page), { timeout: 10_000 })
      .toEqual([order[1], order[2], order[3], order[4], order[0]]);
  });

  test("shortcuts are inert while typing in a slide textarea", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel InertTyping ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 3, lib);
    await openPresentationInEditMode(page, lib);

    // Select a slide so a copy WOULD have something to act on.
    await checkboxFor(page, slideIds[0]).click();

    // Focus a main textarea and type — Ctrl+C must NOT set the clipboard.
    const textarea = page.locator(
      `[data-slide-id="${slideIds[1]}"] textarea[data-field="main"]`,
    );
    await textarea.click();
    await textarea.press("Control+c");
    await expect(page.locator('[data-role="slide-insert-bar"]')).toHaveCount(0);
    expect((await domSlideOrder(page)).length).toBe(3);
  });

  test("paste into a single-slide presentation at gap 0", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel Single ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 1, lib);
    await openPresentationInEditMode(page, lib);

    await checkboxFor(page, slideIds[0]).click();
    await page.locator('[data-role="slide-copy"]').click();
    await insertBar(page, 0).click();
    await expect
      .poll(() => mainTextsInOrder(page), { timeout: 10_000 })
      .toEqual(["Slide 1", "Slide 1"]);
  });

  test("a failed paste (500) loses nothing and keeps the clipboard", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel Fail500 ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 3, lib);
    await openPresentationInEditMode(page, lib);

    await page.route("**/slides/paste", (route) =>
      route.fulfill({ status: 500, body: "boom" }),
    );

    await checkboxFor(page, slideIds[0]).click();
    await page.locator('[data-role="slide-copy"]').click();
    await insertBar(page, 3).click();

    // Toast appears; the list is unchanged; the clipboard is KEPT (bars stay).
    await expect(page.locator('[data-role="toast"]')).toBeVisible({
      timeout: 5_000,
    });
    expect(await domSlideOrder(page)).toEqual(slideIds);
    await expect(insertBar(page, 0)).toBeVisible();
  });

  test("a stale clipboard (422) clears the clipboard", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel Stale422 ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 3, lib);
    await openPresentationInEditMode(page, lib);

    await page.route("**/slides/paste", (route) =>
      route.fulfill({
        status: 422,
        contentType: "application/json",
        body: JSON.stringify({ message: "gone" }),
      }),
    );

    await checkboxFor(page, slideIds[0]).click();
    await page.locator('[data-role="slide-copy"]').click();
    await insertBar(page, 0).click();

    await expect(page.locator('[data-role="toast"]')).toBeVisible({
      timeout: 5_000,
    });
    // Clipboard cleared → the insertion bars disappear.
    await expect(page.locator('[data-role="slide-insert-bar"]')).toHaveCount(0);
    expect(await domSlideOrder(page)).toEqual(slideIds);
  });

  test("dragging the clipboard block onto a gap pastes there", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel BlockDrag ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 4, lib);
    await openPresentationInEditMode(page, lib);

    await checkboxFor(page, slideIds[3]).click();
    await page.locator('[data-role="slide-copy"]').click();
    await expect(page.locator('[data-role="clipboard-drag"]')).toBeVisible();

    const pasteResponse = page.waitForResponse(
      (r) => r.url().includes("/slides/paste") && r.request().method() === "POST",
    );
    await dragClipboardToGap(page, 1);
    expect((await pasteResponse).status()).toBe(200);
    await expect
      .poll(() => mainTextsInOrder(page), { timeout: 10_000 })
      .toEqual(["Slide 1", "Slide 4", "Slide 2", "Slide 3", "Slide 4"]);
  });
});
