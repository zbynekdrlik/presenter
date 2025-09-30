import { test, expect, APIRequestContext } from '@playwright/test';
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from './support';

test.describe.configure({ timeout: 300_000 });

let serverHandle: ServerHandle | undefined;
let baseURL: string;
test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

type SlideSelection = {
  libraryId: string;
  libraryName: string;
  presentationName: string;
  presentationId: string;
  slideId: string;
  main: string;
  translation: string;
  stage: string;
};

async function pickSlideWithContent(request: APIRequestContext, base: string): Promise<SlideSelection> {
  const librariesResp = await request.get(new URL('/libraries', base).toString(), {
    timeout: 120_000,
  });
  expect(librariesResp.ok()).toBeTruthy();
  const libraries: Array<{ id: string; name: string; presentations: Array<{ id: string; name: string }> }> =
    await librariesResp.json();

  for (const library of libraries) {
    for (const presentation of library.presentations) {
      const detailResp = await request.get(
        new URL(`/presentations/${presentation.id}`, base).toString(),
        { timeout: 120_000 }
      );
      expect(detailResp.ok()).toBeTruthy();
      const detail: {
        presentation: {
          id: string;
          slides: Array<{
            id: string;
            content: {
              main: { value: string };
              translation: { value: string };
              stage: { value: string };
            };
          }>;
        };
      } = await detailResp.json();

      for (const slide of detail.presentation.slides) {
        const main = slide.content.main.value.trim();
        const translation = slide.content.translation.value.trim();
        const stage = slide.content.stage.value.trim();
        if (main || translation || stage) {
          return {
            libraryId: library.id,
            libraryName: library.name || 'Library',
            presentationName: presentation.name,
            presentationId: detail.presentation.id,
            slideId: slide.id,
            main,
            translation,
            stage,
          };
        }
      }
    }
  }

  throw new Error('No slide with visible content found');
}

test('tablet operator can trigger and edit slide content', async ({ page, context }) => {
  await expect(async () => {
    const response = await page.request.get(new URL('/healthz', baseURL).toString(), {
      timeout: 120_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  const selection = await pickSlideWithContent(page.request, baseURL);

  const librariesDataResponse = await page.request.get(new URL('/libraries', baseURL).toString(), {
    timeout: 120_000,
  });
  expect(librariesDataResponse.ok()).toBeTruthy();
  const librariesData: Array<{
    id: string;
    name: string;
    presentations: Array<{ id: string; name: string }>;
  }> = await librariesDataResponse.json();

  let additional = {
    libraryId: selection.libraryId,
    libraryName: selection.libraryName,
    presentationId: selection.presentationId,
    presentationName: selection.presentationName,
  };

  const matchingLibrary = librariesData.find((lib) => lib.id === selection.libraryId);
  let alternative = matchingLibrary?.presentations.find((presentation) => presentation.id !== selection.presentationId);
  if (!alternative) {
    for (const lib of librariesData) {
      alternative = lib.presentations.find((presentation) => presentation.id !== selection.presentationId);
      if (alternative) {
        additional = {
          libraryId: lib.id,
          libraryName: lib.name,
          presentationId: alternative.id,
          presentationName: alternative.name,
        };
        break;
      }
    }
  } else if (matchingLibrary) {
    additional = {
      libraryId: matchingLibrary.id,
      libraryName: matchingLibrary.name,
      presentationId: alternative.id,
      presentationName: alternative.name,
    };
  }

  const playlistName = `Tablet Autotest ${Date.now()}`;
  const playlistResponse = await page.request.post(new URL('/playlists', baseURL).toString(), {
    data: { name: playlistName },
    headers: { 'Content-Type': 'application/json' },
    timeout: 60_000,
  });
  expect(playlistResponse.ok()).toBeTruthy();
  const playlist = await playlistResponse.json();

  const playlistEntriesResponse = await page.request.put(
    new URL(`/playlists/${playlist.id}/entries`, baseURL).toString(),
    {
      data: {
        entries: [
          { type: 'presentation', presentationId: selection.presentationId },
        ],
      },
      headers: { 'Content-Type': 'application/json' },
      timeout: 60_000,
    }
  );
  if (!playlistEntriesResponse.ok()) {
    console.error('failed to seed playlist entries', playlistEntriesResponse.status(), await playlistEntriesResponse.text());
  }
  expect(playlistEntriesResponse.ok()).toBeTruthy();

  await page.goto(new URL('/ui/tablet', baseURL).toString());
  await page.waitForLoadState('networkidle');
  await page.waitForFunction(() => window.__presenterTabletReady === true, {
    timeout: 20_000,
  });

  const editToggle = page.locator('[data-role="mode-toggle"][data-mode="edit"]');
  const liveToggle = page.locator('[data-role="mode-toggle"][data-mode="live"]');

  const playlistButton = page.locator(
    `[data-role="playlist-button"][data-playlist-id="${playlist.id}"]`
  );
  await playlistButton.waitFor({ state: 'visible' });
  await expect(playlistButton.locator('[data-role="playlist-count"]')).toHaveText('1');
  await playlistButton.click();
  await expect(page.locator('[data-role="context-title"]')).toHaveText(`Playlist: ${playlistName}`);

  const libraryButton = page.locator(
    `[data-role="library-button"][data-library-id="${selection.libraryId}"]`
  );
  await libraryButton.waitFor({ state: 'visible' });

  await editToggle.click();
  await expect(page.locator('body')).toHaveAttribute('data-mode', 'edit');

  const additionalLibraryButton = page.locator(
    `[data-role="library-button"][data-library-id="${additional.libraryId}"]`
  );
  await additionalLibraryButton.waitFor({ state: 'visible' });
  await additionalLibraryButton.click();

  const addButton = page.locator(
    `[data-role="library-entry"][data-presentation-id="${additional.presentationId}"] [data-action="playlist-add"]`
  );
  await addButton.waitFor({ state: 'visible' });
  await addButton.click();
  await expect(playlistButton.locator('[data-role="playlist-count"]')).toHaveText('2');

  await playlistButton.click();
  await expect(page.locator('[data-role="context-title"]')).toHaveText(`Playlist: ${playlistName}`);
  const playlistEntries = page.locator('[data-role="playlist-entry"]');
  await expect(playlistEntries).toHaveCount(2);

  await page
    .locator(
      `[data-role="playlist-entry"][data-presentation-id="${additional.presentationId}"] [data-action="playlist-up"]`
    )
    .click();
  await expect(
    page.locator('[data-role="playlist-entry"]').first().locator('.tablet-button__label')
  ).toContainText(additional.presentationName);

  await page
    .locator(
      `[data-role="playlist-entry"][data-presentation-id="${selection.presentationId}"] [data-action="playlist-remove"]`
    )
    .click();
  await expect(playlistButton.locator('[data-role="playlist-count"]')).toHaveText('1');

  await liveToggle.click();
  await expect(page.locator('body')).toHaveAttribute('data-mode', 'live');

  await libraryButton.click();
  await expect(page.locator('[data-role="context-title"]')).toHaveText(
    `Library: ${selection.libraryName}`
  );

  const presentationButton = page.locator(
    `[data-role="presentation-button"][data-presentation-id="${selection.presentationId}"]`
  );
  await presentationButton.waitFor({ state: 'visible' });
  await presentationButton.click();

  const slideButton = page.locator(
    `[data-role="tablet-slide"][data-slide-id="${selection.slideId}"]`
  );
  await slideButton.waitFor({ state: 'visible' });

  const stagePage = await context.newPage();
  await stagePage.goto(new URL('/stage/worship-snv', baseURL).toString());
  await stagePage.waitForSelector('#current-text', { state: 'attached' });

  await slideButton.click();
  await page.waitForTimeout(500);
  await expect(async () => {
    const snapshotResponse = await page.request.get(new URL('/stage/snapshots/worship-snv', baseURL).toString(), {
      timeout: 15_000,
    });
    if (!snapshotResponse.ok()) {
      throw new Error('snapshot not ready');
    }
    const snapshot = await snapshotResponse.json();
    if (snapshot.presentationId !== selection.presentationId || snapshot.currentSlideId !== selection.slideId) {
      throw new Error(`stage current=${snapshot.currentSlideId}`);
    }
  }).toPass({ timeout: 15_000, intervals: [300] });
  if (selection.main) {
    await expect(stagePage.locator('#current-text')).toContainText(selection.main, {
      timeout: 10_000,
    });
  }

  await editToggle.click();
  await expect(page.locator('body')).toHaveAttribute('data-mode', 'edit');

  await slideButton.click();
  const editor = page.locator('[data-role="editor"]');
  await expect(editor).toHaveAttribute('data-open', 'true');

  const newMain = selection.main ? `${selection.main} (tablet edit)` : 'Tablet main demo';
  const newTranslation = selection.translation
    ? `${selection.translation} (tablet edit)`
    : 'Tablet translation demo';
  const newStage = selection.stage ? `${selection.stage} (tablet edit)` : 'Tablet stage demo';
  const newGroup = 'Tablet Group';

  await page.fill('[data-role="editor-main"]', newMain);
  await page.fill('[data-role="editor-translation"]', newTranslation);
  await page.fill('[data-role="editor-stage"]', newStage);
  await page.fill('[data-role="editor-group"]', newGroup);
  const saveButton = page.locator('[data-role="editor-save"]');
  await saveButton.evaluate((button) => (button as HTMLButtonElement).click());

  await expect(editor).toHaveAttribute('data-open', 'false');
  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toHaveAttribute('data-visible', 'true', { timeout: 10_000 });

  await expect(async () => {
    const text = await stagePage.locator('#current-text').textContent();
    if (!text || !text.includes(newStage)) {
      throw new Error('stage text not yet updated');
    }
  }).toPass({ timeout: 15_000, intervals: [300] });

  const detailResponse = await page.request.get(
    new URL(`/presentations/${selection.presentationId}`, baseURL).toString(),
    { timeout: 60_000 }
  );
  expect(detailResponse.ok()).toBeTruthy();
  const detail = await detailResponse.json();
  const updatedSlide = detail.presentation.slides.find(
    (slide: { id: string }) => slide.id === selection.slideId
  );
  expect(updatedSlide).toBeTruthy();
  expect(updatedSlide.content.main.value).toBe(newMain);
  expect(updatedSlide.content.translation.value).toBe(newTranslation);
  expect(updatedSlide.content.stage.value).toBe(newStage);
  expect(updatedSlide.content.group.name).toBe(newGroup);

  await stagePage.close();
});
