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

test.describe.configure({ timeout: 300_000 });

function toLocalDateTimeInputValue(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
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
  serverHandle = await startTestServer(config.port, config.dbUrl);
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
    page.on('console', (msg) => {
      console.log('browser console', msg.type(), msg.text());
    });

    await expect(async () => {
      const response = await page.request.get(new URL('/healthz', baseURL).toString(), {
        timeout: 120_000,
      });
      expect(response.ok()).toBeTruthy();
    }).toPass({ timeout: 180_000 });

  await page.goto(new URL('/ui/operator', baseURL).toString());
  await page.waitForLoadState('networkidle');
  await page.waitForFunction(() => window.__presenterLiveConnected === true, {
    timeout: 20_000,
  });

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
  const playlistButtonById = page.locator(
    `[data-role="playlist-list"] [data-role="playlist-item"][data-playlist-id="${playlistId}"]`
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


  const playlistCountBadge = playlistButtonById.locator('[data-role="playlist-count"]');
  await playlistButtonById.waitFor({ state: 'visible' });

  const presentationItems = page.locator('[data-role="presentation-item"]');
  await expect(async () => {
    const count = await presentationItems.count();
    if (count < 2) {
      throw new Error(`insufficient presentations (${count})`);
    }
  }).toPass({ timeout: 10_000, intervals: [250] });

  await presentationItems.nth(0).dragTo(playlistButtonById);
  await expect(playlistCountBadge).toHaveText('1');
  await presentationItems.nth(1).dragTo(playlistButtonById);
  await expect(playlistCountBadge).toHaveText('2');

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
  }).toPass({ timeout: 5_000, intervals: [200] });
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
  if (mainTokens.length >= 1) {
    const sanitize = (value: string) => value.replace(/[.,;:!?]/g, '');
    const first = sanitize(mainTokens[0]);
    const second = sanitize(mainTokens[1] ?? '');
    const libraryToken = sanitize(libraryTokens[0] ?? selection.libraryName);
    const compoundQuery = `${first}, ${second} ${libraryToken}`.trim();
    await searchInput.fill(compoundQuery);
    await expect(searchResults).toHaveAttribute('data-visible', 'true');
    await expect(async () => {
      const count = await searchResults.locator('[data-role="search-result-item"]').count();
      expect(count).toBeGreaterThan(0);
    }).toPass({ timeout: 5_000, intervals: [200] });
    await page.locator('[data-role="global-search-clear"]').click();
  }

  await searchInput.fill('Nadej');
  await expect(searchResults).toHaveAttribute('data-visible', 'true');
  const slideResult = searchResults
    .locator('[data-role="search-result-item"][data-kind="slide"]')
    .first();
  const slidePresentationId = await slideResult.getAttribute('data-presentation-id');
  await slideResult.locator('[data-role="search-result"]').click();
  if (slidePresentationId) {
    await expect(slideContainer).toHaveAttribute(
      'data-slides-placeholder',
      slidePresentationId,
      { timeout: 10_000 }
    );
  }

  await playlistButtonById.click({ timeout: 20_000 });
  await expect(page.locator('[data-role="context-title"]')).toHaveText(
    `Playlist: ${playlistName}`
  );

  await searchInput.fill('Nadej');
  await expect(searchResults).toHaveAttribute('data-visible', 'true');
  const presentationResult = searchResults
    .locator('[data-role="search-result-item"][data-kind="presentation"]')
    .first();
  await expect(presentationResult).toBeVisible();
  const beforeCount = Number((await playlistCountBadge.textContent())?.trim() || '0');
  await presentationResult.dragTo(playlistButtonById);
  await expect(playlistCountBadge).toHaveText(String(beforeCount + 1));
  const afterAppendCount = Number((await playlistCountBadge.textContent())?.trim() || '0');
  await expect(searchInput).toHaveValue('');
  await expect(searchResults).toHaveAttribute('data-visible', 'false');

  await searchInput.fill('Nadej');
  await expect(searchResults).toHaveAttribute('data-visible', 'true');
  const searchPresentationResults = searchResults.locator('[data-role="search-result-item"][data-kind="presentation"]');
  const existingPresentationIds = new Set(
    (
      await playlistItems.evaluateAll((nodes) =>
        nodes.map((node) => node.getAttribute('data-presentation-id') || ''),
      )
    ).filter((value) => Boolean(value))
  );
  const candidateCount = await searchPresentationResults.count();
  expect(candidateCount).toBeGreaterThan(0);
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
  if (insertingNewPresentation) {
    await expect(playlistCountBadge).toHaveText(String(afterAppendCount + 1));
  } else {
    if (insertionPresentationId) {
      expect(existingPresentationIds.has(insertionPresentationId)).toBeTruthy();
    }
    await expect(playlistCountBadge).toHaveText(String(afterAppendCount));
  }
  await expect(playlistItems.first()).toContainText(insertionTitle);
  await expect(searchInput).toHaveValue('');
  await expect(searchResults).toHaveAttribute('data-visible', 'false');

  await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
  await page.locator('[data-role="context-title"]').click();
  await page.keyboard.press('Space');
  await expect(searchInput).toBeFocused();
  await expect(searchInput).toHaveValue('');
  await expect(searchResults).toHaveAttribute('data-visible', 'false');
  await page.locator('[data-role="presentation-list"]').click();

  await expect(slideContainer.locator('[data-action="duplicate"]')).toHaveCount(0);

  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
  const stageStatus = page.locator('[data-role="stage-status"]');
  await expect(stageStatus).toBeVisible();
  await expect(addSlideButton).toBeVisible();
  await expect(clearButton).toBeVisible();
  await expect(clearButton).toBeEnabled();
  await expect(lineLimitControl).toBeVisible();

  await addSlideButton.click();
  const newSlideCard = slideContainer.locator('[data-slide-id]').last();
  await newSlideCard.waitFor({ state: 'visible' });
  const newSlideId = await newSlideCard.getAttribute('data-slide-id');
  expect(newSlideId).toBeTruthy();

  const lineLimitInput = page.locator('[data-role="line-limit"]');
  await expect(lineLimitInput).toHaveValue('32');
  await lineLimitInput.evaluate((input) => {
    input.value = '12';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });

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
  await expect(warningBanner).toContainText('Translation exceeds 2 lines');

  await newTranslationTextarea.fill('Line one\nLine two');
  await newTranslationTextarea.blur();
  await page.waitForLoadState('networkidle');

  await lineLimitInput.evaluate((input) => {
    input.value = '64';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await expect(warningBanner).toHaveAttribute('data-visible', 'false', { timeout: 10_000 });

  await newSlideCard.locator('[data-action="save"]').click();
  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toHaveAttribute('data-visible', 'true');
  await expect(toast).toContainText(/Slide saved/i);

  await safeClick(presentationButton);

  let slideButton = slideContainer.locator(
    `.stage-control__slide[data-slide-id="${selection.slideId}"]`
  );
  await slideButton.waitFor({ state: 'visible' });
  await expect(slideButton.locator('[data-action="duplicate"]')).toHaveCount(1);

  await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
  slideButton = slideContainer.locator(
    `.stage-control__slide[data-slide-id="${selection.slideId}"]`
  );
  await slideButton.waitFor({ state: 'visible' });
  await expect(stageStatus).toBeVisible();
  await expect(lineLimitControl).toBeHidden();
  await expect(slideButton.locator('[data-action="duplicate"]')).toHaveCount(0);
  await expect(addSlideButton).toBeHidden();
  await expect(clearButton).toBeVisible();

    const stagePage = await context.newPage();
    await stagePage.goto(new URL('/stage/worship-snv', baseURL).toString());
    await stagePage.waitForSelector('#current-text', { state: 'attached' });
    await stagePage.waitForFunction(
      () => document.body.dataset.liveState === 'connected',
      undefined,
      { timeout: 10_000 }
    );

    const timerStagePage = await context.newPage();
    await timerStagePage.goto(new URL('/stage/timer', baseURL).toString());
    await timerStagePage.waitForSelector('#countdown-value', { state: 'attached' });

    await slideButton.click();
    const stageTriggerAt = Date.now();
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
    expect(stageLatency).toBeLessThanOrEqual(5_000);
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
    expect(arrowLatency).toBeLessThanOrEqual(2_000);
    await expect(page.locator('[data-role="stage-current"]')).toContainText(forwardRegex, {
      timeout: 2_000,
      ignoreCase: true,
    });
    const leftStart = Date.now();
    await page.keyboard.press('ArrowLeft');
    await expect(currentTextLocator).toContainText(initialRegex, { timeout: 5_000 });
    const leftLatency = Date.now() - leftStart;
    expect(leftLatency).toBeLessThanOrEqual(2_000);
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

    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    const target = new Date(Date.now() + 10 * 60 * 1000);
    await countdownInput.fill(toLocalDateTimeInputValue(target));
    await page.locator('[data-command="set_countdown_target"]').click();

    await page.locator('[data-command="start_countdown"]').click();
    const countdownStartAt = Date.now();
    const countdownSnapshotBefore = await page.evaluate(() => ({
      text: document.getElementById('countdown-state')?.textContent ?? null,
      html: document.getElementById('countdown-state')?.outerHTML ?? null,
    }));
    console.log('countdown before wait', countdownSnapshotBefore);
    const overviewResponse = await page.request.get(new URL('/timers/overview', baseURL).toString());
    console.log('timer overview', await overviewResponse.json());
    await expect(async () => {
      const state = await page.evaluate(() => ({
        display: window.__presenterCountdownDisplay ?? '',
        timers: window.__presenterTimers ?? null,
      }));
      console.log('timers snapshot', state);
      if (!state.timers) {
        throw new Error('timers not yet available');
      }
      if (typeof state.display !== 'string' || state.display.toLowerCase() !== 'running') {
        throw new Error(`countdown display=${state.display}`);
      }
    }).toPass({ timeout: 3_000, intervals: [100] });
    await expect(page.locator('#countdown-state')).toHaveText('Running', {
      timeout: 2_000,
    });
    await expect(async () => {
      const stageValue = await timerStagePage.evaluate(
        () => window.__presenterStageCountdown ?? ''
      );
      if (!stageValue || stageValue === '00:00') {
        throw new Error(`stage countdown=${stageValue}`);
      }
    }).toPass({ timeout: 3_000, intervals: [100] });
    const countdownLatency = Date.now() - countdownStartAt;
    expect(countdownLatency).toBeLessThanOrEqual(3_000);

    await page.locator('[data-command="pause_countdown"]').click();
    await expect(page.locator('#countdown-state')).toHaveText('Paused', {
      timeout: 10_000,
    });

    await page.locator('[data-command="start_preach"]').click();
    await expect(page.locator('#preach-state')).toHaveText('Running', {
      timeout: 10_000,
    });
    await page.locator('[data-command="pause_preach"]').click();
    await expect(page.locator('#preach-state')).toHaveText('Paused', {
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
    await timerStagePage.close();

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
