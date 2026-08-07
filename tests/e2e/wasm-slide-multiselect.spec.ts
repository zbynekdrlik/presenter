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

/** Switch to a DIFFERENT presentation via search, staying on the same page
 * (already in edit mode) — used to simulate the operator switching songs
 * mid-request (#558 V4). */
async function switchToPresentation(
  page: import("@playwright/test").Page,
  name: string,
  firstExpectedSlideId: string,
) {
  const searchInput = page.locator('[data-role="global-search-query"]');
  await searchInput.fill(name);
  const result = page
    .locator('[data-role="search-result-item"][data-kind="presentation"]')
    .first();
  await expect(result).toBeVisible({ timeout: 15_000 });
  await result.click();
  await page.waitForFunction(
    (slideId) => document.querySelector(`[data-slide-id="${slideId}"]`) !== null,
    firstExpectedSlideId,
    { timeout: 15_000 },
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

/** Number of tracks the `[data-role="slides"]` grid currently renders — the
 * browser expands `grid-template-columns` to one pixel value per track, so
 * splitting the computed value on whitespace gives the column count (#602
 * defect 3). `> 1` = normal multi-column grid, `1` = the single-column
 * "choosing a paste target" layout. */
async function gridColumnCount(page: import("@playwright/test").Page) {
  return page.locator('[data-role="slides"]').evaluate((el) => {
    const cols = window.getComputedStyle(el).gridTemplateColumns.trim();
    return cols.length === 0 ? 0 : cols.split(/\s+/).length;
  });
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

  test("a stale paste response never clobbers a different presentation now open (#558 V4)", async ({
    page,
    request,
  }) => {
    const libA = `E2E MSel RaceA ${Date.now()}`;
    const { slideIds: idsA } = await createPresentationWithSlides(request, 2, libA);
    const libB = `E2E MSel RaceB ${Date.now()}`;
    const { slideIds: idsB } = await createPresentationWithSlides(request, 2, libB);

    await openPresentationInEditMode(page, libA);
    await checkboxFor(page, idsA[0]).click();
    await page.locator('[data-role="slide-copy"]').click();

    // Delay A's paste response so the test can switch to B before it lands.
    // Perform the REAL request ourselves (server truth) but hold back
    // DELIVERING it to the page until `released` resolves — decouples "wait
    // for the network" from "hand the response to the page", so this
    // doesn't race the underlying connection the way delaying
    // `route.continue()` itself can. NOTE: deliberately no `page.unroute()`
    // afterwards — calling it right after `releaseA()` races the handler's
    // own still-in-flight `route.fulfill()` and throws "Route is already
    // handled"; nothing else in this test hits `/slides/paste` again, so the
    // route can simply stay registered.
    let releaseA: () => void = () => {};
    const released = new Promise<void>((resolve) => {
      releaseA = resolve;
    });
    const pasteResponse = page.waitForResponse(
      (r) => r.url().includes("/slides/paste") && r.request().method() === "POST",
    );
    await page.route("**/slides/paste", async (route) => {
      const response = await request.fetch(route.request());
      const body = await response.body();
      await released;
      await route.fulfill({
        status: response.status(),
        headers: response.headers(),
        body,
      });
    });

    await insertBar(page, 0).click(); // fires A's (now-delayed) paste

    // Switch to presentation B WHILE A's paste is still in flight.
    await switchToPresentation(page, libB, idsB[0]);

    // Let A's delayed response land, and wait for the BROWSER to actually
    // receive it before asserting. `expect.poll` alone is the WRONG tool
    // here: it passes on the FIRST matching check, so if we polled starting
    // immediately after `releaseA()`, the very first (near-instant) check
    // could see B's still-uncorrupted state and pass BEFORE the response
    // was even processed — a false pass that proves nothing. Anchor on the
    // real network event instead, then assert once B is fully settled.
    releaseA();
    await pasteResponse;
    // Positive sentinel, NOT an arbitrary sleep (#558 W10): perform a
    // subsequent, unrelated, OBSERVABLE action on B and wait for ITS effect
    // to land. The browser drains all queued microtasks (including A's
    // already-resolved response continuation, whether or not it corrupts
    // anything) before dispatching a new UI input event, so by the time
    // this click's effect is visible, A's response has already finished
    // processing. This is NOT a poll-until-match on the negative condition
    // itself (which would pass on a lucky first check before any
    // corruption lands) — it anchors on a REAL, different event instead of
    // guessing a fixed duration.
    await checkboxFor(page, idsB[0]).click();
    await expect(
      page.locator('[data-role="slide-selection-count"]'),
    ).toHaveText("1 selected");

    // B's slide list must be COMPLETELY untouched by A's late paste result —
    // never A's returned slides painted onto B.
    expect(await domSlideOrder(page)).toEqual(idsB);
  });

  test("a 409 (vanished anchor) refreshes the presentation when it is still open (#558 W1 guarded case)", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel Conflict409 ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 3, lib);
    await openPresentationInEditMode(page, lib);

    await page.route("**/slides/paste", (route) =>
      route.fulfill({
        status: 409,
        contentType: "application/json",
        body: JSON.stringify({ message: "the paste position no longer exists" }),
      }),
    );

    await checkboxFor(page, slideIds[0]).click();
    await page.locator('[data-role="slide-copy"]').click();
    const detailRefetch = page.waitForResponse(
      (r) =>
        r.url().includes(`/presentations/`) &&
        !r.url().includes("/slides/") &&
        r.request().method() === "GET",
    );
    await insertBar(page, 0).click();

    // Guarded recovery (#558 W1): still the SAME presentation → the 409
    // handler refetches it fresh (a real GET round-trip) and applies it.
    await detailRefetch;
    await expect(page.locator('[data-role="toast"]')).toBeVisible({
      timeout: 5_000,
    });
    expect(await domSlideOrder(page)).toEqual(slideIds);
  });

  test("a delayed 409 never clobbers a different presentation now open (#558 W1 unguarded case)", async ({
    page,
    request,
  }) => {
    const libA = `E2E MSel Conflict409A ${Date.now()}`;
    const { slideIds: idsA } = await createPresentationWithSlides(request, 2, libA);
    const libB = `E2E MSel Conflict409B ${Date.now()}`;
    const { slideIds: idsB } = await createPresentationWithSlides(request, 2, libB);

    await openPresentationInEditMode(page, libA);
    await checkboxFor(page, idsA[0]).click();
    await page.locator('[data-role="slide-copy"]').click();

    // Hold A's 409 response open until this test explicitly releases it —
    // mirrors the V4 race test's technique above.
    let releaseA: () => void = () => {};
    const released = new Promise<void>((resolve) => {
      releaseA = resolve;
    });
    const pasteResponse = page.waitForResponse(
      (r) => r.url().includes("/slides/paste") && r.request().method() === "POST",
    );
    await page.route("**/slides/paste", async (route) => {
      await released;
      await route.fulfill({
        status: 409,
        contentType: "application/json",
        body: JSON.stringify({ message: "the paste position no longer exists" }),
      });
    });

    await insertBar(page, 0).click(); // fires A's (now-delayed) paste, which will 409

    // Switch to presentation B WHILE A's 409 is still in flight.
    await switchToPresentation(page, libB, idsB[0]);

    releaseA();
    await pasteResponse;
    // Positive sentinel, not an arbitrary sleep (#558 W10) — see the V4
    // race test above for the rationale.
    await checkboxFor(page, idsB[0]).click();
    await expect(
      page.locator('[data-role="slide-selection-count"]'),
    ).toHaveText("1 selected");

    // B must be COMPLETELY untouched — never A's stale 409 refetch (or any
    // recovery UI) painted onto B; and the clipboard recovery must have
    // been silent (no toast fired for a presentation nobody is looking at).
    // NOTE: the toast `<div data-role="toast">` is ALWAYS mounted, fading
    // via `opacity` only (not `display`/`visibility`) — see
    // `components/toast.rs` + `operator.css` — so neither `toHaveCount(0)`
    // nor Playwright's `toBeVisible()` (which ignores `opacity`) can detect
    // "no toast shown"; assert on the component's own `data-visible` flag.
    await expect(page.locator('[data-role="toast"]')).toHaveAttribute(
      "data-visible",
      "false",
    );
    expect(await domSlideOrder(page)).toEqual(idsB);
  });

  test("a 409 recovery GET in flight during a presentation switch is dropped, never applied to the wrong presentation (#558 X1)", async ({
    page,
    request,
  }) => {
    // #558 X1: the W1 guard above only covers the window BEFORE the
    // recovery GET fires. This test targets the SECOND window — the
    // recovery GET's OWN `.await` — by delaying the GET itself (not the
    // paste POST, which 409s immediately) and switching presentations
    // while that GET is held open.
    // NOTE: the differentiator letter is glued onto the preceding word
    // (`RecoverA`/`RecoverB`), NEVER a standalone space-separated token —
    // `query_tokens` splits on non-alphanumeric, and a lone single-char
    // token like "A" broadly OR-matches the letter 'a' against real
    // production library names (search.rs's `search_libraries` token match
    // is `Condition::any()`), flooding `matched_library_ids` and starving
    // the presentation query's result-page LIMIT before it ever reaches
    // this test's own presentation. Every other 409/race test above
    // follows this same glued-suffix convention for exactly this reason.
    const libA = `E2E MSel Conflict409RecoverA ${Date.now()}`;
    const { presentationId: presIdA, slideIds: idsA } =
      await createPresentationWithSlides(request, 3, libA);
    const libB = `E2E MSel Conflict409RecoverB ${Date.now()}`;
    const { slideIds: idsB } = await createPresentationWithSlides(request, 2, libB);

    await openPresentationInEditMode(page, libA);
    await checkboxFor(page, idsA[0]).click();
    await page.locator('[data-role="slide-copy"]').click();

    // The paste itself 409s immediately — the operator is still on A, so
    // the handler's FIRST `still_here` check passes and it fires the
    // recovery GET.
    await page.route("**/slides/paste", (route) =>
      route.fulfill({
        status: 409,
        contentType: "application/json",
        body: JSON.stringify({ message: "the paste position no longer exists" }),
      }),
    );

    // Hold the recovery GET's response open (a REAL server round-trip,
    // delayed DELIVERY only — same technique as the V4 race test above).
    // `recoveryStarted` fires the MOMENT the route handler begins running
    // (proving the GET is genuinely in flight server-side) so the test
    // never switches presentations before the recovery GET has actually
    // been dispatched — unlike the paste POST above, this GET only fires
    // AFTER an internal `.await` chain (the 409 response + the still_here
    // check), so it is never guaranteed in flight the instant the click
    // resolves.
    let recoveryStarted: () => void = () => {};
    const recoveryHasStarted = new Promise<void>((resolve) => {
      recoveryStarted = resolve;
    });
    let releaseRecovery: () => void = () => {};
    const recoveryReleased = new Promise<void>((resolve) => {
      releaseRecovery = resolve;
    });
    const recoveryResponse = page.waitForResponse(
      (r) =>
        r.url().endsWith(`/presentations/${presIdA}`) &&
        r.request().method() === "GET",
    );
    await page.route(`**/presentations/${presIdA}`, async (route) => {
      if (route.request().method() !== "GET") {
        await route.continue();
        return;
      }
      recoveryStarted();
      const response = await request.fetch(route.request());
      const body = await response.body();
      await recoveryReleased;
      await route.fulfill({
        status: response.status(),
        headers: response.headers(),
        body,
      });
    });

    await insertBar(page, 0).click(); // fires the paste -> 409 -> recovery GET
    await recoveryHasStarted;

    // Switch to presentation B WHILE the recovery GET is still held open —
    // the exact race #558 X1 fixes: `still_here` was true when the GET
    // fired, but by the time it resolves the operator has moved on.
    await switchToPresentation(page, libB, idsB[0]);

    releaseRecovery();
    await recoveryResponse;
    // Positive sentinel, not an arbitrary sleep (#558 W10) — see the V4
    // race test above for the rationale.
    await checkboxFor(page, idsB[0]).click();
    await expect(
      page.locator('[data-role="slide-selection-count"]'),
    ).toHaveText("1 selected");

    // B must be COMPLETELY untouched by A's stale recovery refetch, and no
    // misleading "refreshed" toast for a presentation nobody is looking at.
    expect(await domSlideOrder(page)).toEqual(idsB);
    await expect(page.locator('[data-role="toast"]')).toHaveAttribute(
      "data-visible",
      "false",
    );
  });

  test("clipboard shortcuts are inert on a non-worship view (#558 V5)", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel BibleView ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 2, lib);
    await openPresentationInEditMode(page, lib);

    await checkboxFor(page, slideIds[0]).click();

    // Switch to the Bible view — the worship editor panel stays MOUNTED
    // (CSS visibility toggle, not unmount/remount — see pages/operator.rs),
    // so the window keydown listener installed for it is still live.
    await page.locator('[data-role="view-toggle"][data-view="bible"]').click();
    await expect(page.locator('body[data-view="bible"]')).toBeAttached();

    await page.keyboard.press("Control+c");

    // Switch back to worship: if Ctrl+C had set the clipboard while on the
    // Bible view, insertion bars would now render (gated on a non-empty
    // clipboard).
    await page.locator('[data-role="view-toggle"][data-view="worship"]').click();
    await expect(page.locator('[data-role="slide-insert-bar"]')).toHaveCount(0);
  });

  test("Ctrl+V inside the stage-layout dropdown does not mutate the presentation (#558 V10)", async ({
    page,
    request,
  }) => {
    const lib = `E2E MSel StageSelect ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 2, lib);
    await openPresentationInEditMode(page, lib);

    await checkboxFor(page, slideIds[0]).click();
    await page.locator('[data-role="slide-copy"]').click();
    await expect(insertBar(page, 0)).toBeVisible();

    // The paste fires an ASYNC POST that only mutates the DOM once its
    // response lands — checking the DOM immediately after the keypress
    // would pass even on buggy code (the request just hasn't resolved
    // yet). Track whether the request fires at all instead.
    let pasteFired = false;
    page.on("request", (req) => {
      if (req.url().includes("/slides/paste") && req.method() === "POST") {
        pasteFired = true;
      }
    });

    const select = page.locator(
      `[data-slide-id="${slideIds[1]}"] [data-role="slide-stage-layout-select"]`,
    );
    await select.focus();
    await page.keyboard.press("Control+v");

    // Bounded settle: give a (buggy) paste request a chance to actually
    // fire before asserting its absence.
    await page.waitForTimeout(500);
    expect(pasteFired).toBe(false);
    expect(await domSlideOrder(page)).toEqual(slideIds);
  });

  // ---------------------------------------------------------------------
  // #602: checkbox visibility, copy discoverability, grid-stuck regression.
  // ---------------------------------------------------------------------

  test("selecting a slide gives the checkbox AND the card a visible checked appearance (#602 defect 1)", async ({
    page,
    request,
  }) => {
    const consoleIssues: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleIssues.push(`${msg.type()}: ${msg.text()}`);
      }
    });

    const lib = `E2E Sel602 Checked ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 2, lib);
    await openPresentationInEditMode(page, lib);

    const checkbox = checkboxFor(page, slideIds[0]);
    const otherCheckbox = checkboxFor(page, slideIds[1]);
    await expect(checkbox).not.toBeChecked();
    await expect(otherCheckbox).not.toBeChecked();

    const card = page.locator(`[data-slide-id="${slideIds[0]}"]`);
    const boxShadowBefore = await card.evaluate(
      (el) => window.getComputedStyle(el).boxShadow,
    );
    const outlineBefore = await checkbox.evaluate(
      (el) => window.getComputedStyle(el).outlineStyle,
    );

    await checkbox.click();

    // The checked state must reach the DOM (regression test would have
    // already caught a broken binding, but re-assert it here too)...
    await expect(checkbox).toBeChecked();
    await expect(otherCheckbox).not.toBeChecked();

    // ...AND it must be VISUALLY distinguishable — a computed style change
    // on the checkbox itself (defect 1's actual gap: no `:checked` rule
    // existed at all) and on the card.
    const outlineAfter = await checkbox.evaluate(
      (el) => window.getComputedStyle(el).outlineStyle,
    );
    expect(outlineAfter).not.toBe(outlineBefore);

    const boxShadowAfter = await card.evaluate(
      (el) => window.getComputedStyle(el).boxShadow,
    );
    expect(boxShadowAfter).not.toBe(boxShadowBefore);

    // The untouched sibling checkbox stays plain — the change is scoped to
    // the checked one, not a global style shift.
    const otherOutline = await otherCheckbox.evaluate(
      (el) => window.getComputedStyle(el).outlineStyle,
    );
    expect(otherOutline).toBe(outlineBefore);

    expect(consoleIssues, `console issues: ${consoleIssues.join(" | ")}`).toEqual([]);
  });

  test("clipboard action shortcuts are visible on screen the moment a selection exists (#602 defect 2)", async ({
    page,
    request,
  }) => {
    const consoleIssues: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleIssues.push(`${msg.type()}: ${msg.text()}`);
      }
    });

    const lib = `E2E Sel602 Discover ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 2, lib);
    await openPresentationInEditMode(page, lib);

    // No selection, no clipboard yet — the panel (and its shortcut hints)
    // is not shown at all.
    await expect(page.locator('[data-role="slide-selection-panel"]')).toHaveCount(0);

    await checkboxFor(page, slideIds[0]).click();

    const panel = page.locator('[data-role="slide-selection-panel"]');
    await expect(panel).toBeVisible();

    // The available actions and their keyboard shortcuts must be visible
    // ON SCREEN (not merely discoverable via a hover tooltip) the moment a
    // selection exists — ticket acceptance criterion 2.
    const shortcuts = panel.locator('[data-role="slide-selection-shortcut"]');
    await expect(shortcuts).toHaveCount(4);
    expect(await shortcuts.allTextContents()).toEqual([
      "Ctrl+C",
      "Ctrl+X",
      "Ctrl+V",
      "Esc",
    ]);
    for (const hint of await shortcuts.all()) {
      await expect(hint).toBeVisible();
    }

    expect(consoleIssues, `console issues: ${consoleIssues.join(" | ")}`).toEqual([]);
  });

  test("grid returns to multi-column after a completed Copy-paste — never stuck single-column (#602 defect 3)", async ({
    page,
    request,
  }) => {
    const consoleIssues: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleIssues.push(`${msg.type()}: ${msg.text()}`);
      }
    });

    const lib = `E2E Sel602 GridStuck ${Date.now()}`;
    const { slideIds } = await createPresentationWithSlides(request, 4, lib);
    await openPresentationInEditMode(page, lib);

    // Normal grid before anything is selected/copied.
    expect(await gridColumnCount(page)).toBeGreaterThan(1);

    await checkboxFor(page, slideIds[0]).click();
    await page.locator('[data-role="slide-copy"]').click();

    // Actively CHOOSING a paste target: single column, "Paste here" bars.
    await expect(insertBar(page, 0)).toBeVisible();
    expect(await gridColumnCount(page)).toBe(1);

    await insertBar(page, 0).click();
    await expect
      .poll(() => mainTextsInOrder(page), { timeout: 10_000 })
      .toEqual(["Slide 1", "Slide 1", "Slide 2", "Slide 3", "Slide 4"]);

    // #602 defect 3 — the actual regression: a completed Copy-paste used to
    // leave the clipboard (deliberately) non-empty forever, and the
    // single-column class was keyed on "a clipboard exists", so the grid
    // stayed stuck in one column until Clear/Escape/a page reload. The
    // completed paste must restore the normal multi-column grid...
    await expect
      .poll(() => gridColumnCount(page), { timeout: 10_000 })
      .toBeGreaterThan(1);
    await expect(page.locator('[data-role="slide-insert-bar"]')).toHaveCount(0);

    // ...WITHOUT discarding the retained Copy clipboard — the toolbar Paste
    // button must still be enabled for a re-paste with no re-copy needed.
    await expect(page.locator('[data-role="slide-paste"]')).toBeEnabled();

    expect(consoleIssues, `console issues: ${consoleIssues.join(" | ")}`).toEqual([]);
  });
});
