'use strict';

(function () {
  const libraries = __LIBRARIES__;
  const playlistsData = __PLAYLISTS__;
  const initialStageSnapshot = __STAGE__;

  const state = {
    libraries: Array.isArray(libraries) ? libraries : [],
    playlists: Array.isArray(playlistsData) ? playlistsData : [],
    slidesCache: new Map(),
    presentationIndex: new Map(),
    playlistLookup: new Map(),
    currentLibraryId: null,
    activePlaylistId: null,
    currentPresentationId: null,
    stagePresentationId:
      initialStageSnapshot && initialStageSnapshot.presentationId
        ? initialStageSnapshot.presentationId
        : null,
    stageSlideId:
      initialStageSnapshot && initialStageSnapshot.currentSlideId
        ? initialStageSnapshot.currentSlideId
        : null,
    mode: document.body.dataset.mode || 'live',
    toastTimer: null,
    liveSocket: null,
    liveReconnectTimer: null,
  };

  state.playlists = state.playlists
    .map((playlist) => normalisePlaylist(playlist))
    .filter(Boolean);

  const els = {
    libraryList: document.querySelector('[data-role="library-list"]'),
    playlistList: document.querySelector('[data-role="playlist-list"]'),
    presentationList: document.querySelector('[data-role="presentation-list"]'),
    slides: document.querySelector('[data-role="slides"]'),
    modeToggles: document.querySelectorAll('[data-role="mode-toggle"]'),
    modeStatus: document.querySelector('[data-role="mode-status"]'),
    contextTitle: document.querySelector('[data-role="context-title"]'),
    editor: document.querySelector('[data-role="editor"]'),
    editorMain: document.querySelector('[data-role="editor-main"]'),
    editorTranslation: document.querySelector('[data-role="editor-translation"]'),
    editorStage: document.querySelector('[data-role="editor-stage"]'),
    editorGroup: document.querySelector('[data-role="editor-group"]'),
    editorError: document.querySelector('[data-role="editor-error"]'),
    editorSave: document.querySelector('[data-role="editor-save"]'),
    editorCancel: document.querySelector('[data-role="editor-cancel"]'),
    toast: document.querySelector('[data-role="toast"]'),
  };

  let editingSlideId = null;

  function escapeHtml(value) {
    return value
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }

  function formatMultiline(text) {
    if (!text) return '';
    return escapeHtml(text).replace(/\n/g, '<br />');
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

  function rebuildPresentationIndex() {
    state.presentationIndex.clear();
    state.libraries.forEach((library) => {
      (library.presentations || []).forEach((presentation) => {
        if (presentation && presentation.id) {
          state.presentationIndex.set(presentation.id, presentation);
        }
      });
    });
    state.playlists.forEach((playlist) => {
      (playlist.entries || []).forEach((entry) => {
        if (
          entry &&
          entry.entryType === 'presentation' &&
          entry.presentationId &&
          !state.presentationIndex.has(entry.presentationId)
        ) {
          state.presentationIndex.set(entry.presentationId, {
            id: entry.presentationId,
            name: entry.name,
          });
        }
      });
    });

    state.playlistLookup = new Map(
      state.playlists.map((playlist) => [playlist.id, playlist])
    );
  }

  function normalisePlaylist(raw) {
    if (!raw) return null;
    const entries = (raw.entries || []).map((entry) => {
      const entryId = entry.entryId || entry.entry_id || entry.id || null;
      const entryType = String(entry.type || entry.entryType || 'presentation').toLowerCase();
      const presentationId =
        entry.presentationId || entry.presentation_id || (entryType === 'presentation' ? entry.id || null : null);
      const meta = presentationId ? state.presentationIndex.get(presentationId) : null;
      return {
        entryId,
        entryType,
        presentationId,
        id: presentationId || entryId || null,
        name:
          entry.name ||
          (meta && meta.name) ||
          (entryType === 'separator' ? 'Separator' : 'Untitled presentation'),
      };
    });
    return {
      id: raw.id,
      name: raw.name,
      entries,
    };
  }

  function refreshPlaylistState(updated) {
    const playlist = normalisePlaylist(updated);
    if (!playlist) return;
    const existingIndex = state.playlists.findIndex((item) => item.id === playlist.id);
    if (existingIndex >= 0) {
      state.playlists.splice(existingIndex, 1, playlist);
    } else {
      state.playlists.push(playlist);
    }
    state.playlistLookup.set(playlist.id, playlist);
    rebuildPresentationIndex();
    renderPlaylists();
    renderPresentations();
  }

  function currentPlaylist() {
    if (!state.activePlaylistId) return null;
    return state.playlists.find((item) => item.id === state.activePlaylistId) || null;
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
        presentationId: entry.presentationId || entry.id || null,
      };
    });
  }

  async function putPlaylistEntries(playlistId, entries) {
    const response = await apiFetch(`/playlists/${playlistId}/entries`, {
      method: 'PUT',
      body: JSON.stringify({ entries: serialisePlaylistEntries(entries) }),
    });
    refreshPlaylistState(response);
    showToast('Playlist updated', 'success');
  }

  async function addPresentationToActivePlaylist(presentationId) {
    if (!state.activePlaylistId) {
      showToast('Select a playlist first', 'warning');
      return;
    }
    const playlist = currentPlaylist();
    const entries = playlist ? playlist.entries.slice() : [];
    entries.push({
      entryId: null,
      entryType: 'presentation',
      presentationId,
      id: presentationId,
      name:
        (state.presentationIndex.get(presentationId) || {}).name ||
        'Untitled presentation',
    });
    await putPlaylistEntries(state.activePlaylistId, entries);
  }

  async function removePresentationFromPlaylist(presentationId) {
    const playlist = currentPlaylist();
    if (!playlist) return;
    const entries = playlist.entries.filter(
      (entry) => !(entry.entryType === 'presentation' && entry.presentationId === presentationId)
    );
    await putPlaylistEntries(playlist.id, entries);
    if (state.currentPresentationId === presentationId) {
      const firstEntry = entries.find((entry) => entry.entryType === 'presentation');
      state.currentPresentationId = firstEntry ? firstEntry.presentationId : null;
      if (state.currentPresentationId) {
        await loadPresentation(state.currentPresentationId);
      } else if (els.slides) {
        els.slides.innerHTML = '<p class="tablet-slides__empty">Playlist is empty. Add presentations from the operator panel.</p>';
      }
    }
  }

  async function movePresentationInPlaylist(presentationId, offset) {
    const playlist = currentPlaylist();
    if (!playlist) return;
    const entries = playlist.entries.slice();
    const index = entries.findIndex(
      (entry) => entry.entryType === 'presentation' && entry.presentationId === presentationId
    );
    if (index < 0) return;
    const targetIndex = index + offset;
    if (targetIndex < 0 || targetIndex >= entries.length) return;
    const [item] = entries.splice(index, 1);
    entries.splice(targetIndex, 0, item);
    await putPlaylistEntries(playlist.id, entries);
  }

  function handlePlaylistAction(action, presentationId) {
    if (!presentationId) return;
    switch (action) {
      case 'playlist-add':
        addPresentationToActivePlaylist(presentationId);
        break;
      case 'playlist-remove':
        removePresentationFromPlaylist(presentationId);
        break;
      case 'playlist-up':
        movePresentationInPlaylist(presentationId, -1);
        break;
      case 'playlist-down':
        movePresentationInPlaylist(presentationId, 1);
        break;
      default:
        break;
    }
  }

  function updateContextTitle() {
    if (!els.contextTitle) return;
    if (state.currentLibraryId) {
      const library = state.libraries.find((item) => item.id === state.currentLibraryId);
      els.contextTitle.textContent = library ? `Library: ${library.name}` : 'Library';
    } else if (state.activePlaylistId) {
      const playlist = state.playlists.find((item) => item.id === state.activePlaylistId);
      els.contextTitle.textContent = playlist ? `Playlist: ${playlist.name}` : 'Playlist';
    } else {
      els.contextTitle.textContent = 'Presentations';
    }
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
        throw new Error(text || `Request failed with status ${response.status}`);
      }
      const contentType = response.headers.get('content-type') || '';
      if (contentType.includes('application/json')) {
        return response.json();
      }
      return null;
    });
  }

  function setMode(mode) {
    state.mode = mode;
    document.body.dataset.mode = mode;
    els.modeToggles.forEach((button) => {
      button.dataset.active = button.dataset.mode === mode ? 'true' : 'false';
    });
    if (els.modeStatus) {
      els.modeStatus.textContent =
        mode === 'edit'
          ? 'Edit mode — tap slide to open editor.'
          : 'Live mode — tap slide to trigger stage output.';
    }
  }

  function renderLibraries() {
    if (!els.libraryList) return;
    if (!state.libraries.length) {
      els.libraryList.innerHTML = '<p class="tablet-slides__empty">No libraries available.</p>';
      return;
    }
    const html = state.libraries
      .map((library) => {
        const active = library.id === state.currentLibraryId ? ' data-active="true"' : '';
        const count = Array.isArray(library.presentations)
          ? library.presentations.length
          : library.presentation_count || 0;
        return `
          <div class="tablet-list-item">
            <button type="button" class="tablet-button" data-role="library-button" data-library-id="${library.id}"${active}>
              <span class="tablet-button__label">${escapeHtml(library.name)}</span>
              <span class="tablet-button__meta" data-role="library-count">${count}</span>
            </button>
          </div>
        `;
      })
      .join('');
    els.libraryList.innerHTML = html;
  }

  function renderPlaylists() {
    if (!els.playlistList) return;
    if (!state.playlists.length) {
      els.playlistList.innerHTML = '<p class="tablet-slides__empty">No playlists configured.</p>';
      return;
    }
    const html = state.playlists
      .map((playlist) => {
        const active = playlist.id === state.activePlaylistId ? ' data-active="true"' : '';
        const count = Array.isArray(playlist.entries)
          ? playlist.entries.filter((entry) => entry.entryType === 'presentation').length
          : 0;
        return `
          <div class="tablet-list-item">
            <button type="button" class="tablet-button" data-role="playlist-button" data-playlist-id="${playlist.id}"${active}>
              <span class="tablet-button__label">${escapeHtml(playlist.name || 'Untitled playlist')}</span>
              <span class="tablet-button__meta" data-role="playlist-count">${count}</span>
            </button>
          </div>
        `;
      })
      .join('');
    els.playlistList.innerHTML = html;
  }

  function renderPresentations() {
    if (!els.presentationList) return;
    updateContextTitle();
    let html = '';

    if (state.currentLibraryId) {
      const library = state.libraries.find((entry) => entry.id === state.currentLibraryId);
      if (!library) {
        els.presentationList.innerHTML = '<p class="tablet-slides__empty">Select a library.</p>';
        return;
      }
      html = (library.presentations || [])
        .map((presentation) => {
          const active = presentation.id === state.currentPresentationId ? ' data-active="true"' : '';
          const addButton = state.mode === 'edit' && state.activePlaylistId
            ? `<button type="button" class="tablet-list-action" data-action="playlist-add" data-presentation-id="${presentation.id}">Add</button>`
            : '';
          return `
            <div class="tablet-list-item" data-role="library-entry" data-presentation-id="${presentation.id}">
              <button type="button" class="tablet-button" data-role="presentation-button" data-presentation-id="${presentation.id}"${active}>
                <span class="tablet-button__label">${escapeHtml(presentation.name || 'Untitled presentation')}</span>
              </button>
              ${addButton ? `<div class="tablet-list-actions">${addButton}</div>` : ''}
            </div>
          `;
        })
        .join('');
      els.presentationList.innerHTML =
        html || '<p class="tablet-slides__empty">No presentations in this library.</p>';
      updateContextTitle();
      return;
    }

    if (state.activePlaylistId) {
      const playlist = state.playlists.find((entry) => entry.id === state.activePlaylistId);
      if (!playlist) {
        els.presentationList.innerHTML = '<p class="tablet-slides__empty">Select a playlist.</p>';
        updateContextTitle();
        return;
      }
      html = (playlist.entries || [])
        .map((entry, index) => {
          if (entry.entryType === 'separator') {
            return `
              <div class="tablet-list-item" data-role="playlist-separator" data-entry-index="${index}">
                <div class="tablet-separator">
                  <span class="tablet-separator__label">${escapeHtml(entry.name || 'Separator')}</span>
                </div>
              </div>
            `;
          }
          const presentationId = entry.presentationId || entry.id;
          const active = presentationId === state.currentPresentationId ? ' data-active="true"' : '';
          const label = entry.name || (presentationId && (state.presentationIndex.get(presentationId) || {}).name) || 'Untitled presentation';
          const actions = state.mode === 'edit'
            ? `<div class="tablet-list-actions">
                <button type="button" class="tablet-list-action" data-action="playlist-up" data-presentation-id="${presentationId}">Up</button>
                <button type="button" class="tablet-list-action" data-action="playlist-down" data-presentation-id="${presentationId}">Down</button>
                <button type="button" class="tablet-list-action tablet-list-action--danger" data-action="playlist-remove" data-presentation-id="${presentationId}">Remove</button>
              </div>`
            : '';
          return `
            <div class="tablet-list-item" data-role="playlist-entry" data-presentation-id="${presentationId || ''}" data-entry-id="${entry.entryId || ''}" data-entry-index="${index}">
              <button type="button" class="tablet-button" data-role="presentation-button" data-presentation-id="${presentationId || ''}"${active}>
                <span class="tablet-button__label">${escapeHtml(label)}</span>
              </button>
              ${actions}
            </div>
          `;
        })
        .join('');
      els.presentationList.innerHTML =
        html || '<p class="tablet-slides__empty">Playlist is empty. Add presentations from the operator panel.</p>';
      updateContextTitle();
      return;
    }

    els.presentationList.innerHTML = '<p class="tablet-slides__empty">Select a library or playlist.</p>';
    updateContextTitle();
  }

  function slidesForPresentation(presentationId) {
    return state.slidesCache.get(presentationId) || [];
  }

  function renderSlides(presentationId) {
    if (!els.slides) return;
    const slides = slidesForPresentation(presentationId);
    els.slides.setAttribute('data-slides-placeholder', presentationId || '');
    if (!slides.length) {
      els.slides.innerHTML = '<p class="tablet-slides__empty">No slides yet.</p>';
      return;
    }
    els.slides.innerHTML = slides
      .map((slide, index) => {
        const active =
          state.stagePresentationId === presentationId && state.stageSlideId === slide.id
            ? ' is-active'
            : '';
        return `
          <article class="tablet-slide stage-control__slide${active}" data-role="tablet-slide" data-slide-id="${slide.id}">
            <header>
              <strong>${index + 1}</strong>
              ${slide.content.group && slide.content.group.name
                ? `<span class="tablet-slide__group">${escapeHtml(slide.content.group.name)}</span>`
                : ''}
            </header>
            <section class="tablet-slide__body">
              <p>${formatMultiline(slide.content.main.value)}</p>
              <p class="tablet-slide__translation">${formatMultiline(slide.content.translation.value)}</p>
            </section>
          </article>
        `;
      })
      .join('');
  }

  async function loadPresentation(presentationId) {
    if (!presentationId) return;
    if (state.slidesCache.has(presentationId)) {
      renderSlides(presentationId);
      return;
    }
    try {
      const detail = await apiFetch(`/presentations/${presentationId}`, {
        method: 'GET',
      });
      const slides = detail.presentation.slides || [];
      state.slidesCache.set(presentationId, slides);
      renderSlides(presentationId);
    } catch (error) {
      console.error('Failed to load presentation', error);
      showToast('Failed to load presentation', 'error');
    }
  }

  function computeNextSlideId(slides, slideId) {
    const index = slides.findIndex((slide) => slide.id === slideId);
    if (index < 0) return null;
    const next = slides[index + 1];
    return next ? next.id : null;
  }

  async function triggerSlide(presentationId, slideId, element) {
    const slides = slidesForPresentation(presentationId);
    if (!slides.length) return;
    if (element) {
      element.classList.add('is-loading');
    }
    try {
      await apiFetch('/stage/state', {
        method: 'POST',
        body: JSON.stringify({
          presentationId,
          currentSlideId: slideId,
          nextSlideId: computeNextSlideId(slides, slideId),
        }),
      });
      state.stagePresentationId = presentationId;
      state.stageSlideId = slideId;
      renderSlides(presentationId);
    } catch (error) {
      console.error('Failed to trigger slide', error);
      showToast('Failed to trigger slide', 'error');
    } finally {
      if (element) {
        element.classList.remove('is-loading');
      }
    }
  }

  async function saveSlideEdits() {
    if (!state.currentPresentationId || !editingSlideId) return;
    const payload = {
      main: els.editorMain.value,
      translation: els.editorTranslation.value,
      stage: els.editorStage.value,
      group: els.editorGroup.value || null,
    };
    try {
      const updated = await apiFetch(`/presentations/${state.currentPresentationId}/slides/${editingSlideId}`, {
        method: 'PATCH',
        body: JSON.stringify(payload),
      });
      const slides = slidesForPresentation(state.currentPresentationId).map((slide) =>
        slide.id === editingSlideId ? Object.assign({}, slide, { content: updated.content }) : slide
      );
      state.slidesCache.set(state.currentPresentationId, slides);
      renderSlides(state.currentPresentationId);
      closeEditor();
      showToast('Slide updated', 'success');
    } catch (error) {
      console.error('Failed to update slide', error);
      els.editorError.textContent = 'Failed to save changes.';
      els.editorError.dataset.visible = 'true';
    }
  }

  function openEditor(slide) {
    if (!els.editor) return;
    editingSlideId = slide.id;
    els.editorMain.value = slide.content.main.value;
    els.editorTranslation.value = slide.content.translation.value;
    els.editorStage.value = slide.content.stage.value;
    els.editorGroup.value = slide.content.group ? slide.content.group.name : '';
    els.editorError.dataset.visible = 'false';
    els.editor.dataset.open = 'true';
  }

  function closeEditor() {
    if (!els.editor) return;
    els.editor.dataset.open = 'false';
    editingSlideId = null;
  }

  function handleLibraryClick(event) {
    const button = event.target.closest('[data-role="library-button"]');
    if (!button) return;
    const id = button.dataset.libraryId;
    if (!id || id === state.currentLibraryId) return;
    state.currentLibraryId = id;
    state.currentPresentationId = null;
    renderLibraries();
    renderPresentations();
    renderPlaylists();
    els.slides.innerHTML = '<p class="tablet-slides__empty">Select a presentation to load slides.</p>';
  }

  function handlePlaylistClick(event) {
    const button = event.target.closest('[data-role="playlist-button"]');
    if (!button) return;
    const id = button.dataset.playlistId;
    if (!id) return;
    const wasActive = state.activePlaylistId === id;
    state.activePlaylistId = id;
    state.currentLibraryId = null;
    if (!wasActive) {
      state.currentPresentationId = null;
    }
    renderPlaylists();
    renderLibraries();

    const playlist = state.playlists.find((entry) => entry.id === id);
    if (playlist && Array.isArray(playlist.entries) && playlist.entries.length > 0) {
      const firstEntry = playlist.entries.find((entry) => entry.entryType === 'presentation');
      state.currentPresentationId = firstEntry ? firstEntry.presentationId || firstEntry.id : null;
      renderPresentations();
      if (state.currentPresentationId) {
        loadPresentation(state.currentPresentationId);
      } else {
        els.slides.innerHTML = '<p class="tablet-slides__empty">Playlist contains only separators.</p>';
      }
    } else {
      renderPresentations();
      els.slides.innerHTML = '<p class="tablet-slides__empty">Playlist is empty. Add presentations from the operator panel.</p>';
    }
  }

  function handlePresentationClick(event) {
    const actionButton = event.target.closest('[data-action]');
    if (actionButton) {
      event.preventDefault();
      event.stopPropagation();
      const presentationId =
        actionButton.dataset.presentationId ||
        (actionButton.closest('[data-role="playlist-entry"]')?.dataset.presentationId ??
          actionButton.closest('[data-role="library-entry"]')?.dataset.presentationId);
      handlePlaylistAction(actionButton.dataset.action, presentationId);
      return;
    }
    const button = event.target.closest('[data-role="presentation-button"]');
    if (!button) return;
    const id = button.dataset.presentationId;
    if (!id || id === state.currentPresentationId) return;
    state.currentPresentationId = id;
    renderPresentations();
    loadPresentation(id);
  }

  function handleSlideTap(event) {
    const card = event.target.closest('[data-slide-id]');
    if (!card || !state.currentPresentationId) return;
    const slideId = card.dataset.slideId;
    const slides = slidesForPresentation(state.currentPresentationId);
    const slide = slides.find((entry) => entry.id === slideId);
    if (!slide) return;
    if (state.mode === 'edit') {
      openEditor(slide);
    } else {
      triggerSlide(state.currentPresentationId, slideId, card);
    }
  }

  function handleModeToggle(event) {
    const button = event.target.closest('[data-role="mode-toggle"]');
    if (!button) return;
    const mode = button.dataset.mode;
    if (!mode || mode === state.mode) return;
    setMode(mode);
  }

  function bindEvents() {
    if (els.libraryList) {
      els.libraryList.addEventListener('click', handleLibraryClick);
    }
    if (els.playlistList) {
      els.playlistList.addEventListener('click', handlePlaylistClick);
    }
    if (els.presentationList) {
      els.presentationList.addEventListener('click', handlePresentationClick);
    }
    if (els.slides) {
      els.slides.addEventListener('click', handleSlideTap);
    }
    els.modeToggles.forEach((button) => {
      button.addEventListener('click', handleModeToggle);
    });
    if (els.editorSave) {
      els.editorSave.addEventListener('click', saveSlideEdits);
    }
    if (els.editorCancel) {
      els.editorCancel.addEventListener('click', closeEditor);
    }
  }

  function connectLiveSocket() {
    if (state.liveSocket) {
      try {
        state.liveSocket.close();
      } catch (error) {
        console.warn('failed to close tablet socket', error);
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
        if (payload.type === 'stage' || payload.type === 'Stage') {
          const snapshot = payload.snapshot || {};
          const presentationId = snapshot.presentationId || snapshot.presentation_id || null;
          const slideId = snapshot.currentSlideId || snapshot.current_slide_id || null;
          state.stagePresentationId = presentationId;
          state.stageSlideId = slideId;
          if (presentationId && presentationId === state.currentPresentationId) {
            renderSlides(presentationId);
          }
        } else if (payload.type === 'timers' || payload.type === 'Timers') {
          // no-op for tablet today
        }
      } catch (error) {
        console.error('tablet live payload parsing failed', error);
      }
    });
    socket.addEventListener('close', () => {
      if (!state.liveReconnectTimer) {
        state.liveReconnectTimer = setTimeout(connectLiveSocket, 2000);
      }
    });
    socket.addEventListener('error', (error) => {
      console.error('tablet live socket error', error);
      try {
        socket.close();
      } catch (err) {
        console.warn('failed closing socket after error', err);
      }
    });
  }

  function initialise() {
    rebuildPresentationIndex();
    bindEvents();
    setMode(state.mode);

    let initialLibraryId = null;
    let initialPlaylistId = null;
    let initialPresentationId = null;

    if (state.stagePresentationId) {
      const owningLibrary = state.libraries.find((library) =>
        (library.presentations || []).some((presentation) => presentation.id === state.stagePresentationId)
      );
      if (owningLibrary) {
        initialLibraryId = owningLibrary.id;
        initialPresentationId = state.stagePresentationId;
      } else {
        const owningPlaylist = state.playlists.find((playlist) =>
          (playlist.entries || []).some(
            (entry) =>
              entry.entryType === 'presentation' && entry.presentationId === state.stagePresentationId
          )
        );
        if (owningPlaylist) {
          initialPlaylistId = owningPlaylist.id;
          initialPresentationId = state.stagePresentationId;
        }
      }
    }

    if (!initialLibraryId && !initialPlaylistId && state.libraries.length > 0) {
      initialLibraryId = state.libraries[0].id;
      if (state.libraries[0].presentations.length > 0) {
        initialPresentationId = state.libraries[0].presentations[0].id;
      }
    } else if (!initialLibraryId && !initialPlaylistId && state.playlists.length > 0) {
      initialPlaylistId = state.playlists[0].id;
      const firstPresentationEntry =
        state.playlists[0].entries &&
        state.playlists[0].entries.find((entry) => entry.entryType === 'presentation');
      if (firstPresentationEntry && (firstPresentationEntry.presentationId || firstPresentationEntry.id)) {
        initialPresentationId = firstPresentationEntry.presentationId || firstPresentationEntry.id;
      }
    }

    state.currentLibraryId = initialLibraryId;
    state.activePlaylistId = initialPlaylistId;
    state.currentPresentationId = initialPresentationId;

    renderLibraries();
    renderPlaylists();
    renderPresentations();

    if (state.currentPresentationId) {
      loadPresentation(state.currentPresentationId);
    } else if (els.slides) {
      els.slides.innerHTML = '<p class="tablet-slides__empty">Select a presentation to load slides.</p>';
    }

    connectLiveSocket();
  }

  initialise();
  window.__presenterTabletState = state;
  window.__presenterTabletReady = true;
})();
