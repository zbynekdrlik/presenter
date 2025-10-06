'use strict';

(function () {
  if (window.self !== window.top) {
    document.body.dataset.embedded = 'true';
  }
  const API_ROOT = '/integrations/resolume/hosts';
  const initialHosts = __RESOLUME_HOSTS__;
  const initialOscConfig = __OSC_CONFIG__ || null;
  const initialOscStatus = __OSC_STATUS__ || null;
  const initialAbleSetConfig = __ABLESET_CONFIG__ || null;
  const initialAbleSetStatus = __ABLESET_STATUS__ || null;
  const initialFeatures = __FEATURE_FLAGS__ || null;
  const state = {
    hosts: Array.isArray(initialHosts) ? initialHosts : [],
    editingId: null,
    toastTimer: null,
    osc: {
      config: normalizeOscConfig(initialOscConfig),
      status: normalizeOscStatus(initialOscStatus),
      submitting: false,
    },
    ableset: {
      config: normalizeAbleSetConfig(initialAbleSetConfig),
      status: normalizeAbleSetStatus(initialAbleSetStatus),
      submitting: false,
    },
    features: {
      config: normalizeFeatureFlags(initialFeatures),
      submitting: false,
    },
  };
  const STATUS_REFRESH_MS = 5000;

  const els = {
    form: document.querySelector('[data-role="host-form"]'),
    id: document.querySelector('[data-role="host-id"]'),
    label: document.querySelector('[data-role="host-label"]'),
    host: document.querySelector('[data-role="host-host"]'),
    port: document.querySelector('[data-role="host-port"]'),
    enabled: document.querySelector('[data-role="host-enabled"]'),
    submit: document.querySelector('[data-role="host-submit"]'),
    reset: document.querySelector('[data-role="host-reset"]'),
    formStatus: document.querySelector('[data-role="form-status"]'),
    formTitle: document.querySelector('[data-role="form-title"]'),
    formSubtitle: document.querySelector('[data-role="form-subtitle"]'),
    list: document.querySelector('[data-role="resolume-host-list"]'),
    toast: document.querySelector('[data-role="toast"]'),
    hostCount: document.querySelector('[data-role="host-count"]'),
    emptyState: document.querySelector('[data-role="host-empty"]'),
    oscForm: document.querySelector('[data-role="osc-form"]'),
    oscEnabled: document.querySelector('[data-role="osc-enabled"]'),
    oscPort: document.querySelector('[data-role="osc-port"]'),
    oscAddress: document.querySelector('[data-role="osc-address"]'),
    oscMode: document.querySelector('[data-role="osc-mode"]'),
    oscSubmit: document.querySelector('[data-role="osc-submit"]'),
    oscStatusIndicator: document.querySelector('[data-role="osc-status-indicator"]'),
    oscStatusHostPort: document.querySelector('[data-role="osc-status-host-port"]'),
    oscStatusLastMessage: document.querySelector('[data-role="osc-status-last-message"]'),
    oscStatusLastNote: document.querySelector('[data-role="osc-status-last-note"]'),
    oscStatusError: document.querySelector('[data-role="osc-status-error"]'),
    ablesetForm: document.querySelector('[data-role="ableset-form"]'),
    ablesetEnabled: document.querySelector('[data-role="ableset-enabled"]'),
    ablesetHost: document.querySelector('[data-role="ableset-host"]'),
    ablesetHttpPort: document.querySelector('[data-role="ableset-http-port"]'),
    ablesetOscPort: document.querySelector('[data-role="ableset-osc-port"]'),
    ablesetLibrary: document.querySelector('[data-role="ableset-library"]'),
    ablesetPrefix: document.querySelector('[data-role="ableset-prefix"]'),
    ablesetSubmit: document.querySelector('[data-role="ableset-submit"]'),
    ablesetFormStatus: document.querySelector('[data-role="ableset-form-status"]'),
    ablesetStatusIndicator: document.querySelector('[data-role="ableset-status-indicator"]'),
    ablesetStatusSong: document.querySelector('[data-role="ableset-status-song"]'),
    ablesetStatusPrefix: document.querySelector('[data-role="ableset-status-prefix"]'),
    ablesetStatusUpdated: document.querySelector('[data-role="ableset-status-updated"]'),
    ablesetStatusError: document.querySelector('[data-role="ableset-status-error"]'),
    ablesetRefresh: document.querySelector('[data-role="ableset-refresh"]'),
    featureForm: document.querySelector('[data-role="feature-companion-form"]'),
    featureToggle: document.querySelector('[data-role="feature-companion-toggle"]'),
    featurePort: document.querySelector('[data-role="feature-companion-port"]'),
    featureSubmit: document.querySelector('[data-role="feature-submit"]'),
    featureStatus: document.querySelector('[data-role="feature-status"]'),
  };

  const dateFormatter = new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  });

  function normalizeOscConfig(input) {
    const fallback = {
      enabled: true,
      listenPort: 9000,
      addressPattern: '/note',
      velocityMode: 'zero_based',
    };
    if (!input || typeof input !== 'object') {
      return { ...fallback };
    }
    return {
      enabled: Boolean(input.enabled),
      listenPort: Number.isFinite(Number(input.listenPort)) ? Number(input.listenPort) : fallback.listenPort,
      addressPattern: (input.addressPattern || fallback.addressPattern).toString(),
      velocityMode: (input.velocityMode || fallback.velocityMode).toString(),
    };
  }

  function normalizeOscStatus(input) {
    if (!input || typeof input !== 'object') {
      return {
        enabled: false,
        listening: false,
        listenPort: 9000,
        addressPattern: '/note',
        velocityMode: 'zero_based',
        lastMessageAt: null,
        lastNote: null,
        lastVelocity: null,
        lastError: null,
      };
    }
    return {
      enabled: Boolean(input.enabled),
      listening: Boolean(input.listening),
      listenPort: Number.isFinite(Number(input.listenPort)) ? Number(input.listenPort) : 9000,
      hostPort: Number.isFinite(Number(input.hostPort ?? input.host_port)) ? Number(input.hostPort ?? input.host_port) : null,
      addressPattern: (input.addressPattern || '/note').toString(),
      velocityMode: (input.velocityMode || 'zero_based').toString(),
      lastMessageAt: input.lastMessageAt || input.last_message_at || null,
      lastNote: typeof input.lastNote === 'number' ? input.lastNote : input.last_note ?? null,
      lastVelocity: typeof input.lastVelocity === 'number' ? input.lastVelocity : input.last_velocity ?? null,
      lastError: input.lastError || input.last_error || null,
    };
  }
  function normalizeAbleSetConfig(input) {
    const fallback = {
      enabled: true,
      host: 'fohabl.lan',
      httpPort: 80,
      oscPort: 39051,
      libraryName: 'NEW LEVEL',
      songPrefixLength: 3,
    };
    if (!input || typeof input !== 'object') {
      return { ...fallback };
    }
    return {
      enabled: Boolean(input.enabled),
      host: (input.host || fallback.host).toString(),
      httpPort: Number.isFinite(Number(input.httpPort ?? input.http_port)) ? Number(input.httpPort ?? input.http_port) : fallback.httpPort,
      oscPort: Number.isFinite(Number(input.oscPort ?? input.osc_port)) ? Number(input.oscPort ?? input.osc_port) : fallback.oscPort,
      libraryName: (input.libraryName || fallback.libraryName).toString(),
      songPrefixLength: Number.isFinite(Number(input.songPrefixLength ?? input.song_prefix_length)) ? Number(input.songPrefixLength ?? input.song_prefix_length) : fallback.songPrefixLength,
    };
  }

  function normalizeAbleSetStatus(input) {
    if (!input || typeof input !== 'object') {
      return {
        enabled: false,
        tracking: false,
        host: 'fohabl.lan',
        httpPort: 80,
        oscPort: 39051,
        libraryName: 'NEW LEVEL',
        songPrefixLength: 3,
        lastSong: null,
        lastError: null,
      };
    }
    const lastSong = input.lastSong || input.last_song || null;
    const normalisedSong = lastSong && typeof lastSong === 'object' ? {
      name: (lastSong.name || '').toString(),
      prefix: (lastSong.prefix || '').toString(),
      index: Number.isFinite(Number(lastSong.index ?? lastSong.index)) ? Number(lastSong.index ?? lastSong.index) : null,
      lastSeenAt: lastSong.lastSeenAt || lastSong.last_seen_at || null,
    } : null;
    return {
      enabled: Boolean(input.enabled),
      tracking: Boolean(input.tracking),
      host: (input.host || 'fohabl.lan').toString(),
      httpPort: Number.isFinite(Number(input.httpPort ?? input.http_port)) ? Number(input.httpPort ?? input.http_port) : 80,
      oscPort: Number.isFinite(Number(input.oscPort ?? input.osc_port)) ? Number(input.oscPort ?? input.osc_port) : 39051,
      libraryName: (input.libraryName || 'NEW LEVEL').toString(),
      songPrefixLength: Number.isFinite(Number(input.songPrefixLength ?? input.song_prefix_length)) ? Number(input.songPrefixLength ?? input.song_prefix_length) : 3,
      lastSong: normalisedSong,
      lastError: input.lastError || input.last_error || null,
    };
  }

  function normalizeFeatureFlags(input) {
    const fallback = {
      companionEnabled: false,
      companionPort: 18175,
    };
    if (!input || typeof input !== 'object') {
      return { ...fallback };
    }
    const enabled = Boolean(
      input.companion_enabled ?? input.companionEnabled ?? input.enabled ?? false
    );
    const rawPort = input.companion_port ?? input.companionPort ?? input.port;
    const parsed = Number(rawPort);
    const port = Number.isFinite(parsed) ? parsed : fallback.companionPort;
    return {
      companionEnabled: enabled,
      companionPort: port > 0 && port <= 65535 ? port : fallback.companionPort,
    };
  }




  function toNumber(value, fallback) {
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) ? parsed : fallback;
  }

  function formatDate(value) {
    if (!value) return '';
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) {
      return value;
    }
    return dateFormatter.format(date);
  }

  function setFormStatus(message, status) {
    if (!els.formStatus) return;
    els.formStatus.textContent = message || '';
    els.formStatus.dataset.state = status || 'idle';
  }

  function setAbleSetFormStatus(message, status) {
    if (!els.ablesetFormStatus) return;
    els.ablesetFormStatus.textContent = message || '';
    els.ablesetFormStatus.dataset.state = status || 'idle';
  }

  function setFeatureStatus(message, stateName) {
    if (!els.featureStatus) return;
    els.featureStatus.textContent = message || '';
    els.featureStatus.dataset.state = stateName || 'idle';
  }

  function setFeatureBusy(busy) {
    state.features.submitting = Boolean(busy);
    if (els.featureToggle) {
      els.featureToggle.disabled = busy;
    }
    if (els.featurePort) {
      els.featurePort.disabled = busy;
    }
    if (els.featureSubmit) {
      els.featureSubmit.disabled = busy;
    }
  }

  function renderFeatureForm() {
    if (!els.featureForm) return;
    const config = state.features.config || { companionEnabled: false, companionPort: 18175 };
    if (els.featureToggle) {
      els.featureToggle.checked = Boolean(config.companionEnabled);
    }
    if (els.featurePort && document.activeElement !== els.featurePort) {
      els.featurePort.value = String(config.companionPort);
    }
  }

  async function submitFeatureForm(event) {
    event.preventDefault();
    if (!els.featureForm) return;
    const enabled = Boolean(els.featureToggle && els.featureToggle.checked);
    const rawPort = els.featurePort ? els.featurePort.value.trim() : '';
    const port = Number.parseInt(rawPort, 10);
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      setFeatureStatus('Port must be between 1 and 65535.', 'error');
      if (els.featurePort) {
        els.featurePort.focus();
      }
      return;
    }
    setFeatureBusy(true);
    setFeatureStatus('Saving…', 'info');
    try {
      const response = await fetch('/settings/features', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
        body: JSON.stringify({
          companionEnabled: enabled,
          companionPort: port,
        }),
      });
      if (!response.ok) {
        throw new Error(await extractError(response));
      }
      const data = await response.json();
      state.features.config = normalizeFeatureFlags(data);
      renderFeatureForm();
      setFeatureStatus('Saved.', 'success');
    } catch (error) {
      console.error('Failed to update Companion settings', error);
      setFeatureStatus(error.message || 'Unable to save Companion settings.', 'error');
    } finally {
      setFeatureBusy(false);
    }
  }

  function showToast(message, type) {
    if (!els.toast) return;
    window.clearTimeout(state.toastTimer || 0);
    els.toast.textContent = message;
    els.toast.dataset.state = type || 'info';
    els.toast.dataset.visible = 'true';
    state.toastTimer = window.setTimeout(() => {
      els.toast.dataset.visible = 'false';
    }, 4200);
  }

  function setFormMode(mode) {
    document.body.dataset.mode = mode;
    if (!els.submit) return;
    if (mode === 'edit') {
      els.submit.textContent = 'Save Changes';
      els.formTitle && (els.formTitle.textContent = 'Edit Resolume Connection');
      els.formSubtitle && (els.formSubtitle.textContent = 'Update host details or toggle availability.');
    } else {
      els.submit.textContent = 'Add Connection';
      els.formTitle && (els.formTitle.textContent = 'Add Resolume Connection');
      els.formSubtitle && (els.formSubtitle.textContent = 'Specify hostname, port, and availability.');
    }
  }

  function resetForm() {
    state.editingId = null;
    els.id && (els.id.value = '');
    if (els.form) {
      els.form.reset();
    }
    if (els.port) {
      els.port.value = '8090';
    }
    if (els.enabled) {
      els.enabled.checked = true;
    }
    setFormStatus('', 'idle');
    setFormMode('create');
  }

  function setBusy(isBusy) {
    if (!els.submit) return;
    els.submit.disabled = isBusy;
    els.submit.dataset.loading = isBusy ? 'true' : 'false';
  }

  function setOscBusy(isBusy) {
    state.osc.submitting = Boolean(isBusy);
    if (els.oscSubmit) {
      els.oscSubmit.disabled = isBusy;
      els.oscSubmit.dataset.loading = isBusy ? 'true' : 'false';
    }
  }

  function setAbleSetBusy(isBusy) {
    state.ableset.submitting = Boolean(isBusy);
    if (els.ablesetSubmit) {
      els.ablesetSubmit.disabled = isBusy;
      els.ablesetSubmit.dataset.loading = isBusy ? 'true' : 'false';
    }
  }

  function setAbleSetFormValues() {
    if (!els.ablesetForm) return;
    const config = state.ableset.config;
    if (els.ablesetEnabled) {
      els.ablesetEnabled.checked = Boolean(config.enabled);
    }
    if (els.ablesetHost) {
      els.ablesetHost.value = (config.host || '').toString();
    }
    if (els.ablesetHttpPort) {
      const value = Number.isFinite(Number(config.httpPort)) ? String(config.httpPort) : '80';
      els.ablesetHttpPort.value = value;
    }
    if (els.ablesetOscPort) {
      const value = Number.isFinite(Number(config.oscPort)) ? String(config.oscPort) : '39051';
      els.ablesetOscPort.value = value;
    }
    if (els.ablesetLibrary) {
      els.ablesetLibrary.value = (config.libraryName || '').toString();
    }
    if (els.ablesetPrefix) {
      const value = Number.isFinite(Number(config.songPrefixLength)) ? String(config.songPrefixLength) : '3';
      els.ablesetPrefix.value = value;
    }
  }

  function setOscFormValues() {
    if (!els.oscForm) return;
    const config = state.osc.config;
    if (els.oscEnabled) {
      els.oscEnabled.checked = Boolean(config.enabled);
    }
    if (els.oscPort) {
      const portValue = Number.isFinite(Number(config.listenPort)) ? String(config.listenPort) : '9000';
      els.oscPort.value = portValue;
    }
    if (els.oscAddress) {
      els.oscAddress.value = (config.addressPattern || '/note').toString();
    }
    if (els.oscMode) {
      els.oscMode.value = (config.velocityMode || 'zero_based').toString();
    }
  }

  function renderOscStatus() {
    const status = state.osc.status;
    const stateLabel = status.enabled
      ? (status.listening ? 'listening' : 'enabled')
      : 'disabled';
    if (els.oscStatusIndicator) {
      els.oscStatusIndicator.textContent = stateLabel.charAt(0).toUpperCase() + stateLabel.slice(1);
      els.oscStatusIndicator.dataset.state = stateLabel;
    }
    if (els.oscStatusHostPort) {
      const hostPort = status.hostPort != null ? status.hostPort : null;
      const displayPort = hostPort != null ? hostPort : status.listenPort;
      els.oscStatusHostPort.textContent = String(displayPort);
      els.oscStatusHostPort.dataset.state = hostPort != null ? 'resolved' : 'container';
    }
    if (els.oscStatusLastMessage) {
      const value = status.lastMessageAt ? formatDate(status.lastMessageAt) : '—';
      els.oscStatusLastMessage.textContent = value;
    }
    if (els.oscStatusLastNote) {
      if (status.lastNote != null) {
        const vel = status.lastVelocity != null ? ` (vel ${status.lastVelocity})` : '';
        els.oscStatusLastNote.textContent = `note ${status.lastNote}${vel}`;
      } else {
        els.oscStatusLastNote.textContent = '—';
      }
    }
    if (els.oscStatusError) {
      if (status.lastError) {
        els.oscStatusError.textContent = `⚠ ${status.lastError}`;
        els.oscStatusError.dataset.visible = 'true';
      } else {
        els.oscStatusError.textContent = '';
        els.oscStatusError.dataset.visible = 'false';
      }
    }
    if (els.oscForm) {
      els.oscForm.dataset.mode = status.enabled ? 'enabled' : 'disabled';
    }
  }
  function renderAbleSetStatus() {
    const status = state.ableset.status;
    const stateLabel = status.enabled
      ? (status.tracking ? 'tracking' : 'enabled')
      : 'disabled';
    if (els.ablesetStatusIndicator) {
      const label = stateLabel.charAt(0).toUpperCase() + stateLabel.slice(1);
      els.ablesetStatusIndicator.textContent = label;
      els.ablesetStatusIndicator.dataset.state = stateLabel;
    }
    const lastSong = status.lastSong || null;
    if (els.ablesetStatusSong) {
      els.ablesetStatusSong.textContent = lastSong && lastSong.name ? lastSong.name : '—';
    }
    if (els.ablesetStatusPrefix) {
      els.ablesetStatusPrefix.textContent = lastSong && lastSong.prefix ? lastSong.prefix : '—';
    }
    if (els.ablesetStatusUpdated) {
      const value = lastSong && lastSong.lastSeenAt ? formatDate(lastSong.lastSeenAt) : '—';
      els.ablesetStatusUpdated.textContent = value;
    }
    if (els.ablesetStatusError) {
      if (status.lastError) {
        els.ablesetStatusError.textContent = `⚠ ${status.lastError}`;
        els.ablesetStatusError.dataset.visible = 'true';
      } else {
        els.ablesetStatusError.textContent = '';
        els.ablesetStatusError.dataset.visible = 'false';
      }
    }
    if (els.ablesetForm) {
      els.ablesetForm.dataset.mode = status.enabled ? 'enabled' : 'disabled';
    }
  }


  async function refreshOscStatus(showError) {
    try {
      const response = await fetch('/integrations/osc/status', { headers: { Accept: 'application/json' } });
      if (!response.ok) {
        throw new Error(`Failed to load OSC status (${response.status})`);
      }
      const data = await response.json();
      state.osc.status = normalizeOscStatus(data);
      renderOscStatus();
    } catch (error) {
      if (showError) {
        console.warn('Unable to refresh OSC status', error);
      }
    }
  }

  function renderHosts() {
    if (!els.list) return;
    if (!Array.isArray(state.hosts)) {
      state.hosts = [];
    }
    if (state.hosts.length === 0) {
      els.list.innerHTML = '<li class="settings__list-empty" data-role="host-empty">No Resolume connections defined yet.</li>';
    } else {
      const items = state.hosts
        .map((host) => {
          const statusObj = host.status || {};
          const stateLabel = (statusObj.state || host.statusState || (host.isEnabled ? 'connecting' : 'disabled')).toLowerCase();
          const statusLabel = stateLabel.charAt(0).toUpperCase() + stateLabel.slice(1);
          const normalizedState = (stateLabel || 'disabled').toLowerCase();
          const statusClass = `settings__status settings__status--${normalizedState}`;
          const updated = formatDate(host.updatedAtDisplay || host.updatedAt);
          const created = formatDate(host.createdAtDisplay || host.createdAt);
          const latencySource = statusObj.lastLatencyMs ?? host.lastLatencyMs;
          const latency = typeof latencySource === 'number'
            ? `${latencySource.toFixed(1)} ms`
            : '—';
          const errorMessage = statusObj.lastError || host.statusMessage;
          const statusDetail = errorMessage
            ? `<p class="settings__list-meta settings__list-meta--warning">⚠ ${errorMessage}</p>`
            : '';
          return `
<li class="settings__list-item" data-id="${host.id}" data-enabled="${host.isEnabled}">
  <div class="settings__list-primary">
    <div class="settings__list-title">
      <span class="settings__host-label">${host.label}</span>
      <span class="${statusClass}">${statusLabel}</span>
    </div>
    <p class="settings__list-line"><code>${host.host}</code><span class="settings__host-port">:${host.port}</span></p>
    <p class="settings__list-meta">Updated ${updated}</p>
    <p class="settings__list-meta">Created ${created}</p>
    <p class="settings__list-meta">Latency ${latency}</p>
    ${statusDetail}
  </div>
  <div class="settings__list-actions">
    <button type="button" class="settings__button settings__button--ghost" data-role="host-edit" data-id="${host.id}">Edit</button>
    <button type="button" class="settings__button settings__button--danger" data-role="host-delete" data-id="${host.id}">Delete</button>
  </div>
</li>`;
        })
        .join('');
      els.list.innerHTML = items;
    }
    if (els.hostCount) {
      els.hostCount.textContent = String(state.hosts.length);
    }
  }

  async function refreshHosts(showError) {
    try {
      const response = await fetch(API_ROOT, { headers: { Accept: 'application/json' } });
      if (!response.ok) {
        throw new Error(`Failed to load hosts (${response.status})`);
      }
      const data = await response.json();
      if (Array.isArray(data)) {
        state.hosts = data;
      }
      renderHosts();
    } catch (error) {
      if (showError) {
        showToast(error.message || 'Unable to refresh hosts', 'error');
      }
    }
  }

  function startEdit(id) {
    const host = state.hosts.find((item) => item.id === id);
    if (!host || !els.form) {
      return;
    }
    state.editingId = host.id;
    els.id && (els.id.value = host.id);
    if (els.label) {
      els.label.value = host.label || '';
    }
    if (els.host) {
      els.host.value = host.host || '';
    }
    if (els.port) {
      els.port.value = String(host.port || 8090);
    }
    if (els.enabled) {
      els.enabled.checked = Boolean(host.isEnabled);
    }
    setFormStatus('', 'idle');
    setFormMode('edit');
    window.scrollTo({ top: 0, behavior: 'smooth' });
    if (els.label) {
      els.label.focus();
    }
  }

  function validatePayload(payload) {
    if (!payload.label.trim()) {
      throw new Error('Label cannot be empty.');
    }
    if (!payload.host.trim()) {
      throw new Error('Host cannot be empty.');
    }
    if (payload.port < 1 || payload.port > 65535) {
      throw new Error('Port must be between 1 and 65535.');
    }
  }

  async function submitOscForm(event) {
    event.preventDefault();
    if (!els.oscForm) return;
    setOscBusy(true);
    setFormStatus('', 'idle');
    const payload = {
      enabled: els.oscEnabled ? Boolean(els.oscEnabled.checked) : false,
      listenPort: els.oscPort ? Number.parseInt(els.oscPort.value, 10) || 9000 : 9000,
      addressPattern: els.oscAddress ? els.oscAddress.value.trim() || '/note' : '/note',
      velocityMode: els.oscMode ? els.oscMode.value : 'zero_based',
    };

    // optimistic update
    state.osc.config = normalizeOscConfig(payload);

    try {
      const response = await fetch('/integrations/osc/settings', {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'application/json',
        },
        body: JSON.stringify(payload),
      });
      if (!response.ok) {
        const message = await extractError(response);
        throw new Error(message);
      }
      const data = await response.json();
      state.osc.config = normalizeOscConfig(data);
      showToast('OSC settings saved', 'success');
      await refreshOscStatus(false);
    } catch (error) {
      console.error('Failed to save OSC settings', error);
      showToast(error.message || 'Failed to save OSC settings', 'error');
    } finally {
      setOscBusy(false);
      setOscFormValues();
    }
  }

  async function saveHost(event) {
    event.preventDefault();
    if (!els.form || !els.submit) {
      return;
    }
    const payload = {
      label: (els.label ? els.label.value : '').trim(),
      host: (els.host ? els.host.value : '').trim(),
      port: toNumber(els.port ? els.port.value : '8090', 8090),
      isEnabled: els.enabled ? Boolean(els.enabled.checked) : true,
    };

    try {
      validatePayload(payload);
    } catch (error) {
      setFormStatus(error.message, 'error');
      return;
    }

    const editing = Boolean(state.editingId);
    const url = editing ? `${API_ROOT}/${state.editingId}` : API_ROOT;
    const method = editing ? 'PUT' : 'POST';

    try {
      setBusy(true);
      setFormStatus(editing ? 'Saving changes…' : 'Creating connection…', 'info');
      const response = await fetch(url, {
        method,
        headers: {
          'Content-Type': 'application/json',
          Accept: 'application/json',
        },
        body: JSON.stringify(payload),
      });
      if (!response.ok) {
        const message = await extractError(response);
        throw new Error(message);
      }
      await refreshHosts(false);
      showToast(editing ? 'Updated Resolume connection.' : 'Added Resolume connection.', 'success');
      resetForm();
      setFormStatus('', 'success');
    } catch (error) {
      setFormStatus(error.message || 'Unable to save connection.', 'error');
    } finally {
      setBusy(false);
    }
  }

  async function deleteHost(id) {
    if (!id) return;
    const host = state.hosts.find((item) => item.id === id);
    const label = host ? host.label : 'this connection';
    const confirmed = window.confirm(`Remove ${label}? Presenter will stop reconnecting.`);
    if (!confirmed) {
      return;
    }
    try {
      const response = await fetch(`${API_ROOT}/${id}`, { method: 'DELETE' });
      if (!response.ok) {
        const message = await extractError(response);
        throw new Error(message);
      }
      await refreshHosts(false);
      if (state.editingId === id) {
        resetForm();
      }
      showToast('Deleted Resolume connection.', 'success');
    } catch (error) {
      showToast(error.message || 'Unable to delete connection.', 'error');
    }
  }

  async function handleListClick(event) {
    const target = event.target;
    if (!(target instanceof HTMLElement)) {
      return;
    }
    const id = target.dataset.id;
    if (!id) return;
    if (target.dataset.role === 'host-edit') {
      startEdit(id);
    } else if (target.dataset.role === 'host-delete') {
      await deleteHost(id);
    }
  }

  function extractError(response) {
    return response
      .json()
      .then((data) => {
        if (data && typeof data === 'object' && 'message' in data) {
          return String(data.message);
        }
        return `Request failed (${response.status})`;
      })
      .catch(() => {
        return response.status >= 500
          ? 'Server error while processing request.'
          : 'Request failed.';
      });
  }

  async function submitAbleSetForm(event) {
    event.preventDefault();
    if (!els.ablesetForm) return;
    const payload = {
      enabled: Boolean(els.ablesetEnabled && els.ablesetEnabled.checked),
      host: (els.ablesetHost && els.ablesetHost.value ? els.ablesetHost.value : '').trim(),
      httpPort: toNumber(els.ablesetHttpPort && els.ablesetHttpPort.value, state.ableset.config.httpPort),
      oscPort: toNumber(els.ablesetOscPort && els.ablesetOscPort.value, state.ableset.config.oscPort),
      libraryName: (els.ablesetLibrary && els.ablesetLibrary.value ? els.ablesetLibrary.value : '').trim(),
      songPrefixLength: toNumber(els.ablesetPrefix && els.ablesetPrefix.value, state.ableset.config.songPrefixLength),
    };
    setAbleSetBusy(true);
    setAbleSetFormStatus('Saving AbleSet settings…', 'loading');
    try {
      const response = await fetch('/integrations/ableset/settings', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
        body: JSON.stringify(payload),
      });
      if (!response.ok) {
        throw new Error(await extractError(response));
      }
      const data = await response.json();
      state.ableset.config = normalizeAbleSetConfig(data);
      setAbleSetFormValues();
      setAbleSetFormStatus('AbleSet settings saved.', 'success');
      showToast('AbleSet settings saved.', 'success');
      await refreshAbleSetStatus(false);
    } catch (error) {
      console.error('Failed to update AbleSet settings', error);
      setAbleSetFormStatus(error.message || 'Failed to update AbleSet settings.', 'error');
      showToast('Unable to update AbleSet settings.', 'error');
    } finally {
      setAbleSetBusy(false);
    }
  }

  async function refreshAbleSetStatus(showError) {
    try {
      const response = await fetch('/integrations/ableset/status', { headers: { Accept: 'application/json' } });
      if (!response.ok) {
        throw new Error(`Failed to load AbleSet status (${response.status})`);
      }
      const data = await response.json();
      state.ableset.status = normalizeAbleSetStatus(data);
      renderAbleSetStatus();
    } catch (error) {
      if (showError) {
        console.warn('Unable to refresh AbleSet status', error);
      }
    }
  }

  if (els.featureForm) {
    renderFeatureForm();
    setFeatureStatus('', 'idle');
    els.featureForm.addEventListener('submit', submitFeatureForm);
  }
  if (els.featureToggle) {
    els.featureToggle.addEventListener('change', () => {
      setFeatureStatus('', 'idle');
    });
  }
  if (els.featurePort) {
    els.featurePort.addEventListener('input', () => {
      if (state.features.submitting) return;
      setFeatureStatus('', 'idle');
    });
  }

  if (els.ablesetForm) {
    els.ablesetForm.addEventListener('submit', submitAbleSetForm);
  }
  if (els.ablesetRefresh) {
    els.ablesetRefresh.addEventListener('click', () => {
      refreshAbleSetStatus(true);
    });
  }
  if (els.form) {
    els.form.addEventListener('submit', saveHost);
  }
  if (els.reset) {
    els.reset.addEventListener('click', resetForm);
  }
  if (els.oscForm) {
    els.oscForm.addEventListener('submit', submitOscForm);
  }
  if (els.list) {
    els.list.addEventListener('click', (event) => {
      handleListClick(event);
    });
  }
  window.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && state.editingId) {
      resetForm();
    }
  });

  renderHosts();
  setOscFormValues();
  renderOscStatus();
  setAbleSetFormValues();
  renderAbleSetStatus();
  refreshHosts(false);
  refreshOscStatus(false);
  refreshAbleSetStatus(false);

  window.setInterval(() => {
    refreshHosts(false).catch((error) => {
      console.warn('failed to refresh Resolume host statuses', error);
    });
    refreshOscStatus(false).catch((error) => {
      console.warn('failed to refresh OSC status', error);
    });
    refreshAbleSetStatus(false).catch((error) => {
      console.warn('failed to refresh AbleSet status', error);
    });
  }, STATUS_REFRESH_MS);
})();
