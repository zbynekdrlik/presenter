'use strict';

(function () {
  const translations = __TRANSLATIONS__;
  const initialBroadcast = __ACTIVE__;

  const state = {
    translations: Array.isArray(translations) ? translations : [],
    translationCode: translations && translations.length ? translations[0].code : '',
    results: [],
    query: '',
    active: initialBroadcast || null,
    liveSocket: null,
    liveReconnectTimer: null,
    toastTimer: null,
  };

  const els = {
    translationSelect: document.querySelector('[data-role="translation-select"]'),
    searchInput: document.querySelector('[data-role="query-input"]'),
    searchForm: document.querySelector('[data-role="search-form"]'),
    results: document.querySelector('[data-role="results"]'),
    activeContainer: document.querySelector('[data-role="active-passage"]'),
    clearButton: document.querySelector('[data-role="clear-button"]'),
    toast: document.querySelector('[data-role="toast"]'),
  };

  function escapeHtml(value) {
    return value
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }

  function formatReference(reference) {
    if (!reference) return '';
    if (reference.verseStart === reference.verseEnd) {
      return `${reference.book} ${reference.chapter}:${reference.verseStart}`;
    }
    return `${reference.book} ${reference.chapter}:${reference.verseStart}-${reference.verseEnd}`;
  }

  function showToast(message, variant) {
    if (!els.toast) return;
    els.toast.textContent = message;
    els.toast.dataset.visible = 'true';
    els.toast.dataset.variant = variant || 'info';
    clearTimeout(state.toastTimer);
    state.toastTimer = setTimeout(() => {
      els.toast.dataset.visible = 'false';
    }, 2500);
  }

  function apiFetch(path, options) {
    const url = path.startsWith('http') ? path : `${window.location.origin}${path}`;
    const headers = Object.assign({
      'Content-Type': 'application/json',
      Accept: 'application/json',
    }, options && options.headers ? options.headers : {});
    return fetch(url, Object.assign({ method: 'GET', headers }, options || {})).then(async (response) => {
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
  }

  function renderTranslations() {
    if (!els.translationSelect) return;
    const html = state.translations
      .map((translation) => {
        const selected = translation.code === state.translationCode ? ' selected' : '';
        const label = translation.language
          ? `${translation.name} (${translation.language})`
          : translation.name;
        return `<option value="${translation.code}"${selected}>${escapeHtml(label)}</option>`;
      })
      .join('');
    els.translationSelect.innerHTML = html;
  }

  function renderResults() {
    if (!els.results) return;
    if (!state.results.length) {
      els.results.innerHTML = '<p class="bible__empty">Search for a verse or phrase above.</p>';
      return;
    }
    els.results.innerHTML = state.results
      .map((result) => {
        const reference = formatReference(result.reference);
        return `
          <article class="bible__result bible-result" data-reference-book="${escapeHtml(result.reference.book)}" data-reference-chapter="${result.reference.chapter}" data-reference-start="${result.reference.verseStart}" data-reference-end="${result.reference.verseEnd}">
            <header>
              <strong>${escapeHtml(reference)}</strong>
              <div class="bible__result-actions">
                <button type="button" data-role="trigger" data-translation="${result.translation.code}">Trigger</button>
              </div>
            </header>
            <p>${escapeHtml(result.text)}</p>
          </article>
        `;
      })
      .join('');
  }

  function renderActive() {
    if (!els.activeContainer) return;
    if (!state.active) {
      els.activeContainer.innerHTML = `
        <div class="bible__active-card bible__active-card--empty">
          <header>
            <strong data-role="active-reference">No active passage</strong>
            <span class="bible__active-translation"></span>
          </header>
          <p class="bible__empty" data-role="active-text">Select a verse to broadcast.</p>
        </div>
      `;
      return;
    }
    const reference = formatReference(state.active.passage.reference);
    const translationLabel = state.active.passage.translation
      ? state.active.passage.translation.name
      : '';
    els.activeContainer.innerHTML = `
      <div class="bible__active-card">
        <header>
          <strong data-role="active-reference">${escapeHtml(reference)}</strong>
          <span class="bible__active-translation">${escapeHtml(translationLabel)}</span>
        </header>
        <p data-role="active-text">${escapeHtml(state.active.passage.text)}</p>
      </div>
    `;
  }

  async function performSearch(query) {
    const trimmed = query.trim();
    if (!trimmed) {
      state.results = [];
      renderResults();
      return;
    }
    try {
      const params = new URLSearchParams({
        translation: state.translationCode,
        query: trimmed,
        limit: '50',
      });
      const results = await apiFetch(`/bible/search?${params.toString()}`, {
        method: 'GET',
      });
      state.results = Array.isArray(results) ? results : [];
      renderResults();
      if (!state.results.length) {
        showToast('No passages found', 'warning');
      }
    } catch (error) {
      console.error('Bible search failed', error);
      showToast('Search failed', 'error');
    }
  }

  async function triggerPassage(article, translationOverride) {
    const book = article.dataset.referenceBook;
    const chapter = Number(article.dataset.referenceChapter);
    const verseStart = Number(article.dataset.referenceStart);
    const verseEnd = Number(article.dataset.referenceEnd);
    const translation = translationOverride || state.translationCode;
    try {
      const payload = {
        translation,
        book,
        chapter,
        verseStart,
        verseEnd,
      };
      const response = await apiFetch('/bible/trigger', {
        method: 'POST',
        body: JSON.stringify(payload),
      });
      state.active = response;
      renderActive();
      showToast('Passage broadcasted', 'success');
    } catch (error) {
      console.error('Failed to trigger bible passage', error);
      showToast('Failed to trigger passage', 'error');
    }
  }

  async function clearBroadcast() {
    try {
      await apiFetch('/bible/clear', { method: 'POST' });
      state.active = null;
      renderActive();
      showToast('Bible broadcast cleared', 'success');
    } catch (error) {
      console.error('Failed to clear bible broadcast', error);
      showToast('Failed to clear broadcast', 'error');
    }
  }

  function connectLiveSocket() {
    if (state.liveSocket) {
      try {
        state.liveSocket.close();
      } catch (error) {
        console.warn('Failed to close bible socket', error);
      }
    }
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(`${protocol}//${window.location.host}/live/ws`);
    state.liveSocket = socket;
    socket.addEventListener('open', () => {
      if (state.liveReconnectTimer) {
        clearTimeout(state.liveReconnectTimer);
        state.liveReconnectTimer = null;
      }
    });
    socket.addEventListener('message', (event) => {
      try {
        const payload = JSON.parse(event.data);
        if (payload.type === 'bible' || payload.type === 'Bible') {
          state.active = payload.broadcast || null;
          renderActive();
        } else if (payload.type === 'bible_cleared' || payload.type === 'BibleCleared') {
          state.active = null;
          renderActive();
        }
      } catch (error) {
        console.error('Failed to parse bible live payload', error);
      }
    });
    socket.addEventListener('close', () => {
      if (!state.liveReconnectTimer) {
        state.liveReconnectTimer = setTimeout(connectLiveSocket, 2000);
      }
    });
    socket.addEventListener('error', (error) => {
      console.error('Bible live socket error', error);
      try {
        socket.close();
      } catch (err) {
        console.warn('Failed closing bible socket', err);
      }
    });
  }

  function bindEvents() {
    if (els.translationSelect) {
      els.translationSelect.addEventListener('change', (event) => {
        state.translationCode = event.target.value;
        if (state.query) {
          performSearch(state.query);
        }
      });
    }
    if (els.searchForm) {
      els.searchForm.addEventListener('submit', (event) => {
        event.preventDefault();
        const value = els.searchInput ? els.searchInput.value : '';
        state.query = value;
        performSearch(value);
      });
    }
    if (els.results) {
      els.results.addEventListener('click', (event) => {
        const triggerButton = event.target.closest('[data-role="trigger"]');
        if (!triggerButton) return;
        const article = triggerButton.closest('.bible__result');
        if (!article) return;
        triggerPassage(article, triggerButton.dataset.translation);
      });
    }
    if (els.clearButton) {
      els.clearButton.addEventListener('click', () => {
        clearBroadcast();
      });
    }
  }

  function initialise() {
    if (state.translations.length) {
      state.translationCode = state.translations[0].code;
    }
    renderTranslations();
    renderActive();
    renderResults();
    bindEvents();
    connectLiveSocket();
  }

  initialise();
  window.__presenterBibleState = state;
  window.__presenterBibleReady = true;
})();
