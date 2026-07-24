/**
 * #570: copy a presentation from one library to another.
 *
 * API layer: the copy is a deep, independent duplicate in the target library.
 * UI layer: the presentation-edit modal's "Copy to library" flow works end to
 * end in the WASM operator (select target, confirm, toast, list refresh).
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

type Library = { id: string; name: string };
type Detail = {
  libraryId: string;
  libraryName: string;
  presentation: {
    id: string;
    name: string;
    slides: Array<{ id: string; content: { main: { value: string } } }>;
  };
};

async function createLibrary(
  request: import("@playwright/test").APIRequestContext,
  name: string,
): Promise<Library> {
  const resp = await request.post(new URL("/libraries", baseURL).toString(), {
    data: { name },
  });
  expect(resp.ok()).toBeTruthy();
  return resp.json();
}

async function createSong(
  request: import("@playwright/test").APIRequestContext,
  libraryId: string,
  name: string,
): Promise<Detail["presentation"]> {
  const resp = await request.post(
    new URL(`/libraries/${libraryId}/presentations`, baseURL).toString(),
    {
      data: {
        name,
        slides: [{ main: "CopyVerseOne" }, { main: "CopyVerseTwo" }],
      },
    },
  );
  expect(resp.ok()).toBeTruthy();
  const payload: { presentation: Detail["presentation"] } = await resp.json();
  return payload.presentation;
}

test("API: copy is a deep, independent duplicate in the target library", async ({
  request,
}) => {
  const stamp = Date.now();
  const sourceLib = await createLibrary(request, `CopyApiSrc${stamp}`);
  const targetLib = await createLibrary(request, `CopyApiDst${stamp}`);
  const source = await createSong(request, sourceLib.id, `CopyApiSong${stamp}`);

  const copyResp = await request.post(
    new URL(`/presentations/${source.id}/copy`, baseURL).toString(),
    { data: { targetLibraryId: targetLib.id } },
  );
  expect(copyResp.status()).toBe(201);
  const copy: Detail = await copyResp.json();

  expect(copy.libraryId).toBe(targetLib.id);
  expect(copy.presentation.id).not.toBe(source.id);
  expect(copy.presentation.name).toBe(source.name);
  expect(copy.presentation.slides.map((s) => s.content.main.value)).toEqual([
    "CopyVerseOne",
    "CopyVerseTwo",
  ]);
  const sourceSlideIds = new Set(source.slides.map((s) => s.id));
  for (const slide of copy.presentation.slides) {
    expect(sourceSlideIds.has(slide.id)).toBeFalsy();
  }

  // Deleting the ORIGINAL must not touch the copy.
  const delResp = await request.delete(
    new URL(`/presentations/${source.id}`, baseURL).toString(),
  );
  expect(delResp.ok()).toBeTruthy();
  const detailResp = await request.get(
    new URL(`/presentations/${copy.presentation.id}`, baseURL).toString(),
  );
  expect(detailResp.ok()).toBeTruthy();

  // A vanished target library is a clean 404, not a 500.
  const badResp = await request.post(
    new URL(`/presentations/${copy.presentation.id}/copy`, baseURL).toString(),
    { data: { targetLibraryId: "11111111-2222-3333-4444-555555555555" } },
  );
  expect(badResp.status()).toBe(404);
});

test("UI: edit-modal copy flow lands the song in the target library", async ({
  page,
  request,
}) => {
  const stamp = Date.now();
  const sourceLib = await createLibrary(request, `CopyUiSrc${stamp}`);
  const targetLib = await createLibrary(request, `CopyUiDst${stamp}`);
  const source = await createSong(request, sourceLib.id, `CopyUiSong${stamp}`);

  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('[data-role="library-item"]', {
    timeout: 30_000,
  });

  // The sidebar shows only FAVORITE libraries — select a freshly created
  // library through the "Show all libraries" modal.
  const openLibrary = async (libraryId: string) => {
    await page.locator('[data-role="library-more"]').click();
    await page
      .locator(
        `[data-role="library-row"][data-library-id="${libraryId}"] .operator__list-button`,
      )
      .click();
    await page.waitForSelector('[data-role="presentation-list"]', {
      timeout: 15_000,
    });
  };

  // Open the SOURCE library and enter edit mode.
  await openLibrary(sourceLib.id);
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-mode") === "edit",
    { timeout: 5_000 },
  );

  // Open the presentation-edit modal for the source song.
  await page
    .locator(
      `[data-action="presentation-rename"][data-presentation-id="${source.id}"]`,
    )
    .click();
  await page.waitForSelector(
    '[data-role="presentation-edit-modal"][data-open="true"]',
    { timeout: 5_000 },
  );

  // Pick the target library and copy.
  await page
    .locator('[data-role="presentation-copy-target"]')
    .selectOption(targetLib.id);
  await page.locator('[data-role="presentation-copy-confirm"]').click();

  // Modal closes + success toast names the target library.
  await page.waitForFunction(
    () =>
      !document.querySelector(
        '[data-role="presentation-edit-modal"][data-open="true"]',
      ),
    { timeout: 10_000 },
  );
  await expect(page.locator('[data-role="toast"]')).toContainText(
    targetLib.name,
    { timeout: 5_000 },
  );

  // The copy is in the target library; the original stayed in the source.
  await openLibrary(targetLib.id);
  await expect(
    page.locator('[data-role="presentation-item"]', { hasText: source.name }),
  ).toHaveCount(1, { timeout: 10_000 });

  await openLibrary(sourceLib.id);
  await expect(
    page.locator('[data-role="presentation-item"]', { hasText: source.name }),
  ).toHaveCount(1, { timeout: 10_000 });
});
