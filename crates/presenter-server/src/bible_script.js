"use strict";

(function () {
  const translations = Array.isArray(__TRANSLATIONS__) ? __TRANSLATIONS__ : [];
  const initialBroadcast = __ACTIVE__ || null;

  function coerceDashboardFlag(value) {
    if (typeof value === "boolean") return value;
    if (typeof value === "number") return value !== 0;
    if (typeof value === "string") {
      const normalized = value.trim().toLowerCase();
      if (!normalized) return true;
      return !["false", "0", "no", "off"].includes(normalized);
    }
    return true;
  }

  function normalizeTranslation(raw) {
    if (!raw || typeof raw !== "object") {
      return {
        code: "",
        name: "",
        language: "",
        showInDashboard: true,
        source: null,
      };
    }
    const code =
      typeof raw.code === "string" ? raw.code : String(raw.code || "");
    const name =
      typeof raw.name === "string" ? raw.name : String(raw.name || "");
    const language =
      typeof raw.language === "string"
        ? raw.language
        : String(raw.language || "");
    const showInDashboard = coerceDashboardFlag(
      raw.showInDashboard ?? raw.show_in_dashboard,
    );
    const source =
      raw.source == null || raw.source === ""
        ? null
        : typeof raw.source === "string"
          ? raw.source
          : String(raw.source);
    return {
      code,
      name,
      language,
      showInDashboard,
      source,
    };
  }

  const normalizedTranslations = translations.map(normalizeTranslation);

  const state = {
    translations: normalizedTranslations,
    refreshingTranslations: false,
    preferences: {
      mainTranslation: normalizedTranslations.length
        ? normalizedTranslations[0].code
        : "",
      secondaryTranslation: "",
      characterLimit: 320,
    },
    translationIndex: 0,
    books: [],
    filteredBooks: [],
    selectedBook: "",
    selectedBookCode: "",
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
    bibleTab: "live",
    activePresentationId: "",
    activePresentationSlides: [],
    activeBroadcast: initialBroadcast,
    loadedPassages: [],
    liveSocket: null,
    liveReconnectTimer: null,
    activePollTimer: null,
    toastTimer: null,
    loadingSlides: false,
    savingPreferences: false,
    contentSearchQuery: "",
    contentSearchResults: [],
    contentSearchLoading: false,
    contentSearchDebounce: null,
    presentationEditTarget: null,
    bibleEdit: {
      open: false,
      submitting: false,
      translationCode: "",
      name: "",
      language: "",
      showInDashboard: false,
    },
  };

  const loadedPassageKeys = new Map();
  const MAX_LOADED_PASSAGES = 12;

  const els = {
    translationList: document.querySelector('[data-role="translation-list"]'),
    mainTranslation: document.querySelector('[data-role="main-translation"]'),
    secondaryTranslation: document.querySelector(
      '[data-role="secondary-translation"]',
    ),
    charLimit: document.querySelector('[data-role="char-limit"]'),
    savePreferences: document.querySelector('[data-role="save-preferences"]'),
    globalSearchForm: document.querySelector(
      '[data-role="global-search-form"]',
    ),
    globalSearchInput: document.querySelector(
      '[data-role="global-search-query"]',
    ),
    globalSearchClear: document.querySelector(
      '[data-role="global-search-clear"]',
    ),
    globalSearchResults: document.querySelector(
      '[data-role="global-search-results"]',
    ),
    bookFilter: document.querySelector('[data-role="book-filter"]'),
    bookList: document.querySelector('[data-role="book-list"]'),
    chapterInput: document.querySelector('[data-role="chapter-input"]'),
    verseStartInput: document.querySelector('[data-role="verse-start"]'),
    verseEndInput: document.querySelector('[data-role="verse-end"]'),
    loadButton: document.querySelector('[data-role="load-button"]'),
    loadedPassages: document.querySelector('[data-role="loaded-passages"]'),
    slidesContainer: document.querySelector('[data-role="slides"]'),
    modeToggleContainer: document.querySelector(".operator__mode-toggle"),
    selectionCount: document.querySelector('[data-role="selection-count"]'),
    selectAllButton: document.querySelector('[data-role="select-all-slides"]'),
    presentationSelect: document.querySelector(
      '[data-role="presentation-select"]',
    ),
    presentationName: document.querySelector('[data-role="presentation-name"]'),
    addToPresentation: document.querySelector('[data-role="presentation-add"]'),
    refreshPresentations: document.querySelector(
      '[data-role="refresh-presentations"]',
    ),
    presentationsList: document.querySelector(
      '[data-role="presentations-list"]',
    ),
    addEmptySlide: document.querySelector('[data-role="add-empty-slide"]'),
    presentationCreate: document.querySelector(
      '[data-role="presentation-create"]',
    ),
    clearButton: document.querySelector('[data-role="clear-button"]'),
    toast: document.querySelector('[data-role="toast"]'),
    bibleTabNav: document.querySelector('[data-role="bible-tab-nav"]'),
    bibleTabButtons: document.querySelectorAll('[data-role="bible-tab"]'),
    livePanelEl: document.querySelector('[data-bible-panel="live"]'),
    preparedPanelEl: document.querySelector('[data-bible-panel="prepared"]'),
    settingsPanelEl: document.querySelector('[data-bible-panel="settings"]'),
    bibleCount: document.querySelector('[data-role="bible-dashboard"]'),
    bibleImport: document.querySelector('[data-role="bible-import"]'),
    bibleModal: document.querySelector('[data-role="bible-modal"]'),
    bibleModalList: document.querySelector('[data-role="bible-modal-list"]'),
    bibleModalClose: document.querySelector('[data-role="bible-modal-close"]'),
    bibleEditModal: document.querySelector('[data-role="bible-edit-modal"]'),
    bibleEditForm: document.querySelector('[data-role="bible-edit-form"]'),
    bibleEditName: document.querySelector('[data-role="bible-edit-name"]'),
    bibleEditLanguage: document.querySelector(
      '[data-role="bible-edit-language"]',
    ),
    bibleEditDashboard: document.querySelector(
      '[data-role="bible-edit-dashboard"]',
    ),
    bibleEditDelete: document.querySelector('[data-role="bible-edit-delete"]'),
    bibleEditCancel: document.querySelector('[data-role="bible-edit-cancel"]'),
    bibleEditTitle: document.querySelector('[data-role="bible-edit-title"]'),
    presentationEditModal: document.querySelector(
      '[data-role="bible-presentation-edit-modal"]',
    ),
    presentationEditForm: document.querySelector(
      '[data-role="bible-presentation-edit-form"]',
    ),
    presentationEditName: document.querySelector(
      '[data-role="bible-presentation-edit-name"]',
    ),
    presentationEditDelete: document.querySelector(
      '[data-role="bible-presentation-edit-delete"]',
    ),
    presentationEditCancel: document.querySelector(
      '[data-role="bible-presentation-edit-cancel"]',
    ),
  };

  function normalizeTranslationCode(code) {
    if (!code) return "";
    return String(code).trim().toLowerCase();
  }

  function isTranslationPinned(code) {
    const translation = findTranslationByCode(code);
    if (!translation) return false;
    return Boolean(translation.showInDashboard);
  }

  function setTranslationPinState(code, pinned) {
    const translation = findTranslationByCode(code);
    if (!translation) return;
    translation.showInDashboard = Boolean(pinned);
  }

  function showToast(message, variant) {
    if (!els.toast) return;
    els.toast.textContent = message;
    els.toast.dataset.variant = variant || "info";
    els.toast.dataset.visible = "true";
    clearTimeout(state.toastTimer);
    state.toastTimer = setTimeout(() => {
      els.toast.dataset.visible = "false";
    }, 2500);
  }

  function setBibleTab(tab) {
    state.bibleTab = tab;
    // Set data-bible-tab on body for CSS targeting (e.g., showing drag handles in prepared tab)
    document.body.dataset.bibleTab = tab;
    els.bibleTabButtons.forEach((btn) => {
      btn.dataset.active = btn.dataset.tab === tab ? "true" : "false";
    });
    [els.livePanelEl, els.preparedPanelEl, els.settingsPanelEl].forEach(
      (panel) => {
        if (panel) panel.dataset.visible = "false";
      },
    );
    const activePanel = {
      live: els.livePanelEl,
      prepared: els.preparedPanelEl,
      settings: els.settingsPanelEl,
    }[tab];
    if (activePanel) activePanel.dataset.visible = "true";

    if (tab === "live") {
      updateMode();
      renderSlides();
    } else if (tab === "prepared") {
      updateMode();
      renderPresentationSlides();
    }
  }

  function apiFetch(path, options) {
    const url = path.startsWith("http")
      ? path
      : `${window.location.origin}${path}`;
    const opts = Object.assign(
      {
        method: "GET",
        headers: {
          "Content-Type": "application/json",
          Accept: "application/json",
        },
      },
      options || {},
    );
    return fetch(url, opts).then(async (response) => {
      const contentType = response.headers.get("content-type") || "";
      if (!response.ok) {
        let details = "";
        if (contentType.includes("application/json")) {
          try {
            const data = await response.json();
            details = data && data.message ? data.message : "";
          } catch (_) {
            details = await response.text();
          }
        } else {
          details = await response.text();
        }
        const message = details || `Request failed with ${response.status}`;
        throw new Error(message);
      }
      if (contentType.includes("application/json")) {
        return response.json();
      }
      return null;
    });
  }

  function renderTranslationSelect(selectEl, selectedCode, includeNone) {
    if (!selectEl) return;
    let html = includeNone
      ? `<option value=""${!selectedCode ? " selected" : ""}>None</option>`
      : "";
    html += state.translations
      .map((translation) => {
        const selected = translation.code === selectedCode ? " selected" : "";
        const label = translation.language
          ? `${translation.name} (${translation.language})`
          : translation.name;
        return `<option value="${translation.code}"${selected}>${escapeHtml(label)}</option>`;
      })
      .join("");
    selectEl.innerHTML = html;
  }

  function findTranslationIndex(code) {
    if (!Array.isArray(state.translations) || !state.translations.length) {
      return -1;
    }
    if (!code) {
      return 0;
    }
    return state.translations.findIndex(
      (translation) =>
        translation.code.toLowerCase() === String(code).toLowerCase(),
    );
  }

  function alignMainTranslation(code) {
    if (!Array.isArray(state.translations) || !state.translations.length) {
      state.preferences.mainTranslation = "";
      state.translationIndex = 0;
      return;
    }
    const index = findTranslationIndex(code);
    state.translationIndex = index >= 0 ? index : 0;
    state.preferences.mainTranslation =
      state.translations[state.translationIndex].code;
  }

  function updateTranslationHeader() {
    if (!els.bibleCount) return;
    const count = Array.isArray(state.translations)
      ? state.translations.length
      : 0;
    els.bibleCount.textContent = `(${count})`;
    els.bibleCount.dataset.empty = count === 0 ? "true" : "false";
    els.bibleCount.setAttribute(
      "aria-label",
      `Show all Bibles (${count} available)`,
    );
    els.bibleCount.disabled = count === 0;
  }

  function renderTranslationList() {
    if (!els.translationList) return;
    updateTranslationHeader();
    if (!Array.isArray(state.translations) || !state.translations.length) {
      els.translationList.innerHTML =
        '<li class=\"operator__list-item operator__list-item--empty\">No translations available.</li>';
      renderBibleModal();
      return;
    }
    const activeCode =
      state.preferences.mainTranslation || state.translations[0]?.code || "";
    const activeNormalized = normalizeTranslationCode(activeCode);
    const seen = new Set();
    const displayTranslations = state.translations.filter((translation) => {
      if (!translation || !translation.code) {
        return false;
      }
      const code = String(translation.code);
      const normalized = normalizeTranslationCode(code);
      if (seen.has(normalized)) {
        return false;
      }
      const pinned = Boolean(translation.showInDashboard);
      const isActive = activeNormalized && normalized === activeNormalized;
      if (!pinned && !isActive) {
        return false;
      }
      seen.add(normalized);
      return true;
    });

    if (!displayTranslations.length) {
      els.translationList.innerHTML =
        '<li class=\"operator__list-item operator__list-item--empty\">Star Bibles to keep them handy.</li>';
      renderBibleModal();
      return;
    }

    const html = displayTranslations
      .map((translation) => {
        const code = String(translation.code);
        const label = translation.language
          ? `${translation.name} (${translation.language})`
          : translation.name;
        const normalized = normalizeTranslationCode(code);
        const active = activeNormalized && normalized === activeNormalized;
        const activeAttr = active ? "true" : "false";
        const ariaCurrent = active ? ' aria-current=\"true\"' : "";
        const dashboardAttr = isTranslationPinned(code)
          ? ' data-dashboard=\"true\"'
          : "";
        return `<li class=\"operator__list-item\" data-translation-code=\"${escapeHtml(code)}\"${dashboardAttr}>
            <button type=\"button\" class=\"operator__list-button\" data-translation-code=\"${escapeHtml(
              code,
            )}\" data-active=\"${activeAttr}\"${ariaCurrent}>
              <span class=\"operator__list-label\">${escapeHtml(label)}</span>
            </button>
            <div class=\"operator__list-actions\">
              <button type=\"button\" class=\"operator__list-action operator__list-action--icon operator__list-action--menu\" data-action=\"bible-edit\" data-translation-code=\"${escapeHtml(
                code,
              )}\" aria-label=\"Edit ${escapeHtml(label)}\">⋮</button>
            </div>
          </li>`;
      })
      .join("");
    els.translationList.innerHTML = html;
    renderBibleModal();
  }

  function renderBibleModal() {
    if (!els.bibleModalList) return;
    if (!Array.isArray(state.translations) || !state.translations.length) {
      els.bibleModalList.innerHTML =
        '<p class="operator__slides-empty">No Bible translations available.</p>';
      return;
    }
    const html = state.translations
      .map((translation) => {
        if (!translation) {
          return "";
        }
        const code = translation.code ? String(translation.code) : "";
        const label = translation.language
          ? `${translation.name} (${translation.language})`
          : translation.name;
        const pinned = Boolean(translation.showInDashboard);
        const star = pinned ? "★" : "☆";
        const ariaLabel = pinned
          ? `Remove ${label} from dashboard`
          : `Show ${label} on dashboard`;
        return `
          <div class="operator__list-item operator__list-row operator__list-row--modal" data-role="bible-row" data-translation-code="${escapeHtml(
            code,
          )}">
            <button type="button" class="operator__list-favorite operator__list-favorite--inline" data-action="bible-dashboard-toggle" data-translation-code="${escapeHtml(
              code,
            )}" aria-pressed="${pinned ? "true" : "false"}" aria-label="${escapeHtml(ariaLabel)}">${star}</button>
            <button type="button" class="operator__list-button" data-role="bible-item" data-translation-code="${escapeHtml(
              code,
            )}">
              <span class="operator__list-label">${escapeHtml(label)}</span>
            </button>
            <div class="operator__list-actions">
              <button type="button" class="operator__list-action operator__list-action--icon operator__list-action--menu" data-action="bible-edit" data-translation-code="${escapeHtml(
                code,
              )}" aria-label="Edit ${escapeHtml(label)}">⋮</button>
            </div>
          </div>
        `;
      })
      .join("");
    els.bibleModalList.innerHTML = html;
  }

  function openBibleModal() {
    if (!els.bibleModal) return;
    renderBibleModal();
    els.bibleModal.dataset.open = "true";
    document.body.dataset.modalOpen = "bible-list";
  }

  function closeBibleModal() {
    if (!els.bibleModal) return;
    els.bibleModal.dataset.open = "false";
    if (document.body.dataset.modalOpen === "bible-list") {
      delete document.body.dataset.modalOpen;
    }
  }

  async function toggleBibleDashboard(code) {
    if (!code) return;
    const translation = findTranslationByCode(code);
    if (!translation) {
      showToast("Bible not found", "error");
      return;
    }
    const normalizedCode = translation.code;
    const wasPinned = Boolean(translation.showInDashboard);
    const nextPinned = !wasPinned;
    setTranslationPinState(normalizedCode, nextPinned);
    renderTranslationList();
    renderBibleModal();
    try {
      const updated = await apiFetch(
        `/bible/translations/${encodeURIComponent(normalizedCode)}`,
        {
          method: "PATCH",
          body: JSON.stringify({ showInDashboard: nextPinned }),
        },
      );
      if (updated && typeof updated.showInDashboard === "boolean") {
        updateTranslationInState(updated);
      }
      if (
        els.bibleEditDashboard &&
        state.bibleEdit.open &&
        state.bibleEdit.translationCode &&
        normalizeTranslationCode(state.bibleEdit.translationCode) ===
          normalizeTranslationCode(normalizedCode)
      ) {
        const pinnedValue = isTranslationPinned(normalizedCode);
        els.bibleEditDashboard.checked = pinnedValue;
        state.bibleEdit.showInDashboard = pinnedValue;
      }
      renderTranslationList();
      const message = nextPinned
        ? "Bible pinned to dashboard"
        : "Bible removed from dashboard";
      showToast(message, "success");
    } catch (error) {
      console.error("Failed to update Bible dashboard pin", error);
      setTranslationPinState(normalizedCode, wasPinned);
      renderTranslationList();
      renderBibleModal();
      if (
        els.bibleEditDashboard &&
        state.bibleEdit.open &&
        state.bibleEdit.translationCode &&
        normalizeTranslationCode(state.bibleEdit.translationCode) ===
          normalizeTranslationCode(normalizedCode)
      ) {
        els.bibleEditDashboard.checked = wasPinned;
        state.bibleEdit.showInDashboard = wasPinned;
      }
      showToast("Failed to update Bible dashboard pin", "error");
    }
  }

  function findTranslationByCode(code) {
    if (!code || !Array.isArray(state.translations)) {
      return null;
    }
    const target = String(code).toLowerCase();
    return (
      state.translations.find(
        (translation) =>
          translation &&
          typeof translation.code === "string" &&
          translation.code.toLowerCase() === target,
      ) || null
    );
  }

  function updateTranslationInState(updated) {
    if (!updated || typeof updated !== "object") {
      return null;
    }
    const normalized = normalizeTranslation(updated);
    if (!normalized.code) {
      return null;
    }
    const index = state.translations.findIndex(
      (entry) => entry && entry.code === normalized.code,
    );
    if (index >= 0) {
      state.translations[index] = normalized;
      return state.translations[index];
    }
    state.translations.push(normalized);
    return normalized;
  }

  function openBibleEdit(code) {
    const translation = findTranslationByCode(code);
    if (!translation) {
      showToast("Bible not found", "error");
      return;
    }
    state.bibleEdit.open = true;
    state.bibleEdit.submitting = false;
    state.bibleEdit.translationCode = translation.code;
    state.bibleEdit.name = translation.name;
    state.bibleEdit.language = translation.language;
    state.bibleEdit.showInDashboard = Boolean(translation.showInDashboard);

    if (els.bibleEditForm) {
      els.bibleEditForm.dataset.submitting = "false";
    }
    if (els.bibleEditName) {
      els.bibleEditName.value = translation.name;
      els.bibleEditName.disabled = false;
    }
    if (els.bibleEditLanguage) {
      els.bibleEditLanguage.value = translation.language;
      els.bibleEditLanguage.disabled = false;
    }
    if (els.bibleEditDashboard) {
      els.bibleEditDashboard.checked = state.bibleEdit.showInDashboard;
      els.bibleEditDashboard.disabled = false;
    }
    if (els.bibleEditDelete) {
      els.bibleEditDelete.disabled = false;
      els.bibleEditDelete.removeAttribute("hidden");
    }
    if (els.bibleEditTitle) {
      els.bibleEditTitle.textContent = `Edit ${translation.name}`;
    }
    if (els.bibleEditModal) {
      els.bibleEditModal.dataset.open = "true";
      document.body.dataset.modalOpen = "bible-edit";
      window.setTimeout(() => {
        if (els.bibleEditName) {
          els.bibleEditName.focus();
          els.bibleEditName.select();
        }
      }, 15);
    }
  }

  function closeBibleEdit() {
    state.bibleEdit.open = false;
    state.bibleEdit.submitting = false;
    state.bibleEdit.translationCode = "";
    state.bibleEdit.name = "";
    state.bibleEdit.language = "";
    state.bibleEdit.showInDashboard = false;
    if (els.bibleEditModal) {
      els.bibleEditModal.dataset.open = "false";
    }
    if (els.bibleEditDashboard) {
      els.bibleEditDashboard.checked = false;
    }
    if (els.bibleModal && els.bibleModal.dataset.open === "true") {
      document.body.dataset.modalOpen = "bible-list";
    } else {
      delete document.body.dataset.modalOpen;
    }
  }

  function setBibleEditSubmitting(submitting) {
    state.bibleEdit.submitting = submitting;
    if (els.bibleEditForm) {
      els.bibleEditForm.dataset.submitting = submitting ? "true" : "false";
    }
    if (els.bibleEditName) {
      els.bibleEditName.disabled = submitting;
    }
    if (els.bibleEditLanguage) {
      els.bibleEditLanguage.disabled = submitting;
    }
    if (els.bibleEditDashboard) {
      els.bibleEditDashboard.disabled = submitting;
    }
    if (els.bibleEditDelete) {
      els.bibleEditDelete.disabled = submitting;
    }
  }

  async function handleBibleEditSubmit(event) {
    event.preventDefault();
    if (state.bibleEdit.submitting) return;
    const nameInput = els.bibleEditName;
    const languageInput = els.bibleEditLanguage;
    const name = nameInput ? nameInput.value.trim() : "";
    const language = languageInput ? languageInput.value.trim() : "";
    if (!name) {
      showToast("Bible name cannot be empty", "warning");
      if (nameInput) {
        nameInput.focus();
      }
      return;
    }
    if (!language) {
      showToast("Language cannot be empty", "warning");
      if (languageInput) {
        languageInput.focus();
      }
      return;
    }
    const code = state.bibleEdit.translationCode;
    if (!code) {
      showToast("Bible not selected", "error");
      return;
    }
    const wantsDashboard = els.bibleEditDashboard
      ? Boolean(els.bibleEditDashboard.checked)
      : isTranslationPinned(code);
    setBibleEditSubmitting(true);
    try {
      const payload = {
        name,
        language,
        showInDashboard: wantsDashboard,
      };
      const updated = await apiFetch(
        `/bible/translations/${encodeURIComponent(code)}`,
        {
          method: "PATCH",
          body: JSON.stringify(payload),
        },
      );
      if (!updated) {
        throw new Error("Empty response");
      }
      const stored = updateTranslationInState(updated);
      state.bibleEdit.showInDashboard = Boolean(
        stored ? stored.showInDashboard : wantsDashboard,
      );
      renderTranslationList();
      renderTranslationSelect(
        els.mainTranslation,
        state.preferences.mainTranslation,
      );
      renderTranslationSelect(
        els.secondaryTranslation,
        state.preferences.secondaryTranslation,
        true,
      );
      showToast("Bible updated", "success");
      closeBibleEdit();
    } catch (error) {
      console.error("Failed to update Bible", error);
      showToast("Failed to update Bible", "error");
    } finally {
      setBibleEditSubmitting(false);
    }
  }

  async function handleBibleDelete() {
    if (state.bibleEdit.submitting) return;
    const code = state.bibleEdit.translationCode;
    if (!code) {
      showToast("Bible not selected", "error");
      return;
    }
    const confirmMessage =
      "Delete this Bible? This removes the translation and all passages.";
    // eslint-disable-next-line no-alert
    if (!window.confirm(confirmMessage)) {
      return;
    }
    setBibleEditSubmitting(true);
    try {
      await apiFetch(`/bible/translations/${encodeURIComponent(code)}`, {
        method: "DELETE",
      });
      state.translations = state.translations.filter(
        (translation) => translation && translation.code !== code,
      );
      if (state.preferences.mainTranslation === code) {
        const next = state.translations.length
          ? state.translations[0].code
          : "";
        state.preferences.mainTranslation = next;
        state.translationIndex = next ? 0 : -1;
      } else {
        state.translationIndex = findTranslationIndex(
          state.preferences.mainTranslation,
        );
      }
      if (state.preferences.secondaryTranslation === code) {
        state.preferences.secondaryTranslation = "";
      }
      alignMainTranslation(state.preferences.mainTranslation);
      renderTranslationList();
      renderTranslationSelect(
        els.mainTranslation,
        state.preferences.mainTranslation,
      );
      renderTranslationSelect(
        els.secondaryTranslation,
        state.preferences.secondaryTranslation,
        true,
      );
      closeBibleEdit();
      showToast("Bible deleted", "success");
      await loadBooks();
    } catch (error) {
      console.error("Failed to delete Bible", error);
      showToast("Failed to delete Bible", "error");
    } finally {
      setBibleEditSubmitting(false);
    }
  }

  async function refreshBibleTranslations() {
    if (state.refreshingTranslations) return;
    state.refreshingTranslations = true;
    if (els.bibleImport) {
      els.bibleImport.disabled = true;
      els.bibleImport.dataset.loading = "true";
    }
    try {
      const summaries = await apiFetch("/bible/translations/refresh", {
        method: "POST",
      });
      const imported = Array.isArray(summaries) ? summaries.length : 0;
      if (imported > 0) {
        showToast(
          `Imported ${imported} Bible translation${imported === 1 ? "" : "s"}`,
          "success",
        );
      } else {
        showToast("No additional Bible translations available", "info");
      }
      await reloadTranslations();
    } catch (error) {
      console.error("Failed to refresh Bible translations", error);
      showToast("Failed to import Bible translations", "error");
    } finally {
      state.refreshingTranslations = false;
      if (els.bibleImport) {
        els.bibleImport.disabled = false;
        els.bibleImport.dataset.loading = "false";
      }
    }
  }

  async function reloadTranslations() {
    try {
      const next = await apiFetch("/bible/translations");
      if (!Array.isArray(next)) {
        return;
      }
      state.translations = next.map(normalizeTranslation);
      alignMainTranslation(state.preferences.mainTranslation);
      renderTranslationList();
      renderTranslationSelect(
        els.mainTranslation,
        state.preferences.mainTranslation,
      );
      renderTranslationSelect(
        els.secondaryTranslation,
        state.preferences.secondaryTranslation,
        true,
      );
      await loadBooks();
    } catch (error) {
      console.error("Failed to reload Bible translations", error);
      showToast("Failed to reload Bible translations", "error");
    }
  }

  function escapeHtml(value) {
    if (typeof value !== "string") return value;
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
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
      els.loadButton.dataset.loading = loading ? "true" : "false";
    }
  }

  function setSavingPreferences(saving) {
    state.savingPreferences = saving;
    if (els.savePreferences) {
      els.savePreferences.disabled = saving;
      els.savePreferences.dataset.loading = saving ? "true" : "false";
    }
  }

  async function fetchPreferences() {
    try {
      const prefs = await apiFetch("/bible/preferences");
      if (prefs) {
        state.preferences.mainTranslation =
          prefs.mainTranslation || state.preferences.mainTranslation;
        state.preferences.secondaryTranslation =
          prefs.secondaryTranslation || "";
        state.preferences.characterLimit =
          Number(prefs.characterLimit) || state.preferences.characterLimit;
      }
    } catch (error) {
      console.warn("Failed to load Bible preferences", error);
      showToast("Failed to load saved preferences", "warning");
    }
    alignMainTranslation(state.preferences.mainTranslation);
    renderPreferences();
    updateTextareaLines();
  }

  function renderPreferences() {
    renderTranslationList();
    renderTranslationSelect(
      els.mainTranslation,
      state.preferences.mainTranslation,
    );
    renderTranslationSelect(
      els.secondaryTranslation,
      state.preferences.secondaryTranslation,
      true,
    );
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
      await apiFetch("/bible/preferences", {
        method: "PUT",
        body: JSON.stringify(payload),
      });
      updateTextareaLines();
      showToast("Preferences saved", "success");
    } catch (error) {
      console.error("Failed to save preferences", error);
      showToast("Failed to save preferences", "error");
    } finally {
      setSavingPreferences(false);
    }
  }

  async function loadBooks(preserveSelection = true) {
    if (!state.preferences.mainTranslation) return;
    try {
      const data = await apiFetch(
        `/bible/books?translation=${encodeURIComponent(state.preferences.mainTranslation)}`,
      );
      const previousBook = preserveSelection ? state.selectedBook : "";
      const previousBookCode = preserveSelection ? state.selectedBookCode : "";
      const previousBookNumber = preserveSelection
        ? state.selectedBookNumber
        : 0;
      const previousChapter = preserveSelection ? state.selectedChapter : 1;
      const previousVerseStart = preserveSelection ? state.verseStart : 1;
      const previousVerseEnd = preserveSelection ? state.verseEnd : 1;
      const previousVerseEndCustom = preserveSelection
        ? state.verseEndCustom
        : false;

      const rawBooks = Array.isArray(data) ? data : [];
      state.books = rawBooks.map((entry) => {
        const chapters = Array.isArray(entry.chapters)
          ? entry.chapters.map((chapter) => ({
              number: Number(chapter.number) || 1,
              verseCount:
                Number(
                  chapter.verse_count ??
                    chapter.verseCount ??
                    chapter.verse_count ??
                    0,
                ) || 0,
            }))
          : [];
        return {
          name: entry.book || "",
          code: entry.code || "",
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
        state.selectedBookCode = target.code || "";
        state.selectedBookNumber = target.number || 0;
        state.chapters = target.chapters || [];
        const maxChapter = state.chapters.length
          ? state.chapters[state.chapters.length - 1].number || 1
          : 1;
        state.selectedChapter = Math.min(
          Math.max(previousChapter, 1),
          maxChapter,
        );
        state.verseStart = Math.max(previousVerseStart, 1);
        state.verseEnd = Math.max(previousVerseEnd, state.verseStart);
        state.verseEndCustom = previousVerseEndCustom;
        applyChapterDefaults();
      } else {
        state.selectedBook = "";
        state.selectedBookCode = "";
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
      console.error("Failed to load books", error);
      showToast("Failed to load books", "error");
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
        const chapterCount = Array.isArray(entry.chapters)
          ? entry.chapters.length
          : 0;
        const meta = chapterCount
          ? `<span class=\"operator__list-meta\">${chapterCount} ch.</span>`
          : "";
        const activeAttr = isSelected ? ' data-active=\"true\"' : "";
        return `<div class=\"operator__list-item\">
          <button type=\"button\" class=\"operator__list-button\"${activeAttr} data-book=\"${escapeHtml(
            entry.name,
          )}\" data-book-code=\"${escapeHtml(entry.code || "")}\" data-book-number=\"${entry.number}\">\n            <span class=\"operator__list-label\">${escapeHtml(entry.name)}</span>
            ${meta}
          </button>
        </div>`;
      })
      .join("");
    els.bookList.innerHTML = html;
  }

  function applyChapterDefaults() {
    const chapterEntry = (state.chapters || []).find(
      (c) => c.number === state.selectedChapter,
    );
    const maxChapter = state.chapters.length
      ? state.chapters[state.chapters.length - 1].number
      : 1;
    state.selectedChapter = Math.min(
      Math.max(state.selectedChapter, 1),
      maxChapter || 1,
    );
    if (chapterEntry) {
      const verseCount =
        chapterEntry.verseCount || chapterEntry.verse_count || 1;
      state.verseStart = Math.min(Math.max(state.verseStart, 1), verseCount);
      if (state.verseEndCustom) {
        state.verseEnd = Math.min(
          Math.max(state.verseEnd, state.verseStart),
          verseCount,
        );
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
      const maxChapter = state.chapters.length
        ? state.chapters[state.chapters.length - 1].number
        : 1;
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
        els.verseEndInput.value = "";
      }
      els.verseEndInput.placeholder = "All";
      els.verseEndInput.min = state.verseStart;
      els.verseEndInput.max = verseCount;
    }
  }

  function getCurrentVerseCount() {
    const chapterEntry = (state.chapters || []).find(
      (c) => c.number === state.selectedChapter,
    );
    return chapterEntry
      ? chapterEntry.verseCount || chapterEntry.verse_count || 1
      : 1;
  }

  function normalizeForSearch(input) {
    if (!input) return "";
    return String(input)
      .normalize("NFD")
      .replace(/[\u0300-\u036f]/g, "")
      .toLowerCase();
  }

  function filterBooks(value) {
    const term = normalizeForSearch(value.trim());
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
        const nameKey = normalizeForSearch(entry.name || "");
        const codeKey = normalizeForSearch(entry.code || "");
        const nameMatch = nameKey.includes(term);
        const codeMatch = codeKey.includes(term);
        return nameMatch || codeMatch;
      });
      state.bookSelectionLocked = false;
    }
    if (
      !state.filteredBooks.find((entry) => entry.name === state.selectedBook) &&
      state.filteredBooks.length
    ) {
      const next = state.filteredBooks[0];
      state.selectedBook = next.name;
      state.selectedBookCode = next.code || "";
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
      showToast("Select a book first", "warning");
      return;
    }
    const chapterValue =
      Number(
        els.chapterInput ? els.chapterInput.value : state.selectedChapter,
      ) || state.selectedChapter;
    state.selectedChapter = Math.max(1, chapterValue);
    const verseCount = getCurrentVerseCount();
    const verseStartRaw = els.verseStartInput
      ? els.verseStartInput.value
      : `${state.verseStart}`;
    const verseEndRaw = els.verseEndInput ? els.verseEndInput.value : "";
    const candidateStart = Number(verseStartRaw);
    const verseStartValue = Number.isFinite(candidateStart)
      ? candidateStart
      : state.verseStart;
    const candidateEnd =
      verseEndRaw && verseEndRaw.trim().length ? Number(verseEndRaw) : null;
    state.verseStart = Math.min(Math.max(verseStartValue, 1), verseCount);
    if (candidateEnd === null) {
      state.verseEndCustom = false;
      state.verseEnd = verseCount;
    } else {
      state.verseEndCustom = true;
      const safeEnd = Number.isFinite(candidateEnd)
        ? candidateEnd
        : state.verseStart;
      state.verseEnd = Math.min(
        Math.max(safeEnd, state.verseStart),
        verseCount,
      );
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
      const response = await apiFetch("/bible/resolve", {
        method: "POST",
        body: JSON.stringify(payload),
      });
      state.slides = Array.isArray(response.slides)
        ? response.slides.map((slide) => {
            const metadata = slide.metadata || null;
            const mainReference =
              slide.main_reference || deriveReferenceFromMetadata(metadata);
            const translationReference =
              slide.translation_reference ||
              deriveReferenceFromMetadata(metadata);
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
      showToast("Slides loaded", "success");
    } catch (error) {
      console.error("Failed to load slides", error);
      showToast(error.message || "Failed to load slides", "error");
    } finally {
      setLoadingSlides(false);
    }
  }

  function renderSlides() {
    if (!els.slidesContainer) return;
    if (!state.slides.length) {
      els.slidesContainer.innerHTML =
        "<p class='operator__slides-empty'>Load a passage to populate slides.</p>";
      return;
    }
    const html = state.slides
      .map((slide, index) =>
        renderSlideCard(slide, index, state.editMode ? {} : {}),
      )
      .join("");
    els.slidesContainer.innerHTML = html;
  }

  function renderSlideCard(slide, index, options) {
    const triggerOnly = options && options.triggerOnly;
    const selected = state.selectedSlides.has(slide.id) ? " is-selected" : "";
    if (triggerOnly) {
      const translationMarkup =
        slide.translation &&
        slide.translation.trim().length &&
        state.preferences.secondaryTranslation
          ? `<div class='operator__slide-text operator__slide-text--translation operator__slide-text--secondary'>${lineBreakHtml(slide.translation)}</div>`
          : "";
      const references = buildReferenceHtml(slide);
      const liveDragHandle =
        state.bibleTab === "prepared"
          ? `<button type='button' class='operator__slide-handle' data-role='slide-drag-handle' draggable='true' tabindex='-1' aria-label='Reorder slide'>\u2195</button>`
          : "";
      return `
        <article class='operator__slide-card operator__slide-card--bible' data-slide-id='${slide.id}' data-index='${index}'>
          ${liveDragHandle}
          <div class='operator__slide-trigger-zone operator__slide-trigger-zone--full' data-role='slide-trigger'>
            <span class='operator__slide-trigger-icon'>\u25B6</span>
            <span class='operator__slide-index'>${index + 1}</span>
            <section class='operator__slide-bodies operator__slide-bodies--bible'>
              <div class='operator__slide-text operator__slide-text--main'>${lineBreakHtml(slide.main)}</div>
              ${translationMarkup}
              ${references}
            </section>
          </div>
        </article>
      `;
    }
    if (state.editMode) {
      const checked = state.selectedSlides.has(slide.id) ? " checked" : "";
      const isPreparedEdit = state.bibleTab === "prepared";
      const deleteBtn = isPreparedEdit
        ? `<button type='button' class='operator__list-action operator__list-action--danger' data-role='delete-slide' data-slide-id='${slide.id}' title='Delete slide'>\u00D7</button>`
        : "";
      const dragHandle = isPreparedEdit
        ? `<button type='button' class='operator__slide-handle' data-role='slide-drag-handle' draggable='true' tabindex='-1' aria-label='Reorder slide'>\u2195</button>`
        : "";
      const editHeader = `
        <header class='operator__slide-header'>
          <div class='operator__slide-header-left'>
            ${dragHandle}
            <label class='operator__slide-index operator__slide-index--select'>
              <input type='checkbox' data-role='slide-select'${checked} />
              <span>${index + 1}</span>
            </label>
          </div>
          <div class='operator__slide-controls operator__slide-controls--compact'>
            ${deleteBtn}
            <button type='button' class='operator__list-action operator__list-action--primary' data-role='slide-trigger'>Trigger</button>
          </div>
        </header>
      `;
      return `
        <article class='operator__slide-card operator__slide-card--bible operator__slide-card--edit' data-slide-id='${slide.id}' data-index='${index}'>
          ${editHeader}
          <section class='operator__slide-editor operator__slide-editor--bible'>
            <label>
              <span>Main</span>
              <textarea data-role='slide-main'>${escapeHtml(slide.main || "")}</textarea>
            </label>
            <label>
              <span>Translation</span>
              <textarea data-role='slide-translation'>${escapeHtml(slide.translation || "")}</textarea>
            </label>
            <div class='operator__slide-editor-grid'>
              <label>
                <span>Main Reference</span>
                <input type='text' data-role='slide-main-ref' value='${escapeHtml(slide.mainReference || "")}' />
              </label>
              <label>
                <span>Translation Reference</span>
                <input type='text' data-role='slide-translation-ref' value='${escapeHtml(slide.translationReference || "")}' />
              </label>
            </div>
          </section>
        </article>
      `;
    }
    const translationMarkup =
      slide.translation && slide.translation.trim().length
        ? `<div class='operator__slide-text operator__slide-text--translation operator__slide-text--secondary'>${lineBreakHtml(slide.translation)}</div>`
        : "";
    const references = buildReferenceHtml(slide);
    return `
      <article class='operator__slide-card operator__slide-card--bible${selected}' data-slide-id='${slide.id}' data-index='${index}'>
        <div class='operator__slide-trigger-zone' data-role='slide-trigger'>
          <span class='operator__slide-trigger-icon'>\u25B6</span>
          <span class='operator__slide-index'>${index + 1}</span>
        </div>
        <div class='operator__slide-select-zone' data-role='slide-select-zone'>
          <section class='operator__slide-bodies operator__slide-bodies--bible'>
            <div class='operator__slide-text operator__slide-text--main'>${lineBreakHtml(slide.main)}</div>
            ${translationMarkup}
            ${references}
          </section>
        </div>
      </article>
    `;
  }

  function buildReferenceHtml(slide) {
    const pieces = [];
    if (slide.mainReference) {
      pieces.push(
        `<span class='operator__slide-reference'>${escapeHtml(slide.mainReference)}</span>`,
      );
    } else if (slide.metadata && slide.metadata.bible) {
      const verses = slide.metadata.bible.verses || [];
      if (verses.length) {
        const start = verses[0].start;
        const end = verses[verses.length - 1].end;
        pieces.push(
          `<span class='operator__slide-reference'>${escapeHtml(formatReference(slide.metadata.bible.book, slide.metadata.bible.chapter, start, end))}</span>`,
        );
      }
    }
    if (slide.translationReference && state.preferences.secondaryTranslation) {
      pieces.push(
        `<span class='operator__slide-reference operator__slide-reference--secondary'>${escapeHtml(slide.translationReference)}</span>`,
      );
    }
    if (!pieces.length) {
      return "";
    }
    return `<footer class='operator__slide-footer'>${pieces.join("")}</footer>`;
  }

  function mapCoreSlidesToState(slides) {
    if (!Array.isArray(slides)) return [];
    return slides.map(function (slide) {
      // Handle both core Slide format ({content: {main: {value}}}) and DTO format ({main: "..."})
      var mainVal = "";
      var translationVal = "";
      var stageVal = "";
      var groupVal = null;
      if (slide.content && typeof slide.content === "object") {
        mainVal =
          typeof slide.content.main === "object" && slide.content.main
            ? slide.content.main.value || ""
            : slide.content.main || "";
        translationVal =
          typeof slide.content.translation === "object" &&
          slide.content.translation
            ? slide.content.translation.value || ""
            : slide.content.translation || "";
        stageVal =
          typeof slide.content.stage === "object" && slide.content.stage
            ? slide.content.stage.value || ""
            : slide.content.stage || "";
        groupVal = slide.content.group
          ? slide.content.group.name || slide.content.group || null
          : null;
      } else {
        mainVal = slide.main || "";
        translationVal = slide.translation || "";
        stageVal = slide.stage || "";
        groupVal = slide.group || null;
      }
      var meta = slide.metadata || null;
      var mainRef =
        slide.main_reference ||
        slide.mainReference ||
        deriveReferenceFromMetadata(meta);
      var translationRef =
        slide.translation_reference ||
        slide.translationReference ||
        deriveReferenceFromMetadata(meta);
      return {
        id: slide.id,
        order: slide.order,
        main: mainVal,
        translation: translationVal,
        stage: stageVal,
        group: typeof groupVal === "object" ? null : groupVal,
        metadata: meta,
        mainReference: mainRef,
        translationReference: translationRef,
      };
    });
  }

  function lineBreakHtml(value) {
    return escapeHtml(value || "").replace(/\n/g, "<br />");
  }
  function resolveTranslationByCode(code) {
    if (!code) return null;
    const target = String(code).toLowerCase();
    return (
      state.translations.find(
        (translation) => translation.code.toLowerCase() === target,
      ) || null
    );
  }

  function buildLoadedPassageKey(entry) {
    if (!entry) return null;
    const endValue = entry.verseEnd === null ? "all" : entry.verseEnd;
    return [
      entry.mainTranslation || "",
      entry.secondaryTranslation || "",
      entry.bookCode || entry.book || "",
      entry.bookNumber || 0,
      entry.chapter || 0,
      entry.verseStart || 0,
      endValue,
      entry.characterLimit || 0,
    ].join("|");
  }

  function recordLoadedPassage(payload) {
    if (!payload) return;
    const normalized = {
      mainTranslation: payload.mainTranslation || "",
      secondaryTranslation: payload.secondaryTranslation || "",
      book: payload.book || "",
      bookCode: payload.bookCode || "",
      bookNumber:
        typeof payload.bookNumber === "number" ? payload.bookNumber : null,
      chapter: payload.chapter || 1,
      verseStart: payload.verseStart || 1,
      verseEnd: typeof payload.verseEnd === "number" ? payload.verseEnd : null,
      characterLimit:
        payload.characterLimit || state.preferences.characterLimit,
    };
    const key = buildLoadedPassageKey(normalized);
    if (!key) return;
    const existingIndex = loadedPassageKeys.get(key);
    if (typeof existingIndex === "number") {
      state.loadedPassages.splice(existingIndex, 1);
    }
    const main = resolveTranslationByCode(normalized.mainTranslation);
    const secondary = resolveTranslationByCode(normalized.secondaryTranslation);
    const referenceEnd =
      normalized.verseEnd === null ? state.verseEnd : normalized.verseEnd;
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
    state.loadedPassages.forEach((item, index) =>
      loadedPassageKeys.set(item.key, index),
    );
    renderLoadedPassages();
  }

  function renderLoadedPassages() {
    if (!els.loadedPassages) return;
    if (!state.loadedPassages.length) {
      els.loadedPassages.innerHTML =
        "<li class='operator__list-item operator__list-item--empty'>Load a passage to populate this list.</li>";
      return;
    }
    const html = state.loadedPassages
      .map((entry) => {
        const reference = entry.includeFullChapter
          ? `${entry.book} ${entry.chapter}`
          : formatReference(
              entry.book,
              entry.chapter,
              entry.verseStart,
              entry.verseEnd,
            );
        const translationLabel = entry.translationName || entry.translationCode;
        const secondaryLabel =
          entry.secondaryTranslationName || entry.secondaryTranslationCode;
        const secondaryBadge = secondaryLabel
          ? `<span class='operator__list-meta operator__list-meta--secondary'>${escapeHtml(secondaryLabel)}</span>`
          : "";
        return `<li class='operator__list-item' data-loaded-key='${entry.key}'>
          <button type='button' class='operator__list-button'>
            <span class='operator__list-label'>${escapeHtml(reference)}</span>
            <span class='operator__list-meta'>${escapeHtml(translationLabel)}</span>
            ${secondaryBadge}
          </button>
        </li>`;
      })
      .join("");
    els.loadedPassages.innerHTML = html;
  }

  async function applyLoadedPassage(entry) {
    if (!entry) return;
    const translationChanged =
      entry.translationCode &&
      entry.translationCode !== state.preferences.mainTranslation;
    if (translationChanged) {
      alignMainTranslation(entry.translationCode);
      renderTranslationList();
      await loadBooks(false);
    } else {
      await loadBooks();
    }
    state.preferences.secondaryTranslation =
      entry.secondaryTranslationCode || "";
    renderTranslationSelect(
      els.mainTranslation,
      state.preferences.mainTranslation,
    );
    renderTranslationSelect(
      els.secondaryTranslation,
      state.preferences.secondaryTranslation,
      true,
    );
    if (typeof entry.characterLimit === "number" && entry.characterLimit > 0) {
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
      state.selectedBookCode = bookEntry.code || "";
      state.selectedBookNumber = bookEntry.number || 0;
      state.chapters = bookEntry.chapters || [];
    } else if (state.books.length) {
      const fallback = state.books[0];
      state.selectedBook = fallback.name;
      state.selectedBookCode = fallback.code || "";
      state.selectedBookNumber = fallback.number || 0;
      state.chapters = fallback.chapters || [];
    } else {
      showToast("No books available for the selected translation", "error");
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
    state.verseEnd = entry.includeFullChapter
      ? getCurrentVerseCount()
      : entry.verseEnd;
    applyChapterDefaults();
    renderBookList();
    updateReferenceInputs();
    await loadSlides();
  }

  function updateTextareaLines() {
    const charLimit = state.preferences.characterLimit || 320;
    const lineChars = 32;
    const lines = Math.max(3, Math.ceil(charLimit / lineChars));
    document.body.style.setProperty("--bible-textarea-lines", lines);
  }

  function updateSelectionLabel() {
    if (!els.selectionCount) return;
    const count = state.selectedSlides.size;
    els.selectionCount.textContent = `${count} selected`;
  }

  function toggleSelectAllSlides() {
    if (
      state.selectedSlides.size === state.slides.length &&
      state.slides.length > 0
    ) {
      state.selectedSlides.clear();
    } else {
      state.slides.forEach(function (slide) {
        state.selectedSlides.add(slide.id);
      });
    }
    renderSlides();
    updateSelectionLabel();
  }

  function updateMode() {
    if (typeof document !== "undefined" && document.body) {
      document.body.dataset.mode = state.editMode ? "edit" : "live";
    }
    if (els.modeToggleContainer) {
      els.modeToggleContainer.querySelectorAll("[data-mode]").forEach((btn) => {
        btn.dataset.active =
          btn.dataset.mode === (state.editMode ? "edit" : "live")
            ? "true"
            : "false";
      });
    }
  }

  function ensureBibleMetadata(slide) {
    if (!slide || typeof slide !== "object") {
      return {};
    }
    if (!slide.metadata || typeof slide.metadata !== "object") {
      slide.metadata = {};
    }
    if (!slide.metadata.bible || typeof slide.metadata.bible !== "object") {
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
      showToast("Select at least one slide", "warning");
      return;
    }
    let targetPresentationId =
      els.presentationSelect && els.presentationSelect.value;
    if (!targetPresentationId) {
      showToast("Select a presentation first", "warning");
      return;
    }
    if (targetPresentationId === "__new__") {
      const name = window.prompt("New presentation name");
      if (!name || !name.trim()) return;
      try {
        const created = await apiFetch("/bible/presentations", {
          method: "POST",
          body: JSON.stringify({ name: name.trim() }),
        });
        await loadPresentations();
        targetPresentationId = created.id;
        if (els.presentationSelect) {
          els.presentationSelect.value = targetPresentationId;
        }
      } catch (error) {
        showToast(error.message || "Failed to create presentation", "error");
        return;
      }
    }
    try {
      const slides = state.slides
        .filter((slide) => state.selectedSlides.has(slide.id))
        .map(slideToPayload);
      await apiFetch(`/bible/presentations/${targetPresentationId}/append`, {
        method: "POST",
        body: JSON.stringify({ slides }),
      });
      showToast(
        `Added ${slides.length} slide${slides.length === 1 ? "" : "s"}`,
        "success",
      );
      await loadPresentations();
      state.selectedSlides.clear();
      renderSlides();
      updateSelectionLabel();
    } catch (error) {
      console.error("Failed to append slides", error);
      showToast(error.message || "Failed to append slides", "error");
    }
  }

  function slideToPayload(slide) {
    const metadata = slide.metadata
      ? JSON.parse(JSON.stringify(slide.metadata))
      : null;
    if (metadata && metadata.bible) {
      const bibleMeta = metadata.bible;
      const mainLabel =
        slide.mainReference ||
        bibleMeta.mainReferenceLabel ||
        bibleMeta.main_reference_label ||
        null;
      const translationLabel =
        slide.translationReference ||
        bibleMeta.translationReferenceLabel ||
        bibleMeta.translation_reference_label ||
        null;
      bibleMeta.mainReferenceLabel = mainLabel;
      bibleMeta.main_reference_label = mainLabel;
      bibleMeta.translationReferenceLabel = translationLabel;
      bibleMeta.translation_reference_label = translationLabel;
    }
    return {
      main: slide.main,
      translation: slide.translation || "",
      stage: slide.stage || slide.main,
      group: slide.group || null,
      metadata,
    };
  }

  async function loadPresentations() {
    try {
      const data = await apiFetch("/bible/presentations");
      state.presentations = Array.isArray(data) ? data : [];
      renderPresentationSelect();
      renderPresentations();
    } catch (error) {
      console.error("Failed to load presentations", error);
      showToast("Failed to load presentations", "error");
    }
  }

  async function renamePresentation(presentationId, currentName) {
    const next =
      typeof window !== "undefined"
        ? window.prompt("Rename presentation", currentName || "")
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
        method: "PATCH",
        body: JSON.stringify({ name: trimmed }),
      });
      showToast("Presentation renamed", "success");
      await loadPresentations();
    } catch (error) {
      console.error("Failed to rename presentation", error);
      showToast(error.message || "Failed to rename presentation", "error");
    }
  }

  function openPresentationEditModal(id, name) {
    state.presentationEditTarget = { id, name };
    if (els.presentationEditName) {
      els.presentationEditName.value = name;
      els.presentationEditName.disabled = false;
    }
    if (els.presentationEditDelete) {
      els.presentationEditDelete.disabled = false;
    }
    if (els.presentationEditModal) {
      els.presentationEditModal.dataset.open = "true";
      document.body.dataset.modalOpen = "presentation-edit";
      window.setTimeout(() => {
        if (els.presentationEditName) {
          els.presentationEditName.focus();
          els.presentationEditName.select();
        }
      }, 15);
    }
  }

  function closePresentationEditModal() {
    state.presentationEditTarget = null;
    if (els.presentationEditModal) {
      els.presentationEditModal.dataset.open = "false";
    }
    if (document.body.dataset.modalOpen === "presentation-edit") {
      delete document.body.dataset.modalOpen;
    }
  }

  async function handlePresentationEditSubmit(event) {
    event.preventDefault();
    if (!state.presentationEditTarget) return;
    const nameInput = els.presentationEditName;
    const name = nameInput ? nameInput.value.trim() : "";
    if (!name) {
      showToast("Presentation name cannot be empty", "warning");
      if (nameInput) nameInput.focus();
      return;
    }
    const id = state.presentationEditTarget.id;
    if (name === state.presentationEditTarget.name) {
      closePresentationEditModal();
      return;
    }
    if (els.presentationEditName) els.presentationEditName.disabled = true;
    if (els.presentationEditDelete) els.presentationEditDelete.disabled = true;
    try {
      await apiFetch(`/bible/presentations/${id}`, {
        method: "PATCH",
        body: JSON.stringify({ name }),
      });
      showToast("Presentation renamed", "success");
      closePresentationEditModal();
      await loadPresentations();
    } catch (error) {
      console.error("Failed to rename presentation", error);
      showToast(error.message || "Failed to rename presentation", "error");
      if (els.presentationEditName) els.presentationEditName.disabled = false;
      if (els.presentationEditDelete)
        els.presentationEditDelete.disabled = false;
    }
  }

  async function handlePresentationEditDelete() {
    if (!state.presentationEditTarget) return;
    const id = state.presentationEditTarget.id;
    const name = state.presentationEditTarget.name;
    if (
      !window.confirm(`Delete presentation "${name}"? This cannot be undone.`)
    ) {
      return;
    }
    if (els.presentationEditName) els.presentationEditName.disabled = true;
    if (els.presentationEditDelete) els.presentationEditDelete.disabled = true;
    try {
      await apiFetch(`/bible/presentations/${id}`, { method: "DELETE" });
      showToast("Presentation deleted", "success");
      if (state.activePresentationId === id) {
        state.activePresentationId = "";
        state.activePresentationSlides = [];
      }
      closePresentationEditModal();
      await loadPresentations();
    } catch (error) {
      console.error("Failed to delete presentation", error);
      showToast(error.message || "Failed to delete presentation", "error");
      if (els.presentationEditName) els.presentationEditName.disabled = false;
      if (els.presentationEditDelete)
        els.presentationEditDelete.disabled = false;
    }
  }

  function renderPresentationSelect() {
    if (!els.presentationSelect) return;
    const options = state.presentations
      .map(
        (presentation) =>
          `<option value="${presentation.id}">${escapeHtml(presentation.name)}</option>`,
      )
      .join("");
    els.presentationSelect.innerHTML = `<option value="">Add to…</option><option value="__new__">+ New presentation</option>${options}`;
  }

  function renderPresentations() {
    if (!els.presentationsList) return;
    if (!state.presentations.length) {
      els.presentationsList.innerHTML =
        "<p class='operator__slides-empty'>No Bible presentations yet.</p>";
      return;
    }
    const html = state.presentations
      .map((presentation) => {
        const escapedName = escapeHtml(presentation.name);
        const activeClass =
          presentation.id === state.activePresentationId ? " is-active" : "";
        return `
          <article class='operator__presentation-card${activeClass}' data-presentation-id='${presentation.id}'>
            <header>
              <strong>${escapedName}</strong>
              <button type='button' class='operator__presentation-action' data-role='presentation-edit' data-presentation-id='${presentation.id}' data-presentation-name='${escapedName}' title='Edit presentation'>
                <span aria-hidden='true'>\u270E</span>
              </button>
            </header>
            <p>${presentation.slideCount || 0} slide${presentation.slideCount === 1 ? "" : "s"}</p>
          </article>
        `;
      })
      .join("");
    els.presentationsList.innerHTML = html;
  }

  async function loadPresentationSlides(id) {
    try {
      const detail = await apiFetch(`/bible/presentations/${id}`);
      state.activePresentationSlides = Array.isArray(detail.slides)
        ? detail.slides.map((slide) => {
            const metadata = slide.metadata || null;
            const mainReference =
              slide.main_reference ||
              slide.mainReference ||
              deriveReferenceFromMetadata(metadata);
            const translationReference =
              slide.translation_reference ||
              slide.translationReference ||
              deriveReferenceFromMetadata(metadata);
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
      renderPresentationSlides();
    } catch (error) {
      console.error("Failed to load presentation slides", error);
      showToast("Failed to load presentation", "error");
    }
  }

  function renderPresentationSlides() {
    if (!els.slidesContainer) return;
    if (!state.activePresentationSlides.length) {
      els.slidesContainer.innerHTML =
        "<p class='operator__slides-empty'>No slides in this presentation.</p>";
      return;
    }
    const html = state.activePresentationSlides
      .map((slide, index) =>
        renderSlideCard(
          slide,
          index,
          state.editMode ? {} : { triggerOnly: true },
        ),
      )
      .join("");
    els.slidesContainer.innerHTML = html;
  }

  function renderActive() {
    // Active passage card removed — stage preview in header is sufficient
  }

  function connectLiveSocket() {
    if (state.liveSocket) {
      try {
        state.liveSocket.close();
      } catch (_) {
        /* ignore */
      }
    }
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const socket = new WebSocket(
      `${protocol}//${window.location.host}/live/ws`,
    );
    state.liveSocket = socket;
    socket.addEventListener("message", (event) => {
      try {
        const payload = JSON.parse(event.data);
        if (payload.type === "bible" || payload.type === "Bible") {
          state.activeBroadcast = payload.broadcast || null;
          renderActive();
        } else if (
          payload.type === "bible_cleared" ||
          payload.type === "BibleCleared"
        ) {
          state.activeBroadcast = null;
          renderActive();
        } else if (
          payload.type === "bible_preferences_changed" ||
          payload.type === "BiblePreferencesChanged"
        ) {
          if (payload.character_limit != null) {
            state.preferences.characterLimit = payload.character_limit;
            if (els.charLimit) {
              els.charLimit.value = payload.character_limit;
            }
            updateTextareaLines();
          }
        }
      } catch (error) {
        console.warn("Failed to parse bible payload", error);
      }
    });
    socket.addEventListener("close", () => {
      if (state.liveReconnectTimer) return;
      state.liveReconnectTimer = setTimeout(() => {
        state.liveReconnectTimer = null;
        connectLiveSocket();
      }, 2000);
    });
    socket.addEventListener("error", (error) => {
      console.error("Bible live socket error", error);
      try {
        socket.close();
      } catch (_) {
        /* ignore */
      }
    });
  }

  async function refreshActiveFromServer() {
    try {
      const active = await apiFetch("/bible/active");
      // Avoid unnecessary re-renders if unchanged
      const prev =
        state.activeBroadcast && JSON.stringify(state.activeBroadcast);
      const next = active && JSON.stringify(active);
      if (prev !== next) {
        state.activeBroadcast = active || null;
        renderActive();
      }
    } catch (_) {
      /* ignore transient errors */
    }
  }

  function ensureActivePoller() {
    if (state.activePollTimer) return;
    state.activePollTimer = setInterval(() => {
      // Poll as a fallback so external triggers (HTTP) still reflect in UI
      refreshActiveFromServer();
    }, 1000);
  }

  async function triggerSlideById(slideId) {
    const slide =
      state.slides.find((entry) => entry.id === slideId) ||
      state.activePresentationSlides.find((entry) => entry.id === slideId);
    if (!slide) {
      showToast("Slide not found", "error");
      return;
    }
    // Empty slide (no text and no bible metadata) = clear broadcast
    if (!slide.main && (!slide.metadata || !slide.metadata.bible)) {
      clearBroadcast();
      return;
    }
    if (!slide.metadata || !slide.metadata.bible) {
      showToast("Slide metadata missing", "error");
      return;
    }
    const bibleMeta = ensureBibleMetadata(slide);
    const verses = Array.isArray(bibleMeta.verses) ? bibleMeta.verses : [];
    if (!verses.length) {
      showToast("Verse metadata missing", "error");
      return;
    }
    const translationCode =
      bibleMeta.translation_code || bibleMeta.translationCode;
    if (!translationCode) {
      showToast("Translation metadata missing", "error");
      return;
    }
    const verseStart = verses[0].start;
    const verseEnd = verses[verses.length - 1].end;
    const book = bibleMeta.book || state.selectedBook;
    const bookCode =
      bibleMeta.book_code ||
      bibleMeta.bookCode ||
      state.selectedBookCode ||
      null;
    const bookNumber =
      bibleMeta.book_number ||
      bibleMeta.bookNumber ||
      state.selectedBookNumber ||
      null;
    const chapter =
      typeof bibleMeta.chapter === "number"
        ? bibleMeta.chapter
        : state.selectedChapter;
    try {
      const payload = {
        translation: translationCode,
        book,
        chapter,
        verseStart,
        verseEnd,
      };
      if (bookCode) {
        payload.bookCode = bookCode;
      }
      if (bookNumber) {
        payload.bookNumber = bookNumber;
      }
      // Always send the current slide text (supports user edits)
      payload.mainText = slide.main || null;
      payload.translationText = slide.translation || null;
      payload.mainReferenceLabel = slide.mainReference || null;
      payload.translationReferenceLabel = slide.translationReference || null;
      const response = await apiFetch("/bible/trigger", {
        method: "POST",
        body: JSON.stringify(payload),
      });
      state.activeBroadcast = response;
      renderActive();
      showToast("Slide triggered", "success");
    } catch (error) {
      console.error("Failed to trigger slide", error);
      showToast(error.message || "Failed to trigger slide", "error");
    }
  }

  async function clearBroadcast() {
    try {
      await apiFetch("/bible/clear", { method: "POST" });
      state.activeBroadcast = null;
      renderActive();
      showToast("Broadcast cleared", "success");
    } catch (error) {
      console.error("Failed to clear broadcast", error);
      showToast("Failed to clear broadcast", "error");
    }
  }

  async function handleDeleteSlide(slideId) {
    if (!state.activePresentationId) return;
    try {
      var response = await apiFetch(
        "/presentations/" + state.activePresentationId + "/slides/" + slideId,
        { method: "DELETE" },
      );
      state.activePresentationSlides = mapCoreSlidesToState(response);
      renderPresentationSlides();
      showToast("Slide deleted", "success");
    } catch (error) {
      console.error("Failed to delete slide", error);
      showToast("Failed to delete slide", "error");
    }
  }

  var dragState = { slideId: null };

  function onPreparedDragStart(event) {
    var handle = event.target.closest('[data-role="slide-drag-handle"]');
    if (!handle) {
      event.preventDefault();
      return;
    }
    var card = handle.closest("[data-slide-id]");
    if (!card || state.bibleTab !== "prepared") {
      event.preventDefault();
      return;
    }
    dragState.slideId = card.getAttribute("data-slide-id");
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", dragState.slideId);
    card.classList.add("is-dragging");
  }

  function onPreparedDragOver(event) {
    if (!dragState.slideId || state.bibleTab !== "prepared") return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";

    // Auto-scroll when dragging near container edges
    var container = els.slidesContainer;
    if (container) {
      var containerRect = container.getBoundingClientRect();
      var scrollThreshold = 60; // pixels from edge
      var scrollSpeed = 8;
      if (event.clientY < containerRect.top + scrollThreshold) {
        container.scrollTop -= scrollSpeed;
      } else if (event.clientY > containerRect.bottom - scrollThreshold) {
        container.scrollTop += scrollSpeed;
      }
    }

    var target = event.target.closest("[data-slide-id]");
    if (!target || target.getAttribute("data-slide-id") === dragState.slideId) {
      return;
    }
    // Clear all indicators first, then show on target only
    els.slidesContainer
      .querySelectorAll("[data-slide-id]")
      .forEach(function (c) {
        c.classList.remove("drag-over-above", "drag-over-below");
      });
    var rect = target.getBoundingClientRect();
    var midY = rect.top + rect.height / 2;
    if (event.clientY < midY) {
      target.classList.add("drag-over-above");
    } else {
      target.classList.add("drag-over-below");
    }
  }

  function onPreparedDragEnd(event) {
    dragState.slideId = null;
    if (els.slidesContainer) {
      els.slidesContainer
        .querySelectorAll("[data-slide-id]")
        .forEach(function (c) {
          c.classList.remove(
            "is-dragging",
            "drag-over-above",
            "drag-over-below",
          );
        });
    }
  }

  function onPreparedDrop(event) {
    event.preventDefault();
    if (!dragState.slideId || state.bibleTab !== "prepared") return;
    var target = event.target.closest("[data-slide-id]");
    if (!target) return;
    var targetId = target.getAttribute("data-slide-id");
    if (targetId === dragState.slideId) return;

    // Determine insert position
    var rect = target.getBoundingClientRect();
    var midY = rect.top + rect.height / 2;
    var insertBefore = event.clientY < midY;

    // Compute new order
    var orderedIds = state.activePresentationSlides.map(function (s) {
      return s.id;
    });
    // Remove dragged item
    var fromIndex = orderedIds.indexOf(dragState.slideId);
    if (fromIndex < 0) return;
    orderedIds.splice(fromIndex, 1);
    // Find target index
    var toIndex = orderedIds.indexOf(targetId);
    if (toIndex < 0) return;
    if (!insertBefore) {
      toIndex += 1;
    }
    orderedIds.splice(toIndex, 0, dragState.slideId);

    reorderPreparedSlides(orderedIds);
    onPreparedDragEnd(event);
  }

  async function reorderPreparedSlides(orderedIds) {
    if (!state.activePresentationId) return;
    try {
      var response = await apiFetch(
        "/presentations/" + state.activePresentationId + "/slides/reorder",
        {
          method: "POST",
          body: JSON.stringify({ slideIds: orderedIds }),
        },
      );
      state.activePresentationSlides = mapCoreSlidesToState(response);
      renderPresentationSlides();
    } catch (error) {
      console.error("Failed to reorder slides", error);
      showToast("Failed to reorder slides", "error");
    }
  }

  async function addEmptySlide() {
    if (!state.activePresentationId) {
      showToast("Select a presentation first", "warning");
      return;
    }
    try {
      const response = await apiFetch(
        `/presentations/${state.activePresentationId}/slides`,
        { method: "POST", body: JSON.stringify({}) },
      );
      // response is the updated slides array (core Slide format)
      state.activePresentationSlides = mapCoreSlidesToState(response);
      renderPresentationSlides();
      showToast("Empty slide added", "success");
    } catch (error) {
      console.error("Failed to add empty slide", error);
      showToast("Failed to add slide", "error");
    }
  }

  function onSlidesContainerClick(event) {
    const card = event.target.closest("[data-slide-id]");
    if (!card) return;
    const slideId = card.getAttribute("data-slide-id");
    if (!slideId) return;

    // Delete slide button (prepared tab, edit mode)
    if (event.target.closest('[data-role="delete-slide"]')) {
      var deleteSlideId =
        event.target.closest('[data-role="delete-slide"]').dataset.slideId ||
        slideId;
      handleDeleteSlide(deleteSlideId);
      return;
    }

    // Drag handle clicks should not trigger anything
    if (event.target.closest('[data-role="slide-drag-handle"]')) {
      return;
    }

    // Edit mode: keep old checkbox + trigger button behavior
    if (state.editMode) {
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
      return;
    }

    // Prepared tab LIVE mode: clicking anywhere on the card triggers the slide
    if (state.bibleTab === "prepared") {
      triggerSlideById(slideId);
      return;
    }

    // Trigger zone click → fire the slide
    if (event.target.closest('[data-role="slide-trigger"]')) {
      triggerSlideById(slideId);
      return;
    }

    // Select zone click → toggle selection
    if (event.target.closest('[data-role="slide-select-zone"]')) {
      if (state.selectedSlides.has(slideId)) {
        state.selectedSlides.delete(slideId);
      } else {
        state.selectedSlides.add(slideId);
      }
      card.classList.toggle("is-selected", state.selectedSlides.has(slideId));
      updateSelectionLabel();
      return;
    }
  }

  var _slideAutoSaveTimers = new Map();
  function debounceSaveSlide(presentationId, slide) {
    var key = slide.id;
    if (_slideAutoSaveTimers.has(key))
      clearTimeout(_slideAutoSaveTimers.get(key));
    _slideAutoSaveTimers.set(
      key,
      setTimeout(function () {
        _slideAutoSaveTimers.delete(key);
        apiFetch("/presentations/" + presentationId + "/slides/" + slide.id, {
          method: "PATCH",
          body: JSON.stringify({
            main: slide.main || "",
            translation: slide.translation || "",
            stage: slide.stage || "",
            group: slide.group || null,
          }),
        })
          .then(function () {
            showToast("Slide saved", "success");
          })
          .catch(function (err) {
            console.error("Failed to save slide", err);
            showToast("Failed to save slide", "error");
          });
      }, 800),
    );
  }

  function onSlidesContainerInput(event) {
    const wrapper = event.target.closest("[data-slide-id]");
    if (!wrapper) return;
    const slideId = wrapper.getAttribute("data-slide-id");
    var slide = state.slides.find((entry) => entry.id === slideId);
    var isPrepared = false;
    if (!slide) {
      slide = state.activePresentationSlides.find(
        (entry) => entry.id === slideId,
      );
      isPrepared = true;
    }
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
      bibleMeta.translation_reference_label =
        bibleMeta.translationReferenceLabel;
    }
    if (isPrepared && state.activePresentationId) {
      debounceSaveSlide(state.activePresentationId, slide);
    }
  }

  async function performContentSearch(query) {
    if (!query || query.length < 3) {
      state.contentSearchResults = [];
      state.contentSearchLoading = false;
      renderContentSearchResults();
      return;
    }
    state.contentSearchLoading = true;
    renderContentSearchResults();
    try {
      const encoded = encodeURIComponent(query);
      const results = await apiFetch(`/bible/search?query=${encoded}&limit=30`);
      state.contentSearchResults = Array.isArray(results) ? results : [];
    } catch (error) {
      console.error("Content search failed", error);
      state.contentSearchResults = [];
    } finally {
      state.contentSearchLoading = false;
      renderContentSearchResults();
    }
  }

  function renderContentSearchResults() {
    if (!els.globalSearchResults) return;
    if (state.contentSearchLoading) {
      els.globalSearchResults.dataset.visible = "true";
      els.globalSearchResults.innerHTML =
        "<div class='operator__search-group'><p class='operator__search-empty'>Searching\u2026</p></div>";
      return;
    }
    if (!state.contentSearchResults.length) {
      if (state.contentSearchQuery && state.contentSearchQuery.length >= 3) {
        els.globalSearchResults.dataset.visible = "true";
        els.globalSearchResults.innerHTML =
          "<div class='operator__search-group'><p class='operator__search-empty'>No results found.</p></div>";
      } else {
        els.globalSearchResults.dataset.visible = "false";
        els.globalSearchResults.innerHTML = "";
      }
      return;
    }
    var items = state.contentSearchResults
      .map(function (passage, idx) {
        var ref = passage.reference || {};
        var book = ref.book || ref.book_name || "";
        var bookCode = ref.book_code || ref.bookCode || "";
        var bookNumber =
          ref.book_number != null
            ? ref.book_number
            : ref.bookNumber != null
              ? ref.bookNumber
              : 0;
        var chapter = ref.chapter || 0;
        var verseStart =
          ref.verse_start != null ? ref.verse_start : ref.verseStart || 0;
        var verseEnd =
          ref.verse_end != null ? ref.verse_end : ref.verseEnd || verseStart;
        var refLabel = formatReference(book, chapter, verseStart, verseEnd);
        var translation = passage.translation || {};
        var translationCode = translation.code || "";
        var translationName = translation.name || translationCode;
        var text = passage.text || "";
        var snippet =
          text.length > 120 ? text.substring(0, 120) + "\u2026" : text;
        return (
          "<div class='operator__search-result'>" +
          "<button type='button'" +
          " data-idx='" +
          idx +
          "'" +
          " data-book='" +
          escapeHtml(book) +
          "'" +
          " data-book-code='" +
          escapeHtml(bookCode) +
          "'" +
          " data-book-number='" +
          bookNumber +
          "'" +
          " data-chapter='" +
          chapter +
          "'" +
          " data-verse-start='" +
          verseStart +
          "'" +
          " data-verse-end='" +
          verseEnd +
          "'" +
          " data-translation-code='" +
          escapeHtml(translationCode) +
          "'" +
          ">" +
          "<span class='operator__search-result-title'>" +
          escapeHtml(refLabel) +
          "</span>" +
          "<span class='operator__search-result-meta'>" +
          escapeHtml(translationName) +
          "</span>" +
          "<span class='operator__search-result-snippet'>" +
          escapeHtml(snippet) +
          "</span>" +
          "</button>" +
          "</div>"
        );
      })
      .join("");
    els.globalSearchResults.innerHTML =
      "<div class='operator__search-group'><h3>Bible Verses</h3>" +
      items +
      "</div>";
    els.globalSearchResults.dataset.visible = "true";
  }

  async function handleContentSearchResultClick(event) {
    var item = event.target.closest(".operator__search-result button");
    if (!item) return;
    var translationCode = item.getAttribute("data-translation-code") || "";
    var book = item.getAttribute("data-book") || "";
    var bookCode = item.getAttribute("data-book-code") || "";
    var bookNumber = Number(item.getAttribute("data-book-number") || "0") || 0;
    var chapter = Number(item.getAttribute("data-chapter") || "1") || 1;
    var verseStart = Number(item.getAttribute("data-verse-start") || "1") || 1;
    var verseEnd = Number(item.getAttribute("data-verse-end") || "1") || 1;

    // Switch main translation if needed
    if (
      translationCode &&
      translationCode !== state.preferences.mainTranslation
    ) {
      alignMainTranslation(translationCode);
      renderTranslationSelect(
        els.mainTranslation,
        state.preferences.mainTranslation,
      );
      renderTranslationSelect(
        els.secondaryTranslation,
        state.preferences.secondaryTranslation,
        true,
      );
      await loadBooks(false);
    }

    // Match book by code
    var bookEntry = state.books.find(function (bk) {
      if (bookCode && bk.code) {
        return bk.code.toLowerCase() === bookCode.toLowerCase();
      }
      if (bookNumber && bk.number) {
        return bk.number === bookNumber;
      }
      return bk.name === book;
    });
    if (bookEntry) {
      state.selectedBook = bookEntry.name;
      state.selectedBookCode = bookEntry.code || "";
      state.selectedBookNumber = bookEntry.number || 0;
      state.chapters = bookEntry.chapters || [];
    }
    state.selectedChapter = chapter;
    state.verseStart = verseStart;
    state.verseEnd = verseEnd;
    state.verseEndCustom = true;
    state.bookSelectionLocked = true;
    state.filteredBooks = state.books.filter(function (bk) {
      if (state.selectedBookCode && bk.code) {
        return bk.code === state.selectedBookCode;
      }
      if (state.selectedBookNumber && bk.number) {
        return bk.number === state.selectedBookNumber;
      }
      return bk.name === state.selectedBook;
    });

    // Clear search
    state.contentSearchQuery = "";
    state.contentSearchResults = [];
    if (els.globalSearchInput) {
      els.globalSearchInput.value = "";
    }
    if (els.globalSearchClear) {
      els.globalSearchClear.hidden = true;
    }
    renderContentSearchResults();

    // Update UI and load slides
    updateReferenceInputs();
    renderBookList();
    await loadSlides();
  }

  function initialiseEvents() {
    document.querySelectorAll('[data-role="view-toggle"]').forEach((button) => {
      const href = button.getAttribute("data-href");
      if (!href) return;
      button.addEventListener("click", () => {
        window.location.href = href;
      });
    });
    if (els.translationList) {
      els.translationList.addEventListener("click", async (event) => {
        const editControl = event.target.closest('[data-action="bible-edit"]');
        if (editControl && editControl.dataset.translationCode) {
          event.preventDefault();
          openBibleEdit(editControl.dataset.translationCode);
          return;
        }
        const button = event.target.closest("[data-translation-code]");
        if (!button) return;
        const code = button.getAttribute("data-translation-code");
        if (!code || code === state.preferences.mainTranslation) {
          return;
        }
        alignMainTranslation(code);
        renderTranslationList();
        try {
          await savePreferences();
        } catch (error) {
          console.warn("Failed to persist Bible preferences", error);
        }
        await loadBooks();
      });
    }
    if (els.bibleCount) {
      els.bibleCount.addEventListener("click", (event) => {
        event.preventDefault();
        if (els.bibleCount.disabled) return;
        openBibleModal();
      });
    }
    if (els.bibleModalList) {
      els.bibleModalList.addEventListener("click", async (event) => {
        const toggle = event.target.closest(
          '[data-action="bible-dashboard-toggle"]',
        );
        if (toggle && toggle.dataset.translationCode) {
          event.preventDefault();
          await toggleBibleDashboard(toggle.dataset.translationCode);
          return;
        }
        const item = event.target.closest('[data-role="bible-item"]');
        if (item && item.dataset.translationCode) {
          event.preventDefault();
          const code = item.dataset.translationCode;
          if (code && code !== state.preferences.mainTranslation) {
            alignMainTranslation(code);
            renderTranslationList();
            try {
              await savePreferences();
            } catch (error) {
              console.warn("Failed to persist Bible preferences", error);
            }
            await loadBooks();
          }
          closeBibleModal();
        }
        const editButton = event.target.closest('[data-action="bible-edit"]');
        if (editButton && editButton.dataset.translationCode) {
          event.preventDefault();
          openBibleEdit(editButton.dataset.translationCode);
        }
      });
    }
    if (els.bibleModalClose) {
      els.bibleModalClose.addEventListener("click", (event) => {
        event.preventDefault();
        closeBibleModal();
      });
    }
    if (els.bibleModal) {
      els.bibleModal.addEventListener("click", (event) => {
        if (event.target === els.bibleModal) {
          closeBibleModal();
        }
      });
    }
    if (els.bibleImport) {
      els.bibleImport.addEventListener("click", async (event) => {
        event.preventDefault();
        await refreshBibleTranslations();
      });
    }
    if (els.bibleEditModal) {
      els.bibleEditModal.addEventListener("click", (event) => {
        if (event.target === els.bibleEditModal) {
          closeBibleEdit();
        }
      });
    }
    if (els.bibleEditForm) {
      els.bibleEditForm.addEventListener("submit", handleBibleEditSubmit);
    }
    if (els.bibleEditCancel) {
      els.bibleEditCancel.addEventListener("click", (event) => {
        event.preventDefault();
        closeBibleEdit();
      });
    }
    if (els.bibleEditDelete) {
      els.bibleEditDelete.addEventListener("click", async (event) => {
        event.preventDefault();
        await handleBibleDelete();
      });
    }
    if (els.bibleEditDashboard) {
      els.bibleEditDashboard.addEventListener("change", (event) => {
        state.bibleEdit.showInDashboard = Boolean(event.target.checked);
      });
    }
    window.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        if (
          els.presentationEditModal &&
          els.presentationEditModal.dataset.open === "true"
        ) {
          closePresentationEditModal();
          return;
        }
        if (els.bibleEditModal && els.bibleEditModal.dataset.open === "true") {
          closeBibleEdit();
          return;
        }
        if (els.bibleModal && els.bibleModal.dataset.open === "true") {
          closeBibleModal();
        }
      }
    });
    if (els.mainTranslation) {
      els.mainTranslation.addEventListener("change", async (event) => {
        const code = event.target.value;
        if (code && code !== state.preferences.mainTranslation) {
          alignMainTranslation(code);
          renderTranslationList();
          try {
            await savePreferences();
          } catch (error) {
            console.warn("Failed to persist Bible preferences", error);
          }
          await loadBooks();
        }
      });
    }
    if (els.secondaryTranslation) {
      els.secondaryTranslation.addEventListener("change", (event) => {
        state.preferences.secondaryTranslation = event.target.value;
      });
    }
    var charLimitSaveTimer = null;
    if (els.charLimit) {
      els.charLimit.addEventListener("input", (event) => {
        const value = Number(event.target.value) || 0;
        state.preferences.characterLimit = Math.min(Math.max(value, 1), 4000);
        if (charLimitSaveTimer) clearTimeout(charLimitSaveTimer);
        charLimitSaveTimer = setTimeout(function () {
          charLimitSaveTimer = null;
          savePreferences();
        }, 500);
      });
    }
    if (els.savePreferences) {
      els.savePreferences.addEventListener("click", savePreferences);
    }
    if (els.globalSearchForm) {
      els.globalSearchForm.addEventListener("submit", function (e) {
        e.preventDefault();
      });
    }
    if (els.globalSearchInput) {
      els.globalSearchInput.addEventListener("input", function () {
        var query = els.globalSearchInput.value.trim();
        state.contentSearchQuery = query;
        if (els.globalSearchClear) {
          els.globalSearchClear.hidden = !query;
        }
        if (state.contentSearchDebounce) {
          clearTimeout(state.contentSearchDebounce);
        }
        if (query.length < 3) {
          state.contentSearchResults = [];
          state.contentSearchLoading = false;
          renderContentSearchResults();
          return;
        }
        state.contentSearchDebounce = setTimeout(function () {
          performContentSearch(query);
        }, 300);
      });
      els.globalSearchInput.addEventListener("keydown", function (event) {
        if (event.key === "Enter") {
          event.preventDefault();
          var query = els.globalSearchInput.value.trim();
          state.contentSearchQuery = query;
          if (state.contentSearchDebounce) {
            clearTimeout(state.contentSearchDebounce);
            state.contentSearchDebounce = null;
          }
          if (query.length >= 3) {
            performContentSearch(query);
          }
        }
        if (event.key === "Escape") {
          event.preventDefault();
          els.globalSearchInput.value = "";
          state.contentSearchQuery = "";
          state.contentSearchResults = [];
          if (state.contentSearchDebounce) {
            clearTimeout(state.contentSearchDebounce);
            state.contentSearchDebounce = null;
          }
          if (els.globalSearchClear) {
            els.globalSearchClear.hidden = true;
          }
          renderContentSearchResults();
        }
      });
    }
    if (els.globalSearchClear) {
      els.globalSearchClear.addEventListener("click", function () {
        if (els.globalSearchInput) {
          els.globalSearchInput.value = "";
          els.globalSearchInput.focus();
        }
        state.contentSearchQuery = "";
        state.contentSearchResults = [];
        if (state.contentSearchDebounce) {
          clearTimeout(state.contentSearchDebounce);
          state.contentSearchDebounce = null;
        }
        els.globalSearchClear.hidden = true;
        renderContentSearchResults();
      });
    }
    if (els.globalSearchResults) {
      els.globalSearchResults.addEventListener(
        "click",
        handleContentSearchResultClick,
      );
    }
    document.addEventListener("click", function (event) {
      if (
        els.globalSearchResults &&
        els.globalSearchResults.dataset.visible === "true" &&
        els.globalSearchForm &&
        !els.globalSearchForm.contains(event.target) &&
        !els.globalSearchResults.contains(event.target)
      ) {
        if (state.contentSearchDebounce) {
          clearTimeout(state.contentSearchDebounce);
          state.contentSearchDebounce = null;
        }
        els.globalSearchResults.dataset.visible = "false";
        els.globalSearchResults.innerHTML = "";
        state.contentSearchResults = [];
        state.contentSearchQuery = "";
        if (els.globalSearchInput) {
          els.globalSearchInput.value = "";
        }
        if (els.globalSearchClear) {
          els.globalSearchClear.hidden = true;
        }
      }
    });
    if (els.bookFilter) {
      els.bookFilter.addEventListener("input", (event) => {
        filterBooks(event.target.value);
      });
    }
    if (els.bookList) {
      els.bookList.addEventListener("click", (event) => {
        const button = event.target.closest("[data-book]");
        if (!button) return;
        const nextBook = button.getAttribute("data-book");
        if (!nextBook) return;
        const nextCode = button.getAttribute("data-book-code") || "";
        const nextNumber =
          Number(button.getAttribute("data-book-number") || "0") || 0;
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
      els.loadedPassages.addEventListener("click", async (event) => {
        const item = event.target.closest("[data-loaded-key]");
        if (!item) return;
        const key = item.getAttribute("data-loaded-key");
        if (!key) return;
        const entry = state.loadedPassages.find(
          (candidate) => candidate.key === key,
        );
        if (!entry) return;
        try {
          await applyLoadedPassage(entry);
        } catch (error) {
          console.error("Failed to apply saved passage", error);
          showToast("Failed to load saved passage", "error");
        }
      });
    }
    if (els.chapterInput) {
      els.chapterInput.addEventListener("input", (event) => {
        const value = Number(event.target.value) || 1;
        state.selectedChapter = value;
        applyChapterDefaults();
      });
    }
    if (els.verseStartInput) {
      els.verseStartInput.addEventListener("input", (event) => {
        const raw =
          typeof event.target.value === "string"
            ? event.target.value.trim()
            : "";
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
      els.verseEndInput.addEventListener("input", (event) => {
        const raw =
          typeof event.target.value === "string"
            ? event.target.value.trim()
            : "";
        const verseCount = getCurrentVerseCount();
        if (!raw) {
          state.verseEndCustom = false;
          state.verseEnd = verseCount;
        } else {
          const candidate = Number(raw);
          const value = Number.isFinite(candidate)
            ? candidate
            : state.verseStart;
          state.verseEndCustom = true;
          state.verseEnd = Math.min(
            Math.max(value, state.verseStart),
            verseCount,
          );
        }
        updateReferenceInputs();
      });
    }
    if (els.loadButton) {
      els.loadButton.addEventListener("click", loadSlides);
    }
    if (els.modeToggleContainer) {
      els.modeToggleContainer.addEventListener("click", (event) => {
        const btn = event.target.closest("[data-mode]");
        if (!btn) return;
        const newMode = btn.dataset.mode;
        state.editMode = newMode === "edit";
        updateMode();
        if (state.bibleTab === "prepared") {
          renderPresentationSlides();
        } else {
          renderSlides();
        }
      });
    }
    if (els.slidesContainer) {
      els.slidesContainer.addEventListener("click", onSlidesContainerClick);
      els.slidesContainer.addEventListener("input", onSlidesContainerInput);
      els.slidesContainer.addEventListener("dragstart", onPreparedDragStart);
      els.slidesContainer.addEventListener("dragover", onPreparedDragOver);
      els.slidesContainer.addEventListener("drop", onPreparedDrop);
      els.slidesContainer.addEventListener("dragend", onPreparedDragEnd);
    }
    if (els.selectAllButton) {
      els.selectAllButton.addEventListener("click", toggleSelectAllSlides);
    }
    if (els.addToPresentation) {
      els.addToPresentation.addEventListener(
        "click",
        appendSlidesToPresentation,
      );
    }
    if (els.presentationsList) {
      els.presentationsList.addEventListener("click", async (event) => {
        const editBtn = event.target.closest('[data-role="presentation-edit"]');
        if (editBtn) {
          const presentationId = editBtn.getAttribute("data-presentation-id");
          if (!presentationId) return;
          const currentName =
            editBtn.getAttribute("data-presentation-name") || "";
          openPresentationEditModal(presentationId, currentName);
          return;
        }
        const card = event.target.closest("[data-presentation-id]");
        if (!card) return;
        const id = card.getAttribute("data-presentation-id");
        if (!id) return;
        state.activePresentationId = id;
        await loadPresentationSlides(id);
        renderPresentations();
      });
    }
    if (els.addEmptySlide) {
      els.addEmptySlide.addEventListener("click", addEmptySlide);
    }
    if (els.presentationCreate) {
      els.presentationCreate.addEventListener("click", async () => {
        const name = window.prompt("New presentation name");
        if (!name || !name.trim()) return;
        try {
          await apiFetch("/bible/presentations", {
            method: "POST",
            body: JSON.stringify({ name: name.trim() }),
          });
          showToast("Presentation created", "success");
          await loadPresentations();
        } catch (error) {
          showToast(error.message || "Failed to create presentation", "error");
        }
      });
    }
    if (els.bibleTabNav) {
      els.bibleTabNav.addEventListener("click", (event) => {
        const button = event.target.closest('[data-role="bible-tab"]');
        if (!button) return;
        const tab = button.dataset.tab;
        if (tab && tab !== state.bibleTab) {
          setBibleTab(tab);
        }
      });
    }
    if (els.clearButton) {
      els.clearButton.addEventListener("click", clearBroadcast);
    }
    if (els.presentationEditModal) {
      els.presentationEditModal.addEventListener("click", (event) => {
        if (event.target === els.presentationEditModal) {
          closePresentationEditModal();
        }
      });
    }
    if (els.presentationEditForm) {
      els.presentationEditForm.addEventListener(
        "submit",
        handlePresentationEditSubmit,
      );
    }
    if (els.presentationEditCancel) {
      els.presentationEditCancel.addEventListener("click", (event) => {
        event.preventDefault();
        closePresentationEditModal();
      });
    }
    if (els.presentationEditDelete) {
      els.presentationEditDelete.addEventListener("click", async (event) => {
        event.preventDefault();
        await handlePresentationEditDelete();
      });
    }
  }

  async function initialise() {
    renderPreferences();
    renderLoadedPassages();
    initialiseEvents();
    await fetchPreferences();
    renderLoadedPassages();
    await loadBooks();
    updateReferenceInputs();
    updateMode();
    updateTextareaLines();
    await loadPresentations();
    connectLiveSocket();
    ensureActivePoller();
  }

  // Listen for mode changes from parent operator page (when embedded as iframe)
  window.addEventListener("message", function (event) {
    if (event.origin !== window.location.origin) return;
    if (!event.data) return;

    if (event.data.type === "presenter-mode-change") {
      var newMode = event.data.mode;
      state.editMode = newMode === "edit";
      updateMode();
      if (state.bibleTab === "prepared") {
        renderPresentationSlides();
      } else {
        renderSlides();
      }
      return;
    }

    if (event.data.type === "presenter-bible-load-passage") {
      var passage = event.data.passage;
      if (!passage) return;

      (async function () {
        // Switch translation if needed
        if (
          passage.translationCode &&
          passage.translationCode !== state.preferences.mainTranslation
        ) {
          alignMainTranslation(passage.translationCode);
          renderTranslationSelect(
            els.mainTranslation,
            state.preferences.mainTranslation,
          );
          renderTranslationSelect(
            els.secondaryTranslation,
            state.preferences.secondaryTranslation,
            true,
          );
          await loadBooks(false);
        }

        // Match book by code/number/name
        var bookEntry = state.books.find(function (bk) {
          if (passage.bookCode && bk.code) {
            return bk.code.toLowerCase() === passage.bookCode.toLowerCase();
          }
          if (passage.bookNumber && bk.number) {
            return bk.number === passage.bookNumber;
          }
          return bk.name === passage.book;
        });
        if (bookEntry) {
          state.selectedBook = bookEntry.name;
          state.selectedBookCode = bookEntry.code || "";
          state.selectedBookNumber = bookEntry.number || 0;
          state.chapters = bookEntry.chapters || [];
        }
        state.selectedChapter = passage.chapter;
        state.verseStart = passage.verseStart;
        state.verseEnd = passage.verseEnd;
        state.verseEndCustom = true;
        state.bookSelectionLocked = true;
        state.filteredBooks = state.books.filter(function (bk) {
          if (state.selectedBookCode && bk.code) {
            return bk.code === state.selectedBookCode;
          }
          if (state.selectedBookNumber && bk.number) {
            return bk.number === state.selectedBookNumber;
          }
          return bk.name === state.selectedBook;
        });

        // Ensure live tab is active
        if (state.bibleTab !== "live") {
          setBibleTab("live");
        }

        updateReferenceInputs();
        renderBookList();
        await loadSlides();
      })();
    }
  });

  window.__presenterBibleState = state;
  initialise();
})();
