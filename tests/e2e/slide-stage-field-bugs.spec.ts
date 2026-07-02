import { test, expect, Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

/**
 * #515 follow-up — two bugs the user found while testing the new per-slide
 * stage field:
 *
 * (a) EXTREME bug: editing a slide's stage field and leaving it (blur) made
 *     the text disappear from the edit field again, only to reappear once
 *     the slide was triggered. Root cause: `select_presentation` (and the
 *     page-load session restore) re-fetches the whole presentation every
 *     time it's opened. If that fetch is still in flight when a stage-field
 *     save lands — very plausible right after opening a song and
 *     immediately typing a hand-off message for the speaker — the late GET
 *     response overwrote the just-saved edit with pre-edit (empty) content.
 * (b) The stage field is free-form speaker/reading text and must NEVER be
 *     flagged by the lyrics per-line character-limit warning, unlike
 *     main/translation.
 */

test.describe.configure({ timeout: 180_000 });

const ALLOWED_CONSOLE_NOISE = [
  /integrity.*ignored.*preload/i,
  /ResizeObserver loop/i,
];

function collectConsoleErrors(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!ALLOWED_CONSOLE_NOISE.some((pattern) => pattern.test(text))) {
        messages.push(`[${msg.type()}] ${text}`);
      }
    }
  });
  return messages;
}

let server: ServerHandle | undefined;
let baseURL = "";

const LIBRARY_NAME = "_E2E Stage Field Bugs";
let presentationId = "";
let slideIds: string[] = [];

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);

  const libResp = await fetch(new URL("/libraries", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: LIBRARY_NAME }),
  });
  const lib = await libResp.json();

  const presResp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "Stage Field Cases",
        slides: [
          // Slide 0: race-condition case — stage starts EMPTY so the test
          // can prove a fresh edit survives.
          { main: "Slide one main" },
          // Slide 1: warning case — stage seeded non-empty so its preview
          // div is mounted from the first render (its mount is gated on
          // the PERSISTED value being non-empty, not on live typing).
          { main: "Slide two main", stage: "Stage seed" },
        ],
      }),
    },
  );
  const presData = await presResp.json();
  presentationId = presData.presentation.id;
  slideIds = presData.presentation.slides.map((s: { id: string }) => s.id);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

async function openOperatorEditMode(page: Page) {
  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('[data-role="library-more"]', {
    timeout: 30_000,
  });
  await page.locator('[data-role="library-more"]').click();
  await page
    .locator('[data-role="library-row"]', { hasText: LIBRARY_NAME })
    .locator("button.operator__list-button")
    .click();
  await page
    .locator('[data-role="presentation-item"]', { hasText: "Stage Field Cases" })
    .first()
    .click();
  await page.waitForSelector(`[data-slide-id="${slideIds[0]}"]`, {
    timeout: 30_000,
  });
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-mode") === "edit",
  );
}

test("editing the stage field survives a concurrent presentation refetch (#515)", async ({
  page,
}) => {
  const consoleErrors = collectConsoleErrors(page);
  await openOperatorEditMode(page);
  const slideId = slideIds[0];

  // Re-select the (already open) presentation to fire a fresh
  // `get_presentation` fetch, and — in the SAME synchronous script, before
  // that fetch's network round-trip can resolve — type into the stage
  // field and blur it. This deterministically reproduces the real-world
  // race (open a song, immediately type a stage/hand-off message) without
  // depending on real network timing: the whole script below runs to
  // completion before the fetch promise's microtask can fire.
  const NEW_TEXT = "Race-safe stage text";
  const result = await page.evaluate(
    ({ presentationId, slideId, newText }) => {
      const item = document.querySelector(
        `[data-presentation-id="${presentationId}"]`,
      ) as HTMLElement | null;
      if (!item) {
        return { presentationItemFound: false };
      }
      item.click();

      const ta = document.querySelector(
        `[data-slide-id="${slideId}"] [data-field="stage"]`,
      ) as HTMLTextAreaElement | null;
      if (!ta) {
        return { presentationItemFound: true, stageFieldFound: false };
      }
      const nativeSetter = Object.getOwnPropertyDescriptor(
        window.HTMLTextAreaElement.prototype,
        "value",
      )!.set!;
      nativeSetter.call(ta, newText);
      ta.dispatchEvent(new Event("input", { bubbles: true }));
      ta.focus();
      ta.blur();
      return {
        presentationItemFound: true,
        stageFieldFound: true,
        valueRightAfterBlur: ta.value,
      };
    },
    { presentationId, slideId, newText: NEW_TEXT },
  );
  expect(result.presentationItemFound).toBe(true);
  expect(result.stageFieldFound).toBe(true);
  expect(result.valueRightAfterBlur).toBe(NEW_TEXT);

  // Give the (now-stale) refetch triggered by the re-select time to
  // resolve. Before the fix, its response landed here and blanked the
  // field back out to the pre-edit (empty) value.
  await page.waitForTimeout(1500);

  const stageField = page.locator(
    `[data-slide-id="${slideId}"] [data-field="stage"]`,
  );
  await expect(stageField).toHaveValue(NEW_TEXT);

  // The server has the correct value too, and the edit was never lost —
  // only the CLIENT display was at risk of reverting.
  const resp = await page.request.get(
    new URL(`/presentations/${presentationId}`, baseURL).toString(),
  );
  const detail = await resp.json();
  const slide = detail.presentation.slides.find(
    (s: { id: string }) => s.id === slideId,
  );
  expect(slide.content.stage.value).toBe(NEW_TEXT);

  expect(consoleErrors).toEqual([]);
});

test("stage field text over the line limit never warns; main/translation still do (#515)", async ({
  page,
}) => {
  const consoleErrors = collectConsoleErrors(page);
  await openOperatorEditMode(page);
  // Slide 1 (own slide, independent of the race test's slide 0) — seeded
  // with a short, non-empty stage value so its preview div is mounted from
  // the start (mounting is gated on the PERSISTED value being non-empty,
  // not on live typing — see the fixture setup above).
  const slideId = slideIds[1];
  const overLimitText = "a".repeat(40); // default operator line limit is 32

  // The preview div is present in the DOM but CSS-hidden while its own
  // textarea is the visually active editor in edit mode — assert its
  // attribute directly rather than requiring visibility.
  const stagePreview = page.locator(
    `[data-slide-id="${slideId}"] [data-field-display="stage"]`,
  );
  await expect(stagePreview).toHaveCount(1);

  const stageField = page.locator(
    `[data-slide-id="${slideId}"] [data-field="stage"]`,
  );
  await stageField.click();
  await stageField.fill(overLimitText);

  const warningBanner = page.locator(
    `[data-slide-id="${slideId}"] [data-role="slide-warning"]`,
  );
  await expect(stagePreview).toHaveAttribute("data-warning", "false");
  await expect(warningBanner).toHaveAttribute("data-visible", "false");
  await expect(
    page.locator(`[data-slide-id="${slideId}"] .operator__slide-index sup`),
  ).toHaveCount(0);

  // Sanity: the same long text on MAIN (on the SAME slide) still warns —
  // bug-2's fix is stage-field-specific, not a global disabling of the
  // warning feature.
  const mainField = page.locator(
    `[data-slide-id="${slideId}"] [data-field="main"]`,
  );
  await mainField.click();
  await mainField.fill(overLimitText);

  const mainPreview = page.locator(
    `[data-slide-id="${slideId}"] [data-field-display="main"]`,
  );
  await expect(mainPreview).toHaveAttribute("data-warning", "true");
  await expect(warningBanner).toHaveAttribute("data-visible", "true");

  expect(consoleErrors).toEqual([]);
});
