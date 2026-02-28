mod styles;
use crate::stage_connections::StageHeartbeatConfig;
use crate::ui::utils::json_safe;
use axum::response::Html;
use leptos::prelude::*;
use leptos::prelude::{AnyView, IntoAny};
use presenter_core::{
    StageDisplaySlide, StageDisplaySnapshot, StagePlaylistEntry, TimerState,
    DEFAULT_STAGE_LAYOUT_CODE,
};
use reactive_graph::owner::Owner;
use styles::STAGE_STYLES;

pub fn render_stage_display(
    snapshot: StageDisplaySnapshot,
    heartbeat_config: StageHeartbeatConfig,
) -> Html<String> {
    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! { <StageDisplayDocument snapshot=snapshot heartbeat_config=heartbeat_config /> }
            .into_view()
            .to_html()
    });
    Html(format!("<!DOCTYPE html>{html}"))
}

#[component]
fn StageDisplayDocument(
    snapshot: StageDisplaySnapshot,
    heartbeat_config: StageHeartbeatConfig,
) -> impl IntoView {
    let layout = snapshot.layout.clone();
    let layout_view = render_layout(&snapshot);
    let layout_code = layout.code.clone();
    let snapshot_json = json_safe(&snapshot);
    let heartbeat_config_literal = format!(
        "{{ intervalMs: {}, graceMs: {}, disconnectMs: {} }}",
        heartbeat_config.interval_ms(),
        heartbeat_config.grace_ms(),
        heartbeat_config.disconnect_ms(),
    );
    let script = format!(
        r"(function() {{
  const initial = {snapshot_json};
  let currentSnapshot = initial;
  let layout = initial.layout.code;
  const statusEls = {{
    connection: document.getElementById('stage-status-connection'),
    latency: document.getElementById('stage-status-latency'),
    clock: document.getElementById('stage-clock'),
    live: document.getElementById('stage-live'),
  }};
  const heartbeatConfig = {heartbeat_config};
  const parseMs = (value, fallback) => {{
    const num = Number(value);
    return Number.isFinite(num) && num > 0 ? num : fallback;
  }};
  const testConfig = window.PRESENTER_STAGE_TEST_CONFIG || {{}};
  const config = {{
    intervalMs: parseMs(testConfig.heartbeatIntervalMs ?? testConfig.intervalMs, heartbeatConfig.intervalMs),
    graceMs: parseMs(testConfig.heartbeatGraceMs ?? testConfig.graceMs, heartbeatConfig.graceMs),
    disconnectMs: parseMs(testConfig.disconnectAfterMs ?? testConfig.disconnectMs, heartbeatConfig.disconnectMs),
    suppressReconnect: Boolean(testConfig.suppressReconnect ?? false),
  }};
  const useLegacyClientId = Boolean(testConfig.forceLegacyClientId);
  const generateClientId = () => {{
    if (!useLegacyClientId && typeof crypto !== 'undefined') {{
      if (typeof crypto.randomUUID === 'function') {{
        return crypto.randomUUID();
      }}
      if (typeof crypto.getRandomValues === 'function') {{
        const bytes = new Uint8Array(16);
        crypto.getRandomValues(bytes);
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        const segments = [
          bytes.subarray(0, 4),
          bytes.subarray(4, 6),
          bytes.subarray(6, 8),
          bytes.subarray(8, 10),
          bytes.subarray(10, 16),
        ];
        return segments
          .map((segment) => Array.from(segment, (byte) => byte.toString(16).padStart(2, '0')).join(''))
          .join('-');
      }}
    }}
    return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (char) => {{
      const rand = (Math.random() * 16) | 0;
      const value = char === 'x' ? rand : ((rand & 0x3) | 0x8);
      return value.toString(16);
    }});
  }};

  const CLIENT_ID_STORAGE_KEY = 'presenter.stageClientId';
  let clientId = null;
  try {{
    if (window.localStorage) {{
      const stored = window.localStorage.getItem(CLIENT_ID_STORAGE_KEY);
      if (stored && typeof stored === 'string') {{
        clientId = stored;
      }}
    }}
  }} catch (error) {{
    console.warn('Presenter stage client id load failed', error);
  }}
  if (!clientId) {{
    clientId = generateClientId();
    try {{
      if (window.localStorage) {{
        window.localStorage.setItem(CLIENT_ID_STORAGE_KEY, clientId);
      }}
    }} catch (error) {{
      console.warn('Presenter stage client id persist failed', error);
    }}
  }}

  const clientIdLower = clientId.toLowerCase();
  window.__presenterStageClientId = clientId;
  window.__presenterStageConfig = config;
  window.__presenterStageLayout = layout;

  let lastLatencyMs = null;
  let lastHeartbeatAt = Date.now();
  let liveState = 'connecting';
  let reconnectAttempts = 0;
  let reconnectTimer = null;
  let liveSocket = null;
  let reconnectBlocked = Boolean(config.suppressReconnect);

  const setOutputStale = (value) => {{
    document.body.dataset.outputStale = value ? 'true' : 'false';
  }};

  const formatLatency = (valueMs) => {{
    if (valueMs == null || Number.isNaN(valueMs)) {{
      return '';
    }}
    const value = Math.max(0, Number(valueMs));
    if (!Number.isFinite(value)) {{
      return '';
    }}
    if (value >= 1000) {{
      return `${{(value / 1000).toFixed(1).padStart(6, ' ')}} s`;
    }}
    const rounded = Math.round(value);
    return `${{rounded.toString().padStart(3, '0')}} ms`;
  }};

  const renderLatency = (valueMs) => {{
    if (!statusEls.latency) return;
    const formatted = formatLatency(valueMs);
    if (!formatted) {{
      statusEls.latency.textContent = '';
      statusEls.latency.dataset.visible = 'false';
      return;
    }}
    statusEls.latency.textContent = `· ${{formatted}}`;
    statusEls.latency.dataset.visible = 'true';
  }};

  const setLatency = (valueMs) => {{
    if (valueMs == null || !Number.isFinite(valueMs)) {{
      lastLatencyMs = null;
      delete window.__presenterStageLatencyMs;
      renderLatency(null);
      return;
    }}
    lastLatencyMs = Math.max(0, valueMs);
    window.__presenterStageLatencyMs = lastLatencyMs;
    renderLatency(lastLatencyMs);
  }};

  const formatClockTime = () => {{
    const now = new Date();
    const hours = String(now.getHours()).padStart(2, '0');
    const minutes = String(now.getMinutes()).padStart(2, '0');
    const seconds = String(now.getSeconds()).padStart(2, '0');
    return `${{hours}}:${{minutes}}:${{seconds}}`;
  }};

  const updateClock = () => {{
    if (statusEls.clock) {{
      statusEls.clock.textContent = formatClockTime();
    }}
  }};

  let broadcastLiveState = false;

  const setBroadcastLive = (enabled) => {{
    broadcastLiveState = Boolean(enabled);
    document.body.dataset.broadcastLive = broadcastLiveState ? 'true' : 'false';
    if (statusEls.live) {{
      statusEls.live.dataset.active = broadcastLiveState ? 'true' : 'false';
      statusEls.live.textContent = broadcastLiveState ? 'LIVE' : 'VYSIELANIE JE VYPNUTE';
    }}
    window.__presenterStageBroadcastLive = broadcastLiveState;
  }};

  const connectionLabels = {{
    connecting: 'Connecting…',
    connected: 'Connected',
    reconnecting: 'Reconnecting…',
    disconnected: 'Disconnected',
    error: 'Error',
  }};

  const setConnectionState = (state) => {{
    if (liveState === state) return;
    liveState = state;
    document.body.dataset.liveState = state;
    window.__presenterStageConnectionState = state;
    if (statusEls.connection) {{
      statusEls.connection.textContent = connectionLabels[state] || state;
    }}
    if (state === 'connected') {{
      setOutputStale(false);
    }} else {{
      setOutputStale(true);
      setLatency(null);
    }}
  }};

  const sendJson = (payload) => {{
    if (!liveSocket || liveSocket.readyState !== WebSocket.OPEN) {{
      return false;
    }}
    try {{
      liveSocket.send(JSON.stringify(payload));
      return true;
    }} catch (error) {{
      console.warn('Presenter stage send failed', error);
      return false;
    }}
  }};

  const sendStagePresence = () => {{
    sendJson({{ type: 'stage_presence', client_id: clientId, layout_code: layout }});
  }};

  const sendHeartbeatAck = (heartbeatId) => {{
    if (!heartbeatId) {{
      sendJson({{ type: 'stage_heartbeat_ack', client_id: clientId }});
      return;
    }}
    sendJson({{ type: 'stage_heartbeat_ack', client_id: clientId, heartbeat_id: heartbeatId }});
  }};

  // Smart scaling: character-width based font sizing
  let measuredCharWidthPer100px = null;

  // Configurable scaling parameters (updated by appearance settings)
  const scalingConfig = {{
    currentMaxFont: layout === 'worship-pp' ? 100 : 120,
    nextMaxFont: layout === 'worship-pp' ? 64 : 80,
    nextRatio: 0.8,
    baseChars: 25,
    minFont: 12,
    groupFontSize: 1.6,
  }};

  const applyAppearance = (cfg) => {{
    if (!cfg) return;
    const b = document.body;
    if (cfg.bodyPaddingV != null) b.style.setProperty('--body-pad-v', cfg.bodyPaddingV + 'vh');
    if (cfg.bodyPaddingH != null) b.style.setProperty('--body-pad-h', cfg.bodyPaddingH + 'vw');
    if (cfg.lyricsGap != null) b.style.setProperty('--lyrics-gap', cfg.lyricsGap + 'rem');
    if (cfg.groupFontSize != null) b.style.setProperty('--group-font-size', cfg.groupFontSize + 'rem');
    if (cfg.nextPaddingBottom != null) b.style.setProperty('--next-pad-bottom', cfg.nextPaddingBottom + 'vh');
    if (cfg.playlistFontSize != null) b.style.setProperty('--playlist-font-size', cfg.playlistFontSize + 'rem');
    if (cfg.playlistHeaderSize != null) b.style.setProperty('--playlist-header-size', cfg.playlistHeaderSize + 'rem');
    if (cfg.playlistPadding != null) b.style.setProperty('--playlist-padding', cfg.playlistPadding + 'rem');
    if (cfg.slidesPlaylistRatio != null) b.style.setProperty('--slides-playlist-ratio', cfg.slidesPlaylistRatio);

    if (cfg.currentMaxFont != null) scalingConfig.currentMaxFont = cfg.currentMaxFont;
    if (cfg.nextMaxFont != null) scalingConfig.nextMaxFont = cfg.nextMaxFont;
    if (cfg.nextRatio != null) scalingConfig.nextRatio = cfg.nextRatio;
    if (cfg.baseChars != null) scalingConfig.baseChars = cfg.baseChars;
    if (cfg.minFont != null) scalingConfig.minFont = cfg.minFont;
    if (cfg.groupFontSize != null) scalingConfig.groupFontSize = cfg.groupFontSize;

    smartScaleLyrics(layout);
  }};

  const fetchAppearance = async () => {{
    try {{
      const resp = await fetch('/stage/appearance/' + encodeURIComponent(layout), {{ cache: 'no-store' }});
      if (resp.ok) {{
        const cfg = await resp.json();
        applyAppearance(cfg);
      }}
    }} catch (err) {{
      console.warn('Presenter appearance fetch failed', err);
    }}
  }};

  const fetchBroadcastLive = async () => {{
    try {{
      const resp = await fetch('/stage/broadcast-live', {{ cache: 'no-store' }});
      if (resp.ok) {{
        const data = await resp.json();
        setBroadcastLive(data.enabled);
      }}
    }} catch (err) {{
      console.warn('Presenter broadcast live fetch failed', err);
    }}
  }};

  let currentDesign = null;

  const fetchDesign = async () => {{
    try {{
      const resp = await fetch('/stage/design/' + encodeURIComponent(layout), {{ cache: 'no-store' }});
      if (resp.ok) {{
        const design = await resp.json();
        applyDesign(design);
      }}
    }} catch (err) {{
      console.warn('Presenter design fetch failed', err);
    }}
  }};

  const applyDesign = (design) => {{
    if (!design || !design.boxes) return;
    currentDesign = design;
    window.__presenterStageDesign = design;

    // Apply background color
    if (design.backgroundColor) {{
      document.body.style.backgroundColor = design.backgroundColor;
    }}

    // Apply box positions via CSS custom properties
    for (const box of design.boxes) {{
      if (!box.visible) continue;
      document.body.style.setProperty('--box-' + box.id + '-x', box.x + '%');
      document.body.style.setProperty('--box-' + box.id + '-y', box.y + '%');
      document.body.style.setProperty('--box-' + box.id + '-w', box.width + '%');
      document.body.style.setProperty('--box-' + box.id + '-h', box.height + '%');
      document.body.style.setProperty('--box-' + box.id + '-color', box.textColor || '#f8fafc');
      document.body.style.setProperty('--box-' + box.id + '-align', box.textAlign || 'center');
      document.body.style.setProperty('--box-' + box.id + '-weight', box.fontWeight || 700);
      document.body.style.setProperty('--box-' + box.id + '-max-font', (box.maxFontPx || 120) + 'px');
    }}

    // Trigger re-layout
    smartScaleLyrics(layout);
  }};

  const measureCharWidth = () => {{
    if (measuredCharWidthPer100px !== null) return measuredCharWidthPer100px;
    const span = document.createElement('span');
    span.style.cssText = 'position:absolute;left:-9999px;top:-9999px;font-family:Inter,system-ui,sans-serif;font-size:100px;font-weight:700;white-space:nowrap;';
    span.textContent = 'AaBbCcDdEeFfGgHhIiJjKkLl';
    document.body.appendChild(span);
    const totalWidth = span.getBoundingClientRect().width;
    document.body.removeChild(span);
    measuredCharWidthPer100px = totalWidth / 24 / 100;
    return measuredCharWidthPer100px;
  }};

  const smartScaleElement = (element, containerWidth, ratio, maxPx) => {{
    if (!element) return;
    const text = (element.textContent || '').trim();
    if (!text.length) return;

    // Get actual container (parent box) for height constraint
    const container = element.closest('.stage__box');
    const containerHeight = container ? container.clientHeight : window.innerHeight * 0.4;

    // Binary search for largest font that fits
    const minFont = scalingConfig.minFont;
    const targetMax = (maxPx || 120) * ratio;
    let low = minFont;
    let high = targetMax;

    // Quick check: if max font fits, use it
    element.style.fontSize = high + 'px';
    if (element.scrollWidth <= containerWidth && element.scrollHeight <= containerHeight) {{
      return; // Max font fits, we're done
    }}

    // Binary search for optimal size
    while (high - low > 1) {{
      const mid = Math.floor((low + high) / 2);
      element.style.fontSize = mid + 'px';
      if (element.scrollWidth <= containerWidth && element.scrollHeight <= containerHeight) {{
        low = mid;
      }} else {{
        high = mid;
      }}
    }}

    element.style.fontSize = low + 'px';
  }};

  const getDesignMaxFont = (boxType, fallback) => {{
    if (!currentDesign?.boxes) return fallback;
    const box = currentDesign.boxes.find(b => b.boxType === boxType);
    return box?.maxFontPx ?? fallback;
  }};

  const scaleBoxElement = (boxSelector, elementId, boxType, defaultMax) => {{
    const box = document.querySelector(boxSelector);
    const element = document.getElementById(elementId);
    if (!box || !element) return;
    const maxFont = getDesignMaxFont(boxType, defaultMax);
    smartScaleElement(element, box.clientWidth, 1.0, maxFont);
  }};

  const smartScaleLyrics = (snapshotLayout) => {{
    window.requestAnimationFrame(() => {{
      if (snapshotLayout === 'worship-snv') {{
        // Scale slide text
        scaleBoxElement('.stage__box--current-slide', 'current-text', 'current_slide', scalingConfig.currentMaxFont);
        scaleBoxElement('.stage__box--next-slide', 'next-text', 'next_slide', scalingConfig.nextMaxFont);
        // Scale groups
        scaleBoxElement('.stage__box--current-group', 'current-group', 'current_group', 48);
        scaleBoxElement('.stage__box--next-group', 'next-group', 'next_group', 36);
        // Scale status elements
        scaleBoxElement('.stage__box--clock', 'stage-clock', 'clock', 72);
        scaleBoxElement('.stage__box--live-indicator', 'stage-live', 'live_indicator', 48);
        scaleBoxElement('.stage__box--connection-status', 'stage-status-connection', 'connection_status', 32);
      }} else if (snapshotLayout === 'worship-pp') {{
        // worship-pp uses container-based layout
        const currentBox = document.querySelector('.stage__worship-pp-current');
        const nextBox = document.querySelector('.stage__worship-pp-next');
        const currentMax = getDesignMaxFont('current_slide', scalingConfig.currentMaxFont);
        const nextMax = getDesignMaxFont('next_slide', scalingConfig.nextMaxFont);
        if (currentBox) {{
          smartScaleElement(document.getElementById('current-main'), currentBox.clientWidth, 1.0, currentMax);
          const groupEl = document.getElementById('current-group');
          if (groupEl) smartScaleElement(groupEl, currentBox.clientWidth, 1.0, getDesignMaxFont('current_group', 48));
        }}
        if (nextBox) {{
          smartScaleElement(document.getElementById('next-main'), nextBox.clientWidth, 1.0, nextMax);
          const groupEl = document.getElementById('next-group');
          if (groupEl) smartScaleElement(groupEl, nextBox.clientWidth, 1.0, getDesignMaxFont('next_group', 36));
        }}
      }}
    }});
  }};

  const selectPrimary = (slide) => {{
    if (!slide) return '';
    const stage = (slide.stage || '').trim();
    return stage.length ? stage : (slide.main || '');
  }};

  const preferredStage = (slide) => {{
    if (!slide) return '';
    const stage = (slide.stage || '').trim();
    if (stage.length) return stage;
    const translation = (slide.translation || '').trim();
    if (translation.length) return translation;
    return slide.main || '';
  }};

  const setText = (id, value) => {{
    const el = document.getElementById(id);
    if (el) {{
      el.textContent = value || '';
    }}
  }};

  const setHidden = (id, hidden) => {{
    const el = document.getElementById(id);
    if (el) {{
      el.dataset.hidden = hidden ? 'true' : 'false';
    }}
  }};

  const renderPlaylistSidebar = (entries, playlistName) => {{
    const sidebar = document.getElementById('playlist-sidebar');
    const nameEl = document.getElementById('playlist-name');
    const listEl = document.getElementById('playlist-list');
    if (!sidebar) return;
    const section = sidebar.closest('.stage__worship-pp');
    if (section) {{
      section.dataset.hasPlaylist = (entries && entries.length > 0) ? 'true' : 'false';
    }}
    if (nameEl) {{
      nameEl.textContent = playlistName || '';
    }}
    if (!listEl) return;
    if (!entries || entries.length === 0) {{
      listEl.innerHTML = '';
      return;
    }}
    let html = '';
    let presNum = 0;
    for (let i = 0; i < entries.length; i++) {{
      const entry = entries[i];
      const active = entry.isActive ? 'true' : 'false';
      const entryType = entry.entryType || 'presentation';
      const name = (entry.name || '').replace(/&/g, '&amp;').replace(/[<]/g, '&lt;').replace(/[>]/g, '&gt;');
      if (entryType === 'presentation') {{
        presNum++;
        html += '<li class=stage__worship-pp-playlist-entry data-active=' + active + ' data-type=' + entryType + '>' + presNum + '. ' + name + '</li>';
      }} else {{
        html += '<li class=stage__worship-pp-playlist-entry data-active=' + active + ' data-type=' + entryType + '>' + name + '</li>';
      }}
    }}
    listEl.innerHTML = html;
    const activeEl = listEl.querySelector('[data-active=true]');
    if (activeEl) {{
      activeEl.scrollIntoView({{ block: 'nearest', behavior: 'smooth' }});
    }}
  }};

  const formatSeconds = (rawSeconds) => {{
    const total = Math.max(0, Math.floor(rawSeconds || 0));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const seconds = total % 60;
    if (hours > 0) {{
      return `${{String(hours).padStart(2, '0')}}:${{String(minutes).padStart(2, '0')}}:${{String(seconds).padStart(2, '0')}}`;
    }}
    return `${{String(minutes).padStart(2, '0')}}:${{String(seconds).padStart(2, '0')}}`;
  }};

  const applyTimers = (timers) => {{
    if (!timers) return;
    const countdown = timers.countdownToStart || timers.countdown_to_start || {{}};
    const preach = timers.preachTimer || timers.preach_timer || {{}};
    const countdownSeconds = countdown.secondsRemaining ?? countdown.seconds_remaining ?? 0;
    const preachSeconds = preach.secondsElapsed ?? preach.seconds_elapsed ?? 0;

    if (layout === 'timer') {{
      const formatted = formatSeconds(countdownSeconds);
      setText('countdown-value', formatted);
      window.__presenterStageCountdown = formatted;
    }}
    if (layout === 'preach') {{
      setText('preach-value', formatSeconds(preachSeconds));
      setText('preach-status', preach.state || '');
    }}
  }};

  const applyStage = (snapshot) => {{
    applyTimers(snapshot.timers);

    if (layout === 'worship-snv') {{
      const current = snapshot.current;
      const next = snapshot.next;
      setText('current-text', selectPrimary(current));
      const nextText = selectPrimary(next);
      setText('next-text', nextText || '');

      const currentGroup = current && current.group ? current.group : '';
      setText('current-group', currentGroup || '');
      setHidden('current-group', !currentGroup);

      const nextGroup = next && next.group ? next.group : '';
      setText('next-group', nextGroup || '');
      setHidden('next-group', !nextGroup);
    }} else if (layout === 'worship-pp') {{
      const current = snapshot.current;
      const next = snapshot.next;
      setText('current-main', selectPrimary(current));
      const currentGroup = current && current.group ? current.group : '';
      setText('current-group', currentGroup || '');
      setHidden('current-group', !currentGroup);

      setText('next-main', selectPrimary(next));
      const nextGroup = next && next.group ? next.group : '';
      setText('next-group', nextGroup || '');
      setHidden('next-group', !nextGroup);

      renderPlaylistSidebar(snapshot.playlistEntries, snapshot.playlistName);
    }}
    smartScaleLyrics(layout);
  }};

  const layoutEndpoint = '/stage/snapshot';

  const refreshFromServer = async () => {{
    try {{
      const response = await fetch(layoutEndpoint, {{ cache: 'no-store' }});
      if (!response.ok) throw new Error('snapshot fetch failed');
      const snapshot = await response.json();
      currentSnapshot = snapshot;
      applyStage(currentSnapshot);
    }} catch (error) {{
      console.warn('Presenter stage refresh failed', error);
    }}
  }};

  const scheduleReconnect = () => {{
    if (reconnectBlocked) return;
    if (reconnectTimer) return;
    setConnectionState('reconnecting');
    const delay = Math.min(500 * Math.pow(2, reconnectAttempts), 5000);
    reconnectAttempts += 1;
    reconnectTimer = window.setTimeout(() => {{
      reconnectTimer = null;
      connectLive();
    }}, delay);
  }};

  const connectLive = () => {{
    if (reconnectBlocked) return;
    if (liveSocket) {{
      try {{
        liveSocket.close();
      }} catch (_) {{}}
    }}

    const protocol = window.location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${{protocol}}://${{window.location.host}}/live/ws`);
    liveSocket = ws;
    window.__presenterStageSocket = ws;
    setConnectionState('connecting');

    ws.addEventListener('open', () => {{
      reconnectAttempts = 0;
      reconnectBlocked = Boolean(config.suppressReconnect);
      if (reconnectTimer) {{
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }}
      lastHeartbeatAt = Date.now();
      sendStagePresence();
      refreshFromServer();
      fetchAppearance();
      fetchDesign();
      fetchBroadcastLive();
    }});

    ws.addEventListener('message', (event) => {{
      try {{
        const data = JSON.parse(event.data);
        if (data.type === 'stage' && data.snapshot.layout.code === layout) {{
          currentSnapshot = data.snapshot;
          applyStage(currentSnapshot);
        }} else if (data.type === 'timers') {{
          currentSnapshot.timers = data.overview;
          applyTimers(currentSnapshot.timers);
        }} else if (data.type === 'stage_appearance') {{
          if (data.layout === layout && data.appearance) {{
            applyAppearance(data.appearance);
          }}
        }} else if (data.type === 'stage_design') {{
          if (data.layout === layout && data.design) {{
            applyDesign(data.design);
          }}
        }} else if (data.type === 'stage_layout' || data.type === 'StageLayout') {{
          const next =
            data.code || data.layoutCode || data.layout_code || '';
          if (typeof next === 'string' && next && next !== layout) {{
            window.__presenterStageLayout = next;
            layout = next;
            window.location.reload();
          }}
        }} else if (data.type === 'heartbeat' || data.type === 'Heartbeat') {{
          lastHeartbeatAt = Date.now();
          reconnectAttempts = 0;
          reconnectBlocked = Boolean(config.suppressReconnect);
          setConnectionState('connected');
          const heartbeatId = typeof data.id === 'string' ? data.id : null;
          sendHeartbeatAck(heartbeatId);
        }} else if (data.type === 'stage_connection' || data.type === 'StageConnection') {{
          const snapshot = data.snapshot || data;
          const rawId = snapshot.id || snapshot.clientId || snapshot.client_id;
          if (rawId) {{
            const normalised = String(rawId).toLowerCase();
            if (normalised === clientIdLower) {{
              const latencyValue = snapshot.latencyMs ?? snapshot.latency_ms;
              if (typeof latencyValue === 'number' && Number.isFinite(latencyValue)) {{
                setLatency(latencyValue);
              }}
              const status = (snapshot.status || '').toString().toLowerCase();
              if (status === 'connected') {{
                setConnectionState('connected');
                lastHeartbeatAt = Date.now();
              }} else if (status === 'reconnecting') {{
                setConnectionState('reconnecting');
              }} else if (status === 'disconnected') {{
                setConnectionState('disconnected');
              }} else if (status === 'connecting') {{
                setConnectionState('connecting');
              }}
            }}
          }}
        }} else if (data.type === 'broadcast_live' || data.type === 'BroadcastLive') {{
          const enabled = Boolean(data.enabled);
          setBroadcastLive(enabled);
        }}
      }} catch (error) {{
        console.error('Presenter live event error', error);
      }}
    }});

    const handleClose = () => {{
      if (reconnectBlocked) {{
        setConnectionState('disconnected');
        return;
      }}
      setConnectionState('reconnecting');
      scheduleReconnect();
    }};

    ws.addEventListener('close', handleClose);
    ws.addEventListener('error', (event) => {{
      console.warn('Presenter stage socket error', event);
      setConnectionState('error');
      try {{ ws.close(); }} catch (_) {{}}
    }});
  }};

  const checkHeartbeatTimeout = () => {{
    const now = Date.now();
    const elapsed = now - lastHeartbeatAt;
    if (!Number.isFinite(elapsed)) {{
      return;
    }}
    if (elapsed >= config.disconnectMs) {{
      setConnectionState('disconnected');
      if (!reconnectBlocked) {{
        scheduleReconnect();
      }}
    }} else if (elapsed >= config.graceMs) {{
      setConnectionState('reconnecting');
      if (!reconnectBlocked) {{
        scheduleReconnect();
      }}
    }}
  }};

  const timeoutInterval = Math.max(250, Math.min(1000, Math.floor(config.graceMs / 2)));
  window.setInterval(checkHeartbeatTimeout, timeoutInterval);

  applyStage(currentSnapshot);
  fetchAppearance();
  fetchDesign();
  connectLive();

  // Start clock updates
  updateClock();
  window.setInterval(updateClock, 1000);

  document.addEventListener('visibilitychange', () => {{
    if (document.visibilityState === 'visible') {{
      refreshFromServer();
    }}
  }});

  window.addEventListener('resize', () => {{ measuredCharWidthPer100px = null; smartScaleLyrics(layout); }});
  window.addEventListener('focus', refreshFromServer);
  window.__presenterStageRefresh = refreshFromServer;
  window.__presenterStageReconnect = () => {{
    reconnectBlocked = false;
    connectLive();
  }};

  const debugHelpers = window.__presenterStageDebug || {{}};
  debugHelpers.simulateHeartbeatLoss = () => {{
    reconnectBlocked = true;
    if (liveSocket && liveSocket.readyState === WebSocket.OPEN) {{
      try {{ liveSocket.close(); }} catch (_) {{}}
    }}
    lastHeartbeatAt = Date.now() - (config.graceMs + 100);
    checkHeartbeatTimeout();
    const delay = Math.min(200, Math.max(100, config.disconnectMs - config.graceMs));
    window.setTimeout(() => {{
      lastHeartbeatAt = Date.now() - (config.disconnectMs + 100);
      checkHeartbeatTimeout();
    }}, delay);
  }};
  debugHelpers.resumeHeartbeats = () => {{
    reconnectBlocked = Boolean(config.suppressReconnect);
    lastHeartbeatAt = Date.now();
    if (!reconnectBlocked) {{
      scheduleReconnect();
    }}
  }};
  debugHelpers.setBroadcastLive = (enabled) => {{
    setBroadcastLive(enabled);
  }};
  debugHelpers.getBroadcastLive = () => broadcastLiveState;
  window.__presenterStageDebug = debugHelpers;

  // Listen for design updates from parent window (design editor iframe)
  window.addEventListener('message', (e) => {{
    if (e.data?.type === 'presenter:design-update' && e.data.design) {{
      applyDesign(e.data.design);
    }}
  }});

  // Notify parent window that stage is ready
  if (window.parent !== window) {{
    window.parent.postMessage({{ type: 'presenter:stage-ready', layout }}, '*');
  }}
}})();
",
        snapshot_json = snapshot_json,
        heartbeat_config = heartbeat_config_literal
    );

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>{layout.name.clone()}</title>
                <style>{STAGE_STYLES}</style>
            </head>
            <body class="stage" data-layout-code={layout_code} data-output-stale="false" data-broadcast-live="false">
                <main class="stage__body">{layout_view}</main>
                <script>{script}</script>
            </body>
        </html>
    }
}

fn render_layout(snapshot: &StageDisplaySnapshot) -> AnyView {
    match snapshot.layout.code.as_str() {
        DEFAULT_STAGE_LAYOUT_CODE => render_worship_snv(snapshot),
        "worship-pp" => render_worship_pp(snapshot),
        "timer" => render_timer(snapshot),
        "preach" => render_preach(snapshot),
        _ => view! { <p class="stage__empty">"Unsupported layout."</p> }.into_any(),
    }
}

fn render_worship_snv(snapshot: &StageDisplaySnapshot) -> AnyView {
    let current_text = snapshot
        .current
        .as_ref()
        .map(primary_text)
        .unwrap_or_default();
    let current_group = snapshot
        .current
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();
    let next_text = snapshot.next.as_ref().map(primary_text).unwrap_or_default();
    let next_group = snapshot
        .next
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();

    view! {
        <>
            <div class="stage__box stage__box--current-group" data-hidden={(current_group.is_empty()).to_string()}>
                <span id="current-group" class="stage__group">{current_group.clone()}</span>
            </div>
            <div class="stage__box stage__box--current-slide">
                <p id="current-text">{current_text}</p>
            </div>
            <div class="stage__box stage__box--next-group" data-hidden={(next_group.is_empty()).to_string()}>
                <span id="next-group" class="stage__group">{next_group.clone()}</span>
            </div>
            <div class="stage__box stage__box--next-slide">
                <p id="next-text">{next_text}</p>
            </div>
            <div class="stage__box stage__box--clock">
                <span id="stage-clock">"00:00:00"</span>
            </div>
            <div class="stage__box stage__box--live-indicator">
                <span id="stage-live" class="stage__live" data-active="false">"VYSIELANIE JE VYPNUTE"</span>
            </div>
            <div class="stage__box stage__box--connection-status">
                <span id="stage-status-connection">"Connecting..."</span>
                <span id="stage-status-latency" class="stage__status-latency" data-visible="false"></span>
            </div>
        </>
    }
    .into_any()
}

fn render_worship_pp(snapshot: &StageDisplaySnapshot) -> AnyView {
    let current_main = snapshot
        .current
        .as_ref()
        .map(primary_text)
        .unwrap_or_default();
    let current_group = snapshot
        .current
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();
    let next_main = snapshot.next.as_ref().map(primary_text).unwrap_or_default();
    let next_group = snapshot
        .next
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();
    let playlist_name = snapshot.playlist_name.clone().unwrap_or_default();
    let entries = snapshot.playlist_entries.clone().unwrap_or_default();
    let has_playlist = !entries.is_empty();

    view! {
        <section class="stage__worship-pp" data-has-playlist={has_playlist.to_string()}>
            <div class="stage__worship-pp-slides">
                <div class="stage__worship-pp-current">
                    <div class="stage__group-slot">
                        <span
                            id="current-group"
                            class="stage__group"
                            data-hidden={(current_group.is_empty()).to_string()}
                        >
                            {current_group.clone()}
                        </span>
                    </div>
                    <p id="current-main">{current_main}</p>
                </div>
                <div class="stage__worship-pp-next">
                    <div class="stage__group-slot stage__group-slot--next">
                        <span
                            id="next-group"
                            class="stage__group stage__group--next"
                            data-hidden={(next_group.is_empty()).to_string()}
                        >
                            {next_group.clone()}
                        </span>
                    </div>
                    <p id="next-main">{next_main}</p>
                </div>
            </div>
            <aside class="stage__worship-pp-playlist" id="playlist-sidebar">
                <h3 id="playlist-name">{playlist_name}</h3>
                <ul class="stage__worship-pp-playlist-list" id="playlist-list">
                    {render_playlist_entries(&entries)}
                </ul>
            </aside>
        </section>
    }
    .into_any()
}

fn render_playlist_entries(entries: &[StagePlaylistEntry]) -> String {
    let mut pres_num = 0u32;
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let active = if entry.is_active { "true" } else { "false" };
            let entry_type = &entry.entry_type;
            let name = html_escape(&entry.name);
            let label = if entry_type == "presentation" {
                pres_num += 1;
                format!("{pres_num}. {name}")
            } else {
                name
            };
            format!(
                "<li class=\"stage__worship-pp-playlist-entry\" \
                 id=\"playlist-entry-{index}\" \
                 data-active=\"{active}\" \
                 data-type=\"{entry_type}\">{label}</li>"
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_timer(snapshot: &StageDisplaySnapshot) -> AnyView {
    let countdown = snapshot.timers.countdown_to_start.seconds_remaining;
    let formatted = format_hms(countdown);

    view! {
        <>
            <div class="stage__box stage__box--countdown-timer">
                <div>
                    <span id="countdown-value">{formatted}</span>
                    <p class="stage__timer-label">"Service Countdown"</p>
                </div>
            </div>
            <div class="stage__box stage__box--clock">
                <span id="stage-clock">"00:00:00"</span>
            </div>
            <div class="stage__box stage__box--live-indicator">
                <span id="stage-live" class="stage__live" data-active="false">"VYSIELANIE JE VYPNUTE"</span>
            </div>
            <div class="stage__box stage__box--connection-status">
                <span id="stage-status-connection">"Connecting..."</span>
                <span id="stage-status-latency" class="stage__status-latency" data-visible="false"></span>
            </div>
        </>
    }
    .into_any()
}

fn render_preach(snapshot: &StageDisplaySnapshot) -> AnyView {
    let elapsed = snapshot.timers.preach_timer.seconds_elapsed;
    let formatted = format_hms(elapsed);
    let status = match snapshot.timers.preach_timer.state {
        TimerState::Running => "Running",
        TimerState::Paused => "Paused",
        TimerState::Idle => "Idle",
        TimerState::Completed => "Completed",
    };

    view! {
        <>
            <div class="stage__box stage__box--preach-timer">
                <div>
                    <span id="preach-value">{formatted}</span>
                    <p class="stage__timer-label">"Preach Timer ("<span id="preach-status">{status}</span>")"</p>
                </div>
            </div>
            <div class="stage__box stage__box--clock">
                <span id="stage-clock">"00:00:00"</span>
            </div>
            <div class="stage__box stage__box--live-indicator">
                <span id="stage-live" class="stage__live" data-active="false">"VYSIELANIE JE VYPNUTE"</span>
            </div>
            <div class="stage__box stage__box--connection-status">
                <span id="stage-status-connection">"Connecting..."</span>
                <span id="stage-status-latency" class="stage__status-latency" data-visible="false"></span>
            </div>
        </>
    }
    .into_any()
}

fn primary_text(slide: &StageDisplaySlide) -> String {
    if slide.stage.trim().is_empty() {
        slide.main.clone()
    } else {
        slide.stage.clone()
    }
}

fn format_hms(seconds: i64) -> String {
    let t = seconds.max(0);
    let (h, m, s) = (t / 3600, (t % 3600) / 60, t % 60);
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests;
