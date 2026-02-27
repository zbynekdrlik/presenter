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
    container: document.getElementById('stage-status'),
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
    const lines = text.split('\n');
    let longestLen = 0;
    for (const line of lines) {{
      if (line.length > longestLen) longestLen = line.length;
    }}
    if (longestLen === 0) longestLen = 1;
    const charW = measureCharWidth();
    const baseFontPx = containerWidth / (scalingConfig.baseChars * charW);
    let fontPx = longestLen <= scalingConfig.baseChars ? baseFontPx : baseFontPx * (scalingConfig.baseChars / longestLen);
    fontPx *= ratio;
    if (maxPx && fontPx > maxPx) fontPx = maxPx;
    fontPx = Math.max(scalingConfig.minFont, fontPx);
    element.style.fontSize = fontPx + 'px';
  }};

  const smartScaleGroup = (element, containerWidth) => {{
    if (!element) return;
    const text = (element.textContent || '').trim();
    if (!text.length) return;
    element.style.fontSize = scalingConfig.groupFontSize + 'rem';
    element.style.maxWidth = Math.floor(containerWidth * 0.5) + 'px';
  }};

  const smartScaleLyrics = (snapshotLayout) => {{
    window.requestAnimationFrame(() => {{
      if (snapshotLayout === 'worship-snv') {{
        const container = document.querySelector('.stage__lyrics');
        if (!container) return;
        const w = container.clientWidth;
        smartScaleElement(document.getElementById('current-text'), w, 1.0, scalingConfig.currentMaxFont);
        smartScaleElement(document.getElementById('next-text'), w, scalingConfig.nextRatio, scalingConfig.nextMaxFont);
        smartScaleGroup(document.getElementById('current-group'), w);
        smartScaleGroup(document.getElementById('next-group'), w);
      }} else if (snapshotLayout === 'worship-pp') {{
        const container = document.querySelector('.stage__worship-pp-slides');
        if (!container) return;
        const w = container.clientWidth;
        smartScaleElement(document.getElementById('current-main'), w, 1.0, scalingConfig.currentMaxFont);
        smartScaleElement(document.getElementById('next-main'), w, scalingConfig.nextRatio, scalingConfig.nextMaxFont);
        smartScaleGroup(document.getElementById('current-group'), w);
        smartScaleGroup(document.getElementById('next-group'), w);
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
                <div class="stage__status-bar" id="stage-status-bar">
                    <div class="stage__clock" id="stage-clock">"00:00:00"</div>
                    <div class="stage__live" id="stage-live" data-active="false">"LIVE"</div>
                    <div class="stage__status" id="stage-status">
                        <span class="stage__status-connection" id="stage-status-connection">"Connecting..."</span>
                        <span class="stage__status-latency" id="stage-status-latency" data-visible="false"></span>
                    </div>
                </div>
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
        <section class="stage__lyrics">
            <div class="stage__lyrics-current">
                <div class="stage__group-slot">
                    <span
                        id="current-group"
                        class="stage__group"
                        data-hidden={(current_group.is_empty()).to_string()}
                    >
                        {current_group.clone()}
                    </span>
                </div>
                <p id="current-text">{current_text}</p>
            </div>
            <div class="stage__lyrics-next">
                <div class="stage__group-slot stage__group-slot--next">
                    <span
                        id="next-group"
                        class="stage__group stage__group--next"
                        data-hidden={(next_group.is_empty()).to_string()}
                    >
                        {next_group.clone()}
                    </span>
                </div>
                <p id="next-text">{next_text}</p>
            </div>
        </section>
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
        <section class="stage__timer stage__timer--countdown">
            <div class="stage__timer-value" id="countdown-value">{formatted}</div>
            <p class="stage__timer-label">"Service Countdown"</p>
        </section>
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
        <section class="stage__timer stage__timer--preach">
            <div class="stage__timer-value" id="preach-value">{formatted}</div>
            <p class="stage__timer-label">"Preach Timer ("<span id="preach-status">{status}</span>")"</p>
        </section>
    }
    .into_any()
}

fn primary_text(slide: &StageDisplaySlide) -> String {
    if !slide.stage.trim().is_empty() {
        slide.stage.clone()
    } else {
        slide.main.clone()
    }
}

fn format_hms(seconds: i64) -> String {
    let total = seconds.max(0);
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let secs = total % 60;
    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}

const STAGE_STYLES: &str = r#"
* { box-sizing: border-box; }
body.stage { background: #000; color: #f8fafc; font-family: 'Inter', system-ui, sans-serif; margin: 0; min-height: 100vh; display: flex; align-items: stretch; justify-content: center; padding: var(--body-pad-v, 1vh) var(--body-pad-h, 2vw); }
body.stage[data-output-stale="true"] .stage__body { opacity: 0.55; transition: opacity 0.25s ease; }
body.stage[data-output-stale="true"] .stage__status { box-shadow: 0 12px 32px -18px rgba(248, 113, 113, 0.55); }
body.stage[data-output-stale="true"] .stage__lyrics-current,
body.stage[data-output-stale="true"] .stage__lyrics-next,
body.stage[data-output-stale="true"] .stage__timer,
body.stage[data-output-stale="true"] .stage__worship-pp-current,
body.stage[data-output-stale="true"] .stage__worship-pp-next { opacity: 0.65; transition: opacity 0.25s ease; }
body.stage[data-live-state="reconnecting"] .stage__status-connection { color: #fbbf24; }
body.stage[data-live-state="disconnected"] .stage__status-connection,
body.stage[data-live-state="error"] .stage__status-connection { color: #f87171; }
.stage__body { flex: 1; display: flex; align-items: stretch; justify-content: center; width: 100%; }
.stage__lyrics { display: flex; flex-direction: column; justify-content: space-between; gap: var(--lyrics-gap, 0.5rem); text-align: center; width: 100%; height: 100%; padding: 0; box-sizing: border-box; }
.stage__lyrics-current { font-size: 6.5rem; font-weight: 700; display: flex; flex-direction: column; gap: 0.3rem; align-items: center; justify-content: flex-start; letter-spacing: 0.04em; min-height: 0; }
.stage__lyrics-current p { margin: 0; line-height: 1.06; white-space: pre-wrap; text-transform: none; max-width: 100%; }
.stage__lyrics-next { font-size: 5.2rem; color: #cbd5f5; letter-spacing: 0.06em; display: flex; flex-direction: column; gap: 0.3rem; align-items: center; justify-content: center; padding-bottom: var(--next-pad-bottom, 2vh); }
.stage__lyrics-next p { margin: 0; white-space: pre-wrap; text-transform: none; line-height: 1.1; max-width: 100%; }
.stage__group-slot { min-height: 0; display: flex; align-items: center; justify-content: center; }
.stage__group-slot:has([data-hidden="true"]) { display: none; }
.stage__group-slot--next { justify-content: center; }
.stage__worship-pp { display: grid; grid-template-columns: minmax(0, 1fr); gap: var(--lyrics-gap, 0.5rem); width: 100%; height: 100%; }
.stage__worship-pp[data-has-playlist="true"] { grid-template-columns: var(--slides-playlist-ratio, minmax(0, 7fr) minmax(0, 3fr)); }
.stage__worship-pp-slides { display: flex; flex-direction: column; justify-content: space-between; gap: var(--lyrics-gap, 0.5rem); min-height: 0; }
.stage__worship-pp-current { flex: 1; font-size: 5.4rem; font-weight: 700; display: flex; flex-direction: column; align-items: center; justify-content: flex-start; text-align: center; min-height: 0; }
.stage__worship-pp-current p { margin: 0; line-height: 1.08; white-space: pre-wrap; max-width: 100%; }
.stage__worship-pp-next { font-size: 3.2rem; color: #cbd5f5; display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center; padding-bottom: var(--next-pad-bottom, 2vh); }
.stage__worship-pp-next p { margin: 0; white-space: pre-wrap; line-height: 1.1; max-width: 100%; }
.stage__worship-pp-playlist { background: rgba(15, 23, 42, 0.55); border-radius: 0.8rem; padding: var(--playlist-padding, 1rem); overflow-y: auto; display: flex; flex-direction: column; }
.stage__worship-pp[data-has-playlist="false"] .stage__worship-pp-playlist { display: none; }
.stage__worship-pp-playlist h3 { font-size: var(--playlist-header-size, 1.1rem); color: #38bdf8; letter-spacing: 0.1em; text-transform: uppercase; margin: 0 0 0.6rem 0; }
.stage__worship-pp-playlist-list { list-style: none; padding: 0; margin: 0; }
.stage__worship-pp-playlist-entry { padding: 0.45rem 0.8rem; border-radius: 0.4rem; font-size: var(--playlist-font-size, 1.3rem); color: #94a3b8; transition: background 0.2s; }
.stage__worship-pp-playlist-entry[data-active="true"] { background: rgba(56, 189, 248, 0.2); color: #38bdf8; font-weight: 600; }
.stage__worship-pp-playlist-entry[data-type="separator"] { font-size: 0.9rem; color: #475569; text-transform: uppercase; letter-spacing: 0.15em; padding: 0.6rem 0.8rem 0.2rem; }
.stage__timer { text-align: center; width: 100%; }
.stage__timer-value { font-size: 8rem; font-weight: 700; letter-spacing: 0.1em; }
.stage__timer-label { font-size: 1.5rem; color: #94a3b8; letter-spacing: 0.3em; text-transform: uppercase; }
.stage__timer--preach .stage__timer-value { color: #34d399; }
.stage__timer--countdown .stage__timer-value { color: #38bdf8; }
.stage__group { display: inline-flex; align-items: center; justify-content: center; padding: 0.25rem 1rem; background: rgba(56, 189, 248, 0.35); color: #38bdf8; border-radius: 999px; font-size: var(--group-font-size, 1.6rem); letter-spacing: 0.18em; text-transform: uppercase; font-weight: 700; }
.stage__group[data-hidden="true"] { display: none; }
.stage__group--next { background: rgba(250, 204, 21, 0.3); color: #facc15; }
.stage__meta { color: #cbd5f5; display: block; margin-top: 0.5rem; }
.stage__meta[data-hidden="true"] { display: none; }
.stage__empty { color: #94a3b8; font-size: 2rem; }
.stage__status-bar { position: fixed; bottom: 0; left: 0; right: 0; display: flex; align-items: center; justify-content: space-between; padding: 1rem 2.5rem; background: linear-gradient(to top, rgba(0, 0, 0, 0.7) 0%, transparent 100%); }
.stage__clock { font-size: 2.55rem; font-weight: 700; font-variant-numeric: tabular-nums; color: #38bdf8; padding: 0.4rem 1rem; background: rgba(15, 23, 42, 0.7); border-radius: 999px; letter-spacing: 0.05em; }
.stage__live { font-size: 1.5rem; font-weight: 700; padding: 0.5rem 1.5rem; border-radius: 999px; letter-spacing: 0.15em; text-transform: uppercase; transition: all 0.3s ease; background: rgba(15, 23, 42, 0.7); color: #475569; opacity: 0.5; }
.stage__live[data-active="true"] { background: rgba(239, 68, 68, 0.9); color: #fff; opacity: 1; box-shadow: 0 0 20px rgba(239, 68, 68, 0.6), 0 0 40px rgba(239, 68, 68, 0.3); animation: live-pulse 2s ease-in-out infinite; }
@keyframes live-pulse { 0%, 100% { box-shadow: 0 0 20px rgba(239, 68, 68, 0.6), 0 0 40px rgba(239, 68, 68, 0.3); } 50% { box-shadow: 0 0 30px rgba(239, 68, 68, 0.8), 0 0 60px rgba(239, 68, 68, 0.5); } }
.stage__status { display: inline-flex; align-items: center; gap: 0.75rem; padding: 0.6rem 1.2rem; font-size: 0.85rem; letter-spacing: 0.12em; text-transform: uppercase; background: rgba(15, 23, 42, 0.7); border-radius: 999px; box-shadow: 0 12px 32px -24px rgba(15, 23, 42, 0.95); }
.stage__status span { display: inline-flex; align-items: center; }
.stage__status-connection { color: #38bdf8; font-weight: 600; }
.stage__status-latency { font-variant-numeric: tabular-nums; color: #e2e8f0; min-width: 7ch; white-space: pre; text-align: right; display: inline-flex; justify-content: flex-end; text-transform: none; letter-spacing: normal; }
.stage__status-latency[data-visible="false"] { display: none; }
body.stage[data-live-state="disconnected"] .stage__status-connection,
body.stage[data-live-state="error"] .stage__status-connection { color: #f87171; }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_connections::StageHeartbeatConfig;
    use chrono::Utc;
    use presenter_core::{StageDisplayLayout, StageDisplaySlide};

    fn worship_layout() -> StageDisplayLayout {
        StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == DEFAULT_STAGE_LAYOUT_CODE)
            .expect("worship layout")
    }

    #[test]
    fn worship_stage_cleared_snapshot_has_no_placeholders() {
        let now = Utc::now();
        let snapshot = StageDisplaySnapshot::new(
            worship_layout(),
            now,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            presenter_core::timer::TimersOverview::demo(now),
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let html = render_stage_display(snapshot, StageHeartbeatConfig::default_values()).0;
        assert!(!html.contains("No next slide"));
        assert!(!html.contains("No active slide"));
    }

    #[test]
    fn worship_stage_preserves_line_breaks() {
        let now = Utc::now();
        let layout = worship_layout();
        let slide = StageDisplaySlide {
            main: "Line A\nLine B".to_string(),
            translation: String::new(),
            stage: String::new(),
            group: Some("Verse".to_string()),
        };
        let snapshot = StageDisplaySnapshot::new(
            layout,
            now,
            Some(presenter_core::PresentationId::new()),
            Some("Sample".into()),
            None,
            Some("Sample Song".into()),
            Some(presenter_core::SlideId::new()),
            Some(slide),
            None,
            None,
            presenter_core::timer::TimersOverview::demo(now),
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let html = render_stage_display(snapshot, StageHeartbeatConfig::default_values()).0;
        assert!(html.contains("Line A\nLine B"));
        assert!(html.contains("Verse"));
    }

    #[test]
    fn stage_status_overlay_is_rendered() {
        let now = Utc::now();
        let snapshot = StageDisplaySnapshot::new(
            worship_layout(),
            now,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            presenter_core::timer::TimersOverview::demo(now),
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let html = render_stage_display(snapshot, StageHeartbeatConfig::default_values()).0;
        assert!(html.contains("id=\"stage-status-bar\""));
        assert!(html.contains("id=\"stage-clock\""));
        assert!(html.contains("id=\"stage-live\""));
        assert!(html.contains("id=\"stage-status\""));
        assert!(html.contains("id=\"stage-status-connection\""));
        assert!(html.contains("id=\"stage-status-latency\""));
        assert!(html.contains("Connecting"));
    }
}
