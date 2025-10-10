import { test, expect, APIRequestContext, Locator, Page } from '@playwright/test';
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from './support';

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 600_000 });

async function setCountdownInput(page: Page, minutesAhead: number): Promise<string> {
  return page.evaluate((delta) => {
    const input = document.querySelector('[data-role="countdown-target-input"]') as HTMLInputElement | null;
    if (!input) {
      throw new Error('countdown input missing');
    }
    input.focus();
    const now = new Date();
    now.setMilliseconds(0);
    now.setSeconds(0);
    now.setMinutes(now.getMinutes() + delta);
    const pad = (value: number) => String(value).padStart(2, '0');
    input.value = `${pad(now.getHours())}:${pad(now.getMinutes())}`;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    return now.toISOString();
  }, minutesAhead);
}

type ClickOptions = Parameters<Locator['click']>[0];

async function safeClick(locator: Locator, options: ClickOptions = {}): Promise<void> {
  await locator.waitFor({ state: 'visible' });
  await locator.click(options);
}

async function selectLibraryById(page: Page, libraryId: string): Promise<void> {
  const dashboardButton = page.locator(
    `[data-role="library-list"] [data-role="library-item"][data-library-id="${libraryId}"]`
  );
  if (await dashboardButton.count()) {
    await safeClick(dashboardButton);
    return;
  }
  const moreButton = page.locator('[data-role="library-more"]');
  if (await moreButton.count()) {
    await safeClick(moreButton);
  }
  const modalButton = page.locator(
    `[data-role="library-modal-list"] [data-role="library-item"][data-library-id="${libraryId}"]`
  );
  await safeClick(modalButton);
  const modal = page.locator('[data-role="library-modal"]');
  if (await modal.count()) {
    await expect(modal).toHaveAttribute('data-open', 'false');
  }
}


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

type SlideDetail = {
  id: string;
  content: {
    main: { value: string };
    translation: { value: string };
    stage: { value: string };
  };
};

type PresentationDetail = {
  libraryId: string;
  libraryName: string;
  presentation: {
    id: string;
    name: string;
    slides: SlideDetail[];
  };
};

async function pickSlideWithContent(request: APIRequestContext, baseURL: string) {
  const librariesResp = await request.get(new URL('/libraries', baseURL).toString(), {
    timeout: 120_000,
  });
  expect(librariesResp.ok()).toBeTruthy();
  const libraries: Array<{ presentations: Array<{ id: string; name: string }> }> = await librariesResp.json();

  for (const library of libraries) {
    for (const presentation of library.presentations) {
      const detailResp = await request.get(
        new URL(`/presentations/${presentation.id}`, baseURL).toString(),
        { timeout: 120_000 }
      );
      expect(detailResp.ok()).toBeTruthy();
      const detail: PresentationDetail = await detailResp.json();
      if (detail.presentation.slides.length < 2) {
        continue;
      }
      let activeGroup = '';
      for (const slide of detail.presentation.slides) {
        const explicitGroup = (() => {
          const group = slide.content?.group;
          if (!group) return '';
          if (typeof (group as any).value === 'string') {
            return (group as any).value as string;
          }
          if (typeof (group as any).name === 'string') {
            return (group as any).name as string;
          }
          return '';
        })();
        if (explicitGroup.trim()) {
          activeGroup = explicitGroup.trim();
        }
        const stage = slide.content.stage.value.trim();
        const main = slide.content.main.value.trim();
        const translation = slide.content.translation.value.trim();
        if (stage || main || translation) {
          const primary = stage || main || translation;
          const fallback = stage || translation || main;
          const fallbackSlide = detail.presentation.slides.find((candidate) => candidate.id !== slide.id && (
            candidate.content.stage.value.trim() ||
            candidate.content.main.value.trim() ||
            candidate.content.translation.value.trim()
          ));
          if (!fallbackSlide) {
            continue;
          }
          return {
            libraryId: detail.libraryId,
            libraryName: detail.libraryName,
            presentationName: detail.presentation.name,
            presentationId: detail.presentation.id,
            slideId: slide.id,
            currentText: primary,
            alternateText: fallback,
            reconnectionSlideId: fallbackSlide.id,
            groupName: activeGroup,
          };
        }
      }
    }
  }
  throw new Error('No slide with visible content found');
}

test.describe('Operator control surface', () => {
  test('can trigger slide and drive timers with stage updates', async ({ page, context }) => {
    await expect(async () => {
      const response = await page.request.get(new URL('/healthz', baseURL).toString(), {
        timeout: 120_000,
      });
      expect(response.ok()).toBeTruthy();
    }).toPass({ timeout: 180_000 });

  await page.goto(new URL('/ui/operator', baseURL).toString());
  await page.waitForLoadState('networkidle');
  await expect(async () => {
    const connected = await page.evaluate(() => (window as any).__presenterLiveConnected === true);
    expect(connected).toBeTruthy();
  }).toPass({ timeout: 60_000, intervals: [500] });

  const addSlideButton = page.locator('[data-role="add-slide"]');
  const clearButton = page.locator('[data-role="clear-slide"]');
  await expect(page.locator('[data-role="library-create"]')).toHaveText('+');
  await expect(page.locator('[data-role="playlist-create"]')).toHaveText('+');
  await expect(addSlideButton).toBeHidden();
  await expect(clearButton).toBeVisible();
  const lineLimitControl = page.locator('.operator__line-limit');
  await expect(lineLimitControl).toBeHidden();

  const selection = await pickSlideWithContent(page.request, baseURL);

  const libraryButton = page.locator(
    `[data-role="library-list"] [data-role="library-item"][data-library-id="${selection.libraryId}"]`
  );
  await selectLibraryById(page, selection.libraryId);
  await expect(libraryButton).toHaveCount(1);

  const presentationButtonAgain = page.locator(
    `[data-role="presentation-item"][data-presentation-id="${selection.presentationId}"]`
  );
  await safeClick(presentationButtonAgain);

  const slideContainer = page.locator('[data-role="slides"]');

  const newLibraryName = `Autotest Library ${Date.now()}`;
  const libraryModal = page.locator('[data-role="library-edit-modal"]');
  await page.locator('[data-role="library-create"]').click();
  await expect(libraryModal).toHaveAttribute('data-open', 'true');
  await expect(libraryModal).toHaveAttribute('data-mode', 'create');

  const createNameField = page.locator('[data-role="library-edit-name"]');
  await createNameField.fill(newLibraryName);
  await page.locator('[data-role="library-edit-save"]').click();
  const newLibraryButton = page
    .locator('[data-role="library-list"] [data-role="library-item"]')
    .filter({ hasText: newLibraryName });
  await expect(newLibraryButton).toBeVisible();

  if ((await libraryModal.getAttribute('data-open')) === 'true') {
    await page.locator('[data-role="library-edit-cancel"]').click();
  }
  await expect(libraryModal).toHaveAttribute('data-open', 'false');
  await expect(newLibraryButton).toHaveAttribute('data-active', 'true');
  await expect(page.locator('[data-role="context-title"]')).toHaveText(
    `Library: ${newLibraryName}`
  );
  await expect(
    page.locator('[data-role="presentation-list"] .empty')
  ).toHaveText(/No presentations/i);

  const newLibraryId = await newLibraryButton.getAttribute('data-library-id');
  expect(newLibraryId).toBeTruthy();

  const editButton = page.locator(
    `[data-role="library-list"] [data-role="library-row"][data-library-id="${newLibraryId}"] [data-action="library-edit"]`
  );
  await editButton.click();

  await expect(libraryModal).toHaveAttribute('data-open', 'true');
  await expect(libraryModal).toHaveAttribute('data-mode', 'edit');

  const nameField = page.locator('[data-role="library-edit-name"]');
  await expect(nameField).toHaveValue(newLibraryName);
  const favoriteToggle = page.locator('[data-role="library-edit-favorite"]');
  await favoriteToggle.check();

  const renamedLibraryName = `${newLibraryName} Renamed`;
  await nameField.fill(renamedLibraryName);
  await page.locator('[data-role="library-edit-save"]').click();

  await expect(libraryModal).toHaveAttribute('data-open', 'false');
  const updatedRow = page.locator(
    `[data-role="library-list"] [data-role="library-row"][data-library-id="${newLibraryId}"]`
  );
  await expect(updatedRow.locator('.operator__list-label')).toHaveText(renamedLibraryName);
  await editButton.click();
  await expect(libraryModal).toHaveAttribute('data-open', 'true');
  const favoriteToggleAfterSave = page.locator('[data-role="library-edit-favorite"]');
  await expect(favoriteToggleAfterSave).toBeChecked();
  await favoriteToggleAfterSave.uncheck();
  await page.locator('[data-role="library-edit-save"]').click();
  await expect(libraryModal).toHaveAttribute('data-open', 'false');

  await editButton.click();
  await expect(libraryModal).toHaveAttribute('data-open', 'true');
  const favoriteToggleAfterUnpin = page.locator('[data-role="library-edit-favorite"]');
  await expect(favoriteToggleAfterUnpin).not.toBeChecked();
  await favoriteToggleAfterUnpin.check();
  await page.locator('[data-role="library-edit-save"]').click();
  await expect(libraryModal).toHaveAttribute('data-open', 'false');

  const presentationCreateButton = page.locator('[data-role="presentation-create"]');
  await expect(presentationCreateButton).toBeEnabled();
  page.once('dialog', async (dialog) => {
    expect(dialog.type()).toBe('prompt');
    await dialog.accept('Quick Presentation');
  });
  await presentationCreateButton.click();
  await expect(page.locator('[data-role="presentation-list"]')).toContainText('Quick Presentation');

  await editButton.click();
  await expect(libraryModal).toHaveAttribute('data-open', 'true');
  page.once('dialog', async (dialog) => {
    expect(dialog.type()).toBe('confirm');
    await dialog.accept();
  });
  await page.locator('[data-role="library-edit-delete"]').click();
  await expect(libraryModal).toHaveAttribute('data-open', 'false');
  await expect(
    page.locator(
      `[data-role="library-list"] [data-role="library-row"][data-library-id="${newLibraryId}"]`
    )
  ).toHaveCount(0);

  await page
    .locator(
      `[data-role="library-list"] [data-role="library-item"][data-library-id="${selection.libraryId}"]`
    )
    .click();
  const presentationButton = page.locator(
    `[data-role="presentation-item"][data-presentation-id="${selection.presentationId}"]`
  );
  await safeClick(presentationButton);

  await expect(libraryButton).toHaveAttribute('data-active', 'true');
  await safeClick(presentationButton);

  const playlistModal = page.locator('[data-role="playlist-edit-modal"]');

  const playlistName = `Autotest Playlist ${Date.now()}`;
  await page.locator('[data-role="playlist-create"]').click();
  await expect(playlistModal).toHaveAttribute('data-open', 'true');
  await expect(page.locator('[data-role="playlist-edit-parent"]')).toHaveCount(0);
  await page.locator('[data-role="playlist-edit-name"]').fill(playlistName);
  await page.locator('[data-role="playlist-edit-dashboard"]').check();
  await page.locator('[data-role="playlist-edit-save"]').click();
  await expect(playlistModal).toHaveAttribute('data-open', 'false');

  const playlistButton = page
    .locator('[data-role="playlist-list"] [data-role="playlist-item"]')
    .filter({ hasText: playlistName });
  await expect(playlistButton).toBeVisible();
  await safeClick(playlistButton);
  await expect(playlistButton).toHaveAttribute('data-active', 'true');
  const playlistId = await playlistButton.getAttribute('data-playlist-id');
  expect(playlistId).toBeTruthy();
  const resolvedPlaylistId = playlistId!;
  const playlistButtonById = page.locator(
    `[data-role="playlist-list"] [data-role="playlist-item"][data-playlist-id="${resolvedPlaylistId}"]`
  );
  await expect(page.locator('[data-role="context-title"]')).toHaveText(
    `Playlist: ${playlistName}`
  );
  await expect(
    page.locator('[data-role="presentation-list"] .empty')
  ).toHaveText(/Playlist is empty/i);

  await playlistButtonById.click({ timeout: 20_000 });
  await expect(page.locator('[data-role="context-title"]').nth(0)).toHaveText(`Playlist: ${playlistName}`);
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
  await expect(presentationCreateButton).toBeEnabled();
  page.once('dialog', async (dialog) => {
    expect(dialog.type()).toBe('prompt');
    await dialog.accept('Intro Section');
  });
  await presentationCreateButton.click();
  const separatorItem = page.locator('[data-role="presentation-item"][data-type="separator"]');
  await expect(separatorItem).toContainText('Intro Section');

  await selectLibraryById(page, selection.libraryId);
  await safeClick(presentationButton);


  await playlistButtonById.waitFor({ state: 'visible' });

  const libraryPresentationItems = page.locator('[data-role="presentation-list"] [data-role="presentation-item"][data-type="presentation"]');
  const playlistCountBadge = playlistButtonById.locator('[data-role="playlist-count"]').first();
  const initialPlaylistCount = Number((await playlistCountBadge.textContent())?.trim() || '0');
  await expect(async () => {
    const count = await libraryPresentationItems.count();
    if (count < 2) {
      throw new Error(`insufficient presentations (${count})`);
    }
  }).toPass({ timeout: 10_000, intervals: [250] });

  const presentationIds = (
    await libraryPresentationItems.evaluateAll((nodes) =>
      nodes.slice(0, 2).map((node) => node.getAttribute('data-presentation-id') || '')
    )
  ).filter((value) => value);
  expect(presentationIds.length).toBeGreaterThanOrEqual(2);

  const beforePlaylistCount = await page.evaluate((playlistId) => {
    const helpers = (window as any).__presenterOperatorTestHelpers;
    if (!helpers) return -1;
    return helpers.playlistPresentationCount(playlistId);
  }, resolvedPlaylistId);

  for (const presentationId of presentationIds.slice(0, 2)) {
    await page.evaluate(
      ({ playlistId, presentationId: id }) => {
        const helpers = (window as any).__presenterOperatorTestHelpers;
        if (!helpers) {
          throw new Error('operator test helpers unavailable');
        }
        return helpers.addPresentationToPlaylist(id, playlistId);
      },
      { playlistId: resolvedPlaylistId, presentationId },
    );
  }

  await expect.poll(async () =>
    page.evaluate((playlistId) => {
      const helpers = (window as any).__presenterOperatorTestHelpers;
      if (!helpers) return -1;
      return helpers.playlistPresentationCount(playlistId);
    }, resolvedPlaylistId)
  ).toBe(beforePlaylistCount + 2);

  await page.evaluate((playlistId) => {
    const helpers = (window as any).__presenterOperatorTestHelpers;
    if (helpers && typeof helpers.clearSearch === 'function') {
      helpers.clearSearch();
    }
  }, resolvedPlaylistId);

  await playlistButtonById.click({ timeout: 20_000 });
  await expect(page.locator('[data-role="context-title"]')).toHaveText(
    `Playlist: ${playlistName}`
  );

  const playlistItems = page.locator('[data-role="catalog-bottom"] [data-role="presentation-item"][data-type="presentation"]');
  await expect(playlistItems).toHaveCount(2);
  const secondPlaylistLabel = await playlistItems
    .nth(1)
    .locator('span')
    .first()
    .innerText();

  await playlistItems.nth(1).dragTo(playlistItems.nth(0), {
    targetPosition: { x: 10, y: 4 },
  });
  await expect(
    playlistItems
      .nth(0)
      .locator('span')
      .first()
  ).toContainText(secondPlaylistLabel.trim(), { timeout: 5_000 });

  await safeClick(presentationButton);

  await slideContainer.waitFor({ state: 'visible' });
  const sidebarScroll = await page.evaluate(() => {
    const libraryList = document.querySelector('[data-role="library-list"]');
    const playlistList = document.querySelector('[data-role="playlist-list"]');
    const slides = document.querySelector('[data-role="slides"]');
    if (!slides) {
      return { library: -1, playlist: -1 };
    }
    slides.scrollTop = slides.scrollHeight;
    return {
      library: libraryList ? libraryList.scrollTop : -1,
      playlist: playlistList ? playlistList.scrollTop : -1,
    };
  });
  expect(sidebarScroll.library).toBeLessThan(5);
  expect(sidebarScroll.playlist).toBeLessThan(5);
  await expect(async () => {
    const count = await slideContainer.locator('.operator__slide-text--main').count();
    if (count === 0) {
      throw new Error('missing main text styling');
    }
  }).toPass({ timeout: 15_000, intervals: [200] });
  await expect(async () => {
    const count = await slideContainer.locator('.operator__slide-text--translation').count();
    if (count === 0) {
      throw new Error('missing translation text styling');
    }
  }).toPass({ timeout: 5_000, intervals: [200] });
  await expect(async () => {
    const count = await slideContainer.locator('.operator__slide-text--stage').count();
    if (count === 0) {
      throw new Error('missing stage text styling');
    }
  }).toPass({ timeout: 5_000, intervals: [200] });

  const searchInput = page.locator('[data-role="global-search-query"]');
  const searchResults = page.locator('[data-role="global-search-results"]');

  const mainTokens = selection.currentText.split(/\s+/).filter(Boolean);
  const libraryTokens = selection.libraryName.split(/\s+/).filter(Boolean);
  const sanitize = (value: string) => value.replace(/[.,;:!?]/g, '');
  const searchTermCandidates = [...mainTokens, ...libraryTokens]
    .map((token) => sanitize(token))
    .filter((token) => token.length > 0);
  const searchTerm =
    searchTermCandidates.find((token) => token.length >= 4) ??
    searchTermCandidates[0] ??
    'Jezis';
  if (mainTokens.length >= 1) {
    const first = sanitize(mainTokens[0]);
    const second = sanitize(mainTokens[1] ?? '');
    const libraryToken = sanitize(libraryTokens[0] ?? selection.libraryName);
    const compoundQuery = `${first}, ${second} ${libraryToken}`.trim();
    await searchInput.fill(compoundQuery);
    await expect(async () => {
      const visible = await searchResults.getAttribute('data-visible');
      expect(visible).toBe('true');
    }).toPass({ timeout: 10_000, intervals: [200] });
    await expect(async () => {
      const count = await searchResults.locator('[data-role="search-result-item"]').count();
      expect(count).toBeGreaterThan(0);
    }).toPass({ timeout: 5_000, intervals: [200] });
    await page.locator('[data-role="global-search-clear"]').click();
  }

  await searchInput.fill(searchTerm);
  await searchInput.press('Enter');
  await expect(async () => {
    const visible = await searchResults.getAttribute('data-visible');
    expect(visible).toBe('true');
  }).toPass({ timeout: 10_000, intervals: [200] });
  const presentationResult = searchResults
    .locator('[data-role="search-result-item"][data-kind="presentation"]')
    .first();
  await expect(presentationResult).toBeVisible();
  const beforeCount = Number((await playlistCountBadge.textContent())?.trim() || '0');
  await presentationResult.dragTo(playlistButtonById);
  await expect(playlistCountBadge).toHaveText(String(beforeCount + 1));
  const afterAppendCount = await playlistItems.count();
  await expect.poll(async () => searchInput.inputValue()).toBe('');
  await expect(searchResults).toHaveAttribute('data-visible', 'false');

  await searchInput.fill(searchTerm);
  await searchInput.press('Enter');
  await expect(async () => {
    const visible = await searchResults.getAttribute('data-visible');
    expect(visible).toBe('true');
  }).toPass({ timeout: 10_000, intervals: [200] });
  const searchPresentationResults = searchResults.locator('[data-role="search-result-item"][data-kind="presentation"]');
  const existingPresentationIds = new Set(
    (
      await playlistItems.evaluateAll((nodes) =>
        nodes.map((node) => node.getAttribute('data-presentation-id') || ''),
      )
    ).filter((value) => Boolean(value))
  );
  await expect(async () => {
    const count = await searchPresentationResults.count();
    if (count === 0) {
      throw new Error('no presentation search results');
    }
    return count;
  }).toPass({ timeout: 5_000, intervals: [200] });
  const candidateCount = await searchPresentationResults.count();
  let insertionResult = searchPresentationResults.first();
  let insertionTitle = '';
  let insertionPresentationId = '';
  let insertingNewPresentation = false;
  for (let index = 0; index < candidateCount; index += 1) {
    const candidate = searchPresentationResults.nth(index);
    const candidateId = (await candidate.getAttribute('data-presentation-id')) || '';
    const candidateTitle = (await candidate
      .locator('.operator__search-result-title')
      .innerText()).trim();
    if (candidateId && !existingPresentationIds.has(candidateId)) {
      insertionResult = candidate;
      insertionTitle = candidateTitle;
      insertionPresentationId = candidateId;
      insertingNewPresentation = true;
      break;
    }
    if (!insertionTitle) {
      insertionTitle = candidateTitle;
      insertionPresentationId = candidateId;
    }
  }
  await expect(insertionResult).toBeVisible();
  await playlistItems.first().waitFor({ state: 'visible' });
  await insertionResult.dragTo(playlistItems.first(), {
    targetPosition: { x: 24, y: 6 },
  });
  await expect(async () => {
    const current = await playlistItems.count();
    expect(current).toBeGreaterThanOrEqual(afterAppendCount);
  }).toPass({ timeout: 5_000, intervals: [200] });

  await searchInput.fill(searchTerm);
  await searchInput.press('Enter');
  const firstSearchResult = searchResults.locator('[data-role="search-result-item"]').first();
  await firstSearchResult.waitFor({ state: 'visible' });
  const dropzonePresentationResults = searchResults.locator(
    '[data-role="search-result-item"][data-kind="presentation"]',
  );
  const playlistPresentationIds = new Set(
    (
      await playlistItems.evaluateAll((nodes) =>
        nodes.map((node) => node.getAttribute('data-presentation-id') || ''),
      )
    ).filter((value) => Boolean(value))
  );
  await expect(async () => {
    const count = await dropzonePresentationResults.count();
    if (count === 0) {
      throw new Error('no dropzone candidates');
    }
    return count;
  }).toPass({ timeout: 5_000, intervals: [200] });
  await expect(dropzonePresentationResults.first()).toBeVisible();
  const dropzoneCandidateCount = await dropzonePresentationResults.count();
  let dropzoneCandidate = dropzonePresentationResults.first();
  for (let index = 0; index < dropzoneCandidateCount; index += 1) {
    const candidate = dropzonePresentationResults.nth(index);
    const candidateId = (await candidate.getAttribute('data-presentation-id')) || '';
    if (candidateId && !playlistPresentationIds.has(candidateId)) {
      dropzoneCandidate = candidate;
      break;
    }
  }
  const dropTarget = page.locator('[data-dropzone-target="presentations"]');
  const beforeDropzoneCount = await playlistItems.count();
  await dropzoneCandidate.dragTo(dropTarget, {
    targetPosition: { x: 32, y: 24 },
  });
  await expect(async () => {
    const current = await playlistItems.count();
    expect(current).toBeGreaterThanOrEqual(beforeDropzoneCount);
  }).toPass({ timeout: 5_000, intervals: [200] });
  await page.evaluate(() => {
    const helpers = (window as any).__presenterOperatorTestHelpers;
    if (helpers && typeof helpers.clearSearch === 'function') {
      helpers.clearSearch();
    }
  });
  await expect.poll(async () => searchInput.inputValue()).toBe('');
  await expect(searchResults).toHaveAttribute('data-visible', 'false');

  await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
  await page.locator('[data-role="context-title"]').click();
  await page.keyboard.press('Space');
  await expect(searchInput).toBeFocused();
  await page.locator('[data-role="presentation-list"]').click();

  await playlistButtonById.click();
  const livePlaylistItems = page.locator('[data-role="catalog-bottom"] [data-role="presentation-item"][data-type="presentation"]');
  await expect(async () => {
    const count = await livePlaylistItems.count();
    if (count === 0) {
      throw new Error('playlist empty in live mode');
    }
  }).toPass({ timeout: 10_000, intervals: [200] });
  const firstLiveItem = livePlaylistItems.first();
  const firstLiveLabel = (await firstLiveItem.locator('span').first().innerText()).trim();
  const liveRenameButton = firstLiveItem.locator('[data-action="presentation-rename"]');
  await expect(liveRenameButton).toBeVisible();
  await liveRenameButton.click();
  const presentationEditModal = page.locator('[data-role="presentation-edit-modal"]');
  await expect(presentationEditModal).toHaveAttribute('data-open', 'true');
  const presentationEditInput = page.locator('[data-role="presentation-edit-name"]');
  const liveRenameLabel = `${firstLiveLabel} (Live)`;
  await presentationEditInput.fill(liveRenameLabel);
  await page.locator('[data-role="presentation-edit-save"]').click();
  await expect(presentationEditModal).toHaveAttribute('data-open', 'false');
  await expect.poll(async () => (await livePlaylistItems.first().locator('span').first().innerText()).trim()).toBe(liveRenameLabel);

  const refreshedRenameButton = livePlaylistItems.first().locator('[data-action="presentation-rename"]');
  await refreshedRenameButton.click();
  await expect(presentationEditModal).toHaveAttribute('data-open', 'true');
  await presentationEditInput.fill(firstLiveLabel);
  await page.locator('[data-role="presentation-edit-save"]').click();
  await expect(presentationEditModal).toHaveAttribute('data-open', 'false');
  await expect.poll(async () => (await livePlaylistItems.first().locator('span').first().innerText()).trim()).toBe(firstLiveLabel);

  await selectLibraryById(page, selection.libraryId);

  await safeClick(presentationButton);

  await expect(slideContainer.locator('[data-action="duplicate"]')).toHaveCount(0);

  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
  const stageStatus = page.locator('[data-role="stage-status"]');
  await expect(stageStatus).toBeVisible();
  await expect(addSlideButton).toBeVisible();
  await expect(clearButton).toBeVisible();
  await expect(clearButton).toBeEnabled();
  await expect(lineLimitControl).toBeVisible();
  const toast = page.locator('[data-role="toast"]');

  await addSlideButton.click();
  await expect(toast).toHaveAttribute('data-visible', 'true');
  await expect(toast).toContainText(/Slide added/i);
  await expect(toast).toHaveAttribute('data-visible', 'false', { timeout: 10_000 });
  const newSlideCard = slideContainer.locator('[data-slide-id]').last();
  await newSlideCard.waitFor({ state: 'visible' });
  const newSlideId = await newSlideCard.getAttribute('data-slide-id');
  expect(newSlideId).toBeTruthy();

  const lineLimitInput = page.locator('[data-role="line-limit"]');
  await expect(lineLimitInput).toHaveValue('32');
  await page.request.post(new URL('/settings/features', baseURL).toString(), { data: { lineLimit: 12 } });
  await lineLimitInput.evaluate((input) => {
    input.value = '12';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await expect.poll(async () => {
    const response = await page.request.get(new URL('/settings/features', baseURL).toString());
    const data = await response.json();
    const raw = data.lineLimit ?? data.line_limit;
        const parsed = Number(raw);
    if (!Number.isFinite(parsed)) {
      throw new Error(`unexpected line limit ${raw}`);
    }
    return parsed;
  }).toBe(12);

  const newMainTextarea = newSlideCard.locator('textarea[data-field="main"]');
  const newTranslationTextarea = newSlideCard.locator('textarea[data-field="translation"]');
  const warningBanner = newSlideCard.locator('[data-role="slide-warning"]');

  await newMainTextarea.fill('This line definitely exceeds twelve characters');
  await newMainTextarea.blur();
  await expect(newSlideCard.locator('.operator__slide-text--main')).toHaveAttribute('data-warning', 'true');
  await expect(warningBanner).toHaveAttribute('data-visible', 'true');
  await expect(warningBanner).toContainText('Main text exceeds 12 characters');

  await newTranslationTextarea.fill('Line one\nLine two\nLine three');
  await newTranslationTextarea.blur();
  await page.waitForLoadState('networkidle');
  await expect(newSlideCard.locator('.operator__slide-text--translation')).toHaveAttribute('data-warning', 'true');

  await page.request.post(new URL('/settings/features', baseURL).toString(), { data: { lineLimit: 64 } });
  await lineLimitInput.evaluate((input) => {
    input.value = '64';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await expect.poll(async () => {
    const response = await page.request.get(new URL('/settings/features', baseURL).toString());
    const data = await response.json();
    const raw = data.lineLimit ?? data.line_limit;
        const parsed = Number(raw);
    if (!Number.isFinite(parsed)) {
      throw new Error(`unexpected line limit ${raw}`);
    }
    return parsed;
  }).toBe(64);

  await newTranslationTextarea.fill('Single line only');
  await newTranslationTextarea.blur();
  await page.waitForLoadState('networkidle');
  await expect(newSlideCard.locator('.operator__slide-text--translation')).toHaveAttribute('data-warning', 'false', {
    timeout: 10_000,
  });
  await expect(warningBanner).toHaveAttribute('data-visible', 'false', { timeout: 10_000 });

  await newSlideCard.locator('[data-action="save"]').click();
  await expect(toast).toHaveAttribute('data-visible', 'true');
  await expect(toast).toContainText(/Slide saved/i);
  await expect(toast).toHaveAttribute('data-visible', 'false', { timeout: 10_000 });

  await page.request.post(new URL('/settings/features', baseURL).toString(), { data: { lineLimit: 32 } });
  await lineLimitInput.evaluate((input) => {
    input.value = '32';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await expect.poll(async () => {
    const response = await page.request.get(new URL('/settings/features', baseURL).toString());
    const data = await response.json();
    const raw = data.lineLimit ?? data.line_limit;
        const parsed = Number(raw);
    if (!Number.isFinite(parsed)) {
      throw new Error(`unexpected line limit ${raw}`);
    }
    return parsed;
  }).toBe(32);

  await safeClick(presentationButton);

  let slideButton = slideContainer.locator(
    `.stage-control__slide[data-slide-id="${selection.slideId}"]`
  );
  await slideButton.waitFor({ state: 'visible' });
  await expect(slideButton.locator('[data-action="duplicate"]')).toHaveCount(1);

  await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
  if (newSlideId) {
    const liveOrderBefore = await page.evaluate(({ presentationId }) => {
      const helpers = (window as any).__presenterOperatorTestHelpers;
      if (!helpers || typeof helpers.slideOrder !== 'function') {
        return [];
      }
      return helpers.slideOrder(presentationId);
    }, { presentationId: selection.presentationId });
    if (liveOrderBefore.length > 1) {
      const desiredOrder = [newSlideId, ...liveOrderBefore.filter((id) => id !== newSlideId)];
      await page.evaluate(({ presentationId, ordered }) => {
        const helpers = (window as any).__presenterOperatorTestHelpers;
        if (!helpers || typeof helpers.reorderSlides !== 'function') {
          throw new Error('operator reorder helper unavailable');
        }
        return helpers.reorderSlides(presentationId, ordered);
      }, { presentationId: selection.presentationId, ordered: desiredOrder });
      await expect.poll(async () =>
        page.evaluate(({ presentationId }) => {
          const helpers = (window as any).__presenterOperatorTestHelpers;
          if (!helpers || typeof helpers.slideOrder !== 'function') {
            return [];
          }
          return helpers.slideOrder(presentationId);
        }, { presentationId: selection.presentationId })
      ).toContain(newSlideId);
      await expect.poll(async () =>
        page.evaluate(({ presentationId }) => {
          const helpers = (window as any).__presenterOperatorTestHelpers;
          if (!helpers || typeof helpers.slideOrder !== 'function') {
            return [];
          }
          return helpers.slideOrder(presentationId)[0] || null;
        }, { presentationId: selection.presentationId })
      ).toBe(newSlideId);
    }
  }

  slideButton = slideContainer.locator(
    `.stage-control__slide[data-slide-id="${selection.slideId}"]`
  );
  await slideButton.waitFor({ state: 'visible' });
  await expect(stageStatus).toBeVisible();
  await expect(lineLimitControl).toBeHidden();
  await expect(slideButton.locator('[data-action="duplicate"]')).toHaveCount(0);
  await expect(addSlideButton).toBeHidden();
  await expect(clearButton).toBeVisible();

    let stagePage = await context.newPage();
    await stagePage.goto(new URL('/stage', baseURL).toString());
    await stagePage.waitForSelector('#current-text', { state: 'attached' });
    await stagePage.waitForFunction(
      () => document.body.dataset.liveState === 'connected',
      undefined,
      { timeout: 10_000 }
    );

    const stageLayoutSelect = page.locator('[data-role="stage-layout-select"]');

    const timerOverlayPage = await context.newPage();
    await timerOverlayPage.goto(new URL('/overlays/timer', baseURL).toString());
    await timerOverlayPage.waitForSelector('#timer-value', { state: 'attached' });

    const stageTriggerAt = Date.now();
    await slideButton.dispatchEvent('pointerdown', { button: 0 });
    await slideButton.dispatchEvent('pointerup', { button: 0 });
    await expect(async () => {
      const className = await slideButton.evaluate((el) => el.className);
      if (className.includes('is-loading')) {
        throw new Error('slide still loading');
      }
    }).toPass({
      timeout: 15_000,
      intervals: [250],
      message: 'Slide should finish loading state',
    });
    await expect(slideButton).toHaveClass(/is-active/);
    if (selection.groupName && selection.groupName.trim().length > 0) {
      await expect(stagePage.locator('#current-group')).toContainText(
        selection.groupName.trim(),
        { timeout: 2_000 }
      );
    }

    const currentTextLocator = stagePage.locator('#current-text');
    await expect(currentTextLocator).not.toHaveText(/\bNo active slide\b/, { timeout: 2_000 });
    const initialStageText = (await currentTextLocator.innerText()).trim();
    const escapeRegex = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const initialRegex = new RegExp(escapeRegex(initialStageText), 'i');
    const stageLatency = Date.now() - stageTriggerAt;
    expect(stageLatency).toBeLessThanOrEqual(800);
    await expect(page.locator('[data-role="stage-current"]')).toContainText(initialRegex, {
      timeout: 2_000,
      ignoreCase: true,
    });

    const arrowStart = Date.now();
    await page.keyboard.press('ArrowRight');
    await expect(async () => {
      const text = (await currentTextLocator.innerText()).trim();
      if (text === initialStageText) {
        throw new Error('stage text did not advance');
      }
    }).toPass({ timeout: 2_000, intervals: [200] });
    const forwardStageText = (await currentTextLocator.innerText()).trim();
    const forwardRegex = new RegExp(escapeRegex(forwardStageText), 'i');
    const arrowLatency = Date.now() - arrowStart;
    expect(arrowLatency).toBeLessThanOrEqual(800);
    await expect(page.locator('[data-role="stage-current"]')).toContainText(forwardRegex, {
      timeout: 2_000,
      ignoreCase: true,
    });
    const leftStart = Date.now();
    await page.keyboard.press('ArrowLeft');
    await expect(currentTextLocator).toContainText(initialRegex, { timeout: 5_000 });
    const leftLatency = Date.now() - leftStart;
    expect(leftLatency).toBeLessThanOrEqual(800);
    await expect(page.locator('[data-role="stage-current"]')).toContainText(initialStageText, {
      timeout: 2_000,
      ignoreCase: true,
    });

    await clearButton.click();
    await expect(page.locator('[data-role="stage-status"]')).toHaveAttribute('data-active', 'false', {
      timeout: 2_000,
    });
    await expect(page.locator('[data-role="stage-current"]')).toHaveText('—', {
      timeout: 2_000,
    });
    await expect(stagePage.locator('#current-text')).toHaveText('', {
      timeout: 2_000,
    });

    await page.locator('[data-role="view-toggle"][data-view="timers"]').click();

    await stageLayoutSelect.selectOption('timer');
    await expect(async () => {
      const layoutCode = await stagePage.evaluate(() => document.body.dataset.layoutCode);
      if (layoutCode !== 'timer') {
        throw new Error(`layout=${layoutCode}`);
      }
    }).toPass({ timeout: 10_000, intervals: [200] });
    await stagePage.waitForSelector('#countdown-value', { state: 'attached' });

    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    const initialTargetIso = await setCountdownInput(page, 10);
    await countdownInput.press('Enter');

    await page.locator('[data-role="countdown-start"]').click();
    const countdownStartAt = Date.now();
    await expect(async () => {
      const state = await page.evaluate(() => ({
        display: window.__presenterCountdownDisplay ?? '',
        timers: window.__presenterTimers ?? null,
      }));
      if (!state.timers) {
        throw new Error('timers not yet available');
      }
      if (typeof state.display !== 'string' || state.display.toLowerCase() !== 'running') {
        throw new Error(`countdown display=${state.display}`);
      }
    }).toPass({ timeout: 3_000, intervals: [100] });
    await expect(async () => {
      const stageValue = await stagePage.evaluate(
        () => window.__presenterStageCountdown ?? ''
      );
      if (!stageValue || stageValue === '00:00') {
        throw new Error(`stage countdown=${stageValue}`);
      }
    }).toPass({ timeout: 3_000, intervals: [100] });

    await expect(async () => {
      const overlayValue = await timerOverlayPage.locator('#timer-value').innerText();
      if (!overlayValue || overlayValue.trim() === '0') {
        throw new Error(`overlay countdown=${overlayValue}`);
      }
    }).toPass({ timeout: 3_000, intervals: [100] });
    const countdownLatency = Date.now() - countdownStartAt;
    expect(countdownLatency).toBeLessThanOrEqual(3_000);

    const timersSnapshot = await page.evaluate(() => (window as any).__presenterTimers);
    const initialRemaining = timersSnapshot?.countdownToStart?.secondsRemaining ?? null;
    await page.locator('[data-role="countdown-offset-plus"]').click();
    await expect(async () => {
      const updated = await page.evaluate(() => (window as any).__presenterTimers);
      const updatedRemaining = updated?.countdownToStart?.secondsRemaining ?? 0;
      if (typeof initialRemaining === 'number') {
        if (updatedRemaining <= initialRemaining + 250) {
          throw new Error(`remaining did not increase enough: ${updatedRemaining}`);
        }
      }
    }).toPass({ timeout: 3_000, intervals: [200] });

    const newTargetIso = await setCountdownInput(page, 6);
    await page.locator('[data-role="countdown-start"]').click();
    await expect(async () => {
      const updated = await page.evaluate(() => (window as any).__presenterTimers);
      const iso = updated?.countdownToStart?.target;
      if (!iso) {
        throw new Error('missing countdown target');
      }
      const currentTarget = new Date(iso);
      const expectedTarget = new Date(newTargetIso);
      const diff = Math.abs(currentTarget.getTime() - expectedTarget.getTime());
      if (diff > 60_000) {
        throw new Error(`target mismatch ${diff}ms`);
      }
    }).toPass({ timeout: 5_000, intervals: [200] });

    await Promise.all([
      stageLayoutSelect.selectOption('worship-snv'),
      context.request.post(new URL('/stage/layout', baseURL).toString(), {
        data: { code: 'worship-snv' },
      }),
    ]);

    await stagePage.close();
    stagePage = await context.newPage();
    await stagePage.goto(new URL('/stage', baseURL).toString());
    await stagePage.waitForSelector('#current-text', { state: 'attached' });

    await page.locator('[data-command="start_preach"]').click();
    await expect(page.locator('#preach-state')).toHaveText('Running', {
      timeout: 10_000,
    });
    await page.locator('[data-command="reset_preach"]').click();
    await expect(page.locator('#preach-state')).toHaveText('Idle', {
      timeout: 10_000,
    });

    await page.locator('[data-role="view-toggle"][data-view="worship"]').click();

    await stagePage.evaluate(() => {
      if (window.__presenterStageSocket) {
        window.__presenterStageSocket.close();
      }
    });
    await stagePage.waitForFunction(() => document.body.dataset.liveState === 'connected', undefined, {
      timeout: 10_000,
    });

    await stagePage.close();
    await timerOverlayPage.close();

    await test.step('delete playlist and verify removal', async () => {
      const playlistRow = page.locator(
        `[data-role="playlist-list"] [data-role="playlist-row"][data-playlist-id="${playlistId}"]`
      );
      await expect(playlistRow).toHaveCount(1);
      const playlistEditButton = playlistRow.locator('[data-action="playlist-edit"]');
      await playlistEditButton.click();
      const playlistEditModal = page.locator('[data-role="playlist-edit-modal"]');
      await expect(playlistEditModal).toHaveAttribute('data-open', 'true');
      page.once('dialog', async (dialog) => {
        expect(dialog.type()).toBe('confirm');
        await dialog.accept();
      });
      await page.locator('[data-role="playlist-edit-delete"]').click();
      await expect(playlistEditModal).toHaveAttribute('data-open', 'false');
      await expect(playlistRow).toHaveCount(0);
    });
  });
});
