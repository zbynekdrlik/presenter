'use strict';

(function () {
  const libraries = __LIBRARIES__;
  const playlistsData = __PLAYLISTS__;
  const timersOverview = __TIMERS__;
  const stageLayouts = __STAGE_LAYOUTS__;
  const stageLayoutCodeSeed = "__STAGE_LAYOUT_CODE__";
  const ableSetStatus = __ABLESET_STATUS__ || null;
  const DEFAULT_LINE_LIMIT = 32;
  const MAX_SLIDE_LINES = 2;
  const DEFAULT_CATALOG_HEIGHT = 320;
  const CATALOG_MIN_HEIGHT = 200;
  const CATALOG_MAX_HEIGHT = 520;

  const bodyDataset = document.body ? document.body.dataset || {} : {};
  const attrLineLimit = bodyDataset && typeof bodyDataset.lineLimit === 'string'
    ? Number(bodyDataset.lineLimit)
    : Number.NaN;
  const resolvedLineLimit = Number.isFinite(attrLineLimit) && attrLineLimit >= 10
    ? Math.min(Math.round(attrLineLimit), 120)
    : DEFAULT_LINE_LIMIT;
  const storedCatalogHeight = Number(window.localStorage.getItem('presenter.catalogTopHeight'));
  const resolvedCatalogHeight = Number.isFinite(storedCatalogHeight)
    ? Math.min(Math.max(Math.round(storedCatalogHeight), CATALOG_MIN_HEIGHT), CATALOG_MAX_HEIGHT)
    : DEFAULT_CATALOG_HEIGHT;

  const state = {
    libraries: Array.isArray(libraries) ? libraries : [],
    playlists: Array.isArray(playlistsData) ? playlistsData : [],
    timers: timersOverview || null,
    view: document.body.dataset.view || 'worship',
    mode: document.body.dataset.mode || 'live',
    activeLibraryId: null,
    activePlaylistId: null,
    currentPresentationId: null,
    stagePresentationId: null,
    stageSlideId: null,
    focusedSlideId: null,
    slidesCache: new Map(),
    presentationMeta: new Map(),
    playlistLookup: new Map(),
    presentationPlaylistIndex: new Map(),
    toastTimer: null,
    liveSocket: null,
    liveReconnectTimer: null,
    slideFetchAbort: null,
    reorderSnapshot: null,
    playlistReorderSnapshot: null,
    stageSnapshot: null,
    favoriteLibraryIds: new Set(),
    libraryModalOpen: false,
    libraryEditModalOpen: false,
    libraryBeingEditedId: null,
    libraryEditMode: 'edit',
    libraryEditSubmitting: false,
    playlistModalOpen: false,
    playlistEditModalOpen: false,
    playlistBeingEditedId: null,
    playlistEditMode: 'edit',
    playlistEditSubmitting: false,
    playlistEditInitial: null,
    presentationEditModalOpen: false,
    presentationEditTarget: null,
    presentationEditSubmitting: false,
    lineLimit: resolvedLineLimit,
    pendingFocus: null,
    searchQuery: '',
    searchResults: [],
    searchLoading: false,
    searchTimer: null,
    searchAbort: null,
    searchOpen: false,
    clearingSlide: false,
    searchDragging: false,
    skipClickTrigger: null,
    draggingPresentationId: null,
    draggingFromSearch: false,
    catalogTopHeight: resolvedCatalogHeight,
    catalogResizeActive: false,
    catalogResizePointerId: null,
    catalogResizeStartY: 0,
    catalogResizeStartHeight: resolvedCatalogHeight,
    stageConnections: new Map(),
    stageBaseline: new Set(),
    stageMonitorRefreshTimer: null,
    stageLayouts: Array.isArray(stageLayouts) ? stageLayouts : [],
    stageLayoutCode: typeof stageLayoutCodeSeed === 'string' ? stageLayoutCodeSeed : '',
    stageLayoutLoading: false,
    countdownInputActive: false,
    countdownInputDirty: false,
    ableset: {
      status: normalizeAbleSetStatus(ableSetStatus),
      enableLoading: false,
      followLoading: false,
    },
  };

  const STAGE_MONITOR_BASELINE_KEY = 'presenter.stageMonitorBaseline';
  const STAGE_MONITOR_REFRESH_MS = 60_000;

  const els = {
    libraryList: document.querySelector('[data-role="library-list"]'),
    libraryCreate: document.querySelector('[data-role="library-create"]'),
    playlistList: document.querySelector('[data-role="playlist-list"]'),
    playlistCreate: document.querySelector('[data-role="playlist-create"]'),
    catalog: document.querySelector('[data-role="catalog"]'),
    catalogResizer: document.querySelector('[data-role="catalog-resizer"]'),
    contextTitle: document.querySelector('[data-role="context-title"]'),
    presentationDropzone: document.querySelector('[data-dropzone-target="presentations"]'),
    presentationList: document.querySelector('[data-role="presentation-list"]'),
    presentationCount: document.querySelector('[data-role="presentation-count"]'),
    presentationCreate: document.querySelector('[data-role="presentation-create"]'),
    slides: document.querySelector('[data-role="slides"]'),
    stageMonitor: document.querySelector('[data-role="stage-monitor"]'),
    stageMonitorConnected: document.querySelector('[data-role="stage-monitor-connected"]'),
    stageMonitorIssues: document.querySelector('[data-role="stage-monitor-issues"]'),
    addSlide: document.querySelector('[data-role="add-slide"]'),
    ablesetEnable: document.querySelector('[data-role="ableset-enable"]'),
    ablesetFollow: document.querySelector('[data-role="ableset-follow"]'),
    stageSongLine: document.querySelector('[data-role="stage-song-line"]'),
    clearSlide: document.querySelector('[data-role="clear-slide"]'),
    lineLimit: document.querySelector('[data-role="line-limit"]'),
    toast: document.querySelector('[data-role="toast"]'),
    viewButtons: document.querySelectorAll('[data-role="view-toggle"]'),
    modeButtons: document.querySelectorAll('[data-role="mode-toggle"]'),
    countdownInput: document.querySelector('[data-role="countdown-target-input"]'),
    timerOverlayOpen: document.querySelector('[data-role="timer-overlay-open"]'),
    timerOverlayCopy: document.querySelector('[data-role="timer-overlay-copy"]'),
    countdownStart: document.querySelector('[data-role="countdown-start"]'),
    countdownOffsetMinus: document.querySelector('[data-role="countdown-offset-minus"]'),
    countdownOffsetPlus: document.querySelector('[data-role="countdown-offset-plus"]'),
    stageLayoutSelect: document.querySelector('[data-role="stage-layout-select"]'),
    timerCards: document.querySelector('[data-role="timer-cards"]'),
    libraryModal: document.querySelector('[data-role="library-modal"]'),
    libraryModalList: document.querySelector('[data-role="library-modal-list"]'),
    libraryModalClose: document.querySelector('[data-role="library-modal-close"]'),
    libraryCount: document.querySelector('[data-role="library-more"]'),
    libraryEditModal: document.querySelector('[data-role="library-edit-modal"]'),
    libraryEditForm: document.querySelector('[data-role="library-edit-form"]'),
    libraryEditName: document.querySelector('[data-role="library-edit-name"]'),
    libraryEditFavorite: document.querySelector('[data-role="library-edit-favorite"]'),
    libraryEditDelete: document.querySelector('[data-role="library-edit-delete"]'),
    libraryEditCancel: document.querySelector('[data-role="library-edit-cancel"]'),
    libraryEditTitle: document.querySelector('[data-role="library-edit-title"]'),
    playlistModal: document.querySelector('[data-role="playlist-modal"]'),
    playlistModalList: document.querySelector('[data-role="playlist-modal-list"]'),
    playlistModalClose: document.querySelector('[data-role="playlist-modal-close"]'),
    playlistCount: document.querySelector('[data-role="playlist-more"]'),
    playlistEditModal: document.querySelector('[data-role="playlist-edit-modal"]'),
    playlistEditForm: document.querySelector('[data-role="playlist-edit-form"]'),
    playlistEditName: document.querySelector('[data-role="playlist-edit-name"]'),
    playlistEditDashboard: document.querySelector('[data-role="playlist-edit-dashboard"]'),
    playlistEditDelete: document.querySelector('[data-role="playlist-edit-delete"]'),
    playlistEditCancel: document.querySelector('[data-role="playlist-edit-cancel"]'),
    playlistEditSave: document.querySelector('[data-role="playlist-edit-save"]'),
    playlistEditTitle: document.querySelector('[data-role="playlist-edit-title"]'),
    presentationEditModal: document.querySelector('[data-role="presentation-edit-modal"]'),
    presentationEditForm: document.querySelector('[data-role="presentation-edit-form"]'),
    presentationEditName: document.querySelector('[data-role="presentation-edit-name"]'),
    presentationEditCancel: document.querySelector('[data-role="presentation-edit-cancel"]'),
    presentationEditSave: document.querySelector('[data-role="presentation-edit-save"]'),
    presentationEditTitle: document.querySelector('[data-role="presentation-edit-title"]'),
    presentationEditLabel: document.querySelector('[data-role="presentation-edit-label"]'),
    searchForm: document.querySelector('[data-role="global-search-form"]'),
    searchInput: document.querySelector('[data-role="global-search-query"]'),
    searchClear: document.querySelector('[data-role="global-search-clear"]'),
    searchResults: document.querySelector('[data-role="global-search-results"]'),
  };

  function normalizeAbleSetStatus(input) {
    if (!input || typeof input !== 'object') {
      return { enabled: false, tracking: false, followEnabled: false, lastSong: null, lastError: null };
    }
    const rawSong = input.lastSong || input.last_song || null;
    const song = rawSong && typeof rawSong === 'object' ? {
      name: (rawSong.name || '').toString(),
      prefix: (rawSong.prefix || '').toString(),
      index: typeof rawSong.index === 'number' ? rawSong.index : null,
      lastSeenAt: rawSong.lastSeenAt || rawSong.last_seen_at || null,
    } : null;
    return {
      enabled: Boolean(input.enabled),
      tracking: Boolean(input.tracking),
      followEnabled: Boolean(input.followEnabled ?? input.follow_enabled),
      lastSong: song,
      lastError: input.lastError || input.last_error || null,
    };
  }


  state.libraries = state.libraries.map((library) => ({
    ...library,
    isFavorite: Boolean(library.is_favorite ?? library.isFavorite),
  }));

  const presentationIndex = new Map();

  function rebuildPresentationIndex() {
    presentationIndex.clear();
    state.favoriteLibraryIds.clear();
    state.libraries.forEach((library) => {
      if (library.isFavorite) {
        state.favoriteLibraryIds.add(library.id);
      }
      (library.presentations || []).forEach((presentation) => {
        if (!presentation || !presentation.id) {
          return;
        }
        presentationIndex.set(presentation.id, {
          id: presentation.id,
          name: presentation.name || 'Untitled presentation',
          libraryId: library.id,
          libraryName: library.name,
        });
      });
    });
  }

  rebuildPresentationIndex();
  state.playlists = state.playlists
    .map((playlist) => normalisePlaylist(playlist))
    .filter(Boolean);
  indexPlaylists();
  populateStageLayoutSelect();

  function updateLineLimitStyle() {
    const target = document.body || document.documentElement;
    if (!target) return;
    const value = Number.isFinite(state.lineLimit) && state.lineLimit > 0 ? state.lineLimit : DEFAULT_LINE_LIMIT;
    if (target.dataset) {
      target.dataset.lineLimit = String(value);
    }
    target.style.setProperty('--operator-line-limit-ch', String(value));
  }

  let lineLimitSavePromise = null;

  async function persistLineLimit(value, previousValue, inputEl) {
    if (lineLimitSavePromise) {
      try {
        await lineLimitSavePromise;
      } catch (_) { /* ignore */ }
    }
    const request = apiFetch('/settings/features', {
      method: 'POST',
      body: JSON.stringify({ lineLimit: value }),
    });
    lineLimitSavePromise = request;
    try {
      await request;
      if (document.body && document.body.dataset) {
        document.body.dataset.lineLimit = String(value);
      }
      showToast(`Line limit saved (${value})`, 'success');
    } catch (error) {
      console.error('Failed to persist line limit', error);
      if (inputEl) {
        inputEl.value = String(previousValue);
      }
      state.lineLimit = previousValue;
      updateLineLimitStyle();
      repaintSlideWarnings();
      showToast('Failed to save line limit.', 'error');
    } finally {
      lineLimitSavePromise = null;
    }
  }

  updateLineLimitStyle();

  function stageLayoutByCode(code) {
    return state.stageLayouts.find((layout) => layout && layout.code === code) || null;
  }

  function applyStageLayoutSelection(code) {
    if (typeof code !== 'string') {
      return;
    }
    state.stageLayoutCode = code;
    if (els.stageLayoutSelect) {
      const select = els.stageLayoutSelect;
      const normalized = code.toLowerCase();
      for (const option of Array.from(select.options)) {
        option.selected = option.value.toLowerCase() === normalized;
      }
    }
    const layout = stageLayoutByCode(code);
    if (layout) {
      const title = `${layout.name} – ${layout.description}`;
      if (els.stageLayoutSelect) {
        els.stageLayoutSelect.title = title;
      }
      window.__presenterStageLayout = layout.code;
    }
  }

  function populateStageLayoutSelect() {
    if (!els.stageLayoutSelect) return;
    const select = els.stageLayoutSelect;
    const existing = new Set();
    state.stageLayouts.forEach((layout) => {
      if (!layout || !layout.code) return;
      existing.add(layout.code);
      if (!Array.from(select.options).some((option) => option.value === layout.code)) {
        const option = document.createElement('option');
        option.value = layout.code;
        option.textContent = layout.name || layout.code;
        option.title = layout.description || '';
        select.appendChild(option);
      }
    });
    Array.from(select.options).forEach((option) => {
      if (!existing.has(option.value)) {
        option.remove();
      }
    });
    applyStageLayoutSelection(state.stageLayoutCode || (select.options[0]?.value ?? ''));
  }

  async function submitStageLayout(code) {
    const trimmed = (code || '').trim();
    if (!trimmed || state.stageLayoutLoading) {
      return;
    }
    state.stageLayoutLoading = true;
    if (els.stageLayoutSelect) {
      els.stageLayoutSelect.disabled = true;
    }
    try {
      const response = await apiFetch('/stage/layout', {
        method: 'POST',
        body: JSON.stringify({ code: trimmed }),
      });
      if (response && response.code) {
        applyStageLayoutSelection(response.code);
      }
    } catch (error) {
      console.error('Failed to set stage layout', error);
      showToast('Failed to switch stage output', 'error');
      applyStageLayoutSelection(state.stageLayoutCode);
    } finally {
      state.stageLayoutLoading = false;
      if (els.stageLayoutSelect) {
        els.stageLayoutSelect.disabled = false;
      }
    }
  }

  const clockFormatter = typeof Intl !== 'undefined' && typeof Intl.DateTimeFormat === 'function'
    ? new Intl.DateTimeFormat('sk-SK', { hour: '2-digit', minute: '2-digit' })
    : null;

  function formatClock(date) {
    if (!(date instanceof Date)) {
      return '';
    }
    if (Number.isNaN(date.getTime())) {
      return '';
    }
    if (clockFormatter) {
      return clockFormatter.format(date);
    }
    const hours = String(date.getHours()).padStart(2, '0');
    const minutes = String(date.getMinutes()).padStart(2, '0');
    return `${hours}:${minutes}`;
  }

  function hideSearchResults() {
    if (els.searchResults) {
      els.searchResults.dataset.visible = 'false';
      els.searchResults.innerHTML = '';
    }
    state.searchOpen = false;
  }

  function clearSearchResults() {
    state.searchQuery = '';
    state.searchResults = [];
    state.searchLoading = false;
    if (state.searchTimer) {
      clearTimeout(state.searchTimer);
      state.searchTimer = null;
    }
    if (state.searchAbort && typeof state.searchAbort.cancel === 'function') {
      state.searchAbort.cancel();
    }
    state.searchAbort = null;
    if (els.searchInput) {
      els.searchInput.value = '';
    }
    updateSearchClearVisibility();
    hideSearchResults();
  }

  function updateSearchClearVisibility() {
    if (!els.searchClear) return;
    const hasQuery = Boolean(state.searchQuery && state.searchQuery.trim().length > 0);
    els.searchClear.hidden = !hasQuery;
  }

  function formatMatchField(field) {
    const value = String(field || '').toLowerCase();
    switch (value) {
      case 'maintext':
      case 'main_text':
        return 'Main text';
      case 'translationtext':
      case 'translation_text':
        return 'Translation';
      case 'stagetext':
      case 'stage_text':
        return 'Stage';
      case 'presentationname':
      case 'presentation_name':
        return 'Presentation';
      case 'libraryname':
      case 'library_name':
        return 'Library';
      default:
        return value ? value.charAt(0).toUpperCase() + value.slice(1) : '';
    }
  }

  function renderSearchResultRow(result) {
    const kind = String(result.kind || '').toLowerCase();
    const libraryId = String(result.libraryId || result.library_id || '');
    const presentationId = String(result.presentationId || result.presentation_id || '');
    const slideId = String(result.slideId || result.slide_id || '');
    const libraryName = result.libraryName || result.library_name || '';
    const presentationName = result.presentationName || result.presentation_name || '';
    const snippet = result.snippet || '';
    const field = result.matchField || result.match_field || '';

    let title;
    let meta = '';
    if (kind === 'library') {
      title = libraryName || 'Library';
      meta = '';
    } else if (kind === 'presentation') {
      title = presentationName || 'Presentation';
      meta = libraryName || '';
    } else if (kind === 'slide') {
      title = presentationName || 'Slide';
      const fieldLabel = formatMatchField(field);
      if (libraryName && fieldLabel) {
        meta = `${libraryName} • ${fieldLabel}`;
      } else if (libraryName) {
        meta = libraryName;
      } else if (fieldLabel) {
        meta = fieldLabel;
      }
    } else {
      title = presentationName || libraryName || 'Result';
    }

    const metaMarkup = meta
      ? `<span class="operator__search-result-meta">${escapeHtml(meta)}</span>`
      : '';
    const snippetMarkup = snippet
      ? `<span class="operator__search-result-snippet">${escapeHtml(snippet)}</span>`
      : '';

    const safeKind = escapeHtml(kind);
    const safeLibraryId = escapeHtml(libraryId);
    const safePresentationId = escapeHtml(presentationId);
    const safeSlideId = escapeHtml(slideId);

    const draggable = safePresentationId ? 'true' : 'false';
    return `
      <li class="operator__search-result-item" data-role="search-result-item" data-kind="${safeKind}" data-library-id="${safeLibraryId}" data-presentation-id="${safePresentationId}" data-slide-id="${safeSlideId}" draggable="${draggable}">
        <button type="button" data-role="search-result" data-kind="${safeKind}" data-library-id="${safeLibraryId}" data-presentation-id="${safePresentationId}" data-slide-id="${safeSlideId}">
          <span class="operator__search-result-title">${escapeHtml(title)}</span>
          ${metaMarkup}
          ${snippetMarkup}
        </button>
      </li>
    `;
  }

  function renderSearchResults() {
    if (!els.searchResults) return;
    const query = state.searchQuery.trim();
    if (!query) {
      hideSearchResults();
      return;
    }

    if (state.searchLoading) {
      els.searchResults.innerHTML =
        '<section class="operator__search-group"><p class="operator__search-empty">Searching…</p></section>';
      els.searchResults.dataset.visible = 'true';
      state.searchOpen = true;
      return;
    }

    const results = Array.isArray(state.searchResults) ? state.searchResults : [];
    if (!results.length) {
      els.searchResults.innerHTML =
        '<section class="operator__search-group"><p class="operator__search-empty">No matches found.</p></section>';
      els.searchResults.dataset.visible = 'true';
      state.searchOpen = true;
      return;
    }

    const grouped = {
      library: [],
      presentation: [],
      slide: [],
    };
    results.forEach((item) => {
      const key = String(item.kind || '').toLowerCase();
      if (grouped[key]) {
        grouped[key].push(item);
      }
    });

    const sections = [];
    const order = [
      { key: 'library', label: 'Libraries' },
      { key: 'presentation', label: 'Presentations' },
      { key: 'slide', label: 'Slides' },
    ];

    order.forEach(({ key, label }) => {
      const items = grouped[key];
      if (!items || !items.length) {
        return;
      }
      const rows = items.map((item) => renderSearchResultRow(item)).join('');
      sections.push(
        `<section class="operator__search-group"><h3>${escapeHtml(label)}</h3><ul class="operator__search-result">${rows}</ul></section>`
      );
    });

    els.searchResults.innerHTML = sections.join('');
    els.searchResults.dataset.visible = 'true';
    state.searchOpen = true;
  }

  function scheduleSearch(query) {
    state.searchQuery = query;
    const trimmed = query.trim();
    if (state.searchTimer) {
      clearTimeout(state.searchTimer);
      state.searchTimer = null;
    }
    updateSearchClearVisibility();
    if (!trimmed) {
      state.searchResults = [];
      state.searchLoading = false;
      renderSearchResults();
      return;
    }
    state.searchTimer = setTimeout(() => {
      executeSearch(trimmed);
    }, 200);
  }

  async function executeSearch(query) {
    if (!query) {
      clearSearchResults();
      return;
    }
    if (state.searchAbort && typeof state.searchAbort.cancel === 'function') {
      state.searchAbort.cancel();
    }
    state.searchLoading = true;
    renderSearchResults();
    try {
      const url = `/search?query=${encodeURIComponent(query)}&limit=30`;
      const request = apiFetch(url, { method: 'GET' });
      state.searchAbort = request;
      const response = await request;
      if (state.searchQuery.trim() !== query) {
        return;
      }
      state.searchResults = Array.isArray(response) ? response : [];
    } catch (error) {
      if (error.name === 'AbortError') {
        return;
      }
      console.error('Search request failed', error);
      showToast('Search failed', 'error');
    } finally {
      state.searchLoading = false;
      state.searchAbort = null;
      renderSearchResults();
    }
  }

  function handleSearchInput(event) {
    const value = event.target.value || '';
    scheduleSearch(value);
  }

  function handleSearchSubmit(event) {
    event.preventDefault();
    const value = els.searchInput ? els.searchInput.value : '';
    const trimmed = value.trim();
    if (!trimmed) {
      clearSearchResults();
      return;
    }
    state.searchQuery = trimmed;
    state.searchOpen = true;
    if (state.searchTimer) {
      clearTimeout(state.searchTimer);
      state.searchTimer = null;
    }
    executeSearch(trimmed);
  }

  function handleSearchClear(event) {
    event.preventDefault();
    clearSearchResults();
    if (els.searchInput) {
      els.searchInput.focus();
    }
  }

  async function activateSearchResult(kind, libraryId, presentationId, slideId) {
    if (!libraryId) return;

    state.activeLibraryId = libraryId;
    renderLibraries();
    renderPlaylists();
    renderPresentationList();
    updateContextTitleFromLibrary(libraryId);

    if (!presentationId) {
      hideSearchResults();
      return;
    }

    state.currentPresentationId = presentationId;
    state.focusedSlideId = slideId || null;

    try {
      await loadPresentation(presentationId);
    } catch (error) {
      console.error('Failed to load presentation from search', error);
      return;
    }

    renderPresentationList();
    scrollPresentationIntoView(presentationId);
    if (slideId) {
      const slides = getSlidesForPresentation(presentationId);
      const slideExists = slides.some((slide) => slide.id === slideId);
      if (slideExists) {
        state.focusedSlideId = slideId;
        updateActiveSlideIndicators();
        const card = els.slides
          ? els.slides.querySelector(`[data-slide-id="${slideId}"]`)
          : null;
        if (card && typeof card.scrollIntoView === 'function') {
          card.scrollIntoView({ block: 'center', behavior: 'smooth' });
        }
        if (state.mode === 'edit') {
          const textarea = card?.querySelector('[data-field="main"]');
          if (textarea && typeof textarea.focus === 'function') {
            textarea.focus({ preventScroll: true });
          }
        }
      }
    } else {
      updateActiveSlideIndicators();
    }

    hideSearchResults();
  }

  function handleSearchResultClick(event) {
    const button = event.target.closest('[data-role="search-result"]');
    if (!button) return;
    event.preventDefault();
    const kind = button.dataset.kind || '';
    const libraryId = button.dataset.libraryId || '';
    const presentationId = button.dataset.presentationId || '';
    const slideId = button.dataset.slideId || '';
    els.searchResults.dataset.visible = 'false';
    state.searchOpen = false;
    state.searchResults = [];
    state.searchQuery = '';
    if (els.searchInput) {
      els.searchInput.value = '';
    }
    updateSearchClearVisibility();
    activateSearchResult(kind, libraryId, presentationId || null, slideId || null);
  }

  function handleSearchResultDragStart(event) {
    const item = event.target.closest('[data-role="search-result-item"]');
    if (!item || !event.dataTransfer) {
      return;
    }
    const presentationId = item.dataset.presentationId || '';
    if (!presentationId) {
      event.dataTransfer.effectAllowed = 'none';
      return;
    }
    event.dataTransfer.effectAllowed = 'copy';
    event.dataTransfer.setData('application/x-presenter-presentation', presentationId);
    event.dataTransfer.setData('text/plain', presentationId);
    event.dataTransfer.setData('application/x-presenter-search', 'true');
    state.searchDragging = true;
    state.draggingPresentationId = presentationId;
    state.draggingFromSearch = true;
    const title = item.querySelector('.operator__search-result-title');
    if (title) {
      const rect = title.getBoundingClientRect();
      event.dataTransfer.setDragImage(title, rect.width / 2, rect.height / 2);
    }
  }

  function handleSearchResultDragEnd() {
    state.searchDragging = false;
    state.draggingPresentationId = null;
    state.draggingFromSearch = false;
  }

  function handleSearchOutsideClick(event) {
    if (!state.searchOpen) {
      return;
    }
    const withinForm = els.searchForm && els.searchForm.contains(event.target);
    const withinResults = els.searchResults && els.searchResults.contains(event.target);
    if (withinForm || withinResults) {
      return;
    }
    hideSearchResults();
  }

  function qs(selector, parent) {
    return (parent || document).querySelector(selector);
  }

  function qsa(selector, parent) {
    return Array.from((parent || document).querySelectorAll(selector));
  }

  function escapeHtml(value) {
    return value
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }

  function formatMultiline(text, lint = null, { highlightOverflow = false } = {}) {
    const normalised = (text || '').replace(/\r?\n/g, '\n');
    const lines = normalised.split('\n');
    const overflowCounts = lint && Array.isArray(lint.overflowCharacterCounts)
      ? lint.overflowCharacterCounts
      : [];
    return lines
      .map((line, index) => {
        const overflowChars = overflowCounts[index] || 0;
        let safeHtml;
        if (highlightOverflow && overflowChars > 0 && state.lineLimit > 0) {
          const limit = state.lineLimit;
          const safePart = escapeHtml(line.slice(0, limit));
          const overflowPart = escapeHtml(line.slice(limit));
          safeHtml = `${safePart}<span class="operator__slide-overflow" data-overflow-chars="${overflowChars}">${overflowPart}</span>`;
        } else {
          safeHtml = escapeHtml(line);
        }
        if (
          highlightOverflow &&
          index >= MAX_SLIDE_LINES &&
          line.trim().length > 0
        ) {
          return `<span class="operator__slide-overflow" data-overflow-line="true">${safeHtml}</span>`;
        }
        return safeHtml;
      })
      .join('<br />');
  }

  function lintField(value) {
    const normalised = (value || '').replace(/\r?\n/g, '\n');
    const lines = normalised.split('\n');
    const nonEmptyLines = lines.filter((line) => line.trim().length > 0);
    const hasTooManyLines = MAX_SLIDE_LINES > 0 && nonEmptyLines.length > MAX_SLIDE_LINES;
    const overflowCharacterCounts = lines.map((line) => {
      if (state.lineLimit <= 0) {
        return 0;
      }
      return Math.max(0, line.length - state.lineLimit);
    });
    const totalOverflowCharacters = overflowCharacterCounts.reduce((sum, count) => sum + count, 0);
    const totalOverflowLines = hasTooManyLines
      ? Math.max(0, nonEmptyLines.length - MAX_SLIDE_LINES)
      : 0;
    const hasLineTooLong = totalOverflowCharacters > 0;
    return {
      hasTooManyLines,
      hasLineTooLong,
      hasWarning: hasTooManyLines || hasLineTooLong,
      overflowCharacterCounts,
      totalOverflowCharacters,
      totalOverflowLines,
      lineCount: nonEmptyLines.length,
    };
  }

  function emptyLint() {
    return {
      hasTooManyLines: false,
      hasLineTooLong: false,
      hasWarning: false,
    };
  }

  function buildWarningMessage(mainLint, translationLint) {
    const messages = [];
    if (mainLint.hasLineTooLong) {
      const extra = mainLint.totalOverflowCharacters || 0;
      messages.push(
        `Main text exceeds ${state.lineLimit} characters${extra > 0 ? ` (+${extra})` : ''}`,
      );
    }
    if (mainLint.hasTooManyLines) {
      const extraLines = mainLint.totalOverflowLines || 0;
      messages.push(
        `Main text exceeds ${MAX_SLIDE_LINES} lines${extraLines > 0 ? ` (+${extraLines})` : ''}`,
      );
    }
    if (translationLint.hasLineTooLong) {
      const extra = translationLint.totalOverflowCharacters || 0;
      messages.push(
        `Translation exceeds ${state.lineLimit} characters${extra > 0 ? ` (+${extra})` : ''}`,
      );
    }
    if (translationLint.hasTooManyLines) {
      const extraLines = translationLint.totalOverflowLines || 0;
      messages.push(
        `Translation exceeds ${MAX_SLIDE_LINES} lines${extraLines > 0 ? ` (+${extraLines})` : ''}`,
      );
    }
    return messages.join(' • ');
  }

  function formatTimerState(state) {
    if (!state) return 'Idle';
    const normalized = String(state).toLowerCase();
    return normalized.charAt(0).toUpperCase() + normalized.slice(1);
  }

  function formatSeconds(totalSeconds) {
    const total = Math.max(0, Math.floor(Number(totalSeconds) || 0));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const seconds = total % 60;
    const mm = String(minutes).padStart(2, '0');
    const ss = String(seconds).padStart(2, '0');
    if (hours > 0) {
      const hh = String(hours).padStart(2, '0');
      return `${hh}:${mm}:${ss}`;
    }
    return `${mm}:${ss}`;
  }

  function cloneTextField(field) {
    if (!field || typeof field !== 'object') {
      return { value: typeof field === 'string' ? field : '' };
    }
    return { ...field };
  }

  function cloneSlideContent(content) {
    if (!content || typeof content !== 'object') {
      return {
        main: { value: '' },
        translation: { value: '' },
        stage: { value: '' },
        group: undefined,
      };
    }
    return {
      ...content,
      main: cloneTextField(content.main),
      translation: cloneTextField(content.translation),
      stage: cloneTextField(content.stage),
      group: content.group ? { ...content.group } : undefined,
    };
  }

  function cloneSlide(slide) {
    if (!slide || typeof slide !== 'object') {
      return slide;
    }
    return {
      ...slide,
      content: cloneSlideContent(slide.content),
    };
  }

  function getExplicitGroup(slide) {
    if (!slide) return '';
    const possible = slide.content && slide.content.group ? slide.content.group : slide.group;
    if (!possible) return '';
    if (typeof possible === 'string') return possible;
    if (possible && typeof possible === 'object') {
      if (typeof possible.value === 'string' && possible.value.trim()) {
        return possible.value;
      }
      if (typeof possible.name === 'string' && possible.name.trim()) {
        return possible.name;
      }
    }
    return '';
  }

  function normaliseSlides(rawSlides) {
    if (!Array.isArray(rawSlides)) return [];
    let activeGroup = '';
    return rawSlides.map((slide) => {
      const clone = cloneSlide(slide);
      const explicit = getExplicitGroup(clone);
      if (explicit) {
        activeGroup = explicit;
      }
      clone.explicitGroup = explicit;
      clone.effectiveGroup = activeGroup;
      return clone;
    });
  }

  function extractField(slide, key) {
    if (!slide) return '';
    const direct = slide[key];
    if (typeof direct === 'string') return direct || '';
    if (direct && typeof direct === 'object') {
      if (typeof direct.value === 'string') return direct.value || '';
      if (typeof direct.name === 'string') return direct.name || '';
    }
    if (slide.content && slide.content[key]) {
      const nested = slide.content[key];
      if (typeof nested === 'string') return nested || '';
      if (nested && typeof nested === 'object' && typeof nested.value === 'string') {
        return nested.value || '';
      }
    }
    return '';
  }

  function extractGroup(slide) {
    if (!slide) return '';
    if (typeof slide.effectiveGroup === 'string' && slide.effectiveGroup.trim()) {
      return slide.effectiveGroup;
    }
    if (typeof slide.explicitGroup === 'string' && slide.explicitGroup.trim()) {
      return slide.explicitGroup;
    }
    if (typeof slide.group === 'string') return slide.group || '';
    if (slide.group && typeof slide.group === 'object') {
      if (typeof slide.group.name === 'string') return slide.group.name || '';
      if (typeof slide.group.value === 'string') return slide.group.value || '';
    }
    if (slide.content && slide.content.group) {
      const group = slide.content.group;
      if (typeof group === 'string') return group || '';
      if (group && typeof group === 'object') {
        if (typeof group.name === 'string') return group.name || '';
        if (typeof group.value === 'string') return group.value || '';
      }
    }
    return '';
  }

  function stagePrimaryText(slide) {
    return extractField(slide, 'main').trim();
  }

  function renderStageStatus() {
    const container = qs('[data-role="stage-status"]');
    if (!container) return;

    const currentEl = qs('[data-role="stage-current"]', container);
    const nextEl = qs('[data-role="stage-next"]', container);

    let snapshot = state.stageSnapshot;
    let presentationId = snapshot && snapshot.presentationId ? snapshot.presentationId : state.stagePresentationId;

    let currentSlide = snapshot ? snapshot.current : null;
    let nextSlide = snapshot ? snapshot.next : null;

    if (presentationId && !currentSlide) {
      const slides = getSlidesForPresentation(presentationId);
      if (slides.length) {
        const index = slides.findIndex((slide) => slide.id === state.stageSlideId);
        currentSlide = index >= 0 ? slides[index] : slides[0];
        nextSlide = index >= 0 ? slides[index + 1] || null : slides[1] || nextSlide;
      }
    }

    const hasActive = Boolean(presentationId && (currentSlide || nextSlide));
    container.dataset.active = hasActive ? 'true' : 'false';

    const currentText = stagePrimaryText(currentSlide) || '—';
    if (currentEl) {
      currentEl.textContent = currentText || '—';
    }
    const nextText = stagePrimaryText(nextSlide) || '—';
    if (nextEl) {
      nextEl.textContent = nextText || '—';
    }

    if (els.stageSongLine) {
      els.stageSongLine.textContent = resolveSongLine(snapshot);
    }
  }

  function parseStageConnectionSnapshot(raw) {
    if (!raw || typeof raw !== 'object') return null;
    const id = raw.id || raw.clientId || raw.client_id;
    if (!id) return null;
    const status = (raw.status || '').toString().toLowerCase();
    const latency = raw.latencyMs ?? raw.latency_ms;
    return {
      id: String(id),
      status,
      layoutCode: raw.layoutCode ?? raw.layout_code ?? '',
      latencyMs: typeof latency === 'number' ? latency : null,
      lastHeartbeat: raw.lastHeartbeat ?? raw.last_heartbeat ?? null,
    };
  }

  function loadStageMonitorBaseline() {
    try {
      if (!window.localStorage) return new Set();
      const raw = window.localStorage.getItem(STAGE_MONITOR_BASELINE_KEY);
      if (!raw) return new Set();
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        return new Set(parsed.map((value) => String(value)));
      }
    } catch (error) {
      console.warn('Failed to load stage monitor baseline', error);
    }
    return new Set();
  }

  function persistStageMonitorBaseline() {
    try {
      if (!window.localStorage) return;
      const values = Array.from(state.stageBaseline);
      window.localStorage.setItem(STAGE_MONITOR_BASELINE_KEY, JSON.stringify(values));
    } catch (error) {
      console.warn('Failed to persist stage monitor baseline', error);
    }
  }

  function updateStageMonitorUI() {
    if (!els.stageMonitor || !els.stageMonitorConnected || !els.stageMonitorIssues) return;

    const baselineIds = state.stageBaseline.size
      ? Array.from(state.stageBaseline)
      : [];

    if (baselineIds.length === 0) {
      els.stageMonitorConnected.textContent = '0';
      els.stageMonitorIssues.textContent = '0';
      els.stageMonitor.dataset.connected = '0';
      els.stageMonitor.dataset.issues = '0';
      els.stageMonitor.classList.remove('operator__stage-monitor--alert');
      els.stageMonitor.title = 'Stage displays – baseline empty';
      return;
    }

    let connectedCount = 0;
    let issueCount = 0;

    for (const id of baselineIds) {
      const snapshot = state.stageConnections.get(id);
      if (snapshot && snapshot.status === 'connected') {
        connectedCount += 1;
      } else {
        issueCount += 1;
      }
    }

    els.stageMonitorConnected.textContent = String(connectedCount);
    els.stageMonitorIssues.textContent = String(issueCount);
    els.stageMonitor.dataset.connected = String(connectedCount);
    els.stageMonitor.dataset.issues = String(issueCount);
    els.stageMonitor.classList.toggle('operator__stage-monitor--alert', issueCount > 0);

    const totalKnown = baselineIds.length;
    if (issueCount > 0) {
      els.stageMonitor.title = `Stage displays – Connected: ${connectedCount}/${totalKnown}`;
    } else {
      els.stageMonitor.title = `Stage displays – All ${connectedCount} online`;
    }
  }

  function handleStageConnectionSnapshot(raw) {
    const snapshot = parseStageConnectionSnapshot(raw);
    if (!snapshot) return;
    state.stageConnections.set(snapshot.id, snapshot);
    let baselineChanged = false;
    if (!state.stageBaseline.has(snapshot.id) && snapshot.status === 'connected') {
      state.stageBaseline.add(snapshot.id);
      baselineChanged = true;
    }
    if (baselineChanged) {
      persistStageMonitorBaseline();
    }
    updateStageMonitorUI();
  }

  async function refreshStageConnections() {
    try {
      const response = await fetch('/stage/connections', { cache: 'no-store' });
      if (!response.ok) throw new Error(`stage connections request failed (${response.status})`);
      const payload = await response.json();
      if (!Array.isArray(payload)) return;
      const map = new Map();
      for (const entry of payload) {
        const snapshot = parseStageConnectionSnapshot(entry);
        if (!snapshot) continue;
        map.set(snapshot.id, snapshot);
      }
      let baselineChanged = false;
      for (const snapshot of map.values()) {
        if (snapshot.status === 'connected' && !state.stageBaseline.has(snapshot.id)) {
          state.stageBaseline.add(snapshot.id);
          baselineChanged = true;
        }
      }
      state.stageConnections = map;
      if (baselineChanged) {
        persistStageMonitorBaseline();
      }
      updateStageMonitorUI();
    } catch (error) {
      console.warn('Failed to refresh stage connections', error);
    }
  }

  function resetStageMonitorBaseline(showToast = true) {
    const connectedIds = [];
    for (const [id, snapshot] of state.stageConnections) {
      if (snapshot.status === 'connected') {
        connectedIds.push(id);
      }
    }
    state.stageBaseline = new Set(connectedIds);
    persistStageMonitorBaseline();
    updateStageMonitorUI();
    if (showToast) {
      showToast('Stage monitor baseline reset', 'info');
    }
  }

  function initialiseStageMonitor() {
    state.stageBaseline = loadStageMonitorBaseline();
    if (els.stageMonitor) {
      els.stageMonitor.addEventListener('click', (event) => {
        event.preventDefault();
        resetStageMonitorBaseline(true);
      });
    }
    updateStageMonitorUI();
    refreshStageConnections();
    if (state.stageMonitorRefreshTimer) {
      clearInterval(state.stageMonitorRefreshTimer);
    }
    state.stageMonitorRefreshTimer = window.setInterval(refreshStageConnections, STAGE_MONITOR_REFRESH_MS);
  }

  function showToast(message, variant) {
    if (!els.toast) return;
    els.toast.textContent = message;
    els.toast.dataset.visible = 'true';
    els.toast.dataset.variant = variant || 'info';
    clearTimeout(state.toastTimer);
    state.toastTimer = setTimeout(() => {
      els.toast.dataset.visible = 'false';
    }, 3500);
  }

  function setView(view) {
    state.view = view;
    document.body.dataset.view = view;
    els.viewButtons.forEach((button) => {
      button.dataset.active = button.dataset.view === view ? 'true' : 'false';
    });
  }

  function setMode(mode) {
    state.mode = mode;
    document.body.dataset.mode = mode;
    els.modeButtons.forEach((button) => {
      button.dataset.active = button.dataset.mode === mode ? 'true' : 'false';
    });
    updateAddSlideAvailability();
    updateClearSlideAvailability();
    updateLineLimitControl();
    renderPresentationList();
    if (state.currentPresentationId) {
      renderSlides(state.currentPresentationId);
    }
  }

  function updateAddSlideAvailability() {
    if (!els.addSlide) return;
    const isEdit = state.mode === 'edit';
    els.addSlide.hidden = !isEdit;
    els.addSlide.disabled = !isEdit;
  }

  function updateClearSlideAvailability() {
    if (!els.clearSlide) return;
    const disable = state.clearingSlide;
    els.clearSlide.disabled = disable;
  }

  function updateLineLimitControl() {
    if (!els.lineLimit) return;
    const isEdit = state.mode === 'edit';
    els.lineLimit.disabled = !isEdit;
    const wrapper = els.lineLimit.closest('.operator__line-limit');
    if (wrapper) {
      wrapper.dataset.disabled = isEdit ? 'false' : 'true';
      wrapper.hidden = !isEdit;
    }
  }

  function updateContextTitleFromLibrary(libraryId) {
    const library = state.libraries.find((item) => item.id === libraryId);
    if (library && els.contextTitle) {
      els.contextTitle.textContent = `Library: ${library.name}`;
    }
  }

  function updateContextTitleFromPlaylist(playlistId) {
    const playlist = state.playlists.find((item) => item.id === playlistId);
    if (playlist && els.contextTitle) {
      els.contextTitle.textContent = `Playlist: ${playlist.name}`;
    }
  }

  function renderLibraries() {
    if (!els.libraryList) return;
    const totalLibraries = Array.isArray(state.libraries) ? state.libraries.length : 0;
    if (els.libraryCount) {
      els.libraryCount.textContent = String(totalLibraries);
      els.libraryCount.dataset.empty = totalLibraries === 0 ? 'true' : 'false';
      els.libraryCount.disabled = totalLibraries === 0;
    }
    if (!Array.isArray(state.libraries) || totalLibraries === 0) {
      els.libraryList.innerHTML =
        '<div class="operator__favorites-empty" data-role="library-empty">No libraries yet. Create one to start building songs.</div>';
      if (els.libraryModalList) {
        els.libraryModalList.innerHTML = '';
      }
      return;
    }

    const favorites = [];
    const activeLibraryId = state.activeLibraryId;

    state.libraries.forEach((library) => {
      const isFavorite = state.favoriteLibraryIds.has(library.id);
      library.isFavorite = isFavorite;
      if (isFavorite || library.id === activeLibraryId) {
        if (!favorites.some((existing) => existing.id === library.id)) {
          favorites.push(library);
        }
      }
    });

    favorites.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));

    const favoritesMarkup = favorites.map((library) => renderLibraryRow(library, false)).join('');
    const favoritesSection = favoritesMarkup
      ? `<div class="operator__favorites" data-role="favorites">${favoritesMarkup}</div>`
      : '<div class="operator__favorites-empty" data-role="favorites-empty">Star libraries in settings to keep them handy.</div>';

    els.libraryList.innerHTML = favoritesSection;

    if (els.libraryModalList) {
      els.libraryModalList.innerHTML = state.libraries
        .map((library) => renderLibraryRow(library, true))
        .join('');
    }
  }

  function activateLibrary(libraryId) {
    if (!libraryId) {
      return;
    }
    state.activeLibraryId = libraryId;
    updateContextTitleFromLibrary(libraryId);
    renderLibraries();
    renderPlaylists();
    renderPresentationList();
  }

  function openLibraryModal() {
    if (!els.libraryModal) return;
    state.libraryModalOpen = true;
    els.libraryModal.dataset.open = 'true';
    document.body.dataset.modalOpen = 'library-list';
  }

  function closeLibraryModal() {
    if (!els.libraryModal) return;
    state.libraryModalOpen = false;
    delete document.body.dataset.modalOpen;
    els.libraryModal.dataset.open = 'false';
  }

  function configureLibraryEditModal({ mode, name, favorite, showDelete }) {
    state.libraryEditMode = mode;
    if (els.libraryEditModal) {
      els.libraryEditModal.dataset.mode = mode;
    }
    if (els.libraryEditForm) {
      els.libraryEditForm.dataset.submitting = 'false';
    }
    if (els.libraryEditName) {
      els.libraryEditName.value = name || '';
      els.libraryEditName.disabled = false;
    }
    if (els.libraryEditFavorite) {
      els.libraryEditFavorite.checked = Boolean(favorite);
      els.libraryEditFavorite.disabled = false;
    }
    if (els.libraryEditDelete) {
      if (showDelete) {
        els.libraryEditDelete.removeAttribute('hidden');
        els.libraryEditDelete.disabled = false;
      } else {
        els.libraryEditDelete.setAttribute('hidden', 'true');
        els.libraryEditDelete.disabled = true;
      }
    }
    const saveButton = els.libraryEditForm
      ? els.libraryEditForm.querySelector('[data-role="library-edit-save"]')
      : null;
    if (saveButton) {
      saveButton.textContent = mode === 'create' ? 'Create Library' : 'Save changes';
    }
    const cancelButton = els.libraryEditForm
      ? els.libraryEditForm.querySelector('[data-role="library-edit-cancel"]')
      : null;
    if (cancelButton) {
      cancelButton.textContent = 'Cancel';
    }
  }

  async function toggleLibraryFavorite(libraryId) {
    if (!libraryId) {
      return;
    }
    const nextFavorite = !state.favoriteLibraryIds.has(libraryId);
    try {
      await apiFetch(`/libraries/${libraryId}/favorite`, {
        method: 'POST',
        body: JSON.stringify({ favorite: nextFavorite }),
      });
      if (nextFavorite) {
        state.favoriteLibraryIds.add(libraryId);
      } else {
        state.favoriteLibraryIds.delete(libraryId);
      }
      const library = state.libraries.find((item) => item.id === libraryId);
      if (library) {
        library.isFavorite = nextFavorite;
      }
      renderLibraries();
    } catch (error) {
      console.error('Failed to toggle library favorite', error);
      showToast('Failed to update favorite', 'error');
    }
  }

  async function togglePlaylistFavorite(playlistId) {
    if (!playlistId) {
      return;
    }
    const current = state.playlistLookup.get(playlistId);
    const nextFavorite = !(current && current.showInDashboard);
    try {
      const response = await apiFetch(`/playlists/${playlistId}`, {
        method: 'PATCH',
        body: JSON.stringify({ showInDashboard: nextFavorite }),
      });
      const normalised = normalisePlaylist(response);
      if (upsertPlaylist(normalised)) {
        renderPlaylists();
      }
      showToast(
        nextFavorite ? 'Playlist pinned to dashboard' : 'Playlist removed from dashboard',
        'success',
      );
    } catch (error) {
      console.error('Failed to toggle playlist favorite', error);
      showToast('Failed to update playlist pin', 'error');
    }
  }

  function openPresentationRename(presentationId, libraryIdHint) {
    if (!presentationId) return;
    let library = null;
    if (libraryIdHint) {
      library = state.libraries.find((item) => item.id === libraryIdHint);
    }
    if (!library) {
      library = state.libraries.find((item) =>
        (item.presentations || []).some((presentation) => presentation.id === presentationId),
      );
    }
    const presentation = library
      ? (library.presentations || []).find((item) => item.id === presentationId)
      : null;
    const indexMeta = presentationIndex.get(presentationId) || null;
    const currentName = presentation
      ? presentation.name
      : indexMeta
      ? indexMeta.name
      : 'Untitled presentation';
    const libraryId = library ? library.id : indexMeta?.libraryId || null;

    state.presentationEditTarget = {
      type: 'presentation',
      presentationId,
      libraryId,
    };
    if (els.presentationEditModal) {
      els.presentationEditModal.dataset.mode = 'presentation';
      els.presentationEditModal.dataset.open = 'true';
    }
    state.presentationEditModalOpen = true;
    configurePresentationEditModal({
      title: 'Rename Presentation',
      label: 'Presentation name',
      name: currentName || '',
    });
    document.body.dataset.modalOpen = 'presentation-edit';
    window.setTimeout(() => {
      if (els.presentationEditName) {
        els.presentationEditName.focus();
        els.presentationEditName.select();
      }
    }, 10);
  }

  function openSeparatorRename(playlistId, entryId) {
    if (!playlistId || !entryId) {
      return;
    }
    const playlist = state.playlists.find((item) => item.id === playlistId);
    if (!playlist) {
      showToast('Playlist not found', 'error');
      return;
    }
    const entry = playlist.entries.find((item) => item.entryId === entryId);
    if (!entry || entry.entryType !== 'separator') {
      showToast('Separator not found', 'error');
      return;
    }
    state.presentationEditTarget = {
      type: 'separator',
      playlistId,
      entryId,
    };
    if (els.presentationEditModal) {
      els.presentationEditModal.dataset.mode = 'separator';
      els.presentationEditModal.dataset.open = 'true';
    }
    state.presentationEditModalOpen = true;
    configurePresentationEditModal({
      title: 'Rename Separator',
      label: 'Separator name',
      name: entry.name || 'Separator',
    });
    document.body.dataset.modalOpen = 'presentation-edit';
    window.setTimeout(() => {
      if (els.presentationEditName) {
        els.presentationEditName.focus();
        els.presentationEditName.select();
      }
    }, 10);
  }

  function closePresentationEdit() {
    if (!els.presentationEditModal) return;
    els.presentationEditModal.dataset.open = 'false';
    els.presentationEditModal.dataset.mode = 'presentation';
    state.presentationEditModalOpen = false;
    state.presentationEditTarget = null;
    setPresentationEditSubmitting(false);
    if (state.libraryEditModalOpen) {
      document.body.dataset.modalOpen = 'library-edit';
    } else if (state.playlistEditModalOpen) {
      document.body.dataset.modalOpen = 'playlist-edit';
    } else if (state.libraryModalOpen) {
      document.body.dataset.modalOpen = 'library-list';
    } else if (state.playlistModalOpen) {
      document.body.dataset.modalOpen = 'playlist-list';
    } else {
      delete document.body.dataset.modalOpen;
    }
  }

  function openLibraryEdit(libraryId) {
    const library = state.libraries.find((item) => item.id === libraryId);
    if (!library || !els.libraryEditModal || !els.libraryEditForm) {
      return;
    }
    if (state.libraryModalOpen) {
      closeLibraryModal();
    }
    state.libraryBeingEditedId = libraryId;
    state.libraryEditModalOpen = true;
    state.libraryEditSubmitting = false;
    configureLibraryEditModal({
      mode: 'edit',
      name: library.name,
      favorite: state.favoriteLibraryIds.has(libraryId),
      showDelete: true,
    });
    if (els.libraryEditTitle) {
      els.libraryEditTitle.textContent = `Edit ${library.name}`;
    }
    els.libraryEditModal.dataset.open = 'true';
    document.body.dataset.modalOpen = 'library-edit';
    window.setTimeout(() => {
      if (els.libraryEditName) {
        els.libraryEditName.focus();
        els.libraryEditName.select();
      }
    }, 10);
  }

  function openLibraryCreate() {
    if (!els.libraryEditModal || !els.libraryEditForm) {
      return;
    }
    state.libraryBeingEditedId = null;
    state.libraryEditModalOpen = true;
    state.libraryEditSubmitting = false;
    configureLibraryEditModal({ mode: 'create', name: '', favorite: true, showDelete: false });
    if (els.libraryEditTitle) {
      els.libraryEditTitle.textContent = 'Create Library';
    }
    els.libraryEditModal.dataset.open = 'true';
    document.body.dataset.modalOpen = 'library-edit';
    window.setTimeout(() => {
      if (els.libraryEditName) {
        els.libraryEditName.focus();
        els.libraryEditName.select();
      }
    }, 10);
  }

  function closeLibraryEdit() {
    if (!els.libraryEditModal) return;
    state.libraryBeingEditedId = null;
    state.libraryEditSubmitting = false;
    state.libraryEditMode = 'edit';
    state.libraryEditModalOpen = false;
    els.libraryEditModal.dataset.open = 'false';
    els.libraryEditModal.dataset.mode = 'edit';
    if (!state.libraryModalOpen) {
      delete document.body.dataset.modalOpen;
    } else {
      document.body.dataset.modalOpen = 'library-list';
    }
  }

  function setLibraryEditSubmitting(submitting) {
    state.libraryEditSubmitting = submitting;
    if (!els.libraryEditForm) return;
    els.libraryEditForm.dataset.submitting = submitting ? 'true' : 'false';
    if (els.libraryEditName) {
      els.libraryEditName.disabled = submitting;
    }
    if (els.libraryEditFavorite) {
      els.libraryEditFavorite.disabled = submitting;
    }
    if (els.libraryEditDelete && !els.libraryEditDelete.hasAttribute('hidden')) {
      els.libraryEditDelete.disabled = submitting;
    }
    const submitButton = els.libraryEditForm.querySelector('[data-role="library-edit-save"]');
    if (submitButton) {
      submitButton.disabled = submitting;
    }
    const cancelButton = els.libraryEditForm.querySelector('[data-role="library-edit-cancel"]');
    if (cancelButton) {
      cancelButton.disabled = submitting;
    }
  }

  function setPresentationEditSubmitting(submitting) {
    state.presentationEditSubmitting = submitting;
    if (!els.presentationEditForm) return;
    els.presentationEditForm.dataset.submitting = submitting ? 'true' : 'false';
    if (els.presentationEditName) {
      els.presentationEditName.disabled = submitting;
    }
    if (els.presentationEditSave) {
      els.presentationEditSave.disabled = submitting;
    }
  }

  function configurePlaylistEditModal({ mode, name, showInDashboard }) {
    state.playlistEditMode = mode;
    if (els.playlistEditModal) {
      els.playlistEditModal.dataset.mode = mode;
    }
    if (els.playlistEditForm) {
      els.playlistEditForm.dataset.submitting = 'false';
    }
    if (els.playlistEditName) {
      els.playlistEditName.value = name || '';
      els.playlistEditName.disabled = false;
    }
    if (els.playlistEditDashboard) {
      els.playlistEditDashboard.checked = Boolean(showInDashboard);
      els.playlistEditDashboard.disabled = false;
    }
    if (els.playlistEditDelete) {
      if (mode === 'edit') {
        els.playlistEditDelete.removeAttribute('hidden');
        els.playlistEditDelete.disabled = false;
      } else {
        els.playlistEditDelete.setAttribute('hidden', 'true');
        els.playlistEditDelete.disabled = true;
      }
    }
    if (els.playlistEditTitle) {
      els.playlistEditTitle.textContent =
        mode === 'create' ? 'Create Playlist' : `Edit ${name || 'Playlist'}`;
    }
    if (els.playlistEditSave) {
      els.playlistEditSave.textContent =
        mode === 'create' ? 'Create Playlist' : 'Save changes';
      els.playlistEditSave.disabled = false;
    }
  }

  function configurePresentationEditModal({ title, label, name }) {
    if (els.presentationEditTitle) {
      els.presentationEditTitle.textContent = title || 'Rename';
    }
    if (els.presentationEditLabel) {
      els.presentationEditLabel.textContent = label || 'Name';
    }
    if (els.presentationEditName) {
      els.presentationEditName.value = name || '';
      els.presentationEditName.disabled = false;
    }
    if (els.presentationEditSave) {
      els.presentationEditSave.disabled = false;
    }
    setPresentationEditSubmitting(false);
  }

  function openPlaylistEdit(playlistId) {
    const playlist = state.playlistLookup.get(playlistId);
    if (!playlist) {
      showToast('Playlist not found', 'error');
      return;
    }
    if (state.playlistModalOpen) {
      closePlaylistModal();
    }
    state.playlistBeingEditedId = playlistId;
    state.playlistEditInitial = {
      name: playlist.name || '',
      showInDashboard: Boolean(playlist.showInDashboard),
    };
    configurePlaylistEditModal({
      mode: 'edit',
      name: playlist.name,
      showInDashboard: playlist.showInDashboard,
    });
    if (els.playlistEditModal) {
      els.playlistEditModal.dataset.open = 'true';
    }
    state.playlistEditModalOpen = true;
    document.body.dataset.modalOpen = 'playlist-edit';
    window.setTimeout(() => {
      if (els.playlistEditName) {
        els.playlistEditName.focus();
        els.playlistEditName.select();
      }
    }, 10);
  }

  function openPlaylistCreate() {
    if (state.playlistModalOpen) {
      closePlaylistModal();
    }
    state.playlistBeingEditedId = null;
    state.playlistEditInitial = {
      name: '',
      showInDashboard: true,
    };
    configurePlaylistEditModal({
      mode: 'create',
      name: '',
      showInDashboard: true,
    });
    if (els.playlistEditModal) {
      els.playlistEditModal.dataset.open = 'true';
    }
    state.playlistEditModalOpen = true;
    document.body.dataset.modalOpen = 'playlist-edit';
    window.setTimeout(() => {
      if (els.playlistEditName) {
        els.playlistEditName.focus();
        els.playlistEditName.select();
      }
    }, 10);
  }

  function closePlaylistEdit() {
    if (!els.playlistEditModal) return;
    state.playlistBeingEditedId = null;
    state.playlistEditSubmitting = false;
    state.playlistEditInitial = null;
    state.playlistEditModalOpen = false;
    els.playlistEditModal.dataset.open = 'false';
    if (!state.playlistModalOpen) {
      if (state.libraryModalOpen) {
        document.body.dataset.modalOpen = 'library-list';
      } else {
        delete document.body.dataset.modalOpen;
      }
    }
  }

  function setPlaylistEditSubmitting(submitting) {
    state.playlistEditSubmitting = submitting;
    if (!els.playlistEditForm) return;
    els.playlistEditForm.dataset.submitting = submitting ? 'true' : 'false';
    if (els.playlistEditName) {
      els.playlistEditName.disabled = submitting;
    }
    if (els.playlistEditDashboard) {
      els.playlistEditDashboard.disabled = submitting;
    }
    if (els.playlistEditDelete && !els.playlistEditDelete.hasAttribute('hidden')) {
      els.playlistEditDelete.disabled = submitting;
    }
    if (els.playlistEditSave) {
      els.playlistEditSave.disabled = submitting;
    }
    if (els.playlistEditCancel) {
      els.playlistEditCancel.disabled = submitting;
    }
  }

  function openPlaylistModal() {
    if (!els.playlistModal) return;
    state.playlistModalOpen = true;
    els.playlistModal.dataset.open = 'true';
    document.body.dataset.modalOpen = 'playlist-list';
  }

  function closePlaylistModal() {
    if (!els.playlistModal) return;
    state.playlistModalOpen = false;
    els.playlistModal.dataset.open = 'false';
    if (!state.libraryModalOpen && !state.libraryEditModalOpen && !state.playlistEditModalOpen) {
      delete document.body.dataset.modalOpen;
    }
  }

  

  

  

  

  async function handlePlaylistEditSubmit(event) {
    event.preventDefault();
    if (state.playlistEditSubmitting) return;
    if (!els.playlistEditForm) return;

    const name = els.playlistEditName ? els.playlistEditName.value.trim() : '';
    const showInDashboard = els.playlistEditDashboard
      ? Boolean(els.playlistEditDashboard.checked)
      : false;

    if (!name) {
      showToast('Playlist name cannot be empty', 'warning');
      if (els.playlistEditName) {
        els.playlistEditName.focus();
      }
      return;
    }

    const initial = state.playlistEditInitial || {
      name: '',
      showInDashboard: false,
    };

    setPlaylistEditSubmitting(true);

    try {
      if (state.playlistEditMode === 'create') {
        const playlist = await apiFetch('/playlists', {
          method: 'POST',
          body: JSON.stringify({
            name,
            showInDashboard,
          }),
        });
        if (!playlist || !playlist.id) {
          throw new Error('Unexpected response creating playlist');
        }
        const normalized = normalisePlaylist(playlist);
        upsertPlaylist(normalized);
        state.activePlaylistId = normalized.id;
        state.activeLibraryId = null;
        state.currentPresentationId = null;
        state.focusedSlideId = null;
        renderPlaylists();
        renderPresentationList();
        updateContextTitleFromPlaylist(normalized.id);
        closePlaylistEdit();
        showToast('Playlist created', 'success');
      } else if (state.playlistBeingEditedId) {
        const playlistId = state.playlistBeingEditedId;
        const payload = {};
        if (name !== initial.name) {
          payload.name = name;
        }
        if (showInDashboard !== initial.showInDashboard) {
          payload.showInDashboard = showInDashboard;
        }
        if (Object.keys(payload).length === 0) {
          closePlaylistEdit();
          return;
        }
        const playlist = await apiFetch(`/playlists/${playlistId}`, {
          method: 'PATCH',
          body: JSON.stringify(payload),
        });
        if (!playlist || !playlist.id) {
          throw new Error('Unexpected response updating playlist');
        }
        const normalized = normalisePlaylist(playlist);
        upsertPlaylist(normalized);
        renderPlaylists();
        if (state.activePlaylistId === playlistId) {
          updateContextTitleFromPlaylist(playlistId);
          renderPresentationList();
        }
        closePlaylistEdit();
        showToast('Playlist updated', 'success');
      }
    } catch (error) {
      console.error('Failed to save playlist', error);
      showToast('Failed to save playlist', 'error');
    } finally {
      setPlaylistEditSubmitting(false);
    }
  }


  async function handlePlaylistEditDelete(event) {
    event.preventDefault();
    if (state.playlistEditSubmitting) return;
    const playlistId = state.playlistBeingEditedId;
    if (!playlistId) {
      closePlaylistEdit();
      return;
    }
    const playlist = state.playlistLookup.get(playlistId);
    const name = playlist ? playlist.name : 'playlist';
    const confirmed = window.confirm(`Delete playlist "${name}"?`);
    if (!confirmed) {
      return;
    }
    setPlaylistEditSubmitting(true);
    try {
      await apiFetch(`/playlists/${playlistId}`, { method: 'DELETE' });
      removePlaylistFromState(playlistId);
      indexPlaylists();
      renderPlaylists();
      if (state.activePlaylistId === playlistId) {
        state.activePlaylistId = null;
        state.currentPresentationId = null;
        state.focusedSlideId = null;
        renderPresentationList();
      }
      closePlaylistEdit();
      showToast('Playlist deleted', 'success');
    } catch (error) {
      console.error('Failed to delete playlist', error);
      showToast('Failed to delete playlist', 'error');
    } finally {
      setPlaylistEditSubmitting(false);
    }
  }

  function handlePlaylistModalClick(event) {
    const favoriteToggle = event.target.closest('[data-action="playlist-favorite"]');
    if (favoriteToggle && favoriteToggle.dataset.playlistId) {
      event.preventDefault();
      event.stopPropagation();
      togglePlaylistFavorite(favoriteToggle.dataset.playlistId);
      return;
    }

    const editButton = event.target.closest('[data-action="playlist-edit"]');
    if (editButton && editButton.dataset.playlistId) {
      event.preventDefault();
      event.stopPropagation();
      openPlaylistEdit(editButton.dataset.playlistId);
      return;
    }

    const button = event.target.closest('[data-role="playlist-item"]');
    if (!button) return;
    const playlistId = button.dataset.playlistId;
    if (!playlistId) return;
    state.activePlaylistId = playlistId;
    state.activeLibraryId = null;
    state.currentPresentationId = null;
    state.focusedSlideId = null;
    renderPlaylists();
    renderPresentationList();
    closePlaylistModal();
  }

  function renderPlaylistRow(playlist, { forModal = false } = {}) {
    const entryCount = Array.isArray(playlist.entries) ? playlist.entries.length : 0;
    const isActive = playlist.id === state.activePlaylistId ? 'true' : 'false';
    const isFavorite = Boolean(playlist.showInDashboard);
    const wrapperClasses = ['operator__list-item', 'operator__list-row'];
    if (forModal) {
      wrapperClasses.push('operator__list-row--modal');
    }
    const buttonAttrs = forModal
      ? `class="operator__list-button" data-role="playlist-item" data-playlist-id="${playlist.id}"`
      : `class="operator__list-button" data-role="playlist-item" data-playlist-id="${playlist.id}" data-active="${isActive}"`;
    const wrapperAttrs = `class="${wrapperClasses.join(' ')}" data-role="playlist-row" data-playlist-id="${playlist.id}"`;
    const favoriteButton = forModal
      ? `<button type="button" class="operator__list-favorite operator__list-favorite--inline" data-action="playlist-favorite" data-playlist-id="${playlist.id}" aria-pressed="${isFavorite ? 'true' : 'false'}" aria-label="${isFavorite ? 'Remove playlist from dashboard' : 'Show in dashboard'}">${isFavorite ? '★' : '☆'}</button>`
      : '';
    const editButton = `<button type="button" class="operator__list-action operator__list-action--icon operator__list-action--menu" data-action="playlist-edit" data-playlist-id="${playlist.id}" aria-label="Edit playlist">⋮</button>`;

    return `
      <div ${wrapperAttrs}>
        ${favoriteButton}
        <button ${buttonAttrs}>
          <span class="operator__list-label">${escapeHtml(playlist.name || 'Untitled playlist')}</span>
          <span class="operator__list-meta" data-role="playlist-count">${entryCount}</span>
        </button>
        <div class="operator__list-actions">
          ${editButton}
        </div>
      </div>
    `;
  }

  function renderPlaylists() {
    if (!els.playlistList) return;
    const totalPlaylists = Array.isArray(state.playlists) ? state.playlists.length : 0;
    if (els.playlistCount) {
      els.playlistCount.textContent = String(totalPlaylists);
      els.playlistCount.dataset.empty = totalPlaylists === 0 ? 'true' : 'false';
      els.playlistCount.disabled = totalPlaylists === 0;
    }
    if (!Array.isArray(state.playlists) || totalPlaylists === 0) {
      els.playlistList.innerHTML =
        '<div class="empty" data-role="playlist-empty">No playlists yet. Create one to build a run sheet.</div>';
      if (els.playlistModalList) {
        els.playlistModalList.innerHTML = '<p class="empty">No playlists yet.</p>';
      }
      return;
    }

    const collator = new Intl.Collator(undefined, { sensitivity: 'base' });
    const sorted = state.playlists.slice().sort((a, b) => collator.compare(a.name, b.name));
    const dashboard = [];
    sorted.forEach((playlist) => {
      const pinned = Boolean(playlist.showInDashboard);
      if (pinned || playlist.id === state.activePlaylistId) {
        if (!dashboard.some((entry) => entry.id === playlist.id)) {
          dashboard.push(playlist);
        }
      }
    });

    const favoritesMarkup = dashboard
      .map((playlist) => renderPlaylistRow(playlist))
      .join('');

    if (favoritesMarkup) {
      els.playlistList.innerHTML = `<div class="operator__favorites" data-role="playlist-favorites">${favoritesMarkup}</div>`;
    } else {
      const fullList = sorted.map((playlist) => renderPlaylistRow(playlist)).join('');
      els.playlistList.innerHTML = fullList
        ? `<div class="operator__list" data-role="playlist-fallback">${fullList}</div>`
        : '<div class="operator__favorites-empty" data-role="playlist-empty">No playlists yet. Create one to build a run sheet.</div>';
    }

    if (els.playlistModalList) {
      const modalEntries = sorted
        .map((playlist) => renderPlaylistRow(playlist, { forModal: true }))
        .join('');
      els.playlistModalList.innerHTML = sorted.length
        ? `<ul class="operator__list">${modalEntries}</ul>`
        : '<p class="empty">No playlists yet.</p>';
    }
  }


  function renderLibraryRow(library, forModal) {
    const count = Array.isArray(library.presentations)
      ? library.presentations.length
      : library.presentation_count || 0;
    const active = library.id === state.activeLibraryId ? 'true' : 'false';
    const favorite = state.favoriteLibraryIds.has(library.id);
    const classes = ['operator__list-item', 'operator__list-row'];
    if (forModal) {
      classes.push('operator__list-row--modal');
    }
    const rowAttrs = `class="${classes.join(' ')}" data-role="library-row" data-library-id="${library.id}"`;
    const buttonAttrs = forModal
      ? `class="operator__list-button" data-role="library-item" data-library-id="${library.id}"`
      : `class="operator__list-button" data-role="library-item" data-library-id="${library.id}" data-active="${active}"`;
    const favoriteButton = forModal
      ? `<button type="button" class="operator__list-favorite operator__list-favorite--inline" data-action="library-favorite" data-library-id="${library.id}" aria-pressed="${favorite ? 'true' : 'false'}" aria-label="${favorite ? 'Remove from dashboard' : 'Show in dashboard'}">${favorite ? '★' : '☆'}</button>`
      : '';
    const editButton = `<button type="button" class="operator__list-action operator__list-action--icon operator__list-action--menu" data-action="library-edit" data-library-id="${library.id}" aria-label="Edit library">⋮</button>`;

    return `
      <div ${rowAttrs}>
        ${favoriteButton}
        <button ${buttonAttrs}>
          <span class="operator__list-label">${escapeHtml(library.name)}</span>
          <span class="operator__list-meta" data-role="library-count">${count}</span>
        </button>
        <div class="operator__list-actions">
          ${editButton}
        </div>
      </div>
    `;
  }

  function renderPresentationList() {
    if (!els.presentationList) return;
    const container = els.presentationList;
    let html = '';
    let count = 0;
    const isEditMode = state.mode === 'edit';

    if (els.presentationCreate) {
      const canCreate = Boolean(state.activeLibraryId || state.activePlaylistId);
      els.presentationCreate.disabled = !canCreate;
      const label = state.activePlaylistId
        ? 'Add separator to playlist'
        : 'Create presentation';
      els.presentationCreate.setAttribute('aria-label', label);
    }

    if (state.activeLibraryId) {
      const library = state.libraries.find((item) => item.id === state.activeLibraryId);
      if (!library || library.presentations.length === 0) {
        html = '<li class="empty">No presentations in this library.</li>';
        count = 0;
      } else {
        const sortedPresentations = library.presentations
          .slice()
          .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
        count = sortedPresentations.length;
        html = sortedPresentations
          .map((presentation) => {
            const active = state.currentPresentationId === presentation.id ? ' is-active' : '';
            const meta = presentationIndex.get(presentation.id);
            const libraryName = meta ? meta.libraryName : library.name;
            const renameAction = `
                <button type="button" class="operator__presentation-action" data-action="presentation-rename" data-presentation-id="${presentation.id}" data-library-id="${library.id}" title="Rename presentation">
                  <span aria-hidden="true">✎</span>
                  <span class="sr-only">Rename presentation</span>
                </button>`;
            const actions = `
                <div class="operator__presentation-actions">
                  ${renameAction}
                </div>`;
            return `
              <li class="operator__presentation-item${active}" data-role="presentation-item" data-type="presentation" data-presentation-id="${presentation.id}" draggable="true">
                <span>${escapeHtml(presentation.name)}</span>
                <span class="operator__presentation-meta">${escapeHtml(libraryName || '')}</span>
                ${actions}
              </li>
            `;
          })
          .join('');
      }
    } else if (state.activePlaylistId) {
      const playlist = state.playlists.find((item) => item.id === state.activePlaylistId);
      if (!playlist || playlist.entries.length === 0) {
        html = '<li class="empty">Playlist is empty. Drag songs from a library or add a separator with the + button.</li>';
        count = 0;
      } else {
        count = playlist.entries.length;
        html = playlist.entries
          .map((entry, index) => {
            if (entry.entryType === 'separator') {
              const actions = isEditMode
                ? `<div class="operator__presentation-actions">
                    <button type="button" class="operator__presentation-action" data-action="separator-rename" data-playlist-id="${playlist.id}" data-entry-id="${entry.entryId}" title="Rename separator">
                      <span aria-hidden="true">✎</span>
                      <span class="sr-only">Rename separator</span>
                    </button>
                  </div>`
                : '';
              return `
                <li class="operator__presentation-item" data-role="presentation-item" data-type="separator" data-entry-index="${index}" data-entry-id="${entry.entryId}">
                  <span>${escapeHtml(entry.name)}</span>
                  <span class="operator__presentation-meta">Separator</span>
                  ${actions}
                </li>
              `;
            }
            const presentationId = entry.presentationId;
            const meta = presentationId ? presentationIndex.get(presentationId) : null;
            const label = meta ? meta.libraryName : 'Unknown library';
            const active = presentationId && state.currentPresentationId === presentationId ? ' is-active' : '';
            const renameAction = presentationId
              ? `<button type="button" class="operator__presentation-action" data-action="presentation-rename" data-presentation-id="${presentationId || ''}" data-library-id="${meta ? meta.libraryId || '' : ''}" title="Rename presentation">
                    <span aria-hidden="true">✎</span>
                    <span class="sr-only">Rename presentation</span>
                  </button>`
              : '';
            const removeAction = `<button type="button" data-action="playlist-remove" title="Remove from playlist">×</button>`;
            const hasActions = Boolean(renameAction || removeAction);
            const actions = hasActions
              ? `<div class="operator__presentation-actions">
                  ${renameAction}
                  ${removeAction}
                </div>`
              : '';
            return `
              <li class="operator__presentation-item${active}" data-role="presentation-item" data-type="presentation" data-entry-id="${entry.entryId}" data-presentation-id="${presentationId || ''}" data-entry-index="${index}" draggable="true">
                <span>${escapeHtml(entry.name)}</span>
                <span class="operator__presentation-meta">${escapeHtml(label || '')}</span>
                ${actions}
              </li>
            `;
          })
          .join('');
      }
    } else {
      html = '<li class="empty">Select a library or playlist to view presentations.</li>';
      count = 0;
    }

    container.innerHTML = html;
    if (els.presentationCount) {
      els.presentationCount.textContent = count.toString();
    }
    updateActivePresentationIndicators();
  }

  function scrollPresentationIntoView(presentationId) {
    if (!presentationId || !els.presentationList) {
      return;
    }
    const target = els.presentationList.querySelector(
      `[data-role="presentation-item"][data-presentation-id="${presentationId}"]`
    );
    if (target && typeof target.scrollIntoView === 'function') {
      target.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }
  }

  function scrollSlideIntoView(slideId) {
    if (!slideId || !els.slides) {
      return;
    }
    const card = els.slides.querySelector(`[data-slide-id="${slideId}"]`);
    if (card && typeof card.scrollIntoView === 'function') {
      card.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }
  }


  function updateActivePresentationIndicators() {
    if (!els.presentationList) return;
    const items = qsa('[data-role="presentation-item"]', els.presentationList);
    items.forEach((item) => {
      const presentationId = item.dataset.presentationId;
      const isActive = state.currentPresentationId === presentationId;
      const isStage = state.stagePresentationId === presentationId;
      item.classList.toggle('is-active', isActive);
      item.classList.toggle('is-stage-active', isStage);
    });
  }

  function updateSlidesPlaceholder(presentationId) {
    if (!els.slides) return;
    els.slides.setAttribute('data-slides-placeholder', presentationId || '');
  }

  function getSlidesForPresentation(presentationId) {
    return state.slidesCache.get(presentationId) || [];
  }

  function renderSlides(presentationId) {
    if (!els.slides) return;
    applySlideSize();
    const slides = getSlidesForPresentation(presentationId);
    updateSlidesPlaceholder(presentationId);
    if (!slides || slides.length === 0) {
      els.slides.innerHTML = '<p class="empty">No slides yet. Use "Add Slide" to create one.</p>';
      return;
    }
    const lintEnabled = state.mode === 'edit';
    const html = slides
      .map((slide, index) => {
        const { content } = slide;
        const active = state.stagePresentationId === presentationId && state.stageSlideId === slide.id ? ' is-active' : '';
        const focused = state.focusedSlideId === slide.id ? ' is-focused' : '';
        const effectiveGroup = (slide.effectiveGroup || '').trim();
        const hasExplicitGroup = Boolean(slide.explicitGroup && slide.explicitGroup.trim());
        const groupPlaceholder = !hasExplicitGroup && effectiveGroup
          ? ` placeholder="${escapeHtml(effectiveGroup)}"`
          : '';
        const mainLint = lintField(content.main.value);
        const translationLint = lintField(content.translation.value);
        const hasWarning = mainLint.hasWarning || translationLint.hasWarning;
        const warningMessage = lintEnabled ? buildWarningMessage(mainLint, translationLint) : '';
        const cardWarningAttr = hasWarning ? ' data-warning="true"' : '';
        const warningMarkup = lintEnabled
          ? `<div class="operator__slide-warning" data-role="slide-warning" data-visible="${warningMessage ? 'true' : 'false'}">${warningMessage ? escapeHtml(warningMessage) : ''}</div>`
          : '';
        const mainWarningClass = lintEnabled && mainLint.hasWarning ? ' is-warning' : '';
        const translationWarningClass = lintEnabled && translationLint.hasWarning ? ' is-warning' : '';
        const mainWarningAttr = lintEnabled && mainLint.hasWarning ? ' data-warning="true"' : '';
        const translationWarningAttr = lintEnabled && translationLint.hasWarning ? ' data-warning="true"' : '';
        const controlsMarkup = state.mode === 'edit'
          ? `<div class="operator__slide-controls">
              <button type="button" tabindex="-1" data-action="save" title="Save slide">Save</button>
              <button type="button" tabindex="-1" data-action="duplicate" title="Duplicate slide">Duplicate</button>
              <button type="button" tabindex="-1" data-action="delete" title="Delete slide">Delete</button>
            </div>`
          : '';
        const footerMarkup = '';
        const showGroup = state.mode === 'live' && Boolean(effectiveGroup);
        const groupBadge = `<div class="operator__slide-group" data-role="slide-group" data-hidden="${showGroup ? 'false' : 'true'}">${showGroup ? escapeHtml(effectiveGroup) : ''}</div>`;
        const highlightOptions = { highlightOverflow: lintEnabled };
        const mainTextHtml = formatMultiline(content.main.value, mainLint, highlightOptions);
        const translationTextHtml = formatMultiline(
          content.translation.value,
          translationLint,
          highlightOptions,
        );
        const stageTextHtml = formatMultiline(content.stage.value);
        const overLimitBadge = !lintEnabled && hasWarning
          ? `<span class="operator__slide-warning-dot" role="img" aria-label="Slide exceeds configured limits" title="Slide exceeds configured limits">&#9888;</span>`
          : '';
        const editorMarkup = state.mode === 'edit'
          ? `<div class="operator__slide-editor">
                <label>
                  <span>Main</span>
                  <textarea data-field="main" rows="2"${mainWarningAttr}>${escapeHtml(content.main.value)}</textarea>
                </label>
                <label>
                  <span>Translation</span>
                  <textarea data-field="translation" rows="2"${translationWarningAttr}>${escapeHtml(content.translation.value)}</textarea>
                </label>
                <label>
                   <span>Stage</span>
                  <textarea data-field="stage" rows="2">${escapeHtml(content.stage.value)}</textarea>
                </label>
                <label>
                  <span>Group</span>
                  <input type="text" data-field="group" value="${hasExplicitGroup ? escapeHtml(slide.explicitGroup) : ''}"${groupPlaceholder} />
                </label>
              </div>`
          : '';
        return `
          <article class="operator__slide-card stage-control__slide${active}${focused}" data-slide-id="${slide.id}" data-slide-index="${index}" data-group-inherited="${hasExplicitGroup ? 'false' : 'true'}"${cardWarningAttr}>
            <header class="operator__slide-header">
              <div class="operator__slide-header-left">
                <button type="button" class="operator__slide-handle" data-role="slide-drag-handle" draggable="true" tabindex="-1" aria-label="Reorder slide">↕</button>
                <span class="operator__slide-index">${index + 1}${overLimitBadge}</span>
              </div>
              ${controlsMarkup}
            </header>
            <section class="operator__slide-bodies">
              <div class="operator__slide-text operator__slide-text--main${mainWarningClass}" data-field-display="main" data-warning="${lintEnabled && mainLint.hasWarning ? 'true' : 'false'}">${mainTextHtml}</div>
              <div class="operator__slide-text operator__slide-text--translation${translationWarningClass}" data-field-display="translation" data-warning="${lintEnabled && translationLint.hasWarning ? 'true' : 'false'}">${translationTextHtml}</div>
              <div class="operator__slide-text operator__slide-text--stage" data-field-display="stage">${stageTextHtml}</div>
              ${warningMarkup}
              ${groupBadge}
              ${editorMarkup}
            </section>
            ${footerMarkup}
          </article>
        `;
    })
    .join('');
    els.slides.innerHTML = html;
    if (lintEnabled) {
      repaintSlideWarnings();
    }
    updateActiveSlideIndicators();
    renderStageStatus();
    restorePendingFocus();
  }

function updateActiveSlideIndicators() {
  if (!els.slides) return;
  const cards = qsa('[data-slide-id]', els.slides);
  cards.forEach((card) => {
    const slideId = card.dataset.slideId;
    const presentationId = state.currentPresentationId;
    const isActive = presentationId && state.stagePresentationId === presentationId && state.stageSlideId === slideId;
    card.classList.toggle('is-active', Boolean(isActive));
    card.classList.toggle('is-focused', state.focusedSlideId === slideId);
  });
}

function restorePendingFocus() {
  if (!state.pendingFocus || state.mode !== 'edit') {
    state.pendingFocus = null;
    return;
  }
  const target = state.pendingFocus;
  const slideId = target.slideId;
  const field = target.field;
  state.pendingFocus = null;
  if (!slideId || !field || !els.slides) {
    return;
  }
  const card = els.slides.querySelector(`[data-slide-id="${slideId}"]`);
  if (!card) {
    return;
  }
  const control = card.querySelector(`[data-field="${field}"]`);
  if (!control) {
    return;
  }
  const caretMode = target.caret || 'end';
  if (typeof control.focus === 'function') {
    control.focus({ preventScroll: false });
  }
  if (typeof control.setSelectionRange === 'function') {
    if (caretMode === 'preserve') {
      const start = typeof target.selectionStart === 'number' ? target.selectionStart : control.value.length;
      const end = typeof target.selectionEnd === 'number' ? target.selectionEnd : start;
      control.setSelectionRange(start, end);
    } else {
      const length = control.value.length;
      control.setSelectionRange(length, length);
    }
  }
}

function repaintSlideWarnings() {
  if (!els.slides) return;
  qsa('[data-slide-id]', els.slides).forEach((card) => updateCardWarnings(card));
}

function updateCardWarnings(card) {
  if (!card) return;
  if (state.mode !== 'edit') {
    card.dataset.warning = 'false';
    const warningEl = card.querySelector('[data-role="slide-warning"]');
    if (warningEl) {
      warningEl.dataset.visible = 'false';
      warningEl.textContent = '';
    }
    const mainTextEl = card.querySelector('[data-field-display="main"]');
    if (mainTextEl) {
      mainTextEl.classList.remove('is-warning');
      mainTextEl.dataset.warning = 'false';
    }
    const translationTextEl = card.querySelector('[data-field-display="translation"]');
    if (translationTextEl) {
      translationTextEl.classList.remove('is-warning');
      translationTextEl.dataset.warning = 'false';
    }
    const mainInput = card.querySelector('[data-field="main"]');
    if (mainInput) {
      mainInput.removeAttribute('data-warning');
    }
    const translationInput = card.querySelector('[data-field="translation"]');
    if (translationInput) {
      translationInput.removeAttribute('data-warning');
    }
    return;
  }
  const mainInput = card.querySelector('[data-field="main"]');
  const translationInput = card.querySelector('[data-field="translation"]');
  const stageInput = card.querySelector('[data-field="stage"]');
  const mainValue = mainInput ? mainInput.value : card.querySelector('[data-field-display="main"]')?.textContent || '';
  const translationValue = translationInput
    ? translationInput.value
    : card.querySelector('[data-field-display="translation"]')?.textContent || '';
  const stageValue = stageInput
    ? stageInput.value
    : card.querySelector('[data-field-display="stage"]')?.textContent || '';

  const mainLint = lintField(mainValue);
  const translationLint = lintField(translationValue);
  const warningMessage = buildWarningMessage(mainLint, translationLint);

  card.dataset.warning = warningMessage ? 'true' : 'false';

  const mainTextEl = card.querySelector('[data-field-display="main"]');
  if (mainTextEl) {
    mainTextEl.innerHTML = formatMultiline(mainValue, mainLint, { highlightOverflow: true });
    mainTextEl.dataset.warning = mainLint.hasWarning ? 'true' : 'false';
    mainTextEl.classList.toggle('is-warning', mainLint.hasWarning);
  }

  const translationTextEl = card.querySelector('[data-field-display="translation"]');
  if (translationTextEl) {
    translationTextEl.innerHTML = formatMultiline(
      translationValue,
      translationLint,
      { highlightOverflow: true },
    );
    translationTextEl.dataset.warning = translationLint.hasWarning ? 'true' : 'false';
    translationTextEl.classList.toggle('is-warning', translationLint.hasWarning);
  }

  const stageTextEl = card.querySelector('[data-field-display="stage"]');
  if (stageTextEl) {
    stageTextEl.innerHTML = formatMultiline(stageValue);
  }

  if (mainInput) {
    if (mainLint.hasWarning) {
      mainInput.setAttribute('data-warning', 'true');
    } else {
      mainInput.removeAttribute('data-warning');
    }
  }

  if (translationInput) {
    if (translationLint.hasWarning) {
      translationInput.setAttribute('data-warning', 'true');
    } else {
      translationInput.removeAttribute('data-warning');
    }
  }

  const warningEl = card.querySelector('[data-role="slide-warning"]');
  if (warningEl) {
    if (warningMessage) {
      warningEl.dataset.visible = 'true';
      warningEl.textContent = warningMessage;
    } else {
      warningEl.dataset.visible = 'false';
      warningEl.textContent = '';
    }
  }
}

  function normaliseLibrary(raw) {
    if (!raw) return null;
    const presentations = (raw.presentations || []).map((presentation) => ({
      id: presentation.id || presentation.presentationId || presentation.presentation_id,
      name: presentation.name || 'Untitled presentation',
    }));
    return {
      id: raw.id,
      name: raw.name,
      presentations,
      presentation_count: Array.isArray(raw.presentations)
        ? raw.presentations.length
        : raw.presentation_count || presentations.length,
      isFavorite: Boolean(raw.is_favorite ?? raw.isFavorite ?? false),
    };
  }

  function normalisePlaylist(raw) {
    if (!raw) return null;
    const entries = (raw.entries || []).map((entry) => {
      const entryId = entry.entryId || entry.entry_id || entry.entryid || entry.id;
      const entryType = String(entry.type || entry.entryType || 'presentation').toLowerCase();
      const presentationId = entry.presentationId || entry.presentation_id || null;
      const baseName = entry.name || (presentationId ? (presentationIndex.get(presentationId) || {}).name : null);
      return {
        entryId,
        entryType,
        presentationId: presentationId || null,
        name: baseName || (entryType === 'separator' ? 'Separator' : 'Untitled'),
      };
    });
    return {
      id: raw.id,
      name: raw.name || 'Untitled playlist',
      entries,
      showInDashboard: Boolean(
        raw.showInDashboard ?? raw.show_in_dashboard ?? false,
      ),
    };
  }

  function indexPlaylists() {
    const lookup = new Map();
    const presentationMap = new Map();
    state.playlists.forEach((playlist) => {
      lookup.set(playlist.id, playlist);
      (playlist.entries || []).forEach((entry, index) => {
        if (entry.entryType === 'presentation' && entry.presentationId) {
          if (!presentationMap.has(entry.presentationId)) {
            presentationMap.set(entry.presentationId, {
              playlistId: playlist.id,
              entryIndex: index,
            });
          }
        }
      });
    });
    state.playlistLookup = lookup;
    state.presentationPlaylistIndex = presentationMap;
  }

  function upsertPlaylist(playlist) {
    if (!playlist) return false;
    const index = state.playlists.findIndex((item) => item.id === playlist.id);
    if (index >= 0) {
      state.playlists.splice(index, 1, playlist);
    } else {
      state.playlists.push(playlist);
    }
    indexPlaylists();
    return true;
  }

  function removePlaylistFromState(playlistId) {
    const index = state.playlists.findIndex((item) => item.id === playlistId);
    if (index >= 0) {
      state.playlists.splice(index, 1);
      if (state.activePlaylistId === playlistId) {
        state.activePlaylistId = null;
        state.currentPresentationId = null;
        state.focusedSlideId = null;
      }
      indexPlaylists();
    }
  }

  function refreshPlaylistState(updated) {
    const playlist = normalisePlaylist(updated);
    if (!upsertPlaylist(playlist)) {
      return;
    }
    renderPlaylists();
    if (playlist && state.activePlaylistId === playlist.id) {
      updateContextTitleFromPlaylist(playlist.id);
      renderPresentationList();
    }
  }

  function apiFetch(path, options) {
    const controller = new AbortController();
    const headers = {
      'Content-Type': 'application/json',
      Accept: 'application/json',
    };
    const merged = Object.assign({ method: 'GET', headers }, options || {}, {
      headers: Object.assign(headers, options && options.headers ? options.headers : {}),
      signal: controller.signal,
    });
    const url = path.startsWith('http') ? path : `${window.location.origin}${path}`;
    const promise = fetch(url, merged).then(async (response) => {
      if (!response.ok) {
        const text = await response.text();
        throw new Error(text || `Request failed with ${response.status}`);
      }
      const contentType = response.headers.get('content-type') || '';
      if (contentType.includes('application/json')) {
        return response.json();
      }
      return null;
    });
    promise.cancel = () => controller.abort();
    return promise;
  }

  async function loadPresentation(presentationId) {
    if (state.slidesCache.has(presentationId)) {
      state.currentPresentationId = presentationId;
      renderSlides(presentationId);
      return;
    }
    if (state.slideFetchAbort) {
      state.slideFetchAbort.abort();
      state.slideFetchAbort = null;
    }
    const controller = new AbortController();
    state.slideFetchAbort = controller;
    try {
      const detail = await fetch(`/presentations/${presentationId}`, {
        method: 'GET',
        headers: { Accept: 'application/json' },
        signal: controller.signal,
      }).then(async (response) => {
        if (!response.ok) {
          const text = await response.text();
          throw new Error(text || 'Failed to load presentation');
        }
        return response.json();
      });
      const presentation = detail.presentation;
      presentationIndex.set(presentation.id, {
        id: presentation.id,
        name: presentation.name,
        libraryId: detail.libraryId,
        libraryName: detail.libraryName,
      });
      state.slidesCache.set(
        presentation.id,
        normaliseSlides(presentation.slides || [])
      );
      state.presentationMeta.set(presentation.id, presentation);
      state.currentPresentationId = presentation.id;
      state.focusedSlideId = null;
      renderSlides(presentation.id);
      renderStageStatus();
    } catch (error) {
      if (error.name === 'AbortError') {
        return;
      }
      console.error('Failed to load presentation', error);
      showToast('Failed to load presentation', 'error');
    } finally {
      state.slideFetchAbort = null;
    }
  }

  function computeNextSlideId(presentationId, slideId) {
    const slides = getSlidesForPresentation(presentationId);
    if (!slides.length) return null;
    const index = slides.findIndex((slide) => slide.id === slideId);
    if (index < 0) return null;
    const next = slides[index + 1];
    return next ? next.id : null;
  }

  function navigateSlides(offset) {
    if (!offset) return;
    const presentationId = state.currentPresentationId || state.stagePresentationId;
    if (!presentationId) return;
    if (!state.slidesCache.has(presentationId)) {
      loadPresentation(presentationId)
        .then(() => navigateSlides(offset))
        .catch((error) => console.error('Failed to preload presentation for navigation', error));
      return;
    }
    const slides = getSlidesForPresentation(presentationId);
    if (!slides.length) return;
    let currentSlideId = null;
    if (state.stagePresentationId === presentationId && state.stageSlideId) {
      currentSlideId = state.stageSlideId;
    } else if (state.focusedSlideId) {
      currentSlideId = state.focusedSlideId;
    }
    if (!currentSlideId) {
      currentSlideId = offset > 0 ? slides[0].id : slides[slides.length - 1].id;
    }
    let currentIndex = slides.findIndex((slide) => slide.id === currentSlideId);
    if (currentIndex === -1) {
      currentIndex = offset > 0 ? -1 : slides.length;
    }
    const targetIndex = currentIndex + offset;
    if (targetIndex < 0 || targetIndex >= slides.length) {
      return;
    }
    const targetSlide = slides[targetIndex];
    state.currentPresentationId = presentationId;
    state.focusedSlideId = targetSlide.id;
    const card = els.slides
      ? els.slides.querySelector(`[data-slide-id="${targetSlide.id}"]`)
      : null;
    triggerSlide(presentationId, targetSlide.id, card);
  }

  function serialisePlaylistEntries(entries) {
    if (!Array.isArray(entries)) return [];
    return entries.map((entry) => {
      if (!entry) {
        return { type: 'presentation' };
      }
      if (entry.entryType === 'separator') {
        return {
          type: 'separator',
          entryId: entry.entryId || null,
          name: entry.name || 'Separator',
        };
      }
      return {
        type: 'presentation',
        entryId: entry.entryId || null,
        presentationId: entry.presentationId || entry.presentation_id || entry.id || null,
      };
    });
  }

  async function triggerSlide(presentationId, slideId, card) {
    if (!presentationId || !slideId) return;
    if (card) {
      card.classList.add('is-loading');
    }
    const nextSlideId = computeNextSlideId(presentationId, slideId);
    try {
      await apiFetch('/stage/state', {
        method: 'POST',
        body: JSON.stringify({
          presentationId,
          currentSlideId: slideId,
          nextSlideId,
        }),
      });
      state.stagePresentationId = presentationId;
      state.stageSlideId = slideId;
      state.focusedSlideId = slideId;
      const slides = getSlidesForPresentation(presentationId);
      if (slides.length) {
        const index = slides.findIndex((slide) => slide.id === slideId);
        const currentSlide = index >= 0 ? slides[index] : slides[0];
        const followingSlide = index >= 0 ? slides[index + 1] || null : slides[1] || null;
        state.stageSnapshot = {
          presentationId,
          presentationName: presentationIndex.get(presentationId)?.name || '',
          current: currentSlide
            ? {
                main: extractField(currentSlide, 'main'),
                translation: extractField(currentSlide, 'translation'),
                stage: extractField(currentSlide, 'stage'),
                group: extractGroup(currentSlide) || null,
              }
            : null,
          next: followingSlide
            ? {
                main: extractField(followingSlide, 'main'),
                translation: extractField(followingSlide, 'translation'),
                stage: extractField(followingSlide, 'stage'),
                group: extractGroup(followingSlide) || null,
              }
            : null,
          timers: state.timers,
          latencyMs: null,
          currentPosition: index >= 0 ? index + 1 : null,
          totalSlides: slides.length ? slides.length : null,
        };
      }
      updateActivePresentationIndicators();
      updateActiveSlideIndicators();
      renderStageStatus();
      renderAbleSetPanel();
    } catch (error) {
      console.error('Failed to trigger slide', error);
      showToast('Failed to trigger slide', 'error');
    } finally {
      if (card) {
        card.classList.remove('is-loading');
        card.classList.add('is-active');
      }
    }
  }

  async function clearActiveSlide() {
    if (state.clearingSlide) return;
    state.clearingSlide = true;
    updateClearSlideAvailability();
    try {
      await apiFetch('/stage/clear', { method: 'POST' });
      state.stagePresentationId = null;
      state.stageSlideId = null;
      state.stageSnapshot = {
        presentationId: null,
        presentationName: '',
        current: null,
        next: null,
        timers: state.timers,
        latencyMs: null,
        currentPosition: null,
        totalSlides: null,
      };
      updateActivePresentationIndicators();
      updateActiveSlideIndicators();
      renderStageStatus();
      renderAbleSetPanel();
      showToast('Slide outputs cleared', 'success');
    } catch (error) {
      console.error('Failed to clear slide', error);
      showToast('Failed to clear slide', 'error');
    } finally {
      state.clearingSlide = false;
      updateClearSlideAvailability();
    }
  }

  function saveSlide(presentationId, slideId, card) {
    if (!presentationId || !slideId || !card) return;
    const mainInput = card.querySelector('[data-field="main"]');
    const translationInput = card.querySelector('[data-field="translation"]');
    const stageInput = card.querySelector('[data-field="stage"]');
    const groupInput = card.querySelector('[data-field="group"]');
    const payload = {
      main: mainInput ? mainInput.value : '',
      translation: translationInput ? translationInput.value : '',
      stage: stageInput ? stageInput.value : '',
      group: groupInput && groupInput.value ? groupInput.value : null,
    };
    const activeElement = document.activeElement;
    if (activeElement && card.contains(activeElement) && activeElement.matches('[data-field]')) {
      const value = typeof activeElement.value === 'string' ? activeElement.value : '';
      const start = typeof activeElement.selectionStart === 'number' ? activeElement.selectionStart : value.length;
      const end = typeof activeElement.selectionEnd === 'number' ? activeElement.selectionEnd : start;
      state.pendingFocus = {
        slideId,
        field: activeElement.dataset.field || 'main',
        caret: 'preserve',
        selectionStart: start,
        selectionEnd: end,
      };
    } else {
      state.pendingFocus = {
        slideId,
        field: 'main',
        caret: 'end',
      };
    }
    updateSlideContent(presentationId, slideId, payload);
    showToast('Slide saved', 'success');
  }

  async function updateSlideContent(presentationId, slideId, payload) {
    try {
      const updated = await apiFetch(`/presentations/${presentationId}/slides/${slideId}`, {
        method: 'PATCH',
        body: JSON.stringify(payload),
      });
      const slides = getSlidesForPresentation(presentationId).map((slide) =>
        slide.id === slideId ? Object.assign({}, slide, { content: updated.content }) : slide
      );
      const normalised = normaliseSlides(slides);
      state.slidesCache.set(presentationId, normalised);
      renderSlides(presentationId);
    } catch (error) {
      console.error('Failed to update slide', error);
      showToast('Failed to update slide', 'error');
    }
  }

  async function insertSlide(presentationId, position) {
    try {
      const response = await apiFetch(`/presentations/${presentationId}/slides`, {
        method: 'POST',
        body: JSON.stringify({ position }),
      });
      const normalised = normaliseSlides(response);
      state.slidesCache.set(presentationId, normalised);
      const inserted = normalised[position || normalised.length - 1] || null;
      state.focusedSlideId = inserted ? inserted.id : null;
      if (inserted) {
        state.pendingFocus = {
          slideId: inserted.id,
          field: 'main',
          caret: 'end',
        };
      }
      renderSlides(presentationId);
      showToast('Slide added', 'success');
    } catch (error) {
      console.error('Failed to create slide', error);
      showToast('Failed to create slide', 'error');
    }
  }

  async function duplicateSlide(presentationId, slideId) {
    try {
      const response = await apiFetch(`/presentations/${presentationId}/slides/${slideId}/duplicate`, {
        method: 'POST',
      });
      const normalised = normaliseSlides(response);
      state.slidesCache.set(presentationId, normalised);
      const index = normalised.findIndex((slide) => slide.id === slideId);
      const duplicate = normalised[index + 1];
      state.focusedSlideId = duplicate ? duplicate.id : slideId;
      if (duplicate) {
        state.pendingFocus = {
          slideId: duplicate.id,
          field: 'main',
          caret: 'end',
        };
      }
      renderSlides(presentationId);
      showToast('Slide duplicated', 'success');
    } catch (error) {
      console.error('Failed to duplicate slide', error);
      showToast('Failed to duplicate slide', 'error');
    }
  }

  async function deleteSlide(presentationId, slideId) {
    try {
      const response = await apiFetch(`/presentations/${presentationId}/slides/${slideId}`, {
        method: 'DELETE',
      });
      const normalised = normaliseSlides(response);
      state.slidesCache.set(presentationId, normalised);
      state.focusedSlideId = null;
      renderSlides(presentationId);
      showToast('Slide deleted', 'success');
    } catch (error) {
      console.error('Failed to delete slide', error);
      showToast('Failed to delete slide', 'error');
    }
  }

  async function reorderSlides(presentationId, ordered) {
    try {
      const response = await apiFetch(`/presentations/${presentationId}/slides/reorder`, {
        method: 'POST',
        body: JSON.stringify({ slideIds: ordered }),
      });
      const normalised = normaliseSlides(response);
      state.slidesCache.set(presentationId, normalised);
      renderSlides(presentationId);
    } catch (error) {
      console.error('Failed to reorder slides', error);
      showToast('Failed to reorder slides', 'error');
    }
  }

  async function reorderPlaylistEntries(playlistId, orderedEntryIds) {
    if (!playlistId || !Array.isArray(orderedEntryIds) || !orderedEntryIds.length) {
      return;
    }
    const playlist = state.playlists.find((item) => item.id === playlistId) || state.playlistLookup.get(playlistId);
    if (!playlist) return;
    const reordered = orderedEntryIds
      .map((entryId) => playlist.entries.find((entry) => entry.entryId === entryId))
      .filter(Boolean);
    if (reordered.length !== playlist.entries.length) {
      console.warn('Mismatch in playlist reorder payload; aborting', {
        orderedEntryIds,
        playlistEntries: playlist.entries.map((entry) => entry.entryId),
      });
      renderPresentationList();
      return;
    }
    try {
      const response = await apiFetch(`/playlists/${playlistId}/entries`, {
        method: 'PUT',
        body: JSON.stringify({ entries: serialisePlaylistEntries(reordered) }),
      });
      refreshPlaylistState(response);
      showToast('Playlist order updated', 'success');
    } catch (error) {
      console.error('Failed to reorder playlist', error);
      showToast('Failed to reorder playlist', 'error');
      renderPresentationList();
    }
  }

  async function handlePlaylistInsertion(
    presentationId,
    playlistId,
    insertIndex = null,
    options = {},
  ) {
    if (!playlistId) {
      showToast('Select a playlist before adding presentations.', 'warning');
      state.draggingFromSearch = false;
      state.draggingPresentationId = null;
      return;
    }
    const playlist = state.playlists.find((item) => item.id === playlistId) || state.playlistLookup.get(playlistId);
    if (!playlist) {
      showToast('Playlist not found.', 'error');
      return;
    }
    const entries = playlist.entries.slice();
    const insertionPoint = Number.isInteger(insertIndex)
      ? Math.min(Math.max(insertIndex, 0), entries.length)
      : entries.length;
    entries.splice(insertionPoint, 0, {
      entryId: null,
      entryType: 'presentation',
      presentationId,
      name: presentationIndex.get(presentationId)?.name || 'Untitled',
    });
    try {
      const response = await apiFetch(`/playlists/${playlist.id || playlistId}/entries`, {
        method: 'PUT',
        body: JSON.stringify({ entries: serialisePlaylistEntries(entries) }),
      });
      refreshPlaylistState(response);
      showToast('Presentation added to playlist', 'success');
      if (options && options.clearSearch) {
        clearSearchResults();
      }
    } catch (error) {
      console.error('Failed to add to playlist', error);
      showToast('Failed to add to playlist', 'error');
    }
  }

  async function handleCreatePresentation() {
    if (!state.activeLibraryId) {
      showToast('Select a library first', 'warning');
      return;
    }
    const defaultName = `New Presentation ${new Date().toLocaleTimeString([], {
      hour: '2-digit',
      minute: '2-digit',
    })}`;
    const nameInput = window.prompt('Presentation name', defaultName);
    if (nameInput === null) {
      return;
    }
    const trimmed = nameInput.trim() || defaultName;
    try {
      const response = await apiFetch(`/libraries/${state.activeLibraryId}/presentations`, {
        method: 'POST',
        body: JSON.stringify({ name: trimmed }),
      });
      if (!response || !response.presentation) {
        throw new Error('Unexpected response creating presentation');
      }
      const libraryId = response.libraryId || state.activeLibraryId;
      if (response.librarySummary) {
        state.libraries = state.libraries
          .filter((library) => library.id !== response.librarySummary.id)
          .concat(response.librarySummary)
          .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
      }
      const presentation = response.presentation;
      presentationIndex.set(presentation.id, {
        id: presentation.id,
        name: presentation.name,
        libraryId,
        libraryName:
          response.librarySummary?.name || presentation.libraryName || presentation.library_name || '',
      });
      state.slidesCache.set(presentation.id, normaliseSlides(presentation.slides || []));
      rebuildPresentationIndex();
      renderLibraries();
      state.activeLibraryId = libraryId;
      state.currentPresentationId = presentation.id;
      state.focusedSlideId = null;
      renderPresentationList();
      loadPresentation(presentation.id);
      showToast('Presentation created', 'success');
    } catch (error) {
      console.error('Failed to create presentation', error);
      showToast('Failed to create presentation', 'error');
    }
  }

  async function handleAddSeparator() {
    if (!state.activePlaylistId) {
      showToast('Select a playlist first', 'warning');
      return;
    }
    const playlist =
      state.playlists.find((item) => item.id === state.activePlaylistId) ||
      state.playlistLookup.get(state.activePlaylistId);
    if (!playlist) {
      showToast('Playlist not found', 'error');
      return;
    }
    const nameInput = window.prompt('Separator name', 'Section');
    if (nameInput === null) {
      return;
    }
    const label = nameInput.trim() || 'Section';
    const entries = playlist.entries.slice();
    entries.push({
      entryId: null,
      entryType: 'separator',
      name: label,
    });
    try {
      const response = await apiFetch(`/playlists/${playlist.id}/entries`, {
        method: 'PUT',
        body: JSON.stringify({ entries: serialisePlaylistEntries(entries) }),
      });
      refreshPlaylistState(response);
      showToast('Separator added', 'success');
    } catch (error) {
      console.error('Failed to add separator', error);
      showToast('Failed to add separator', 'error');
    }
  }

  function applySlideSize() {
    if (els.slides) {
      els.slides.dataset.size = 'medium';
    }
  }

  function applyCatalogHeight() {
    if (!els.catalog) return;
    const height = Math.min(
      Math.max(Math.round(state.catalogTopHeight), CATALOG_MIN_HEIGHT),
      CATALOG_MAX_HEIGHT,
    );
    state.catalogTopHeight = height;
    els.catalog.style.setProperty('--catalog-top-size', `${height}px`);
  }

  function handleCatalogResizePointerDown(event) {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    state.catalogResizeActive = true;
    state.catalogResizePointerId = event.pointerId;
    state.catalogResizeStartY = event.clientY;
    state.catalogResizeStartHeight = state.catalogTopHeight;
    if (els.catalogResizer && typeof els.catalogResizer.setPointerCapture === 'function') {
      try {
        els.catalogResizer.setPointerCapture(event.pointerId);
      } catch (error) {
        console.warn('failed to capture pointer for catalog resize', error);
      }
    }
    document.addEventListener('pointermove', handleCatalogResizePointerMove);
    document.addEventListener('pointerup', handleCatalogResizePointerUp);
  }

  function handleCatalogResizePointerMove(event) {
    if (!state.catalogResizeActive) {
      return;
    }
    const delta = event.clientY - state.catalogResizeStartY;
    const next = Math.min(
      Math.max(state.catalogResizeStartHeight + delta, CATALOG_MIN_HEIGHT),
      CATALOG_MAX_HEIGHT,
    );
    if (Math.abs(next - state.catalogTopHeight) >= 1) {
      state.catalogTopHeight = next;
      applyCatalogHeight();
    }
  }

  function handleCatalogResizePointerUp(event) {
    if (!state.catalogResizeActive) {
      return;
    }
    if (
      state.catalogResizePointerId !== null &&
      event.pointerId !== undefined &&
      event.pointerId !== state.catalogResizePointerId
    ) {
      return;
    }

    state.catalogResizeActive = false;
    state.catalogResizePointerId = null;
    if (els.catalogResizer && typeof els.catalogResizer.releasePointerCapture === 'function') {
      try {
        els.catalogResizer.releasePointerCapture(event.pointerId);
      } catch (error) {
        console.warn('failed to release pointer capture', error);
      }
    }
    document.removeEventListener('pointermove', handleCatalogResizePointerMove);
    document.removeEventListener('pointerup', handleCatalogResizePointerUp);
    try {
      window.localStorage.setItem('presenter.catalogTopHeight', String(Math.round(state.catalogTopHeight)));
    } catch (error) {
      console.warn('failed to persist catalog height', error);
    }
  }

  function parseLineLimitValue(raw) {
    const numeric = Number(raw);
    if (!Number.isFinite(numeric)) {
      return null;
    }
    const rounded = Math.round(numeric);
    return Math.min(Math.max(rounded, 10), 120);
  }

  function handleLineLimitPreview(event) {
    const value = parseLineLimitValue(event.target.value);
    if (value === null) {
      return;
    }
    if (value !== state.lineLimit) {
      state.lineLimit = value;
      updateLineLimitStyle();
      repaintSlideWarnings();
    }
  }

  function handleLineLimitChange(event) {
    const previous = state.lineLimit;
    const value = parseLineLimitValue(event.target.value);
    const finalValue = value === null ? previous : value;
    state.lineLimit = finalValue;
    event.target.value = String(finalValue);
    updateLineLimitStyle();
    repaintSlideWarnings();
    if (finalValue !== previous) {
      persistLineLimit(finalValue, previous, event.target);
    }
  }

  async function removePlaylistEntry(index) {
    if (!state.activePlaylistId) return;
    const playlist = state.playlists.find((item) => item.id === state.activePlaylistId);
    if (!playlist) return;
    const entries = playlist.entries.slice();
    entries.splice(index, 1);
    try {
      const response = await apiFetch(`/playlists/${playlist.id}/entries`, {
        method: 'PUT',
        body: JSON.stringify({ entries: serialisePlaylistEntries(entries) }),
      });
      refreshPlaylistState(response);
      showToast('Removed from playlist', 'success');
    } catch (error) {
      console.error('Failed to remove playlist entry', error);
      showToast('Failed to update playlist', 'error');
    }
  }

  async function deleteLibrary(libraryId, options = {}) {
    const library = state.libraries.find((item) => item.id === libraryId);
    if (!library) return;
    const count = Array.isArray(library.presentations) ? library.presentations.length : library.presentation_count || 0;
    if (!options.skipConfirm) {
      const confirmed = window.confirm(
        `Delete library "${library.name}"? This will remove ${count} presentation${count === 1 ? '' : 's'}.`
      );
      if (!confirmed) return;
    }
    try {
      await apiFetch(`/libraries/${libraryId}`, { method: 'DELETE' });
      const removedPresentationIds = new Set(
        (library.presentations || []).map((presentation) => presentation.id)
      );
      state.libraries = state.libraries
        .filter((item) => item.id !== libraryId)
        .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
      state.favoriteLibraryIds.delete(libraryId);

      // prune caches pointing to deleted presentations
      removedPresentationIds.forEach((presentationId) => {
        presentationIndex.delete(presentationId);
        state.presentationMeta.delete(presentationId);
        state.slidesCache.delete(presentationId);
      });

      state.playlists = state.playlists.map((playlist) => {
        const filtered = playlist.entries.filter((entry) => !entry.presentationId || !removedPresentationIds.has(entry.presentationId));
        const updated = Object.assign({}, playlist, { entries: filtered });
        state.playlistLookup.set(updated.id, updated);
        return updated;
      });

      if (state.currentPresentationId && removedPresentationIds.has(state.currentPresentationId)) {
        state.currentPresentationId = null;
      }
      if (state.stagePresentationId && removedPresentationIds.has(state.stagePresentationId)) {
      state.stagePresentationId = null;
      state.stageSlideId = null;
      state.stageSnapshot = null;
      renderAbleSetPanel();
      }

      if (state.activeLibraryId === libraryId) {
        state.activeLibraryId = state.libraries.length > 0 ? state.libraries[0].id : null;
        if (!state.activeLibraryId && state.playlists.length > 0) {
          state.activePlaylistId = state.playlists[0].id;
        } else if (!state.activeLibraryId) {
          state.activePlaylistId = null;
        }
      }

      rebuildPresentationIndex();
      renderLibraries();
      renderPlaylists();
      renderPresentationList();

      if (state.activeLibraryId) {
        updateContextTitleFromLibrary(state.activeLibraryId);
      } else if (state.activePlaylistId) {
        updateContextTitleFromPlaylist(state.activePlaylistId);
      } else if (els.contextTitle) {
        els.contextTitle.textContent = 'Presentations';
      }

      if (!state.currentPresentationId && els.slides) {
        els.slides.innerHTML = '<p class="empty">Select a presentation to load slides.</p>';
        els.slides.removeAttribute('data-slides-placeholder');
      }

      renderStageStatus();
      if (!options.silent) {
        showToast('Library deleted', 'success');
      }
    } catch (error) {
      console.error('Failed to delete library', error);
      showToast('Failed to delete library', 'error');
    }
  }

  async function executeTimerCommand(command, payload) {
    try {
      const body = Object.assign({ command }, payload || {});
      const response = await apiFetch('/timers/command', {
        method: 'POST',
        body: JSON.stringify(body),
      });
      state.timers = response;
      applyTimers(response);
      if (command === 'set_countdown_target') {
        state.countdownInputDirty = false;
      }
    } catch (error) {
      console.error('Timer command failed', error);
      if (command === 'set_countdown_target') {
        state.countdownInputDirty = true;
      }
      showToast('Timer command failed', 'error');
    }
  }

  function applyTimers(overview) {
    if (!overview) return;
    window.__presenterTimers = overview;
    const countdown = overview.countdownToStart || overview.countdown_to_start || {};
    const preach = overview.preachTimer || overview.preach_timer || {};
    const countdownState = formatTimerState(countdown.state || 'idle');
    const countdownSeconds = countdown.secondsRemaining ?? countdown.seconds_remaining ?? 0;
    const target = countdown.target || countdown.targetUtc || countdown.target_utc;
    const preachState = formatTimerState(preach.state || 'idle');
    const preachSeconds = preach.secondsElapsed ?? preach.seconds_elapsed ?? 0;

    const countdownValueEl = qs('#countdown-value');
    const countdownTargetEl = qs('#countdown-target');
    window.__presenterCountdownDisplay = countdownState;
    if (countdownValueEl) {
      countdownValueEl.textContent = formatSeconds(countdownSeconds);
    }
    let targetDate = null;
    if (target) {
      try {
        targetDate = new Date(target);
      } catch (error) {
        targetDate = null;
        if (countdownTargetEl) {
          countdownTargetEl.textContent = `Target ${target}`;
        }
      }
    }
    if (countdownTargetEl) {
      if (targetDate instanceof Date && !Number.isNaN(targetDate.getTime())) {
        countdownTargetEl.textContent = `Target ${formatClock(targetDate)}`;
      } else if (target) {
        countdownTargetEl.textContent = `Target ${target}`;
      } else {
        countdownTargetEl.textContent = 'Target —';
      }
    }
    if (
      els.countdownInput &&
      !state.countdownInputActive &&
      !state.countdownInputDirty
    ) {
      if (targetDate instanceof Date && !Number.isNaN(targetDate.getTime())) {
        const hours = String(targetDate.getHours()).padStart(2, '0');
        const minutes = String(targetDate.getMinutes()).padStart(2, '0');
        els.countdownInput.value = `${hours}:${minutes}`;
      } else {
        els.countdownInput.value = '';
      }
    }

    const preachStateEl = qs('#preach-state');
    const preachValueEl = qs('#preach-value');
    if (preachStateEl) {
      preachStateEl.textContent = preachState;
    }
    if (preachValueEl) {
      preachValueEl.textContent = formatSeconds(preachSeconds);
    }
  }

  function syncOperatorSelectionFromStage(presentationId, slideId) {
    if (!presentationId) {
      updateActivePresentationIndicators();
      updateActiveSlideIndicators();
      return;
    }
    const isLiveMode = state.mode === 'live';
    if (!isLiveMode || !state.ableset.status.followEnabled) {
      updateActiveSlideIndicators();
      return;
    }

    const mapping = state.presentationPlaylistIndex.get(presentationId) || null;
    let playlistChanged = false;
    let listNeedsRender = false;

    if (mapping) {
      if (state.activePlaylistId !== mapping.playlistId) {
        state.activePlaylistId = mapping.playlistId;
        state.activeLibraryId = null;
        playlistChanged = true;
        listNeedsRender = true;
        renderPlaylists();
        renderLibraries();
        updateContextTitleFromPlaylist(mapping.playlistId);
      }
    } else {
      let libraryMatched = false;
      if (Array.isArray(state.libraries)) {
        const library = state.libraries.find((item) =>
          Array.isArray(item.presentations) && item.presentations.some((entry) => entry.id === presentationId)
        );
        if (library) {
          libraryMatched = true;
          if (state.activeLibraryId !== library.id || state.activePlaylistId) {
            state.activeLibraryId = library.id;
            state.activePlaylistId = null;
            listNeedsRender = true;
            renderPlaylists();
            renderLibraries();
          }
          updateContextTitleFromLibrary(library.id);
        }
      }
      if (!libraryMatched && state.stageSnapshot && state.stageSnapshot.presentationName && els.contextTitle) {
        els.contextTitle.textContent = `Live: ${state.stageSnapshot.presentationName}`;
      }
    }

    const presentationChanged = isLiveMode && state.currentPresentationId !== presentationId;
    listNeedsRender = listNeedsRender && !presentationChanged;
    if (presentationChanged) {
      state.currentPresentationId = presentationId;
      state.focusedSlideId = null;
      renderPresentationList();
      scrollPresentationIntoView(presentationId);
      if (state.slidesCache.has(presentationId)) {
        renderSlides(presentationId);
        if (slideId) {
          scrollSlideIntoView(slideId);
        }
      } else {
        loadPresentation(presentationId)
          .then(() => {
            if (state.currentPresentationId === presentationId) {
              scrollPresentationIntoView(presentationId);
              if (slideId) {
                scrollSlideIntoView(slideId);
              } else {
                updateActiveSlideIndicators();
              }
            }
          })
          .catch((error) => console.error('Failed to load presentation for stage sync', error));
      }
      updateActivePresentationIndicators();
    } else {
      if (playlistChanged) {
        scrollPresentationIntoView(presentationId);
      }
      if (listNeedsRender) {
        renderPresentationList();
      }
      updateActivePresentationIndicators();
      if (state.currentPresentationId === presentationId && slideId) {
        scrollSlideIntoView(slideId);
      }
      updateActiveSlideIndicators();
    }
  }

  function connectLiveSocket() {
    if (state.liveSocket) {
      try {
        state.liveSocket.close();
      } catch (error) {
        console.warn('Error closing live socket', error);
      }
    }
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(`${protocol}//${window.location.host}/live/ws`);
    state.liveSocket = socket;
    socket.addEventListener('open', () => {
      window.__presenterLiveConnected = true;
      if (state.liveReconnectTimer) {
        clearTimeout(state.liveReconnectTimer);
        state.liveReconnectTimer = null;
      }
    });
    socket.addEventListener('message', (event) => {
      try {
        const payload = JSON.parse(event.data);
        if (payload.type === 'timers' || payload.type === 'Timers') {
          state.timers = payload.overview;
          applyTimers(payload.overview);
        } else if (payload.type === 'stage' || payload.type === 'Stage') {
          const snapshot = payload.snapshot || {};
          const presentationId = snapshot.presentationId ?? snapshot.presentation_id ?? null;
          const currentSlideId = snapshot.currentSlideId ?? snapshot.current_slide_id ?? null;
          const latencyMsValue =
            typeof snapshot.latencyMs === 'number'
              ? snapshot.latencyMs
              : typeof snapshot.latency_ms === 'number'
              ? snapshot.latency_ms
              : null;
          const currentPositionValue =
            typeof snapshot.currentPosition === 'number'
              ? snapshot.currentPosition
              : typeof snapshot.current_position === 'number'
              ? snapshot.current_position
              : null;
          const totalSlidesValue =
            typeof snapshot.totalSlides === 'number'
              ? snapshot.totalSlides
              : typeof snapshot.total_slides === 'number'
              ? snapshot.total_slides
              : null;
          state.stageSnapshot = {
            presentationId,
            presentationName: snapshot.presentationName ?? snapshot.presentation_name ?? '',
            current: snapshot.current || null,
            next: snapshot.next || null,
            timers: snapshot.timers || null,
            latencyMs: latencyMsValue,
            currentPosition: currentPositionValue,
            totalSlides: totalSlidesValue,
          };
          state.stagePresentationId = presentationId;
          state.stageSlideId = currentSlideId;
          state.timers = snapshot.timers || state.timers;
          syncOperatorSelectionFromStage(presentationId, currentSlideId);
          if (snapshot.timers) {
            applyTimers(snapshot.timers);
          }
          renderStageStatus();
          renderAbleSetPanel();
        } else if (payload.type === 'stage_layout' || payload.type === 'StageLayout') {
          const nextCode = String(payload.code || '').trim();
          if (nextCode.length > 0) {
            state.stageLayoutCode = nextCode;
            applyStageLayoutSelection(nextCode);
          }
        } else if (payload.type === 'stage_connection' || payload.type === 'StageConnection') {
          handleStageConnectionSnapshot(payload.snapshot || payload);
        } else if (payload.type === 'bible' || payload.type === 'Bible') {
          // no-op for operator for now
        }
      } catch (error) {
        console.error('Failed to parse live payload', error);
      }
    });
    socket.addEventListener('close', () => {
      window.__presenterLiveConnected = false;
      if (!state.liveReconnectTimer) {
        state.liveReconnectTimer = setTimeout(() => {
          connectLiveSocket();
        }, 2000);
      }
    });
    socket.addEventListener('error', (error) => {
      console.error('Live websocket error', error);
      try {
        socket.close();
      } catch (err) {
        console.error('Failed to close socket after error', err);
      }
    });
  }

  function handleLibraryClick(event) {
    const moreButton = event.target.closest('[data-role="library-more"]');
    if (moreButton) {
      event.preventDefault();
      openLibraryModal();
      return;
    }

    const favoriteToggle = event.target.closest('[data-action="library-favorite"]');
    if (favoriteToggle && favoriteToggle.dataset.libraryId) {
      event.preventDefault();
      event.stopPropagation();
      toggleLibraryFavorite(favoriteToggle.dataset.libraryId);
      return;
    }

    const editButton = event.target.closest('[data-action="library-edit"]');
    if (editButton && editButton.dataset.libraryId) {
      event.preventDefault();
      event.stopPropagation();
      openLibraryEdit(editButton.dataset.libraryId);
      return;
    }

    const button = event.target.closest('[data-role="library-item"]');
    if (!button) return;
    const libraryId = button.dataset.libraryId;
    if (!libraryId) return;
    activateLibrary(libraryId);
  }

  function handlePlaylistClick(event) {
    const moreButton = event.target.closest('[data-role="playlist-more"]');
    if (moreButton) {
      event.preventDefault();
      openPlaylistModal();
      return;
    }

    const favoriteToggle = event.target.closest('[data-action="playlist-favorite"]');
    if (favoriteToggle && favoriteToggle.dataset.playlistId) {
      event.preventDefault();
      event.stopPropagation();
      togglePlaylistFavorite(favoriteToggle.dataset.playlistId);
      return;
    }

    const editButton = event.target.closest('[data-action="playlist-edit"]');
    if (editButton && editButton.dataset.playlistId) {
      event.preventDefault();
      event.stopPropagation();
      openPlaylistEdit(editButton.dataset.playlistId);
      return;
    }

    const button = event.target.closest('[data-role="playlist-item"]');
    if (!button) return;
    const playlistId = button.dataset.playlistId;
    if (!playlistId) return;
    state.activePlaylistId = playlistId;
    state.activeLibraryId = null;
    state.currentPresentationId = null;
    state.focusedSlideId = null;
    renderLibraries();
    updateContextTitleFromPlaylist(playlistId);
    renderPlaylists();
    renderPresentationList();
    if (els.slides) {
      els.slides.innerHTML = '<p class="empty">Select a presentation to load slides.</p>';
      els.slides.removeAttribute('data-slides-placeholder');
    }
  }

  function handleLibraryModalClick(event) {
    const closeButton = event.target.closest('[data-role="library-modal-close"]');
    if (closeButton) {
      event.preventDefault();
      closeLibraryModal();
      return;
    }

    const favoriteToggle = event.target.closest('[data-action="library-favorite"]');
    if (favoriteToggle && favoriteToggle.dataset.libraryId) {
      event.preventDefault();
      event.stopPropagation();
      toggleLibraryFavorite(favoriteToggle.dataset.libraryId);
      return;
    }

    const editButton = event.target.closest('[data-action="library-edit"]');
    if (editButton && editButton.dataset.libraryId) {
      event.preventDefault();
      event.stopPropagation();
      openLibraryEdit(editButton.dataset.libraryId);
      return;
    }

    const button = event.target.closest('[data-role="library-item"]');
    if (button && button.dataset.libraryId) {
      event.preventDefault();
      activateLibrary(button.dataset.libraryId);
      closeLibraryModal();
    }
  }

  async function handleLibraryEditSubmit(event) {
    event.preventDefault();
    if (state.libraryEditSubmitting) return;
    const nameInput = els.libraryEditName;
    const favoriteInput = els.libraryEditFavorite;
    const mode = state.libraryEditMode || 'edit';
    const name = nameInput ? nameInput.value.trim() : '';
    if (!name) {
      showToast('Library name cannot be empty', 'warning');
      if (nameInput) {
        nameInput.focus();
      }
      return;
    }
    const favorite = favoriteInput ? Boolean(favoriteInput.checked) : false;

    setLibraryEditSubmitting(true);
    try {
      if (mode === 'create') {
        const library = await apiFetch('/libraries', {
          method: 'POST',
          body: JSON.stringify({ name }),
        });
        const normalised = normaliseLibrary(library);
        if (normalised) {
          normalised.isFavorite = favorite;
          state.libraries.push(normalised);
          state.libraries.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
          state.activeLibraryId = normalised.id;
          state.activePlaylistId = null;
          rebuildPresentationIndex();
          if (favorite) {
            await apiFetch(`/libraries/${normalised.id}/favorite`, {
              method: 'POST',
              body: JSON.stringify({ favorite: true }),
            });
            state.favoriteLibraryIds.add(normalised.id);
          } else {
            state.favoriteLibraryIds.delete(normalised.id);
          }
          renderLibraries();
          renderPlaylists();
          renderPresentationList();
          updateContextTitleFromLibrary(normalised.id);
          showToast('Library created', 'success');
        }
        closeLibraryEdit();
        return;
      }

      const libraryId = state.libraryBeingEditedId;
      if (!libraryId) {
        closeLibraryEdit();
        return;
      }
      const library = state.libraries.find((item) => item.id === libraryId);
      if (!library) {
        closeLibraryEdit();
        return;
      }
      const previousFavorite = state.favoriteLibraryIds.has(libraryId);
      if (name === library.name && favorite === previousFavorite) {
        closeLibraryEdit();
        return;
      }

      if (name !== library.name) {
        await apiFetch(`/libraries/${libraryId}`, {
          method: 'PATCH',
          body: JSON.stringify({ name }),
        });
        library.name = name;
      }

      if (favorite !== previousFavorite) {
        await apiFetch(`/libraries/${libraryId}/favorite`, {
          method: 'POST',
          body: JSON.stringify({ favorite }),
        });
        if (favorite) {
          state.favoriteLibraryIds.add(libraryId);
        } else {
          state.favoriteLibraryIds.delete(libraryId);
        }
      }

      library.isFavorite = favorite;
      state.libraries.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
      rebuildPresentationIndex();
      renderLibraries();
      renderPlaylists();
      if (state.activeLibraryId === libraryId) {
        updateContextTitleFromLibrary(libraryId);
        renderPresentationList();
      }
      showToast('Library updated', 'success');
      closeLibraryEdit();
    } catch (error) {
      console.error('Failed to update library', error);
      showToast('Failed to update library', 'error');
    } finally {
      setLibraryEditSubmitting(false);
    }
  }

  async function handleLibraryEditDelete(event) {
    event.preventDefault();
    if (state.libraryEditSubmitting) return;
    if (state.libraryEditMode !== 'edit') return;
    const libraryId = state.libraryBeingEditedId;
    if (!libraryId) {
      closeLibraryEdit();
      return;
    }
    const library = state.libraries.find((item) => item.id === libraryId);
    if (!library) {
      closeLibraryEdit();
      return;
    }
    const count = Array.isArray(library.presentations)
      ? library.presentations.length
      : library.presentation_count || 0;
    const confirmed = window.confirm(
      `Delete library "${library.name}"? This will remove ${count} presentation${count === 1 ? '' : 's'}.`
    );
    if (!confirmed) {
      return;
    }
    setLibraryEditSubmitting(true);
    try {
      await deleteLibrary(libraryId, { skipConfirm: true });
      closeLibraryEdit();
    } catch (error) {
      console.error('Failed to delete library', error);
      showToast('Failed to delete library', 'error');
    } finally {
      setLibraryEditSubmitting(false);
    }
  }

  async function handlePresentationEditSubmit(event) {
    event.preventDefault();
    if (state.presentationEditSubmitting) return;
    const target = state.presentationEditTarget;
    if (!target) {
      closePresentationEdit();
      return;
    }
    const nameInput = els.presentationEditName;
    const name = nameInput ? nameInput.value.trim() : '';
    if (!name) {
      showToast('Name cannot be empty', 'warning');
      if (nameInput) {
        nameInput.focus();
        nameInput.select();
      }
      return;
    }

    setPresentationEditSubmitting(true);
    try {
      if (target.type === 'presentation') {
        await apiFetch(`/presentations/${target.presentationId}`, {
          method: 'PATCH',
          body: JSON.stringify({ name }),
        });
        let library = null;
        if (target.libraryId) {
          library = state.libraries.find((item) => item.id === target.libraryId);
        }
        if (!library) {
          library = state.libraries.find((item) =>
            (item.presentations || []).some((presentation) => presentation.id === target.presentationId),
          );
        }
        if (library) {
          const presentation = (library.presentations || []).find(
            (item) => item.id === target.presentationId,
          );
          if (presentation) {
            presentation.name = name;
          }
        }
        if (state.presentationMeta.has(target.presentationId)) {
          const cached = state.presentationMeta.get(target.presentationId);
          if (cached && typeof cached === 'object') {
            cached.name = name;
          }
        }
        const existingIndex = presentationIndex.get(target.presentationId);
        state.playlists.forEach((playlist) => {
          if (!Array.isArray(playlist.entries)) {
            return;
          }
          playlist.entries.forEach((entry) => {
            if (entry.entryType === 'presentation' && entry.presentationId === target.presentationId) {
              entry.name = name;
            }
          });
        });
        if (existingIndex) {
          existingIndex.name = name;
        }
        rebuildPresentationIndex();
        renderLibraries();
        renderPlaylists();
        renderPresentationList();
        if (state.currentPresentationId === target.presentationId) {
          renderSlides(target.presentationId);
        }
        if (state.activeLibraryId) {
          updateContextTitleFromLibrary(state.activeLibraryId);
        } else if (state.activePlaylistId) {
          updateContextTitleFromPlaylist(state.activePlaylistId);
        }
        if (
          state.stageSnapshot &&
          state.stageSnapshot.presentationId === target.presentationId
        ) {
          state.stageSnapshot.presentationName = name;
          renderStageStatus();
          renderAbleSetPanel();
        }
        showToast('Presentation renamed', 'success');
      } else if (target.type === 'separator') {
        const playlist = state.playlists.find((item) => item.id === target.playlistId);
        if (!playlist) {
          throw new Error('Playlist not found');
        }
        const entryIndex = playlist.entries.findIndex((item) => item.entryId === target.entryId);
        if (entryIndex < 0) {
          throw new Error('Separator not found');
        }
        playlist.entries[entryIndex].name = name;
        const response = await apiFetch(`/playlists/${playlist.id}/entries`, {
          method: 'PUT',
          body: JSON.stringify({ entries: serialisePlaylistEntries(playlist.entries) }),
        });
        const updated = normalisePlaylist(response);
        if (updated) {
          upsertPlaylist(updated);
        }
        renderPlaylists();
        if (state.activePlaylistId === playlist.id) {
          renderPresentationList();
        }
        showToast('Separator renamed', 'success');
      }
      closePresentationEdit();
    } catch (error) {
      console.error('Failed to save changes', error);
      showToast('Failed to save changes', 'error');
    } finally {
      setPresentationEditSubmitting(false);
    }
  }

  function handleGlobalKeydown(event) {
    const target = event.target;
    const tag = target && target.tagName ? target.tagName.toLowerCase() : '';
    const isEditable = Boolean(
      (target && target.isContentEditable) ||
      tag === 'input' ||
      tag === 'textarea' ||
      tag === 'select'
    );
    const modalOpen =
      state.libraryModalOpen ||
      state.libraryEditModalOpen ||
      state.playlistModalOpen ||
      state.playlistEditModalOpen ||
      state.presentationEditModalOpen;

    if (!isEditable && !modalOpen) {
      if ((event.key === ' ' || event.key === 'Space') && state.mode === 'live') {
        if (els.searchInput) {
          event.preventDefault();
          els.searchInput.focus();
          els.searchInput.select();
          if (state.searchQuery.trim()) {
            renderSearchResults();
          }
        }
        return;
      }
      if (event.key === 'ArrowRight') {
        event.preventDefault();
        navigateSlides(1);
        return;
      }
      if (event.key === 'ArrowLeft') {
        event.preventDefault();
        navigateSlides(-1);
        return;
      }
    }

    if (event.key === 'Escape') {
      if (state.searchOpen) {
        hideSearchResults();
        return;
      }
      if (state.presentationEditModalOpen) {
        event.preventDefault();
        if (!state.presentationEditSubmitting) {
          closePresentationEdit();
        }
        return;
      }
      if (state.libraryEditModalOpen) {
        event.preventDefault();
        if (!state.libraryEditSubmitting) {
          closeLibraryEdit();
        }
        return;
      }
      if (state.playlistEditModalOpen) {
        event.preventDefault();
        if (!state.playlistEditSubmitting) {
          closePlaylistEdit();
        }
        return;
      }
      if (state.libraryModalOpen) {
        event.preventDefault();
        closeLibraryModal();
        return;
      }
      if (state.playlistModalOpen) {
        event.preventDefault();
        closePlaylistModal();
      }
    }
  }

  function handlePresentationClick(event) {
    const renamePresentationButton = event.target.closest('[data-action="presentation-rename"]');
    if (renamePresentationButton) {
      event.preventDefault();
      event.stopPropagation();
      const presentationId =
        renamePresentationButton.dataset.presentationId ||
        renamePresentationButton.closest('[data-presentation-id]')?.dataset.presentationId;
      if (!presentationId) {
        return;
      }
      const libraryId = renamePresentationButton.dataset.libraryId || null;
      openPresentationRename(presentationId, libraryId);
      return;
    }

    const renameSeparatorButton = event.target.closest('[data-action="separator-rename"]');
    if (renameSeparatorButton) {
      event.preventDefault();
      event.stopPropagation();
      const playlistId =
        renameSeparatorButton.dataset.playlistId || state.activePlaylistId || null;
      const entryId = renameSeparatorButton.dataset.entryId || null;
      if (playlistId && entryId) {
        openSeparatorRename(playlistId, entryId);
      }
      return;
    }

    const removeButton = event.target.closest('[data-action="playlist-remove"]');
    if (removeButton) {
      event.stopPropagation();
      const item = removeButton.closest('[data-role="presentation-item"]');
      if (!item) return;
      const index = Number(item.dataset.entryIndex);
      if (!Number.isNaN(index)) {
        removePlaylistEntry(index);
      }
      return;
    }

    const item = event.target.closest('[data-role="presentation-item"]');
    if (!item) return;
    const itemType = item.dataset.type || 'presentation';
    if (itemType === 'separator') {
      state.currentPresentationId = null;
      state.focusedSlideId = null;
      renderPresentationList();
      return;
    }
    const presentationId = item.dataset.presentationId;
    if (!presentationId) return;
    state.currentPresentationId = presentationId;
    state.focusedSlideId = null;
    renderPresentationList();
    loadPresentation(presentationId);
  }

  function handlePresentationDragStart(event) {
    const item = event.target.closest('[data-role="presentation-item"]');
    if (!item) return;
    const entryId = item.dataset.entryId || null;
    const presentationId = item.dataset.presentationId || null;
    state.draggingFromSearch = false;
    const entryIndex = item.dataset.entryIndex;
    const isPlaylistEntry =
      entryId && typeof entryIndex !== 'undefined' && entryIndex !== null && entryIndex !== '' && state.activePlaylistId;
    if (isPlaylistEntry) {
      const playlist = state.playlists.find((list) => list.id === state.activePlaylistId);
      if (playlist) {
        state.playlistReorderSnapshot = {
          playlistId: playlist.id,
          sourceId: entryId,
          initialOrder: playlist.entries.map((entry) => entry.entryId),
        };
      }
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('application/x-presenter-playlist-entry', entryId);
      event.dataTransfer.setData('application/x-presenter-playlist-id', state.activePlaylistId);
      state.draggingPresentationId = null;
      return;
    }

    if (!presentationId) {
      event.preventDefault();
      return;
    }
    state.draggingPresentationId = presentationId;
    event.dataTransfer.effectAllowed = 'copyMove';
    event.dataTransfer.setData('application/x-presenter-presentation', presentationId);
    event.dataTransfer.setData('text/plain', presentationId);
    event.dataTransfer.setData('application/x-presenter-presentation', presentationId);
    event.dataTransfer.setDragImage(item, item.clientWidth / 2, item.clientHeight / 2);
  }

  function resolvePlaylistTargetFromEvent(event) {
    const button = event.target.closest('[data-role="playlist-item"]');
    if (button && button.dataset.playlistId) {
      return button.dataset.playlistId;
    }
    return state.activePlaylistId || null;
  }

  function handlePlaylistDragOver(event) {
    const playlistId = resolvePlaylistTargetFromEvent(event);
    if (!playlistId) {
      return;
    }
    if (
      event.dataTransfer.types.includes('application/x-presenter-presentation') ||
      event.dataTransfer.types.includes('text/plain')
    ) {
      event.preventDefault();
      const isReorder =
        state.playlistReorderSnapshot &&
        state.playlistReorderSnapshot.playlistId === playlistId;
      event.dataTransfer.dropEffect = isReorder ? 'move' : 'copy';
    }
  }

  async function handlePlaylistDrop(event) {
    if (state.playlistReorderSnapshot) {
      return;
    }
    const playlistId = resolvePlaylistTargetFromEvent(event);
    if (!playlistId) {
      showToast('Select a playlist before adding presentations.', 'warning');
      return;
    }
    const transfer = event.dataTransfer;
    let id = transfer
      ? transfer.getData('application/x-presenter-presentation') || transfer.getData('text/plain')
      : '';
    if (!id && state.draggingPresentationId) {
      id = state.draggingPresentationId;
    }
    if (!id) {
      clearPlaylistDropIndicators();
      state.draggingPresentationId = null;
      state.draggingFromSearch = false;
      return;
    }
    event.preventDefault();
    const fromSearch =
      state.searchDragging ||
      state.draggingFromSearch ||
      (transfer
        ? Array.from(transfer.types || []).includes('application/x-presenter-search')
        : false) ||
      (state.searchOpen && typeof state.searchQuery === 'string' && state.searchQuery.trim().length > 0);
    await handlePlaylistInsertion(id, playlistId, null, { clearSearch: fromSearch });
    if (fromSearch) {
      clearSearchResults();
    }
    clearPlaylistDropIndicators();
    state.searchDragging = false;
    state.draggingPresentationId = null;
    state.draggingFromSearch = false;
  }

  function handleAddSlide() {
    if (!state.currentPresentationId) {
      showToast('Select a presentation first', 'warning');
      return;
    }
    const slides = getSlidesForPresentation(state.currentPresentationId);
    let position = slides.length;
    if (state.focusedSlideId) {
      const index = slides.findIndex((slide) => slide.id === state.focusedSlideId);
      if (index >= 0) {
        position = index + 1;
      }
    }
    insertSlide(state.currentPresentationId, position);
  }

  function handleClearSlide(event) {
    if (event) {
      event.preventDefault();
      event.stopPropagation();
    }
    clearActiveSlide();
  }

  function handleSlidesClick(event) {
    const card = event.target.closest('[data-slide-id]');
    if (!card) return;
    const slideId = card.dataset.slideId;
    const now = typeof performance !== 'undefined' && performance.now ? performance.now() : Date.now();
    if (state.mode === 'live' && state.skipClickTrigger) {
      if (state.skipClickTrigger.slideId === slideId && state.skipClickTrigger.expiresAt >= now) {
        state.skipClickTrigger = null;
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      if (state.skipClickTrigger.expiresAt < now) {
        state.skipClickTrigger = null;
      }
    }
    const presentationId = state.currentPresentationId;
    if (!presentationId) return;
    const actionButton = event.target.closest('[data-action]');
    if (actionButton) {
      const action = actionButton.dataset.action;
      switch (action) {
        case 'trigger':
          triggerSlide(presentationId, slideId, card);
          break;
        case 'save':
          saveSlide(presentationId, slideId, card);
          break;
        case 'duplicate':
          duplicateSlide(presentationId, slideId);
          break;
        case 'delete':
          deleteSlide(presentationId, slideId);
          break;
        default:
          break;
      }
      event.preventDefault();
      event.stopPropagation();
      return;
    }

    const isTextField = event.target.matches('textarea, input, textarea *, input *');
    if (isTextField) {
      return;
    }

    state.focusedSlideId = slideId;
    if (state.mode === 'live') {
      triggerSlide(presentationId, slideId, card);
    } else {
      updateActiveSlideIndicators();
    }
  }

  function handleSlidesPointerDown(event) {
    if (state.mode === 'live') {
      if (event.button !== 0) {
        state.pendingFocus = null;
        return;
      }
      const actionButton = event.target.closest('[data-action]');
      const editableField = event.target.closest('[data-field]');
      if (actionButton || editableField) {
        state.pendingFocus = null;
        return;
      }
      const card = event.target.closest('[data-slide-id]');
      const presentationId = state.currentPresentationId || state.stagePresentationId;
      const slideId = card ? card.dataset.slideId : null;
      if (card && presentationId && slideId) {
        const now = typeof performance !== 'undefined' && performance.now ? performance.now() : Date.now();
        state.skipClickTrigger = { slideId, expiresAt: now + 250 };
        state.currentPresentationId = presentationId;
        state.focusedSlideId = slideId;
        triggerSlide(presentationId, slideId, card);
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      state.pendingFocus = null;
      return;
    }
    const field = event.target.closest('[data-field]');
    if (!field) {
      state.pendingFocus = null;
      return;
    }
    const card = field.closest('[data-slide-id]');
    if (!card) {
      state.pendingFocus = null;
      return;
    }
    const fieldName = field.dataset.field || 'main';
    const selectionStart = typeof field.selectionStart === 'number'
      ? field.selectionStart
      : field.value.length;
    const selectionEnd = typeof field.selectionEnd === 'number'
      ? field.selectionEnd
      : selectionStart;
    state.pendingFocus = {
      slideId: card.dataset.slideId,
      field: fieldName,
      caret: 'preserve',
      selectionStart,
      selectionEnd,
    };
  }

  function handleSlideFieldFocus(event) {
    const field = event.target.closest('[data-field]');
    if (!field) return;
    const card = field.closest('[data-slide-id]');
    if (!card) return;
    const slideId = card.dataset.slideId;
    const fieldName = field.dataset.field || 'main';
    state.focusedSlideId = slideId;
    const updateSelection = () => {
      const start = typeof field.selectionStart === 'number' ? field.selectionStart : field.value.length;
      const end = typeof field.selectionEnd === 'number' ? field.selectionEnd : start;
      state.pendingFocus = {
        slideId,
        field: fieldName,
        caret: 'preserve',
        selectionStart: start,
        selectionEnd: end,
      };
    };
    updateSelection();
    requestAnimationFrame(updateSelection);
  }

  function handleSlideDragStart(event) {
    const handle = event.target.closest('[data-role="slide-drag-handle"]');
    if (!handle) {
      event.preventDefault();
      return;
    }
    const card = handle.closest('[data-slide-id]');
    if (!card) return;
    state.reorderSnapshot = {
      sourceId: card.dataset.slideId,
      initialOrder: qsa('[data-slide-id]', els.slides).map((node) => node.dataset.slideId),
    };
    event.dataTransfer.effectAllowed = 'move';
    event.dataTransfer.setData('application/x-presenter-slide', card.dataset.slideId);
    event.dataTransfer.setDragImage(card, card.clientWidth / 2, card.clientHeight / 2);
  }

  function handleSlideDragOver(event) {
    const target = event.target.closest('[data-slide-id]');
    if (!target || !state.reorderSnapshot) return;
    event.preventDefault();
    const draggingId = state.reorderSnapshot.sourceId;
    if (target.dataset.slideId === draggingId) return;
    const cards = qsa('[data-slide-id]', els.slides);
    const dragging = cards.find((card) => card.dataset.slideId === draggingId);
    if (!dragging) return;
    const targetRect = target.getBoundingClientRect();
    const isBefore = event.clientY < targetRect.top + targetRect.height / 2;
    if (isBefore) {
      els.slides.insertBefore(dragging, target);
    } else {
      els.slides.insertBefore(dragging, target.nextSibling);
    }
  }

  function handleSlideDrop(event) {
    if (!state.reorderSnapshot) return;
    event.preventDefault();
    const newOrder = qsa('[data-slide-id]', els.slides).map((card) => card.dataset.slideId);
    if (!state.currentPresentationId) return;
    if (newOrder.join(',') === state.reorderSnapshot.initialOrder.join(',')) {
      state.reorderSnapshot = null;
      return;
    }
    reorderSlides(state.currentPresentationId, newOrder);
    state.reorderSnapshot = null;
  }

  function handleSlideDragEnd() {
    state.reorderSnapshot = null;
    if (state.currentPresentationId) {
      renderSlides(state.currentPresentationId);
    }
  }

  function setPresentationDropzoneState(state) {
    if (els.presentationDropzone) {
      if (state) {
        els.presentationDropzone.dataset.dropzone = state;
      } else {
        delete els.presentationDropzone.dataset.dropzone;
      }
    }
    if (els.presentationList) {
      if (state) {
        els.presentationList.dataset.dropzone = state;
      } else {
        delete els.presentationList.dataset.dropzone;
      }
    }
  }

  function clearPlaylistDropIndicators() {
    if (els.presentationList) {
      qsa('[data-role="presentation-item"][data-drop-position]', els.presentationList).forEach((node) => {
        node.removeAttribute('data-drop-position');
      });
    }
    setPresentationDropzoneState(null);
  }

  function handlePlaylistEntryDragOver(event) {
    if (!state.activePlaylistId) {
      clearPlaylistDropIndicators();
      return;
    }
    const transfer = event.dataTransfer;
    const types = transfer ? Array.from(transfer.types || []) : [];
    const isPresentationDrag =
      types.includes('application/x-presenter-presentation') || types.includes('text/plain');
    const isReorder =
      !!state.playlistReorderSnapshot &&
      state.activePlaylistId &&
      state.activePlaylistId === state.playlistReorderSnapshot.playlistId &&
      types.includes('application/x-presenter-playlist-entry');

    const target = event.target.closest('[data-role="presentation-item"]');

    if (isReorder) {
      if (!target) return;
      event.preventDefault();
      clearPlaylistDropIndicators();
      const draggingId = state.playlistReorderSnapshot.sourceId;
      if (target.dataset.entryId === draggingId) return;
      const items = qsa('[data-role="presentation-item"]', els.presentationList);
      const dragging = items.find((node) => node.dataset.entryId === draggingId);
      if (!dragging) return;
      const rect = target.getBoundingClientRect();
      const isBefore = event.clientY < rect.top + rect.height / 2;
      if (isBefore) {
        els.presentationList.insertBefore(dragging, target);
      } else {
        els.presentationList.insertBefore(dragging, target.nextSibling);
      }
      return;
    }

    if (!isPresentationDrag) {
      clearPlaylistDropIndicators();
      return;
    }

    event.preventDefault();
    event.stopPropagation();
    if (!target) {
      if (els.presentationList) {
        qsa('[data-role="presentation-item"][data-drop-position]', els.presentationList).forEach((node) => {
          node.removeAttribute('data-drop-position');
        });
      }
      setPresentationDropzoneState('append');
      return;
    }
    const rect = target.getBoundingClientRect();
    const isBefore = event.clientY < rect.top + rect.height / 2;
    clearPlaylistDropIndicators();
    setPresentationDropzoneState(null);
    target.dataset.dropPosition = isBefore ? 'before' : 'after';
  }

  async function handlePlaylistEntryDrop(event) {
    if (!state.activePlaylistId) {
      clearPlaylistDropIndicators();
      state.draggingPresentationId = null;
      state.draggingFromSearch = false;
      return;
    }
    const transfer = event.dataTransfer;
    const types = transfer ? Array.from(transfer.types || []) : [];
    let presentationId = transfer
      ? transfer.getData('application/x-presenter-presentation') || transfer.getData('text/plain')
      : '';
    if (!presentationId && state.draggingPresentationId) {
      presentationId = state.draggingPresentationId;
    }

    setPresentationDropzoneState(null);

    if (
      state.playlistReorderSnapshot &&
      state.activePlaylistId &&
      state.activePlaylistId === state.playlistReorderSnapshot.playlistId
    ) {
      event.preventDefault();
      event.stopPropagation();
      const ordered = qsa('[data-role="presentation-item"]', els.presentationList)
        .map((node) => node.dataset.entryId)
        .filter(Boolean);
      if (
        ordered.length &&
        state.playlistReorderSnapshot.initialOrder &&
        ordered.join(',') === state.playlistReorderSnapshot.initialOrder.join(',')
      ) {
        state.playlistReorderSnapshot = null;
        clearPlaylistDropIndicators();
        return;
      }
      reorderPlaylistEntries(state.activePlaylistId, ordered);
      state.playlistReorderSnapshot = null;
      clearPlaylistDropIndicators();
      return;
    }

    if (presentationId) {
      const playlistId = state.activePlaylistId;
    if (!playlistId) {
      clearPlaylistDropIndicators();
      state.draggingFromSearch = false;
      state.draggingPresentationId = null;
      return;
    }
      const playlist =
        state.playlists.find((item) => item.id === playlistId) || state.playlistLookup.get(playlistId);
      if (!playlist) {
        clearPlaylistDropIndicators();
        showToast('Playlist not found.', 'error');
        state.draggingPresentationId = null;
        state.draggingFromSearch = false;
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      const target = event.target.closest('[data-role="presentation-item"]');
      let insertIndex = playlist.entries.length;
      if (target && target.dataset.entryIndex) {
        const baseIndex = Number(target.dataset.entryIndex);
        if (!Number.isNaN(baseIndex)) {
          const rect = target.getBoundingClientRect();
          const isBefore = event.clientY < rect.top + rect.height / 2;
          insertIndex = isBefore ? baseIndex : baseIndex + 1;
        }
      }
      const fromSearch =
        state.searchDragging ||
        state.draggingFromSearch ||
        types.includes('application/x-presenter-search');
      await handlePlaylistInsertion(presentationId, playlistId, insertIndex, { clearSearch: fromSearch });
      if (fromSearch) {
        clearSearchResults();
      }
      clearPlaylistDropIndicators();
      state.searchDragging = false;
      state.draggingPresentationId = null;
      state.draggingFromSearch = false;
      return;
    }

    clearPlaylistDropIndicators();
    state.draggingPresentationId = null;
    state.draggingFromSearch = false;
  }

  function handlePresentationDropzoneDragLeave(event) {
    if (!els.presentationDropzone) return;
    const nextTarget = event.relatedTarget;
    if (!nextTarget || !els.presentationDropzone.contains(nextTarget)) {
      setPresentationDropzoneState(null);
    }
  }

  function handlePlaylistEntryDragEnd() {
    if (state.playlistReorderSnapshot) {
      state.playlistReorderSnapshot = null;
      if (state.activePlaylistId) {
        renderPresentationList();
      }
    }
    clearPlaylistDropIndicators();
    state.draggingPresentationId = null;
  }

  function handleSlideInputBlur(event) {
    const field = event.target;
    if (!field.matches('[data-field]')) return;
    if (!state.currentPresentationId) return;
    const card = field.closest('[data-slide-id]');
    if (!card) return;
    const slideId = card.dataset.slideId;
    const slides = getSlidesForPresentation(state.currentPresentationId);
    const slide = slides.find((item) => item.id === slideId);
    if (!slide) return;
    const main = card.querySelector('[data-field="main"]').value;
    const translation = card.querySelector('[data-field="translation"]').value;
    const stage = card.querySelector('[data-field="stage"]').value;
    const groupValue = card.querySelector('[data-field="group"]').value;
    const payload = {
      main,
      translation,
      stage,
      group: groupValue || null,
    };
    const original = slide.content;
    if (
      original.main.value === payload.main &&
      original.translation.value === payload.translation &&
      original.stage.value === payload.stage &&
      ((original.group && original.group.value) || '') === (payload.group || '')
    ) {
      updateCardWarnings(card);
      return;
    }
    if (!state.pendingFocus) {
      const activeElement = document.activeElement;
      if (activeElement) {
        const targetField = activeElement.closest('[data-field]');
        if (targetField) {
          const targetCard = targetField.closest('[data-slide-id]');
          if (targetCard) {
            const value = typeof targetField.value === 'string' ? targetField.value : '';
            const start = typeof targetField.selectionStart === 'number'
              ? targetField.selectionStart
              : value.length;
            const end = typeof targetField.selectionEnd === 'number'
              ? targetField.selectionEnd
              : start;
            state.pendingFocus = {
              slideId: targetCard.dataset.slideId,
              field: targetField.dataset.field || 'main',
              caret: 'preserve',
              selectionStart: start,
              selectionEnd: end,
            };
          }
        }
      }
    }
    updateSlideContent(state.currentPresentationId, slideId, payload);
  }

  function handleSlideInputChange(event) {
    const field = event.target;
    if (!field.matches('[data-field]')) return;
    const card = field.closest('[data-slide-id]');
    if (!card) return;
    updateCardWarnings(card);
  }

  function parseCountdownTarget(rawValue) {
    const trimmed = (rawValue || '').replace(/\s+/g, '');
    if (!trimmed) {
      return null;
    }

    let hours = 0;
    let minutes = 0;
    let seconds = 0;

    if (trimmed.includes(':')) {
      const parts = trimmed.split(':').map((part) => part.trim()).filter(Boolean);
      if (parts.length < 2 || parts.length > 3) {
        return null;
      }
      hours = Number(parts[0]);
      minutes = Number(parts[1]);
      seconds = parts.length === 3 ? Number(parts[2]) : 0;
    } else if (/^\d+$/.test(trimmed)) {
      if (trimmed.length <= 2) {
        minutes = Number(trimmed);
      } else if (trimmed.length <= 4) {
        minutes = Number(trimmed.slice(-2));
        hours = Number(trimmed.slice(0, trimmed.length - 2));
      } else {
        return null;
      }
    } else {
      return null;
    }

    if (
      !Number.isFinite(hours) ||
      !Number.isFinite(minutes) ||
      !Number.isFinite(seconds) ||
      hours < 0 ||
      hours > 23 ||
      minutes < 0 ||
      minutes > 59 ||
      seconds < 0 ||
      seconds > 59
    ) {
      return null;
    }

    const now = new Date();
    const target = new Date(now);
    target.setMilliseconds(0);
    target.setHours(hours, minutes, seconds, 0);
    if (target.getTime() <= now.getTime()) {
      target.setDate(target.getDate() + 1);
    }

    const display = `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}`;
    return { target, display };
  }

  async function submitCountdownTarget() {
    if (!els.countdownInput) {
      showToast('Countdown input missing', 'error');
      return false;
    }
    const result = parseCountdownTarget(els.countdownInput.value);
    if (!result) {
      showToast('Invalid time format', 'error');
      return false;
    }
    els.countdownInput.value = result.display;
    state.countdownInputDirty = false;
    await executeTimerCommand('set_countdown_target', { target: result.target.toISOString() });
    return true;
  }

  function handleTimerButtonClick(event) {
    const button = event.target.closest('[data-command]');
    if (!button) return;
    const command = button.dataset.command;
    if (!command) return;
    if (command === 'set_countdown_target') {
      submitCountdownTarget();
      return;
    }
    executeTimerCommand(command, {});
  }

  async function startCountdownFromInput() {
    const currentTargetIso = state.timers?.countdownToStart?.target || null;
    const updated = await submitCountdownTarget();
    if (!updated && !currentTargetIso) {
      showToast('Set a target time first', 'warning');
      return;
    }
    await executeTimerCommand('start_countdown', {});
  }

  async function offsetCountdown(minutesDelta) {
    const overview = state.timers?.countdownToStart || state.timers?.countdown_to_start;
    let targetDate = null;
    if (overview && overview.target) {
      const parsed = new Date(overview.target);
      if (!Number.isNaN(parsed.getTime())) {
        targetDate = parsed;
      }
    }

    if (!targetDate) {
      const parsedInput = parseCountdownTarget(els.countdownInput ? els.countdownInput.value : '');
      if (!parsedInput) {
        showToast('Set a target time first', 'warning');
        return;
      }
      targetDate = parsedInput.target;
    }

    targetDate = new Date(targetDate.getTime());
    targetDate.setMinutes(targetDate.getMinutes() + minutesDelta);
    targetDate.setSeconds(0, 0);
    if (targetDate.getTime() <= Date.now()) {
      targetDate.setDate(targetDate.getDate() + 1);
    }

    if (els.countdownInput && !state.countdownInputActive) {
      const hours = String(targetDate.getHours()).padStart(2, '0');
      const minutes = String(targetDate.getMinutes()).padStart(2, '0');
      els.countdownInput.value = `${hours}:${minutes}`;
    }

    await executeTimerCommand('set_countdown_target', { target: targetDate.toISOString() });
  }

  function handleModeToggle(event) {
    const button = event.target.closest('[data-role="mode-toggle"]');
    if (!button) return;
    const mode = button.dataset.mode;
    if (!mode || mode === state.mode) return;
    setMode(mode);
    updateActiveSlideIndicators();
  }

  function handleViewToggle(event) {
    const button = event.target.closest('[data-role="view-toggle"]');
    if (!button) return;
    const view = button.dataset.view;
    if (!view || view === state.view) return;
    setView(view);
  }

  function renderAbleSetPanel() {
    const status = state.ableset.status || {
      enabled: false,
      tracking: false,
      followEnabled: false,
      lastSong: null,
      lastError: null,
    };

    if (els.ablesetEnable) {
      const label = status.enabled ? 'Ableton ON' : 'Ableton OFF';
      els.ablesetEnable.textContent = label;
      els.ablesetEnable.dataset.state = status.enabled ? 'on' : 'off';
      els.ablesetEnable.dataset.loading = state.ableset.enableLoading ? 'true' : 'false';
      els.ablesetEnable.disabled = state.ableset.enableLoading;
    }

    if (els.ablesetFollow) {
      const label = status.followEnabled ? 'Follow ON' : 'Follow OFF';
      els.ablesetFollow.textContent = label;
      els.ablesetFollow.dataset.state = status.followEnabled ? 'on' : 'off';
      els.ablesetFollow.dataset.loading = state.ableset.followLoading ? 'true' : 'false';
      els.ablesetFollow.disabled = !status.enabled || state.ableset.followLoading;
    }

    const snapshot = state.stageSnapshot;
    if (els.stageSongLine) {
      els.stageSongLine.textContent = resolveSongLine(snapshot);
    }
  }

  function resolvePresentationNameByPrefix(prefix) {
    const raw = (prefix || '').toString().trim();
    if (!raw) return null;
    const normalized = raw.toLowerCase();

    for (const entry of presentationIndex.values()) {
      if (!entry) continue;
      const name = (entry.name || '').toString().trim();
      if (!name) continue;
      if (name.toLowerCase().startsWith(normalized)) {
        return name;
      }
    }

    if (Array.isArray(state.libraries)) {
      for (const library of state.libraries) {
        const presentations = Array.isArray(library?.presentations) ? library.presentations : [];
        for (const presentation of presentations) {
          const name = (presentation?.name || '').toString().trim();
          if (!name) continue;
          if (name.toLowerCase().startsWith(normalized)) {
            return name;
          }
        }
      }
    }

    return null;
  }

  function resolveSongLine(snapshot) {
    const status = state.ableset.status || {};
    let presentationName = snapshot && snapshot.presentationName ? snapshot.presentationName.toString().trim() : '';
    if (!presentationName && status.lastSong) {
      const prefix = status.lastSong.prefix || '';
      const fromPrefix = resolvePresentationNameByPrefix(prefix);
      if (fromPrefix) {
        presentationName = fromPrefix;
      } else if (typeof status.lastSong.name === 'string') {
        presentationName = status.lastSong.name;
      }
    }
    if (!presentationName && status.lastSong && typeof status.lastSong.name === 'string') {
      presentationName = status.lastSong.name;
    }
    if (!presentationName) {
      presentationName = '';
    }

    let slideIndex = null;
    if (snapshot && typeof snapshot.currentPosition === 'number') {
      slideIndex = snapshot.currentPosition;
    } else if (status.lastSong && typeof status.lastSong.index === 'number') {
      slideIndex = status.lastSong.index + 1;
    }

    if (!presentationName && slideIndex == null) {
      return '—';
    }

    const slideSuffix = slideIndex != null ? ` (${slideIndex})` : '';
    if (presentationName) {
      return `${presentationName}${slideSuffix}`.trim();
    }
    if (slideIndex != null) {
      return `Slide ${slideIndex}`;
    }
    return '—';
  }

  async function refreshAbleSetStatus(showError) {
    try {
      const response = await fetch('/integrations/ableset/status', { headers: { Accept: 'application/json' } });
      if (!response.ok) {
        throw new Error(`Failed to load AbleSet status (${response.status})`);
      }
      const data = await response.json();
      state.ableset.status = normalizeAbleSetStatus(data);
      renderAbleSetPanel();
    } catch (error) {
      if (showError) {
        console.warn('Unable to refresh AbleSet status', error);
      }
    }
  }

  async function toggleAbleSetAutomation() {
    if (state.ableset.enableLoading) return;
    state.ableset.enableLoading = true;
    renderAbleSetPanel();
    try {
      const response = await fetch('/integrations/ableset/settings', { headers: { Accept: 'application/json' } });
      if (!response.ok) {
        throw new Error(`Failed to fetch AbleSet settings (${response.status})`);
      }
      const settings = await response.json();
      const config = settings && typeof settings === 'object' ? settings : {};
      const payload = {
        enabled: !Boolean(config.enabled),
        host: (config.host || 'fohabl.lan').toString(),
        httpPort: Number.isFinite(Number(config.httpPort ?? config.http_port)) ? Number(config.httpPort ?? config.http_port) : 80,
        oscPort: Number.isFinite(Number(config.oscPort ?? config.osc_port)) ? Number(config.oscPort ?? config.osc_port) : 39051,
        libraryName: (config.libraryName || config.library_name || 'NEW LEVEL').toString(),
        songPrefixLength: Number.isFinite(Number(config.songPrefixLength ?? config.song_prefix_length)) ? Number(config.songPrefixLength ?? config.song_prefix_length) : 3,
      };
      const update = await fetch('/integrations/ableset/settings', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
        body: JSON.stringify(payload),
      });
      if (!update.ok) {
        throw new Error(`Failed to toggle AbleSet (${update.status})`);
      }
      const updated = await update.json();
      state.ableset.status = normalizeAbleSetStatus(updated);
      renderAbleSetPanel();
      showToast(`Ableton automation ${state.ableset.status.enabled ? 'enabled' : 'disabled'}.`, 'info');
    } catch (error) {
      console.error('Unable to toggle AbleSet automation', error);
      showToast('Unable to toggle Ableton automation.', 'error');
    } finally {
      state.ableset.enableLoading = false;
      renderAbleSetPanel();
      refreshAbleSetStatus(false);
    }
  }

  async function toggleAbleSetFollow() {
    const status = state.ableset.status;
    if (!status.enabled) {
      showToast('Enable Ableton automation first.', 'warning');
      return;
    }
    if (state.ableset.followLoading) return;
    state.ableset.followLoading = true;
    renderAbleSetPanel();
    try {
      const response = await fetch('/integrations/ableset/follow', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
        body: JSON.stringify({ enabled: !Boolean(status.followEnabled) }),
      });
      if (!response.ok) {
        throw new Error(`Failed to toggle AbleSet follow (${response.status})`);
      }
      const snapshot = await response.json();
      state.ableset.status = normalizeAbleSetStatus(snapshot);
      renderAbleSetPanel();
      showToast(`Ableton follow ${state.ableset.status.followEnabled ? 'enabled' : 'disabled'}.`, 'info');
      if (state.ableset.status.followEnabled && state.stageSnapshot) {
        syncOperatorSelectionFromStage(state.stageSnapshot.presentationId, state.stageSlideId);
      }
    } catch (error) {
      console.error('Unable to toggle AbleSet follow', error);
      showToast('Unable to toggle Ableton follow.', 'error');
    } finally {
      state.ableset.followLoading = false;
      renderAbleSetPanel();
      refreshAbleSetStatus(false);
    }
  }

  function bindEvents() {
    if (els.ablesetEnable) {
      els.ablesetEnable.addEventListener('click', toggleAbleSetAutomation);
    }
    if (els.ablesetFollow) {
      els.ablesetFollow.addEventListener('click', toggleAbleSetFollow);
    }

    if (els.libraryList) {
      els.libraryList.addEventListener('click', handleLibraryClick);
      els.libraryList.addEventListener('dragover', handlePlaylistDragOver);
      els.libraryList.addEventListener('drop', handlePlaylistDrop);
    }
    if (els.libraryModalList) {
      els.libraryModalList.addEventListener('click', handleLibraryModalClick);
    }
    if (els.libraryModalClose) {
      els.libraryModalClose.addEventListener('click', (event) => {
        event.preventDefault();
        closeLibraryModal();
      });
    }
    if (els.libraryModal) {
      els.libraryModal.addEventListener('click', (event) => {
        if (event.target === els.libraryModal) {
          closeLibraryModal();
        }
      });
    }
    if (els.libraryEditForm) {
      els.libraryEditForm.addEventListener('submit', handleLibraryEditSubmit);
    }
    if (els.libraryEditCancel) {
      els.libraryEditCancel.addEventListener('click', (event) => {
        event.preventDefault();
        if (!state.libraryEditSubmitting) {
          closeLibraryEdit();
        }
      });
    }
    if (els.libraryEditDelete) {
      els.libraryEditDelete.addEventListener('click', handleLibraryEditDelete);
    }
    if (els.libraryEditModal) {
      els.libraryEditModal.addEventListener('click', (event) => {
        if (event.target === els.libraryEditModal && !state.libraryEditSubmitting) {
          closeLibraryEdit();
        }
      });
    }
    if (els.presentationEditForm) {
      els.presentationEditForm.addEventListener('submit', handlePresentationEditSubmit);
    }
    if (els.presentationEditCancel) {
      els.presentationEditCancel.addEventListener('click', (event) => {
        event.preventDefault();
        if (!state.presentationEditSubmitting) {
          closePresentationEdit();
        }
      });
    }
    if (els.presentationEditModal) {
      els.presentationEditModal.addEventListener('click', (event) => {
        if (event.target === els.presentationEditModal && !state.presentationEditSubmitting) {
          closePresentationEdit();
        }
      });
    }
    if (els.countdownInput) {
      els.countdownInput.addEventListener('focus', () => {
        state.countdownInputActive = true;
      });
      els.countdownInput.addEventListener('blur', () => {
        state.countdownInputActive = false;
        if (!state.countdownInputDirty && state.timers) {
          applyTimers(state.timers);
        }
      });
      els.countdownInput.addEventListener('input', () => {
        state.countdownInputDirty = true;
      });
      els.countdownInput.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') {
          event.preventDefault();
          submitCountdownTarget();
        }
      });
    }
    if (els.countdownStart) {
      els.countdownStart.addEventListener('click', (event) => {
        event.preventDefault();
        startCountdownFromInput();
      });
    }
    if (els.countdownOffsetMinus) {
      els.countdownOffsetMinus.addEventListener('click', (event) => {
        event.preventDefault();
        offsetCountdown(-5);
      });
    }
    if (els.countdownOffsetPlus) {
      els.countdownOffsetPlus.addEventListener('click', (event) => {
        event.preventDefault();
        offsetCountdown(5);
      });
    }
    if (els.stageLayoutSelect) {
      els.stageLayoutSelect.addEventListener('change', (event) => {
        submitStageLayout(event.target.value || '');
      });
      els.stageLayoutSelect.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') {
          event.preventDefault();
          submitStageLayout(event.target.value || '');
        }
      });
    }
    if (els.timerOverlayOpen) {
      els.timerOverlayOpen.addEventListener('click', (event) => {
        event.preventDefault();
        const url = new URL('/overlays/timer', window.location.href);
        window.open(url.toString(), '_blank', 'noopener');
      });
    }
    if (els.timerOverlayCopy) {
      els.timerOverlayCopy.addEventListener('click', async (event) => {
        event.preventDefault();
        const url = new URL('/overlays/timer', window.location.href).toString();
        try {
          if (navigator.clipboard && navigator.clipboard.writeText) {
            await navigator.clipboard.writeText(url);
          } else {
            const temp = document.createElement('input');
            temp.value = url;
            document.body.appendChild(temp);
            temp.select();
            document.execCommand('copy');
            document.body.removeChild(temp);
          }
          showToast('Overlay link copied', 'success');
        } catch (copyError) {
          console.error('Failed to copy overlay URL', copyError);
          showToast('Failed to copy link', 'error');
        }
      });
    }
    if (els.libraryCreate) {
      els.libraryCreate.addEventListener('click', (event) => {
        event.preventDefault();
        openLibraryCreate();
      });
    }
    if (els.libraryCount) {
      els.libraryCount.addEventListener('click', (event) => {
        event.preventDefault();
        openLibraryModal();
      });
    }
    if (els.presentationCreate) {
      els.presentationCreate.addEventListener('click', (event) => {
        event.preventDefault();
        if (state.activePlaylistId) {
          handleAddSeparator();
        } else if (state.activeLibraryId) {
          handleCreatePresentation();
        } else {
          showToast('Select a library or playlist first', 'warning');
        }
      });
    }
    if (els.playlistModalList) {
      els.playlistModalList.addEventListener('click', handlePlaylistModalClick);
    }
    if (els.playlistCount) {
      els.playlistCount.addEventListener('click', (event) => {
        event.preventDefault();
        openPlaylistModal();
      });
    }
    if (els.playlistModalClose) {
      els.playlistModalClose.addEventListener('click', (event) => {
        event.preventDefault();
        closePlaylistModal();
      });
    }
    if (els.playlistModal) {
      els.playlistModal.addEventListener('click', (event) => {
        if (event.target === els.playlistModal && !state.playlistEditModalOpen) {
          closePlaylistModal();
        }
      });
    }
    if (els.playlistEditForm) {
      els.playlistEditForm.addEventListener('submit', handlePlaylistEditSubmit);
    }
    if (els.playlistEditCancel) {
      els.playlistEditCancel.addEventListener('click', (event) => {
        event.preventDefault();
        if (!state.playlistEditSubmitting) {
          closePlaylistEdit();
        }
      });
    }
    if (els.playlistEditDelete) {
      els.playlistEditDelete.addEventListener('click', handlePlaylistEditDelete);
    }
    if (els.playlistEditModal) {
      els.playlistEditModal.addEventListener('click', (event) => {
        if (event.target === els.playlistEditModal && !state.playlistEditSubmitting) {
          closePlaylistEdit();
        }
      });
    }
    if (els.playlistList) {
      els.playlistList.addEventListener('click', handlePlaylistClick);
      els.playlistList.addEventListener('dragover', handlePlaylistDragOver);
      els.playlistList.addEventListener('drop', handlePlaylistDrop);
    }
    if (els.playlistCount) {
      els.playlistCount.addEventListener('click', (event) => {
        event.preventDefault();
        openPlaylistModal();
      });
    }
    if (els.presentationList) {
      els.presentationList.addEventListener('click', handlePresentationClick);
      els.presentationList.addEventListener('dragstart', handlePresentationDragStart);
      els.presentationList.addEventListener('dragover', handlePlaylistEntryDragOver);
      els.presentationList.addEventListener('drop', handlePlaylistEntryDrop);
      els.presentationList.addEventListener('dragend', handlePlaylistEntryDragEnd);
    }
    if (els.presentationDropzone) {
      els.presentationDropzone.addEventListener('dragover', handlePlaylistEntryDragOver);
      els.presentationDropzone.addEventListener('drop', handlePlaylistEntryDrop);
      els.presentationDropzone.addEventListener('dragleave', handlePresentationDropzoneDragLeave);
    }
    if (els.slides) {
      els.slides.addEventListener('click', handleSlidesClick);
      els.slides.addEventListener('pointerdown', handleSlidesPointerDown);
      els.slides.addEventListener('dragstart', handleSlideDragStart);
      els.slides.addEventListener('dragover', handleSlideDragOver);
      els.slides.addEventListener('drop', handleSlideDrop);
      els.slides.addEventListener('dragend', handleSlideDragEnd);
      els.slides.addEventListener('blur', handleSlideInputBlur, true);
      els.slides.addEventListener('input', handleSlideInputChange, true);
      els.slides.addEventListener('focusin', handleSlideFieldFocus, true);
    }
    if (els.addSlide) {
      els.addSlide.addEventListener('click', handleAddSlide);
    }
    if (els.clearSlide) {
      els.clearSlide.addEventListener('click', handleClearSlide);
    }
    if (els.lineLimit) {
      els.lineLimit.value = String(state.lineLimit);
      els.lineLimit.addEventListener('change', handleLineLimitChange);
      els.lineLimit.addEventListener('input', handleLineLimitPreview);
    }
    if (els.searchForm) {
      els.searchForm.addEventListener('submit', handleSearchSubmit);
    }
    if (els.searchInput) {
      els.searchInput.addEventListener('input', handleSearchInput);
      els.searchInput.addEventListener('focus', () => {
        if (state.searchQuery.trim()) {
          renderSearchResults();
        }
      });
    }
    if (els.searchClear) {
      els.searchClear.addEventListener('click', handleSearchClear);
    }
    if (els.searchResults) {
      els.searchResults.addEventListener('click', handleSearchResultClick);
      els.searchResults.addEventListener('dragstart', handleSearchResultDragStart, true);
      els.searchResults.addEventListener('dragend', handleSearchResultDragEnd, true);
    }
    if (els.playlistCreate) {
      els.playlistCreate.addEventListener('click', (event) => {
        event.preventDefault();
        openPlaylistCreate();
      });
    }
    if (els.catalogResizer) {
      els.catalogResizer.addEventListener('pointerdown', handleCatalogResizePointerDown);
    }
    document.addEventListener('click', handleSearchOutsideClick);
    document.addEventListener('click', handleTimerButtonClick);
    document.addEventListener('click', handleModeToggle);
    document.addEventListener('click', handleViewToggle);
    document.addEventListener('keydown', handleGlobalKeydown);
  }

  function initialise() {
    bindEvents();
    updateSearchClearVisibility();
    updateAddSlideAvailability();
    updateClearSlideAvailability();
    if (state.libraries.length > 0) {
      state.activeLibraryId = state.libraries[0].id;
      updateContextTitleFromLibrary(state.activeLibraryId);
    } else if (state.playlists.length > 0) {
      state.activePlaylistId = state.playlists[0].id;
      updateContextTitleFromPlaylist(state.activePlaylistId);
    }
    renderLibraries();
    renderPlaylists();
    applyCatalogHeight();
    applySlideSize();

    if (state.activeLibraryId) {
      const library = state.libraries.find((entry) => entry.id === state.activeLibraryId);
      if (library && library.presentations.length > 0) {
        state.currentPresentationId = library.presentations[0].id;
        renderPresentationList();
        loadPresentation(state.currentPresentationId).catch((error) => {
          console.error('Failed to auto-load presentation', error);
        });
      } else {
        renderPresentationList();
      }
    } else if (state.activePlaylistId) {
      renderPresentationList();
    } else {
      renderPresentationList();
    }

    setView(state.view);
    setMode(state.mode);
    applyTimers(state.timers);
    renderStageStatus();
    initialiseStageMonitor();
    renderAbleSetPanel();
    refreshAbleSetStatus(false);
    connectLiveSocket();
  }

  window.addEventListener('beforeunload', () => {
    if (state.stageMonitorRefreshTimer) {
      clearInterval(state.stageMonitorRefreshTimer);
      state.stageMonitorRefreshTimer = null;
    }
  });

  window.__presenterOperatorState = state;
  window.__presenterOperatorTestHelpers = {
    addPresentationToPlaylist: (presentationId, playlistId) =>
      handlePlaylistInsertion(presentationId, playlistId, null, { clearSearch: false }),
    playlistPresentationCount: (playlistId) => {
      if (!playlistId) return -1;
      const playlist = state.playlists.find((item) => item.id === playlistId)
        || state.playlistLookup.get(playlistId);
      if (!playlist || !Array.isArray(playlist.entries)) {
        return -1;
      }
      return playlist.entries.filter((entry) => entry.entryType === 'presentation').length;
    },
    reorderSlides: (presentationId, orderedIds) => reorderSlides(presentationId, orderedIds),
    slideOrder: (presentationId) => {
      if (!presentationId) return [];
      const slides = getSlidesForPresentation(presentationId);
      if (!Array.isArray(slides)) return [];
      return slides.map((slide) => slide.id);
    },
    stageMonitorCounts: () => {
      if (!els.stageMonitor) return { connected: 0, issues: 0 };
      return {
        connected: Number(els.stageMonitor.dataset.connected ?? 0),
        issues: Number(els.stageMonitor.dataset.issues ?? 0),
      };
    },
    resetStageMonitorBaseline: () => resetStageMonitorBaseline(false),
    clearSearch: () => {
      if (els.searchInput) {
        els.searchInput.value = '';
        try {
          els.searchInput.dispatchEvent(new Event('input', { bubbles: true }));
          els.searchInput.dispatchEvent(new Event('change', { bubbles: true }));
        } catch (error) {
          console.warn('dispatch search clear events failed', error);
        }
      }
      state.searchQuery = '';
      state.searchOpen = false;
      clearSearchResults();
      updateSearchClearVisibility();
    },
  };
  initialise();
})();
