'use strict';

(function () {
  const translations = Array.isArray(__TRANSLATIONS__) ? __TRANSLATIONS__ : [];
  const initialBroadcast = __ACTIVE__ || null;

  const state = {
    translations,
    preferences: {
      mainTranslation: translations.length ? translations[0].code : '',
      secondaryTranslation: '',
      characterLimit: 320,
    },
    translationIndex: 0,
    books: [],
    filteredBooks: [],
    selectedBook: '',
    selectedBookCode: '',
    selectedBookNumber: 0,
    chapters: [],
    selectedChapter: 1,
    bookSelectionLocked: false,
    verseStart: 1,
    verseEnd: 1,
    verseEndCustom: false,
    slides: [],
    editMode: false,
    selectedSlides: new Set(),
    presentations: [],
    activePresentationId: '',
    activeBroadcast: initialBroadcast,
    loadedPassages: [],
    liveSocket: null,
    liveReconnectTimer: null,
    toastTimer: null,
    loadingSlides: false,
    savingPreferences: false,
  };

  const loadedPassageKeys = new Map();
  const MAX_LOADED_PASSAGES = 12;

  const els = {
    translationList: document.querySelector('[data-role="translation-list"]'),
    secondaryTranslation: document.querySelector('[data-role="secondary-translation"]'),
    charLimit: document.querySelector('[data-role="char-limit"]'),
    savePreferences: document.querySelector('[data-role="save-preferences"]'),
    bookFilter: document.querySelector('[data-role="book-filter"]'),
    bookList: document.querySelector('[data-role="book-list"]'),
    chapterInput: document.querySelector('[data-role="chapter-input"]'),
    verseStartInput: document.querySelector('[data-role="verse-start"]'),
    verseEndInput: document.querySelector('[data-role="verse-end"]'),
    loadButton: document.querySelector('[data-role="load-button"]'),
    loadedPassages: document.querySelector('[data-role="loaded-passages"]'),
    slidesContainer: document.querySelector('[data-role="slides"]'),
    selectAllSlides: document.querySelector('[data-role="select-all-slides"]'),
    toggleMode: document.querySelector('[data-role="toggle-mode"]'),
    selectionCount: document.querySelector('[data-role="selection-count"]'),
    presentationSelect: document.querySelector('[data-role="presentation-select"]'),
    presentationName: document.querySelector('[data-role="presentation-name"]'),
    addToPresentation: document.querySelector('[data-role="presentation-add"]'),
    refreshPresentations: document.querySelector('[data-role="refresh-presentations"]'),
    presentationsList: document.querySelector('[data-role="presentations-list"]'),
    clearButton: document.querySelector('[data-role="clear-button"]'),
    activeContainer: document.querySelector('[data-role="active-passage"]'),
    toast: document.querySelector('[data-role="toast"]'),
  };

  function showToast(message, variant) {
    if (!els.toast) return;
    els.toast.textContent = message;
    els.toast.dataset.variant = variant || 'info';
    els.toast.dataset.visible = 'true';
    clearTimeout(state.toastTimer);
    state.toastTimer = setTimeout(() => {
      els.toast.dataset.visible = 'false';
    }, 2500);
  }

  function apiFetch(path, options) {
    const url = path.startsWith('http') ? path : `${window.location.origin}${path}`;
    const opts = Object.assign(
      {
        method: 'GET',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'application/json',
        },
      },
      options || {}
    );
    return fetch(url, opts).then(async (response) => {
      const contentType = response.headers.get('content-type') || '';
      if (!response.ok) {
        let details = '';
        if (contentType.includes('application/json')) {
          try {
            const data = await response.json();
            details = data && data.message ? data.message : '';
          } catch (_) {
            details = await response.text();
          }
        } else {
          details = await response.text();
        }
        const message = details || `Request failed with ${response.status}`;
        throw new Error(message);
      }
      if (contentType.includes('application/json')) {
        return response.json();
      }
      return null;
    });
  }

  function renderTranslationSelect(selectEl, selectedCode) {
    if (!selectEl) return;
    const html = state.translations
      .map((translation) => {
        const selected = translation.code === selectedCode ? ' selected' : '';
        const label = translation.language
          ? `${translation.name} (${translation.language})`
          : translation.name;
        return `<option value="${translation.code}"${selected}>${escapeHtml(label)}</option>`;
      })
      .join('');
    selectEl.innerHTML = html;
  }

  function findTranslationIndex(code) {
    if (!Array.isArray(state.translations) || !state.translations.length) {
      return -1;
    }
    if (!code) {
      return 0;
    }
    return state.translations.findIndex((translation) =>
      translation.code.toLowerCase() === String(code).toLowerCase()
    );
  }

  function alignMainTranslation(code) {
    if (!Array.isArray(state.translations) || !state.translations.length) {
      state.preferences.mainTranslation = '';
      state.translationIndex = 0;
      return;
    }
    const index = findTranslationIndex(code);
    state.translationIndex = index >= 0 ? index : 0;
    state.preferences.mainTranslation = state.translations[state.translationIndex].code;
  }

  function renderTranslationList() {
    if (!els.translationList) return;
    if (!Array.isArray(state.translations) || !state.translations.length) {
      els.translationList.innerHTML =
        '<li class=\"operator__list-item operator__list-item--empty\">No translations available.</li>';
      return;
    }
    const html = state.translations
      .map((translation, index) => {
        const label = translation.language
          ? `${translation.name} (${translation.language})`
          : translation.name;
        const active = index === state.translationIndex;
        const activeAttr = active ? ' data-active=\"true\" aria-pressed=\"true\"' : ' aria-pressed=\"false\"';
        return `<li class=\"operator__list-item\" data-index=\"${index}\">
            <button type=\"button\" class=\"operator__list-button\" data-translation-code=\"${escapeHtml(
          translation.code
        )}\"${activeAttr}>
              <span class=\"operator__list-label\">${escapeHtml(label)}</span>
              <span class=\"operator__list-meta\">${escapeHtml(translation.code)}</span>
            </button>
          </li>`;
      })
      .join('');
    els.translationList.innerHTML = html;
  }

  function escapeHtml(value) {
    if (typeof value !== 'string') return value;
    return value
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }

  function formatReference(book, chapter, start, end) {
    if (start === end) {
      return `${book} ${chapter}:${start}`;
    }
    return `${book} ${chapter}:${start}-${end}`;
  }

  function setLoadingSlides(loading) {
    state.loadingSlides = loading;
    if (els.loadButton) {
      els.loadButton.disabled = loading;
      els.loadButton.dataset.loading = loading ? 'true' : 'false';
    }
  }

  function setSavingPreferences(saving) {
    state.savingPreferences = saving;
    if (els.savePreferences) {
      els.savePreferences.disabled = saving;
      els.savePreferences.dataset.loading = saving ? 'true' : 'false';
    }
  }

  async function fetchPreferences() {
    try {
      const prefs = await apiFetch('/bible/preferences');
      if (prefs) {
        state.preferences.mainTranslation = prefs.mainTranslation || state.preferences.mainTranslation;
        state.preferences.secondaryTranslation = prefs.secondaryTranslation || '';
        state.preferences.characterLimit = Number(prefs.characterLimit) || state.preferences.characterLimit;
      }
    } catch (error) {
      console.warn('Failed to load Bible preferences', error);
      showToast('Failed to load saved preferences', 'warning');
    }
    alignMainTranslation(state.preferences.mainTranslation);
    renderPreferences();
  }

  function renderPreferences() {
    renderTranslationList();
    renderTranslationSelect(els.secondaryTranslation, state.preferences.secondaryTranslation);
    if (els.charLimit) {
      els.charLimit.value = state.preferences.characterLimit;
    }
  }

  async function savePreferences() {
    setSavingPreferences(true);
    try {
      const payload = {
        mainTranslation: state.preferences.mainTranslation,
        secondaryTranslation: state.preferences.secondaryTranslation || null,
        characterLimit: state.preferences.characterLimit,
      };
      await apiFetch('/bible/preferences', {
        method: 'PUT',
        body: JSON.stringify(payload),
      });
      showToast('Preferences saved', 'success');
    } catch (error) {
      console.error('Failed to save preferences', error);
      showToast('Failed to save preferences', 'error');
    } finally {
      setSavingPreferences(false);
    }
  }

  async function loadBooks(preserveSelection = true) {
    if (!state.preferences.mainTranslation) return;
    try {
      const data = await apiFetch(`/bible/books?translation=${encodeURIComponent(state.preferences.mainTranslation)}`);
      const previousBook = preserveSelection ? state.selectedBook : '';
      const previousBookCode = preserveSelection ? state.selectedBookCode : '';
      const previousBookNumber = preserveSelection ? state.selectedBookNumber : 0;
      const previousChapter = preserveSelection ? state.selectedChapter : 1;
      const previousVerseStart = preserveSelection ? state.verseStart : 1;
      const previousVerseEnd = preserveSelection ? state.verseEnd : 1;
      const previousVerseEndCustom = preserveSelection ? state.verseEndCustom : false;

      const rawBooks = Array.isArray(data) ? data : [];
      state.books = rawBooks.map((entry) => {
        const chapters = Array.isArray(entry.chapters)
          ? entry.chapters.map((chapter) => ({
              number: Number(chapter.number) || 1,
              verseCount:
                Number(chapter.verse_count ?? chapter.verseCount ?? chapter.verse_count ?? 0) || 0,
            }))
          : [];
        return {
          name: entry.book || '',
          code: entry.code || '',
          number: Number(entry.number) || 0,
          chapters,
        };
      });
      state.filteredBooks = state.books.slice();
      state.bookSelectionLocked = false;

      if (state.books.length) {
        const matchedEntry = state.books.find((entry) => {
          if (previousBookCode && entry.code) {
            return entry.code.toLowerCase() === previousBookCode.toLowerCase();
          }
          if (previousBookNumber && entry.number) {
            return entry.number === previousBookNumber;
          }
          return entry.name === previousBook;
        });
        const target = matchedEntry || state.books[0];
        state.selectedBook = target.name;
        state.selectedBookCode = target.code || '';
        state.selectedBookNumber = target.number || 0;
        state.chapters = target.chapters || [];
        const maxChapter = state.chapters.length
          ? state.chapters[state.chapters.length - 1].number || 1
          : 1;
        state.selectedChapter = Math.min(Math.max(previousChapter, 1), maxChapter);
        state.verseStart = Math.max(previousVerseStart, 1);
        state.verseEnd = Math.max(previousVerseEnd, state.verseStart);
        state.verseEndCustom = previousVerseEndCustom;
        applyChapterDefaults();
      } else {
        state.selectedBook = '';
        state.selectedBookCode = '';
        state.selectedBookNumber = 0;
        state.chapters = [];
        state.selectedChapter = 1;
        state.verseStart = 1;
        state.verseEnd = 1;
        state.verseEndCustom = false;
      }

      renderBookList();
      updateReferenceInputs();
    } catch (error) {
      console.error('Failed to load books', error);
      showToast('Failed to load books', 'error');
    }
  }

  function renderBookList() {
    if (!els.bookList) return;
    if (!state.filteredBooks.length) {
      els.bookList.innerHTML =
        '<div class=\"operator__list-item operator__list-item--empty\">No books available.</div>';
      return;
    }
    const html = state.filteredBooks
      .map((entry) => {
        const isSelected = entry.name === state.selectedBook;
        const chapterCount = Array.isArray(entry.chapters) ? entry.chapters.length : 0;
        const meta = chapterCount
          ? `<span class=\"operator__list-meta\">${chapterCount} ch.</span>`
          : '';
        const activeAttr = isSelected ? ' data-active=\"true\"' : '';
        return `<div class=\"operator__list-item\">
          <button type=\"button\" class=\"operator__list-button\"${activeAttr} data-book=\"${escapeHtml(
          entry.name
        )}\" data-book-code=\"${escapeHtml(entry.code || '')}\" data-book-number=\"${entry.number}\">\n            <span class=\"operator__list-label\">${escapeHtml(entry.name)}</span>
            ${meta}
          </button>
        </div>`;
      })
      .join('');
    els.bookList.innerHTML = html;
  }

  function applyChapterDefaults() {
    const chapterEntry = (state.chapters || []).find((c) => c.number === state.selectedChapter);
    const maxChapter = state.chapters.length ? state.chapters[state.chapters.length - 1].number : 1;
    state.selectedChapter = Math.min(Math.max(state.selectedChapter, 1), maxChapter || 1);
    if (chapterEntry) {
      const verseCount = chapterEntry.verseCount || chapterEntry.verse_count || 1;
      state.verseStart = Math.min(Math.max(state.verseStart, 1), verseCount);
      if (state.verseEndCustom) {
        state.verseEnd = Math.min(Math.max(state.verseEnd, state.verseStart), verseCount);
      } else {
        state.verseEnd = verseCount;
      }
    } else {
      state.verseStart = 1;
      state.verseEnd = 1;
      state.verseEndCustom = false;
    }
    updateReferenceInputs();
  }

  function updateReferenceInputs() {
    if (els.chapterInput) {
      const maxChapter = state.chapters.length ? state.chapters[state.chapters.length - 1].number : 1;
      els.chapterInput.value = state.selectedChapter;
      els.chapterInput.min = 1;
      els.chapterInput.max = Math.max(1, maxChapter);
    }
    const verseCount = getCurrentVerseCount();
    if (els.verseStartInput) {
      els.verseStartInput.value = state.verseStart;
      els.verseStartInput.min = 1;
      els.verseStartInput.max = verseCount;
    }
    if (els.verseEndInput) {
      if (state.verseEndCustom) {
        els.verseEndInput.value = state.verseEnd;
      } else {
        els.verseEndInput.value = '';
      }
      els.verseEndInput.placeholder = 'All';
      els.verseEndInput.min = state.verseStart;
      els.verseEndInput.max = verseCount;
    }
  }

  function getCurrentVerseCount() {
    const chapterEntry = (state.chapters || []).find((c) => c.number === state.selectedChapter);
    return chapterEntry ? chapterEntry.verseCount || chapterEntry.verse_count || 1 : 1;
  }

  function filterBooks(value) {
    const term = value.trim().toLowerCase();
    if (!term) {
      if (state.bookSelectionLocked && state.selectedBook) {
        state.filteredBooks = state.books.filter((entry) => {
          if (state.selectedBookCode && entry.code) {
            return entry.code === state.selectedBookCode;
          }
          if (state.selectedBookNumber && entry.number) {
            return entry.number === state.selectedBookNumber;
          }
          return entry.name === state.selectedBook;
        });
      } else {
        state.filteredBooks = state.books.slice();
      }
    } else {
      state.filteredBooks = state.books.filter((entry) => {
        const nameMatch = entry.name && entry.name.toLowerCase().includes(term);
        const codeMatch = entry.code && entry.code.toLowerCase().includes(term);
        return nameMatch || codeMatch;
      });
      state.bookSelectionLocked = false;
    }
    if (!state.filteredBooks.find((entry) => entry.name === state.selectedBook) && state.filteredBooks.length) {
      const next = state.filteredBooks[0];
      state.selectedBook = next.name;
      state.selectedBookCode = next.code || '';
      state.selectedBookNumber = next.number || 0;
      state.chapters = next.chapters || [];
      state.selectedChapter = 1;
      state.verseStart = 1;
      state.verseEnd = 1;
      state.verseEndCustom = false;
      applyChapterDefaults();
    }
    renderBookList();
    updateReferenceInputs();
  }

  async function loadSlides() {
    if (!state.selectedBook) {
      showToast('Select a book first', 'warning');
      return;
    }
    const chapterValue = Number(els.chapterInput ? els.chapterInput.value : state.selectedChapter) || state.selectedChapter;
    state.selectedChapter = Math.max(1, chapterValue);
    const verseCount = getCurrentVerseCount();
    const verseStartRaw = els.verseStartInput ? els.verseStartInput.value : `${state.verseStart}`;
    const verseEndRaw = els.verseEndInput ? els.verseEndInput.value : '';
    const candidateStart = Number(verseStartRaw);
    const verseStartValue = Number.isFinite(candidateStart) ? candidateStart : state.verseStart;
    const candidateEnd = verseEndRaw && verseEndRaw.trim().length ? Number(verseEndRaw) : null;
    state.verseStart = Math.min(Math.max(verseStartValue, 1), verseCount);
    if (candidateEnd === null) {
      state.verseEndCustom = false;
      state.verseEnd = verseCount;
    } else {
      state.verseEndCustom = true;
      const safeEnd = Number.isFinite(candidateEnd) ? candidateEnd : state.verseStart;
      state.verseEnd = Math.min(Math.max(safeEnd, state.verseStart), verseCount);
    }
    updateReferenceInputs();
    setLoadingSlides(true);
    const payload = {
      mainTranslation: state.preferences.mainTranslation,
      secondaryTranslation: state.preferences.secondaryTranslation || null,
      book: state.selectedBook,
      bookCode: state.selectedBookCode || null,
      bookNumber: state.selectedBookNumber || null,
      chapter: state.selectedChapter,
      verseStart: state.verseStart,
      verseEnd: state.verseEndCustom ? state.verseEnd : null,
      characterLimit: state.preferences.characterLimit,
    };
    try {
      const response = await apiFetch('/bible/resolve', {
        method: 'POST',
        body: JSON.stringify(payload),
      });
      state.slides = Array.isArray(response.slides)
        ? response.slides.map((slide) => {
            const metadata = slide.metadata || null;
            const mainReference = slide.main_reference || deriveReferenceFromMetadata(metadata);
            const translationReference = slide.translation_reference || deriveReferenceFromMetadata(metadata);
            return {
              id: slide.id,
              order: slide.order,
              main: slide.main,
              translation: slide.translation,
              stage: slide.stage,
              group: slide.group || null,
              metadata,
              mainReference,
              translationReference,
            };
          })
        : [];
      state.selectedSlides.clear();
      state.editMode = false;
      updateMode();
      renderSlides();
      updateSelectionLabel();
      recordLoadedPassage(payload);
      showToast('Slides loaded', 'success');
    } catch (error) {
      console.error('Failed to load slides', error);
      showToast(error.message || 'Failed to load slides', 'error');
    } finally {
      setLoadingSlides(false);
    }
  }

  function renderSlides() {
    if (!els.slidesContainer) return;
    if (!state.slides.length) {
      els.slidesContainer.innerHTML = "<p class='operator__slides-empty'>Load a passage to populate slides.</p>";
      return;
    }
    const html = state.slides
      .map((slide, index) => renderSlideCard(slide, index))
      .join('');
    els.slidesContainer.innerHTML = html;
  }

  function renderSlideCard(slide, index) {
    const checked = state.selectedSlides.has(slide.id) ? ' checked' : '';
    const header = `
      <header class='operator__slide-header'>
        <div class='operator__slide-header-left'>
          <label class='operator__slide-index operator__slide-index--select'>
            <input type='checkbox' data-role='slide-select'${checked} />
            <span>${index + 1}</span>
          </label>
        </div>
        <div class='operator__slide-controls operator__slide-controls--compact'>
          <button type='button' class='operator__list-action operator__list-action--primary' data-role='slide-trigger'>Trigger</button>
        </div>
      </header>
    `;
    if (state.editMode) {
      return `
        <article class='operator__slide-card operator__slide-card--bible operator__slide-card--edit' data-slide-id='${slide.id}' data-index='${index}'>
          ${header}
          <section class='operator__slide-editor operator__slide-editor--bible'>
            <label>
              <span>Main</span>
              <textarea data-role='slide-main'>${escapeHtml(slide.main || '')}</textarea>
            </label>
            <label>
              <span>Translation</span>
              <textarea data-role='slide-translation'>${escapeHtml(slide.translation || '')}</textarea>
            </label>
            <div class='operator__slide-editor-grid'>
              <label>
                <span>Main Reference</span>
                <input type='text' data-role='slide-main-ref' value='${escapeHtml(slide.mainReference || '')}' />
              </label>
              <label>
                <span>Translation Reference</span>
                <input type='text' data-role='slide-translation-ref' value='${escapeHtml(slide.translationReference || '')}' />
              </label>
            </div>
          </section>
        </article>
      `;
    }
    const translationMarkup = slide.translation && slide.translation.trim().length
      ? `<div class='operator__slide-text operator__slide-text--translation operator__slide-text--secondary'>${lineBreakHtml(slide.translation)}</div>`
      : '';
    const references = buildReferenceHtml(slide);
    return `
      <article class='operator__slide-card operator__slide-card--bible' data-slide-id='${slide.id}' data-index='${index}'>
        ${header}
        <section class='operator__slide-bodies operator__slide-bodies--bible'>
          <div class='operator__slide-text operator__slide-text--main'>${lineBreakHtml(slide.main)}</div>
          ${translationMarkup}
          ${references}
        </section>
      </article>
    `;
  }

  function buildReferenceHtml(slide) {
    const pieces = [];
    if (slide.mainReference) {
      pieces.push(`<span class='operator__slide-reference'>${escapeHtml(slide.mainReference)}</span>`);
    } else if (slide.metadata && slide.metadata.bible) {
      const verses = slide.metadata.bible.verses || [];
      if (verses.length) {
        const start = verses[0].start;
        const end = verses[verses.length - 1].end;
        pieces.push(`<span class='operator__slide-reference'>${escapeHtml(formatReference(slide.metadata.bible.book, slide.metadata.bible.chapter, start, end))}</span>`);
      }
    }
    if (slide.translationReference) {
      pieces.push(`<span class='operator__slide-reference operator__slide-reference--secondary'>${escapeHtml(slide.translationReference)}</span>`);
    }
    if (!pieces.length) {
      return '';
    }
    return `<footer class='operator__slide-footer'>${pieces.join('')}</footer>`;
  }

  function lineBreakHtml(value) {
    return escapeHtml(value || '').replace(/\n/g, '<br />');
  }
  function resolveTranslationByCode(code) {
    if (!code) return null;
    const target = String(code).toLowerCase();
    return state.translations.find((translation) => translation.code.toLowerCase() === target) || null;
  }

  function buildLoadedPassageKey(entry) {
    if (!entry) return null;
    const endValue = entry.verseEnd === null ? 'all' : entry.verseEnd;
    return [
      entry.mainTranslation || '',
      entry.secondaryTranslation || '',
      entry.bookCode || entry.book || '',
      entry.bookNumber || 0,
      entry.chapter || 0,
      entry.verseStart || 0,
      endValue,
      entry.characterLimit || 0,
    ].join('|');
  }

  function recordLoadedPassage(payload) {
    if (!payload) return;
    const normalized = {
      mainTranslation: payload.mainTranslation || '',
      secondaryTranslation: payload.secondaryTranslation || '',
      book: payload.book || '',
      bookCode: payload.bookCode || '',
      bookNumber: typeof payload.bookNumber === 'number' ? payload.bookNumber : null,
      chapter: payload.chapter || 1,
      verseStart: payload.verseStart || 1,
      verseEnd: typeof payload.verseEnd === 'number' ? payload.verseEnd : null,
      characterLimit: payload.characterLimit || state.preferences.characterLimit,
    };
    const key = buildLoadedPassageKey(normalized);
    if (!key) return;
    const existingIndex = loadedPassageKeys.get(key);
    if (typeof existingIndex === 'number') {
      state.loadedPassages.splice(existingIndex, 1);
    }
    const main = resolveTranslationByCode(normalized.mainTranslation);
    const secondary = resolveTranslationByCode(normalized.secondaryTranslation);
    const referenceEnd = normalized.verseEnd === null ? state.verseEnd : normalized.verseEnd;
    const entry = {
      key,
      translationCode: normalized.mainTranslation,
      translationName: main ? main.name : normalized.mainTranslation,
      secondaryTranslationCode: normalized.secondaryTranslation || null,
      secondaryTranslationName: secondary ? secondary.name : null,
      book: normalized.book,
      bookCode: normalized.bookCode || null,
      bookNumber: normalized.bookNumber,
      chapter: normalized.chapter,
      verseStart: normalized.verseStart,
      verseEnd: referenceEnd,
      includeFullChapter: normalized.verseEnd === null,
      characterLimit: normalized.characterLimit,
      timestamp: Date.now(),
    };
    state.loadedPassages.unshift(entry);
    while (state.loadedPassages.length > MAX_LOADED_PASSAGES) {
      const removed = state.loadedPassages.pop();
      if (removed) {
        loadedPassageKeys.delete(removed.key);
      }
    }
    state.loadedPassages.forEach((item, index) => loadedPassageKeys.set(item.key, index));
    renderLoadedPassages();
  }

  function renderLoadedPassages() {
    if (!els.loadedPassages) return;
    if (!state.loadedPassages.length) {
      els.loadedPassages.innerHTML = "<li class='operator__list-item operator__list-item--empty'>Load a passage to populate this list.</li>";
      return;
    }
    const html = state.loadedPassages
      .map((entry) => {
        const reference = entry.includeFullChapter
          ? `${entry.book} ${entry.chapter}`
          : formatReference(entry.book, entry.chapter, entry.verseStart, entry.verseEnd);
        const translationLabel = entry.translationName || entry.translationCode;
        const secondaryLabel = entry.secondaryTranslationName || entry.secondaryTranslationCode;
        const secondaryBadge = secondaryLabel
          ? `<span class='operator__list-meta operator__list-meta--secondary'>${escapeHtml(secondaryLabel)}</span>`
          : '';
        return `<li class='operator__list-item' data-loaded-key='${entry.key}'>
          <button type='button' class='operator__list-button'>
            <span class='operator__list-label'>${escapeHtml(reference)}</span>
            <span class='operator__list-meta'>${escapeHtml(translationLabel)}</span>
            ${secondaryBadge}
          </button>
        </li>`;
      })
      .join('');
    els.loadedPassages.innerHTML = html;
  }

  async function applyLoadedPassage(entry) {
    if (!entry) return;
    const translationChanged = entry.translationCode && entry.translationCode !== state.preferences.mainTranslation;
    if (translationChanged) {
      alignMainTranslation(entry.translationCode);
      renderTranslationList();
      await loadBooks(false);
    } else {
      await loadBooks();
    }
    state.preferences.secondaryTranslation = entry.secondaryTranslationCode || '';
    renderTranslationSelect(els.secondaryTranslation, state.preferences.secondaryTranslation);
    if (typeof entry.characterLimit === 'number' && entry.characterLimit > 0) {
      state.preferences.characterLimit = entry.characterLimit;
      if (els.charLimit) {
        els.charLimit.value = entry.characterLimit;
      }
    }
    const bookEntry = state.books.find((bk) => {
      if (entry.bookCode && bk.code) {
        return bk.code === entry.bookCode;
      }
      if (entry.bookNumber && bk.number) {
        return bk.number === entry.bookNumber;
      }
      return bk.name === entry.book;
    });
    if (bookEntry) {
      state.selectedBook = bookEntry.name;
      state.selectedBookCode = bookEntry.code || '';
      state.selectedBookNumber = bookEntry.number || 0;
      state.chapters = bookEntry.chapters || [];
    } else if (state.books.length) {
      const fallback = state.books[0];
      state.selectedBook = fallback.name;
      state.selectedBookCode = fallback.code || '';
      state.selectedBookNumber = fallback.number || 0;
      state.chapters = fallback.chapters || [];
    } else {
      showToast('No books available for the selected translation', 'error');
      return;
    }
    state.bookSelectionLocked = true;
    state.filteredBooks = state.books.filter((bk) => {
      if (state.selectedBookCode && bk.code) {
        return bk.code === state.selectedBookCode;
      }
      if (state.selectedBookNumber && bk.number) {
        return bk.number === state.selectedBookNumber;
      }
      return bk.name === state.selectedBook;
    });
    state.selectedChapter = entry.chapter;
    state.verseStart = entry.verseStart;
    state.verseEndCustom = !entry.includeFullChapter;
    state.verseEnd = entry.includeFullChapter ? getCurrentVerseCount() : entry.verseEnd;
    applyChapterDefaults();
    renderBookList();
    updateReferenceInputs();
    await loadSlides();
  }

  function updateSelectionLabel() {
    if (!els.selectionCount) return;
    const count = state.selectedSlides.size;
    els.selectionCount.textContent = `${count} selected`;
  }

  function updateMode() {
    if (typeof document !== 'undefined' && document.body) {
      document.body.dataset.mode = state.editMode ? 'edit' : 'live';
    }
    if (els.toggleMode) {
      els.toggleMode.textContent = state.editMode ? 'Switch to Live Mode' : 'Switch to Edit Mode';
    }
  }

  function toggleMode() {
    state.editMode = !state.editMode;
    updateMode();
    renderSlides();
  }

  function ensureBibleMetadata(slide) {
    if (!slide || typeof slide !== 'object') {
      return {};
    }
    if (!slide.metadata || typeof slide.metadata !== 'object') {
      slide.metadata = {};
    }
    if (!slide.metadata.bible || typeof slide.metadata.bible !== 'object') {
      slide.metadata.bible = {};
    }
    return slide.metadata.bible;
  }

  function deriveReferenceFromMetadata(metadata) {
    if (!metadata || !metadata.bible) {
      return null;
    }
    const bible = metadata.bible;
    const verses = Array.isArray(bible.verses) ? bible.verses : [];
    if (!verses.length) {
      return null;
    }
    const start = verses[0].start;
    const end = verses[verses.length - 1].end;
    return formatReference(bible.book, bible.chapter, start, end);
  }

  async function appendSlidesToPresentation() {
    if (!state.selectedSlides.size) {
      showToast('Select at least one slide', 'warning');
      return;
    }
    let targetPresentationId = els.presentationSelect && els.presentationSelect.value;
    const newName = els.presentationName ? els.presentationName.value.trim() : '';
    try {
      let presentationDetail = null;
      if (newName) {
        presentationDetail = await apiFetch('/bible/presentations', {
          method: 'POST',
          body: JSON.stringify({ name: newName }),
        });
        await loadPresentations();
        targetPresentationId = presentationDetail.id;
        if (els.presentationName) {
          els.presentationName.value = '';
        }
      }
      if (!targetPresentationId) {
        showToast('Select or create a presentation', 'warning');
        return;
      }
      const slides = state.slides
        .filter((slide) => state.selectedSlides.has(slide.id))
        .map(slideToPayload);
      const detail = await apiFetch(`/bible/presentations/${targetPresentationId}/append`, {
        method: 'POST',
        body: JSON.stringify({ slides }),
      });
      showToast(`Added ${slides.length} slide${slides.length === 1 ? '' : 's'}`, 'success');
      if (presentationDetail === null) {
        // fetch detail to keep UI fresh
        await loadPresentations();
      }
      state.selectedSlides.clear();
      renderSlides();
      updateSelectionLabel();
    } catch (error) {
      console.error('Failed to append slides', error);
      showToast(error.message || 'Failed to append slides', 'error');
    }
  }

  function slideToPayload(slide) {
    const metadata = slide.metadata ? JSON.parse(JSON.stringify(slide.metadata)) : null;
    if (metadata && metadata.bible) {
      const bibleMeta = metadata.bible;
      const mainLabel = slide.mainReference || bibleMeta.mainReferenceLabel || bibleMeta.main_reference_label || null;
      const translationLabel =
        slide.translationReference || bibleMeta.translationReferenceLabel || bibleMeta.translation_reference_label || null;
      bibleMeta.mainReferenceLabel = mainLabel;
      bibleMeta.main_reference_label = mainLabel;
      bibleMeta.translationReferenceLabel = translationLabel;
      bibleMeta.translation_reference_label = translationLabel;
    }
    return {
      main: slide.main,
      translation: slide.translation || '',
      stage: slide.stage || slide.main,
      group: slide.group || null,
      metadata,
    };
  }

  async function loadPresentations() {
    try {
      const data = await apiFetch('/bible/presentations');
      state.presentations = Array.isArray(data) ? data : [];
      renderPresentationSelect();
      renderPresentations();
    } catch (error) {
      console.error('Failed to load presentations', error);
      showToast('Failed to load presentations', 'error');
    }
  }

  async function renamePresentation(presentationId, currentName) {
    const next = typeof window !== 'undefined'
      ? window.prompt('Rename presentation', currentName || '')
      : null;
    if (next === null) {
      return;
    }
    const trimmed = next.trim();
    if (!trimmed.length || trimmed === currentName) {
      return;
    }
    try {
      await apiFetch(`/bible/presentations/${presentationId}`, {
        method: 'PATCH',
        body: JSON.stringify({ name: trimmed }),
      });
      showToast('Presentation renamed', 'success');
      await loadPresentations();
    } catch (error) {
      console.error('Failed to rename presentation', error);
      showToast(error.message || 'Failed to rename presentation', 'error');
    }
  }

  function renderPresentationSelect() {
    if (!els.presentationSelect) return;
    const options = state.presentations
      .map((presentation) => `<option value="${presentation.id}">${escapeHtml(presentation.name)}</option>`)
      .join('');
    els.presentationSelect.innerHTML = `<option value="">Select existing…</option>${options}`;
  }

  function renderPresentations() {
    if (!els.presentationsList) return;
    if (!state.presentations.length) {
      els.presentationsList.innerHTML = "<p class='operator__slides-empty'>No Bible presentations yet.</p>";
      return;
    }
    const html = state.presentations
      .map((presentation) => {
        const escapedName = escapeHtml(presentation.name);
        return `
          <article class='operator__presentation-card' data-presentation-id='${presentation.id}'>
            <header>
              <strong>${escapedName}</strong>
              <button type='button' class='operator__list-action operator__list-action--secondary' data-role='presentation-rename' data-presentation-id='${presentation.id}' data-presentation-name='${escapedName}'>Rename</button>
            </header>
            <p>${presentation.slide_count || 0} slide${presentation.slide_count === 1 ? '' : 's'}</p>
          </article>
        `;
      })
      .join('');
    els.presentationsList.innerHTML = html;
  }

  function renderActive() {
    if (!els.activeContainer) return;
    if (!state.activeBroadcast) {
      els.activeContainer.innerHTML = `
        <article class='operator__active-card operator__active-card--empty'>
          <header>
            <strong>No active passage</strong>
            <span></span>
          </header>
          <p>Trigger a slide to broadcast scripture.</p>
        </article>
      `;
      return;
    }
    const broadcast = state.activeBroadcast.passage;
    const verses = broadcast.reference;
    const verseStart = verses.verse_start ?? verses.verseStart;
    const verseEnd = verses.verse_end ?? verses.verseEnd ?? verseStart;
    const reference = formatReference(verses.book, verses.chapter, verseStart, verseEnd);
    const translationLabel = broadcast.translation ? broadcast.translation.name : '';
    els.activeContainer.innerHTML = `
      <article class='operator__active-card'>
        <header>
          <strong>${escapeHtml(reference)}</strong>
          <span>${escapeHtml(translationLabel)}</span>
        </header>
        <p>${escapeHtml(broadcast.text || '')}</p>
      </article>
    `;
  }

  function connectLiveSocket() {
    if (state.liveSocket) {
      try {
        state.liveSocket.close();
      } catch (_) {
        /* ignore */
      }
    }
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(`${protocol}//${window.location.host}/live/ws`);
    state.liveSocket = socket;
    socket.addEventListener('message', (event) => {
      try {
        const payload = JSON.parse(event.data);
        if (payload.type === 'bible' || payload.type === 'Bible') {
          state.activeBroadcast = payload.broadcast || null;
          renderActive();
        } else if (payload.type === 'bible_cleared' || payload.type === 'BibleCleared') {
          state.activeBroadcast = null;
          renderActive();
        }
      } catch (error) {
        console.warn('Failed to parse bible payload', error);
      }
    });
    socket.addEventListener('close', () => {
      if (state.liveReconnectTimer) return;
      state.liveReconnectTimer = setTimeout(() => {
        state.liveReconnectTimer = null;
        connectLiveSocket();
      }, 2000);
    });
    socket.addEventListener('error', (error) => {
      console.error('Bible live socket error', error);
      try {
        socket.close();
      } catch (_) {
        /* ignore */
      }
    });
  }

  async function triggerSlideById(slideId) {
    const slide = state.slides.find((entry) => entry.id === slideId);
    if (!slide || !slide.metadata || !slide.metadata.bible) {
      showToast('Slide metadata missing', 'error');
      return;
    }
    const bibleMeta = ensureBibleMetadata(slide);
    const verses = Array.isArray(bibleMeta.verses) ? bibleMeta.verses : [];
    if (!verses.length) {
      showToast('Verse metadata missing', 'error');
      return;
    }
    const translationCode = bibleMeta.translation_code || bibleMeta.translationCode;
    if (!translationCode) {
      showToast('Translation metadata missing', 'error');
      return;
    }
    const verseStart = verses[0].start;
    const verseEnd = verses[verses.length - 1].end;
    const book = bibleMeta.book || state.selectedBook;
    const chapter = typeof bibleMeta.chapter === 'number' ? bibleMeta.chapter : state.selectedChapter;
    try {
      const payload = {
        translation: translationCode,
        book,
        chapter,
        verseStart,
        verseEnd,
      };
      const response = await apiFetch('/bible/trigger', {
        method: 'POST',
        body: JSON.stringify(payload),
      });
      state.activeBroadcast = response;
      renderActive();
      showToast('Slide triggered', 'success');
    } catch (error) {
      console.error('Failed to trigger slide', error);
      showToast(error.message || 'Failed to trigger slide', 'error');
    }
  }

  async function clearBroadcast() {
    try {
      await apiFetch('/bible/clear', { method: 'POST' });
      state.activeBroadcast = null;
      renderActive();
      showToast('Broadcast cleared', 'success');
    } catch (error) {
      console.error('Failed to clear broadcast', error);
      showToast('Failed to clear broadcast', 'error');
    }
  }

  function onSlidesContainerClick(event) {
    const card = event.target.closest('[data-slide-id]');
    if (!card) return;
    const slideId = card.getAttribute('data-slide-id');
    if (!slideId) return;
    if (event.target.matches('[data-role="slide-select"]')) {
      if (event.target.checked) {
        state.selectedSlides.add(slideId);
      } else {
        state.selectedSlides.delete(slideId);
      }
      updateSelectionLabel();
      return;
    }
    if (event.target.matches('[data-role="slide-trigger"]')) {
      triggerSlideById(slideId);
    }
  }

  function onSlidesContainerInput(event) {
    const wrapper = event.target.closest('[data-slide-id]');
    if (!wrapper) return;
    const slideId = wrapper.getAttribute('data-slide-id');
    const slide = state.slides.find((entry) => entry.id === slideId);
    if (!slide) return;
    if (event.target.matches('[data-role="slide-main"]')) {
      slide.main = event.target.value;
    } else if (event.target.matches('[data-role="slide-translation"]')) {
      slide.translation = event.target.value;
    } else if (event.target.matches('[data-role="slide-main-ref"]')) {
      const value = event.target.value;
      slide.mainReference = value;
      const bibleMeta = ensureBibleMetadata(slide);
      bibleMeta.mainReferenceLabel = value || null;
      bibleMeta.main_reference_label = bibleMeta.mainReferenceLabel;
    } else if (event.target.matches('[data-role="slide-translation-ref"]')) {
      const value = event.target.value;
      slide.translationReference = value;
      const bibleMeta = ensureBibleMetadata(slide);
      bibleMeta.translationReferenceLabel = value || null;
      bibleMeta.translation_reference_label = bibleMeta.translationReferenceLabel;
    }
  }

  function selectAllSlides() {
    if (!state.slides.length) return;
    const allSelected = state.selectedSlides.size === state.slides.length;
    if (allSelected) {
      state.selectedSlides.clear();
    } else {
      state.slides.forEach((slide) => state.selectedSlides.add(slide.id));
    }
    renderSlides();
    updateSelectionLabel();
  }

  function initialiseEvents() {
    document.querySelectorAll('[data-role="view-toggle"]').forEach((button) => {
      const href = button.getAttribute('data-href');
      if (!href) return;
      button.addEventListener('click', () => {
        window.location.href = href;
      });
    });
    if (els.translationList) {
      els.translationList.addEventListener('click', async (event) => {
        const button = event.target.closest('[data-translation-code]');
        if (!button) return;
        const code = button.getAttribute('data-translation-code');
        if (!code || code === state.preferences.mainTranslation) {
          return;
        }
        alignMainTranslation(code);
        renderTranslationList();
        try {
          await savePreferences();
        } catch (error) {
          console.warn('Failed to persist Bible preferences', error);
        }
        await loadBooks();
      });
    }
    if (els.secondaryTranslation) {
      els.secondaryTranslation.addEventListener('change', (event) => {
        state.preferences.secondaryTranslation = event.target.value;
      });
    }
    if (els.charLimit) {
      els.charLimit.addEventListener('input', (event) => {
        const value = Number(event.target.value) || 0;
        state.preferences.characterLimit = Math.min(Math.max(value, 1), 4000);
      });
    }
    if (els.savePreferences) {
      els.savePreferences.addEventListener('click', savePreferences);
    }
    if (els.bookFilter) {
      els.bookFilter.addEventListener('input', (event) => {
        filterBooks(event.target.value);
      });
    }
    if (els.bookList) {
      els.bookList.addEventListener('click', (event) => {
        const button = event.target.closest('[data-book]');
        if (!button) return;
        const nextBook = button.getAttribute('data-book');
        if (!nextBook) return;
        const nextCode = button.getAttribute('data-book-code') || '';
        const nextNumber = Number(button.getAttribute('data-book-number') || '0') || 0;
        state.selectedBook = nextBook;
        state.selectedBookCode = nextCode;
        state.selectedBookNumber = nextNumber;
        const entry = state.books.find((bk) => {
          if (nextCode && bk.code) {
            return bk.code === nextCode;
          }
          if (nextNumber && bk.number) {
            return bk.number === nextNumber;
          }
          return bk.name === nextBook;
        });
        state.chapters = entry ? entry.chapters || [] : [];
        state.selectedChapter = 1;
        state.verseStart = 1;
        state.verseEnd = 1;
        state.verseEndCustom = false;
        state.bookSelectionLocked = true;
        state.filteredBooks = state.books.filter((bk) => {
          if (state.selectedBookCode && bk.code) {
            return bk.code === state.selectedBookCode;
          }
          if (state.selectedBookNumber && bk.number) {
            return bk.number === state.selectedBookNumber;
          }
          return bk.name === state.selectedBook;
        });
        applyChapterDefaults();
        renderBookList();
      });
    }
    if (els.loadedPassages) {
      els.loadedPassages.addEventListener('click', async (event) => {
        const item = event.target.closest('[data-loaded-key]');
        if (!item) return;
        const key = item.getAttribute('data-loaded-key');
        if (!key) return;
        const entry = state.loadedPassages.find((candidate) => candidate.key === key);
        if (!entry) return;
        try {
          await applyLoadedPassage(entry);
        } catch (error) {
          console.error('Failed to apply saved passage', error);
          showToast('Failed to load saved passage', 'error');
        }
      });
    }
    if (els.chapterInput) {
      els.chapterInput.addEventListener('input', (event) => {
        const value = Number(event.target.value) || 1;
        state.selectedChapter = value;
        applyChapterDefaults();
      });
    }
    if (els.verseStartInput) {
      els.verseStartInput.addEventListener('input', (event) => {
        const raw = typeof event.target.value === 'string' ? event.target.value.trim() : '';
        const candidate = Number(raw);
        const verseCount = getCurrentVerseCount();
        const value = Number.isFinite(candidate) ? candidate : 1;
        state.verseStart = Math.min(Math.max(value, 1), verseCount);
        if (state.verseEndCustom) {
          if (state.verseEnd < state.verseStart) {
            state.verseEnd = state.verseStart;
          }
        } else {
          state.verseEnd = verseCount;
        }
        updateReferenceInputs();
      });
    }
    if (els.verseEndInput) {
      els.verseEndInput.addEventListener('input', (event) => {
        const raw = typeof event.target.value === 'string' ? event.target.value.trim() : '';
        const verseCount = getCurrentVerseCount();
        if (!raw) {
          state.verseEndCustom = false;
          state.verseEnd = verseCount;
        } else {
          const candidate = Number(raw);
          const value = Number.isFinite(candidate) ? candidate : state.verseStart;
          state.verseEndCustom = true;
          state.verseEnd = Math.min(Math.max(value, state.verseStart), verseCount);
        }
        updateReferenceInputs();
      });
    }
    if (els.loadButton) {
      els.loadButton.addEventListener('click', loadSlides);
    }
    if (els.toggleMode) {
      els.toggleMode.addEventListener('click', toggleMode);
    }
    if (els.selectAllSlides) {
      els.selectAllSlides.addEventListener('click', selectAllSlides);
    }
    if (els.slidesContainer) {
      els.slidesContainer.addEventListener('click', onSlidesContainerClick);
      els.slidesContainer.addEventListener('input', onSlidesContainerInput);
    }
    if (els.addToPresentation) {
      els.addToPresentation.addEventListener('click', appendSlidesToPresentation);
    }
    if (els.refreshPresentations) {
      els.refreshPresentations.addEventListener('click', loadPresentations);
    }
    if (els.presentationsList) {
      els.presentationsList.addEventListener('click', (event) => {
        const button = event.target.closest('[data-role="presentation-rename"]');
        if (!button) return;
        const presentationId = button.getAttribute('data-presentation-id');
        if (!presentationId) return;
        const currentName = button.getAttribute('data-presentation-name') || '';
        renamePresentation(presentationId, currentName);
      });
    }
    if (els.clearButton) {
      els.clearButton.addEventListener('click', clearBroadcast);
    }
  }

  async function initialise() {
    renderPreferences();
    renderActive();
    renderLoadedPassages();
    initialiseEvents();
    await fetchPreferences();
    renderLoadedPassages();
    await loadBooks();
    updateReferenceInputs();
    updateMode();
    await loadPresentations();
    connectLiveSocket();
  }

  window.__presenterBibleState = state;
  initialise();
})();
