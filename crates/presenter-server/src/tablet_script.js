"use strict";

(function () {
  const presentationsData = __PRESENTATIONS__;

  const state = {
    presentations: Array.isArray(presentationsData) ? presentationsData : [],
    currentPresentationId: null,
    slidesCache: new Map(),
    toastTimer: null,
    liveSocket: null,
    liveReconnectTimer: null,
    activeBibleBroadcast: null,
    sidebarOpen: true,
    // Touch tracking for tap vs scroll detection
    touchStartX: 0,
    touchStartY: 0,
    touchMoved: false,
  };

  const els = {
    presentationList: document.querySelector('[data-role="presentation-list"]'),
    slides: document.querySelector('[data-role="slides"]'),
    contextTitle: document.querySelector('[data-role="context-title"]'),
    toast: document.querySelector('[data-role="toast"]'),
    scaleSlider: document.querySelector('[data-role="scale-slider"]'),
    scaleValue: document.querySelector('[data-role="scale-value"]'),
    sidebar: document.querySelector(".tablet-sidebar"),
    sidebarToggle: document.querySelector('[data-role="sidebar-toggle"]'),
  };

  function escapeHtml(value) {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }

  function formatMultiline(text) {
    if (!text) return "";
    return escapeHtml(text).replace(/\n/g, "<br />");
  }

  function showToast(message, variant) {
    if (!els.toast) return;
    els.toast.textContent = message;
    els.toast.dataset.visible = "true";
    els.toast.dataset.variant = variant || "info";
    clearTimeout(state.toastTimer);
    state.toastTimer = setTimeout(() => {
      els.toast.dataset.visible = "false";
    }, 2500);
  }

  function toggleSidebar(open) {
    state.sidebarOpen = open;
    if (els.sidebar) {
      els.sidebar.classList.toggle("is-collapsed", !open);
    }
  }

  function apiFetch(path, options) {
    var url = path.startsWith("http") ? path : window.location.origin + path;
    var headers = Object.assign(
      { "Content-Type": "application/json", Accept: "application/json" },
      options && options.headers ? options.headers : {},
    );
    return fetch(
      url,
      Object.assign({ method: "GET", headers: headers }, options || {}),
    ).then(async function (response) {
      if (!response.ok) {
        var text = await response.text();
        throw new Error(
          text || "Request failed with status " + response.status,
        );
      }
      var contentType = response.headers.get("content-type") || "";
      if (contentType.includes("application/json")) {
        return response.json();
      }
      return null;
    });
  }

  function renderPresentations() {
    if (!els.presentationList) return;
    if (!state.presentations.length) {
      els.presentationList.innerHTML =
        '<p class="tablet-slides__empty">No Bible presentations available.</p>';
      return;
    }
    var html = state.presentations
      .map(function (presentation) {
        var active =
          presentation.id === state.currentPresentationId
            ? ' data-active="true"'
            : "";
        return (
          '<div class="tablet-list-item">' +
          '<button type="button" class="tablet-button" data-role="presentation-button" data-presentation-id="' +
          presentation.id +
          '"' +
          active +
          ">" +
          '<span class="tablet-button__label">' +
          escapeHtml(presentation.name) +
          "</span>" +
          '<span class="tablet-button__meta">' +
          presentation.slideCount +
          "</span>" +
          "</button>" +
          "</div>"
        );
      })
      .join("");
    els.presentationList.innerHTML = html;
  }

  function renderSlides(presentationId) {
    if (!els.slides) return;
    var slides = state.slidesCache.get(presentationId) || [];
    if (!slides.length) {
      els.slides.innerHTML =
        '<p class="tablet-slides__empty">No slides in this presentation.</p>';
      return;
    }
    var lastReference = null;
    var groupIndex = 0;
    els.slides.innerHTML = slides
      .map(function (slide) {
        var isActive = isSlideActive(slide) ? " is-active" : "";

        // Track reference groups for alternating shades and separators
        var isNewGroup = slide.mainReference !== lastReference;
        if (isNewGroup) {
          if (lastReference !== null) {
            groupIndex++;
          }
          lastReference = slide.mainReference;
        }
        var shadeClass =
          groupIndex % 2 === 0 ? " tablet-slide--light" : " tablet-slide--dark";
        var separatorClass =
          isNewGroup && groupIndex > 0 ? " tablet-slide--group-start" : "";

        // Build reference header (top, prominent)
        var refHeader = slide.mainReference
          ? '<header class="tablet-slide__ref">' +
            escapeHtml(slide.mainReference) +
            "</header>"
          : "";

        var mainHtml = slide.main
          ? '<p class="tablet-slide__main">' +
            formatMultiline(slide.main) +
            "</p>"
          : "";
        var translationHtml = slide.translation
          ? '<p class="tablet-slide__translation">' +
            formatMultiline(slide.translation) +
            "</p>"
          : "";

        // Build footer with group badge only
        var footerHtml = slide.group
          ? '<footer class="tablet-slide__footer"><span class="tablet-slide__group">' +
            escapeHtml(slide.group) +
            "</span></footer>"
          : "";

        return (
          '<article class="tablet-slide' +
          isActive +
          shadeClass +
          separatorClass +
          '" data-role="tablet-slide" data-slide-id="' +
          slide.id +
          '">' +
          refHeader +
          '<section class="tablet-slide__body">' +
          mainHtml +
          translationHtml +
          "</section>" +
          footerHtml +
          "</article>"
        );
      })
      .join("");
  }

  function isSlideActive(slide) {
    if (!state.activeBibleBroadcast || !slide.metadata || !slide.metadata.bible)
      return false;
    var broadcast = state.activeBibleBroadcast;
    var ref = broadcast.passage && broadcast.passage.reference;
    var trans = broadcast.passage && broadcast.passage.translation;
    if (!ref || !trans) return false;
    var meta = slide.metadata.bible;
    var translationCode = meta.translationCode || meta.translation_code;
    var chapter = meta.chapter;
    var verses = Array.isArray(meta.verses) ? meta.verses : [];
    if (!verses.length) return false;
    var verseStart = verses[0].start;
    var verseEnd = verses[verses.length - 1].end;
    var broadcastChapter = ref.chapter;
    var broadcastVerseStart = ref.verseStart || ref.verse_start;
    var broadcastVerseEnd = ref.verseEnd || ref.verse_end;
    var broadcastTranslation = trans.code;
    return (
      broadcastTranslation === translationCode &&
      broadcastChapter === chapter &&
      broadcastVerseStart === verseStart &&
      broadcastVerseEnd === verseEnd
    );
  }

  async function loadPresentation(presentationId) {
    if (!presentationId) return;
    if (state.slidesCache.has(presentationId)) {
      renderSlides(presentationId);
      return;
    }
    try {
      var detail = await apiFetch("/bible/presentations/" + presentationId, {
        method: "GET",
      });
      var slides = detail.slides || [];
      state.slidesCache.set(presentationId, slides);
      renderSlides(presentationId);
    } catch (error) {
      console.error("Failed to load presentation", error);
      showToast("Failed to load presentation", "error");
    }
  }

  async function triggerSlide(slide) {
    if (!slide || !slide.metadata || !slide.metadata.bible) {
      showToast("Slide has no Bible metadata", "error");
      return;
    }
    var bibleMeta = slide.metadata.bible;
    var verses = Array.isArray(bibleMeta.verses) ? bibleMeta.verses : [];
    if (!verses.length) {
      showToast("Verse metadata missing", "error");
      return;
    }
    var translationCode =
      bibleMeta.translationCode || bibleMeta.translation_code;
    if (!translationCode) {
      showToast("Translation metadata missing", "error");
      return;
    }
    var verseStart = verses[0].start;
    var verseEnd = verses[verses.length - 1].end;
    var book = bibleMeta.book;
    var bookCode = bibleMeta.bookCode || bibleMeta.book_code || null;
    var bookNumber = bibleMeta.bookNumber || bibleMeta.book_number || null;
    var chapter = bibleMeta.chapter;

    var card = els.slides
      ? els.slides.querySelector('[data-slide-id="' + slide.id + '"]')
      : null;
    if (card) card.classList.add("is-loading");

    try {
      var payload = {
        translation: translationCode,
        book: book,
        chapter: chapter,
        verseStart: verseStart,
        verseEnd: verseEnd,
      };
      if (bookCode) payload.bookCode = bookCode;
      if (bookNumber) payload.bookNumber = bookNumber;

      var response = await apiFetch("/bible/trigger", {
        method: "POST",
        body: JSON.stringify(payload),
      });
      state.activeBibleBroadcast = response;
      renderSlides(state.currentPresentationId);
      showToast("Slide triggered", "success");
    } catch (error) {
      console.error("Failed to trigger slide", error);
      showToast(error.message || "Failed to trigger slide", "error");
    } finally {
      if (card) card.classList.remove("is-loading");
    }
  }

  function handlePresentationClick(event) {
    var button = event.target.closest('[data-role="presentation-button"]');
    if (!button) return;
    var id = button.dataset.presentationId;
    if (!id || id === state.currentPresentationId) return;
    state.currentPresentationId = id;
    var presentation = state.presentations.find(function (p) {
      return p.id === id;
    });
    if (els.contextTitle && presentation) {
      els.contextTitle.textContent = presentation.name;
    }
    renderPresentations();
    loadPresentation(id);
    toggleSidebar(false);
  }

  function handleSlideTap(event) {
    // Skip if this click was synthesized from a touch event (already handled)
    if (event.sourceCapabilities && event.sourceCapabilities.firesTouchEvents) {
      return;
    }
    var card = event.target.closest("[data-slide-id]");
    if (!card || !state.currentPresentationId) return;
    var slideId = card.dataset.slideId;
    var slides = state.slidesCache.get(state.currentPresentationId) || [];
    var slide = slides.find(function (entry) {
      return entry.id === slideId;
    });
    if (!slide) return;
    triggerSlide(slide);
  }

  function connectLiveSocket() {
    if (state.liveSocket) {
      try {
        state.liveSocket.close();
      } catch (error) {
        console.warn("failed to close tablet socket", error);
      }
    }
    var protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    var socket = new WebSocket(
      protocol + "//" + window.location.host + "/live/ws",
    );
    state.liveSocket = socket;
    socket.addEventListener("open", function () {
      if (state.liveReconnectTimer) {
        clearTimeout(state.liveReconnectTimer);
        state.liveReconnectTimer = null;
      }
    });
    socket.addEventListener("message", function (event) {
      try {
        var payload = JSON.parse(event.data);
        if (payload.type === "bible" || payload.type === "Bible") {
          var broadcast = payload.broadcast || payload.data || null;
          state.activeBibleBroadcast = broadcast;
          if (state.currentPresentationId) {
            renderSlides(state.currentPresentationId);
          }
        }
      } catch (error) {
        console.error("tablet live payload parsing failed", error);
      }
    });
    socket.addEventListener("close", function () {
      if (!state.liveReconnectTimer) {
        state.liveReconnectTimer = setTimeout(connectLiveSocket, 2000);
      }
    });
    socket.addEventListener("error", function (error) {
      console.error("tablet live socket error", error);
      try {
        socket.close();
      } catch (err) {
        console.warn("failed closing socket after error", err);
      }
    });
  }

  async function fetchActiveBroadcast() {
    try {
      var active = await apiFetch("/bible/active", { method: "GET" });
      state.activeBibleBroadcast = active;
    } catch (error) {
      console.warn("Failed to fetch active broadcast", error);
    }
  }

  async function refreshPresentations() {
    try {
      var fresh = await apiFetch("/bible/presentations", { method: "GET" });
      if (!Array.isArray(fresh)) return;
      // Detect changes: new presentations or slide count updates
      var changed = fresh.length !== state.presentations.length;
      if (!changed) {
        for (var i = 0; i < fresh.length; i++) {
          var oldP = state.presentations.find(function (p) {
            return p.id === fresh[i].id;
          });
          if (!oldP || oldP.slideCount !== fresh[i].slideCount) {
            changed = true;
            break;
          }
        }
      }
      if (!changed) return;
      // Invalidate slides cache for presentations whose counts changed
      fresh.forEach(function (p) {
        var oldP = state.presentations.find(function (o) {
          return o.id === p.id;
        });
        if (!oldP || oldP.slideCount !== p.slideCount) {
          state.slidesCache.delete(p.id);
        }
      });
      state.presentations = fresh;
      renderPresentations();
      // If current presentation was removed, clear selection
      if (
        state.currentPresentationId &&
        !fresh.find(function (p) {
          return p.id === state.currentPresentationId;
        })
      ) {
        state.currentPresentationId = null;
        if (els.contextTitle) {
          els.contextTitle.textContent = "Select a presentation";
        }
        if (els.slides) {
          els.slides.innerHTML =
            '<p class="tablet-slides__empty">Select a presentation to view slides.</p>';
        }
      }
      // If current presentation slides were invalidated, reload them
      if (
        state.currentPresentationId &&
        !state.slidesCache.has(state.currentPresentationId)
      ) {
        loadPresentation(state.currentPresentationId);
      }
    } catch (error) {
      console.warn("Failed to refresh presentations", error);
    }
  }

  function applyScale(percent) {
    var scale = percent / 100;
    document.body.style.setProperty("--tablet-scale", scale);
    if (els.scaleValue) {
      els.scaleValue.textContent = percent + "%";
    }
    if (els.scaleSlider) {
      els.scaleSlider.value = percent;
    }
    try {
      localStorage.setItem("tablet-scale", String(percent));
    } catch (e) {
      // localStorage unavailable
    }
  }

  function loadSavedScale() {
    var saved = 100;
    try {
      var raw = localStorage.getItem("tablet-scale");
      if (raw) {
        var parsed = parseInt(raw, 10);
        if (parsed >= 50 && parsed <= 200) {
          saved = parsed;
        }
      }
    } catch (e) {
      // localStorage unavailable
    }
    applyScale(saved);
  }

  // Threshold in pixels - if touch moves more than this, it's a scroll not a tap
  var TAP_THRESHOLD = 10;

  function handleSlideTouchStart(event) {
    if (event.touches.length !== 1) return;
    var touch = event.touches[0];
    state.touchStartX = touch.clientX;
    state.touchStartY = touch.clientY;
    state.touchMoved = false;
  }

  function handleSlideTouchMove(event) {
    if (state.touchMoved) return;
    if (event.touches.length !== 1) {
      state.touchMoved = true;
      return;
    }
    var touch = event.touches[0];
    var deltaX = Math.abs(touch.clientX - state.touchStartX);
    var deltaY = Math.abs(touch.clientY - state.touchStartY);
    if (deltaX > TAP_THRESHOLD || deltaY > TAP_THRESHOLD) {
      state.touchMoved = true;
    }
  }

  function handleSlideTouchEnd(event) {
    if (state.touchMoved) return;
    // Find the element at touch end position
    var touch = event.changedTouches[0];
    var target = document.elementFromPoint(touch.clientX, touch.clientY);
    if (!target) return;
    var card = target.closest("[data-slide-id]");
    if (!card || !state.currentPresentationId) return;
    // Prevent ghost click
    event.preventDefault();
    var slideId = card.dataset.slideId;
    var slides = state.slidesCache.get(state.currentPresentationId) || [];
    var slide = slides.find(function (entry) {
      return entry.id === slideId;
    });
    if (!slide) return;
    triggerSlide(slide);
  }

  function bindEvents() {
    if (els.presentationList) {
      els.presentationList.addEventListener("click", handlePresentationClick);
    }
    if (els.slides) {
      // Touch events for tap detection (mobile)
      els.slides.addEventListener("touchstart", handleSlideTouchStart, {
        passive: true,
      });
      els.slides.addEventListener("touchmove", handleSlideTouchMove, {
        passive: true,
      });
      els.slides.addEventListener("touchend", handleSlideTouchEnd);
      // Click event fallback for non-touch devices (mouse)
      els.slides.addEventListener("click", handleSlideTap);
    }
    if (els.scaleSlider) {
      els.scaleSlider.addEventListener("input", function () {
        applyScale(parseInt(els.scaleSlider.value, 10));
      });
    }
    if (els.sidebarToggle) {
      els.sidebarToggle.addEventListener("click", function () {
        toggleSidebar(true);
      });
    }
    document.addEventListener("visibilitychange", function () {
      if (!document.hidden) {
        refreshPresentations();
      }
    });
  }

  async function initialise() {
    loadSavedScale();
    bindEvents();
    renderPresentations();

    await fetchActiveBroadcast();

    if (state.presentations.length > 0) {
      state.currentPresentationId = state.presentations[0].id;
      if (els.contextTitle) {
        els.contextTitle.textContent = state.presentations[0].name;
      }
      renderPresentations();
      await loadPresentation(state.currentPresentationId);
    }

    connectLiveSocket();
    setInterval(refreshPresentations, 10000);
  }

  initialise();
  window.__presenterTabletState = state;
  window.__presenterTabletReady = true;
})();
