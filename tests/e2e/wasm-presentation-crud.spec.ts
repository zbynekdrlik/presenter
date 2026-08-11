/**
 * WASM Operator Presentation CRUD Tests
 *
 * Tests presentation creation, editing, and deletion in the WASM operator.
 */

import path from "path";
import { test, expect } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  REPO_ROOT,
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

async function initPage(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="library-item"]', { timeout: 30_000 });
}

async function selectLibrary(page: import("@playwright/test").Page) {
  await initPage(page);
  await page.locator('[data-role="library-item"]').first().click();
  await page.waitForSelector('[data-role="presentation-list"]', {
    timeout: 15_000,
  });
}

// #641: the double-submit-guard tests below need a library GUARANTEED to
// start empty (the sidebar's "first favorite" library can already contain a
// same-named presentation from seeded dev data — the import fixture always
// produces a fixed name, see below). Create one via the API and select it
// through the "Show all libraries" modal — fresh libraries are never
// favorited, so they never show in the sidebar (#570).
type TestLibrary = { id: string; name: string };

async function createLibrary(
  request: import("@playwright/test").APIRequestContext,
  name: string,
): Promise<TestLibrary> {
  const resp = await request.post(new URL("/libraries", baseURL).toString(), {
    data: { name },
  });
  expect(resp.ok()).toBeTruthy();
  return resp.json();
}

async function openLibrary(
  page: import("@playwright/test").Page,
  libraryId: string,
) {
  await initPage(page);
  await page.locator('[data-role="library-more"]').click();
  await page
    .locator(
      `[data-role="library-row"][data-library-id="${libraryId}"] .operator__list-button`,
    )
    .click();
  await page.waitForSelector('[data-role="presentation-list"]', {
    timeout: 15_000,
  });
}

const IMPORT_FIXTURE = path.join(
  REPO_ROOT,
  "tests",
  "e2e",
  "fixtures",
  "test-import.pro",
);

test.describe("WASM Operator Presentation CRUD", () => {
  test("create blank presentation", async ({ page }) => {
    await selectLibrary(page);

    // Open create modal
    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Fill in name
    const nameInput = page.locator('[data-role="presentation-create-name"]');
    await nameInput.fill("E2E Test Presentation");

    // Click blank option
    await page.locator('[data-role="presentation-create-blank"]').click();

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Toast should show success
    await expect(page.locator('[data-role="toast"]')).toContainText(
      /created|success/i,
      { timeout: 5_000 },
    );
  });

  test("create presentation from paste", async ({ page }) => {
    await selectLibrary(page);

    // Open create modal
    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Fill in name
    await page
      .locator('[data-role="presentation-create-name"]')
      .fill("Pasted Presentation");

    // Click paste option
    await page.locator('[data-role="presentation-create-paste"]').click();

    // Wait for paste area
    const pasteArea = page.locator(
      '[data-role="presentation-create-paste-area"]',
    );
    await expect(pasteArea).toBeVisible();

    // Fill in paste text with verse markers
    const pasteText = `Verse 1
Line one of verse
Line two of verse

Chorus
This is the chorus
Multiple lines here`;

    await page
      .locator('[data-role="presentation-create-paste-text"]')
      .fill(pasteText);

    // Click confirm
    await page
      .locator('[data-role="presentation-create-paste-confirm"]')
      .click();

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Toast should show success
    await expect(page.locator('[data-role="toast"]')).toContainText(
      /created|paste|success/i,
      { timeout: 5_000 },
    );
  });

  test("rename presentation", async ({ page }) => {
    await selectLibrary(page);

    // Wait for presentations to load
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });

    // Switch to edit mode (rename buttons only visible in edit mode)
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Find rename button (uses data-action)
    const renameButton = page
      .locator('[data-action="presentation-rename"]')
      .first();
    const renameCount = await renameButton.count();
    expect(renameCount, "No presentation rename button found").toBeGreaterThan(
      0,
    );
    if (renameCount === 0) return;

    await renameButton.click();

    // Wait for edit modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Get current name
    const nameInput = page.locator('[data-role="presentation-edit-name"]');
    const originalName = await nameInput.inputValue();

    // Change name
    await nameInput.fill(originalName + "_RENAMED");

    // Save
    await page.locator('[data-role="presentation-edit-save"]').click();

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Toast should show success
    await expect(page.locator('[data-role="toast"]')).toContainText(
      /renamed|saved|success/i,
      { timeout: 5_000 },
    );

    // Restore original name
    await renameButton.click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
    await nameInput.fill(originalName);
    await page.locator('[data-role="presentation-edit-save"]').click();
  });

  test("delete presentation with confirmation", async ({ page }) => {
    await selectLibrary(page);

    // First create a test presentation to delete
    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    await page
      .locator('[data-role="presentation-create-name"]')
      .fill("To Be Deleted");
    await page.locator('[data-role="presentation-create-blank"]').click();

    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    await page
      .waitForSelector("[data-slide-id]", { timeout: 5_000 })
      .catch(() => {});

    // Switch to edit mode (rename buttons only visible in edit mode)
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Find the rename button for the newly created presentation (to open edit modal)
    const renameButtons = page.locator('[data-action="presentation-rename"]');
    const renameCount = await renameButtons.count();
    expect(
      renameCount,
      "No rename buttons available for delete test",
    ).toBeGreaterThan(0);
    if (renameCount === 0) return;

    // Click the first one (most recently created is usually first)
    await renameButtons.first().click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Set up dialog handler
    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });

    // Click delete
    await page.locator('[data-role="presentation-edit-delete"]').click();

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Toast should show success
    await expect(page.locator('[data-role="toast"]')).toContainText(
      /deleted|success/i,
      { timeout: 5_000 },
    );
  });

  test("delete cancellation preserves presentation", async ({ page }) => {
    await selectLibrary(page);

    // Wait for presentations
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });

    // Switch to edit mode (rename buttons only visible in edit mode)
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    const renameButton = page
      .locator('[data-action="presentation-rename"]')
      .first();
    const renameCount = await renameButton.count();
    expect(
      renameCount,
      "No presentation found for cancel test",
    ).toBeGreaterThan(0);
    if (renameCount === 0) return;

    await renameButton.click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Get the presentation name
    const nameInput = page.locator('[data-role="presentation-edit-name"]');
    const originalName = await nameInput.inputValue();

    // Set up dialog handler to DISMISS
    page.once("dialog", async (dialog) => {
      await dialog.dismiss();
    });

    // Click delete
    await page.locator('[data-role="presentation-edit-delete"]').click();

    // Modal should remain open (dialog was dismissed)
    await page
      .waitForFunction(
        () =>
          !!document.querySelector(
            '[data-role="presentation-edit-modal"][data-open="true"]',
          ),
        { timeout: 5_000 },
      )
      .catch(() => {});
    const modalStillOpen = await page.evaluate(
      () =>
        !!document.querySelector(
          '[data-role="presentation-edit-modal"][data-open="true"]',
        ),
    );
    expect(modalStillOpen).toBe(true);

    // Close modal
    await page.keyboard.press("Escape");
  });

  test("presentation create modal: back navigation", async ({ page }) => {
    await selectLibrary(page);

    // Open create modal
    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Click paste
    await page.locator('[data-role="presentation-create-paste"]').click();

    // Paste area should be visible
    const pasteArea = page.locator(
      '[data-role="presentation-create-paste-area"]',
    );
    await expect(pasteArea).toBeVisible();

    // Click back
    await page.locator('[data-role="presentation-create-paste-back"]').click();

    // Options should be visible again
    const options = page.locator('[data-role="presentation-create-options"]');
    await expect(options).toBeVisible();

    // Click import
    await page.locator('[data-role="presentation-create-import"]').click();

    // Import area should be visible
    const importArea = page.locator(
      '[data-role="presentation-create-import-area"]',
    );
    await expect(importArea).toBeVisible();

    // Click back
    await page.locator('[data-role="presentation-create-import-back"]').click();

    // Options should be visible again
    await expect(options).toBeVisible();

    // Close modal
    await page.keyboard.press("Escape");
  });

  // #571: op.submitting is set by the create handlers but was never READ by
  // any button — nothing actually prevented a double-submit during the
  // network round-trip. A double-click on the non-idempotent "create blank"
  // action must yield exactly ONE presentation, never two.
  test("double-click create blank yields exactly one presentation (#571)", async ({
    page,
  }) => {
    await selectLibrary(page);

    const uniqueName = `DblClickBlank${Date.now()}`;

    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    await page.locator('[data-role="presentation-create-name"]').fill(uniqueName);

    // The second click must be swallowed by the re-entry guard AND the
    // disabled button — never create a second presentation.
    await page.locator('[data-role="presentation-create-blank"]').dblclick();

    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    await expect(
      page.locator('[data-role="presentation-item"]', { hasText: uniqueName }),
    ).toHaveCount(1, { timeout: 10_000 });
  });

  // #641: same #571 re-entry guard as "create blank" above, but on the
  // create-from-PASTE path (`on_paste_confirm`) — never covered by an E2E.
  // A double-click on "Create" must yield exactly ONE presentation.
  test("double-click create-from-paste yields exactly one presentation (#641)", async ({
    page,
    request,
  }) => {
    const consoleMessages: string[] = [];
    attachConsoleErrorCollector(page, consoleMessages);

    const lib = await createLibrary(request, `DblSubmitPasteLib${Date.now()}`);
    await openLibrary(page, lib.id);

    const uniqueName = `DblClickPaste${Date.now()}`;

    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    await page.locator('[data-role="presentation-create-paste"]').click();
    await expect(
      page.locator('[data-role="presentation-create-paste-area"]'),
    ).toBeVisible();

    // create-from-paste derives the presentation name from the pasted
    // text's "Title:" line, not the (hidden-on-this-step) name input.
    await page
      .locator('[data-role="presentation-create-paste-text"]')
      .fill(`Title: ${uniqueName}\nVerse 1\nLine one\nLine two`);

    // The second click must be swallowed by the re-entry guard AND the
    // disabled button — never create a second presentation.
    await page.locator('[data-role="presentation-create-paste-confirm"]').dblclick();

    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // DOM check: exactly one item in the currently-open library's list.
    await expect(
      page.locator('[data-role="presentation-item"]', { hasText: uniqueName }),
    ).toHaveCount(1, { timeout: 10_000 });

    // Server-side check: a double-submit race (guard removed) would have
    // created TWO presentations in this library.
    const summaryResp = await fetch(`${baseURL}/libraries/summary`);
    expect(summaryResp.ok).toBe(true);
    const summaries = (await summaryResp.json()) as Array<{
      id: string;
      presentations: Array<{ id: string; name: string }>;
    }>;
    const libSummary = summaries.find((s) => s.id === lib.id);
    expect(libSummary, `library ${lib.id} missing from /libraries/summary`).toBeTruthy();
    const created = (libSummary?.presentations ?? []).filter(
      (p) => p.name === uniqueName,
    );
    expect(created).toHaveLength(1);

    // Cleanup.
    for (const pres of created) {
      await fetch(`${baseURL}/presentations/${pres.id}`, { method: "DELETE" });
    }
    await fetch(`${baseURL}/libraries/${lib.id}`, { method: "DELETE" });

    expect(consoleMessages).toEqual([]);
  });

  // #641: same #571 re-entry guard as "create blank" above, but on the
  // create-from-IMPORT path (`on_import_confirm`) — never covered by an
  // E2E. A double-click on "Import" must yield exactly ONE presentation.
  test("double-click create-from-import yields exactly one presentation (#641)", async ({
    page,
    request,
  }) => {
    const consoleMessages: string[] = [];
    attachConsoleErrorCollector(page, consoleMessages);

    const lib = await createLibrary(request, `DblSubmitImportLib${Date.now()}`);
    await openLibrary(page, lib.id);

    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    await page.locator('[data-role="presentation-create-import"]').click();
    await expect(
      page.locator('[data-role="presentation-create-import-area"]'),
    ).toBeVisible();

    // create-from-import reads the presentation name from the .pro file
    // itself (there is no name field to fill).
    await page
      .locator('[data-role="presentation-create-import-file"]')
      .setInputFiles(IMPORT_FIXTURE);

    // The second click must be swallowed by the re-entry guard AND the
    // disabled button — never create a second presentation. Both clicks
    // re-read the SAME file input, so an unguarded double-click would
    // import the fixture twice.
    await page.locator('[data-role="presentation-create-import-confirm"]').dblclick();

    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // The fixture's internal name (from its protobuf `.pro` payload) is
    // fixed — this is what a real double-submit would duplicate.
    const importedName = "088 Alive with you";

    await expect(
      page.locator('[data-role="presentation-item"]', { hasText: importedName }),
    ).toHaveCount(1, { timeout: 10_000 });

    const summaryResp = await fetch(`${baseURL}/libraries/summary`);
    expect(summaryResp.ok).toBe(true);
    const summaries = (await summaryResp.json()) as Array<{
      id: string;
      presentations: Array<{ id: string; name: string }>;
    }>;
    const libSummary = summaries.find((s) => s.id === lib.id);
    expect(libSummary, `library ${lib.id} missing from /libraries/summary`).toBeTruthy();
    const created = (libSummary?.presentations ?? []).filter(
      (p) => p.name === importedName,
    );
    expect(created).toHaveLength(1);

    // Cleanup.
    for (const pres of created) {
      await fetch(`${baseURL}/presentations/${pres.id}`, { method: "DELETE" });
    }
    await fetch(`${baseURL}/libraries/${lib.id}`, { method: "DELETE" });

    expect(consoleMessages).toEqual([]);
  });

  // Regression guard for issue #275 follow-up: full pipeline on a
  // spevnik song export must produce the right name + correctly
  // chunked slides + empty bookends.
  test("paste pipeline produces named presentation with bookends (#275)", async ({
    page,
  }) => {
    await selectLibrary(page);

    // Open the create modal.
    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Click "Paste" to enter paste sub-screen.
    await page.locator('[data-role="presentation-create-paste"]').click();
    await expect(
      page.locator('[data-role="presentation-create-paste-area"]'),
    ).toBeVisible();

    // Name input must be hidden on the paste sub-screen.
    await expect(
      page.locator('[data-role="presentation-create-name"]'),
    ).toBeHidden();

    // Canonical spevnik song export — Title: populates the presentation
    // name (zero-padded), Misc 1 lines are filtered, each section gets
    // chunked to ≤ 2 lines with empty bookends.
    const userInput = `Title: 76 Arriba
Misc 1

Verse 1
Môj Boh je nekonečný
Má všetku moc a je večný
Jeho Duch vo mne je živý
A všetko pre mňa učiní

Pre-Chorus
Žehná ma žehná
Priazeñ nad priazeň
A padá to na mňa, padá na mňa
Žehná ma žehná
Priazeň nad priazeň
A tá ku mne prúdi, ku mne prúdi

Chorus
Jeden dva tri
Zakrič On je najlepší
Jeden dva tri
Ja som v ňom požehnaný
Jeden dva tri
Všetka sláva Jemu, v Ňom
Ja som tým, kým hovorí že som
Jeden dva tri

Verse 2
Ja vidím veci na nebi
Tie zmeny sú aj na zemi
Jeho Duch vo mne je živý
Nie je nič čo neučiní

Bridge
Ak Boh je za mňa,  kto je proti mne
Ja chválim Ho, ja chválim Ho
Misc 1`;

    await page
      .locator('[data-role="presentation-create-paste-text"]')
      .fill(userInput);
    await page
      .locator('[data-role="presentation-create-paste-confirm"]')
      .click();

    // Modal closes.
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Wait for the new presentation to render with all 15 slides.
    await page.waitForFunction(
      () => {
        const slides = document.querySelector(
          '[data-view-panel="worship"] [data-role="slides"]',
        );
        return (
          slides && slides.querySelectorAll("[data-slide-id]").length === 15
        );
      },
      { timeout: 15_000 },
    );

    // Read all slide group + main pairs. Use `innerText` for the main
    // so rendered <br> tags become real \n in the output (textContent
    // collapses line breaks, which would make multi-line slides look
    // like one giant line).
    const slidePairs = await page.evaluate(() => {
      const slides = Array.from(
        document.querySelectorAll(
          '[data-view-panel="worship"] [data-role="slides"] [data-slide-id]',
        ),
      );
      return slides.map((slide) => {
        const groupEl = slide.querySelector('[data-role="slide-group"]');
        const mainEl = slide.querySelector(
          '[data-field-display="main"]',
        ) as HTMLElement | null;
        return {
          group: (groupEl?.textContent || "").trim(),
          main: (mainEl?.innerText || "").trim(),
        };
      });
    });

    // 15 slides total — empty bookend + 13 lyric chunks + empty bookend.
    expect(slidePairs).toHaveLength(15);

    // Bookends are empty.
    expect(slidePairs[0].main).toBe("");
    expect(slidePairs[0].group).toBe("");
    expect(slidePairs[14].main).toBe("");
    expect(slidePairs[14].group).toBe("");

    // First chunk of each section carries the section header.
    expect(slidePairs[1].group).toBe("Verse 1");
    expect(slidePairs[3].group).toBe("Pre-Chorus");
    expect(slidePairs[6].group).toBe("Chorus");
    expect(slidePairs[10].group).toBe("Verse 2");
    expect(slidePairs[12].group).toBe("Bridge");

    // No slide should leak metadata.
    for (const slide of slidePairs) {
      expect(slide.main).not.toMatch(/Title:/);
      expect(slide.main).not.toMatch(/Misc 1/);
      expect(slide.main).not.toContain("^B");
    }

    // No content slide may contain a line longer than 32 characters
    // (the default line_limit). Bookends excluded (already asserted empty).
    for (let i = 1; i <= 13; i++) {
      const lines = slidePairs[i].main.split("\n");
      for (const line of lines) {
        // Use Array.from for accurate codepoint count of Slovak diacritics.
        const len = Array.from(line).length;
        expect(len, `slide ${i} line "${line}" is ${len} chars (over 32)`).toBeLessThanOrEqual(32);
      }
    }
  });
});
