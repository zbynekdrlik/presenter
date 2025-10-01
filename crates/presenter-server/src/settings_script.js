'use strict';

(function () {
  const API_ROOT = '/integrations/resolume/hosts';
  const initialHosts = __RESOLUME_HOSTS__;
  const state = {
    hosts: Array.isArray(initialHosts) ? initialHosts : [],
    editingId: null,
    toastTimer: null,
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
  };

  const dateFormatter = new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  });

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

  if (els.form) {
    els.form.addEventListener('submit', saveHost);
  }
  if (els.reset) {
    els.reset.addEventListener('click', resetForm);
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
  refreshHosts(false);

  window.setInterval(() => {
    refreshHosts(false).catch((error) => {
      console.warn('failed to refresh Resolume host statuses', error);
    });
  }, STATUS_REFRESH_MS);
})();
