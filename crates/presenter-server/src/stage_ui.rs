#![allow(named_arguments_used_positionally)]
use crate::stage_connections::StageHeartbeatConfig;
use axum::response::Html;
use chrono::Local;
use leptos::prelude::*;
use leptos::prelude::{AnyView, IntoAny};
use presenter_core::{StageDisplaySlide, StageDisplaySnapshot, TimerState};
use reactive_graph::owner::Owner;
use serde_json::to_string;

pub fn render_stage_display(
    snapshot: StageDisplaySnapshot,
    heartbeat_config: StageHeartbeatConfig,
    line_limit: u16,
) -> Html<String> {
    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! { <StageDisplayDocument snapshot=snapshot heartbeat_config=heartbeat_config line_limit=line_limit /> }
            .into_view()
            .to_html()
    });
    Html(format!("<!DOCTYPE html>{html}"))
}

#[component]
fn StageDisplayDocument(
    snapshot: StageDisplaySnapshot,
    heartbeat_config: StageHeartbeatConfig,
    line_limit: u16,
) -> impl IntoView {
    let layout = snapshot.layout.clone();
    let layout_view = render_layout(&snapshot);
    let layout_code = layout.code.clone();
    let snapshot_json = to_string(&snapshot).unwrap_or_else(|_| "{}".to_string());
    let heartbeat_config_literal = format!(
        "{{ intervalMs: {}, graceMs: {}, disconnectMs: {} }}",
        heartbeat_config.interval_ms(),
        heartbeat_config.grace_ms(),
        heartbeat_config.disconnect_ms(),
    );
    let initial_line_limit = line_limit.clamp(10, 120);
    let script = format!(
        r"(function() {{
  const initial = {snapshot_json};
  let currentSnapshot = initial;
  let layout = initial.layout.code;
  const statusEls = {{
    container: document.getElementById('stage-status'),
    connection: document.getElementById('stage-status-connection'),
    latency: document.getElementById('stage-status-latency'),
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
  const clockFormatter = new Intl.DateTimeFormat('sk-SK', {{ hour: '2-digit', minute: '2-digit', hour12: false }});
  const LYRIC_FIT_CONFIG = {{
    'worship-snv': [
      {{ id: 'current-text', baseRem: 14.8, minRem: 1.9 }},
      {{ id: 'next-text', baseRem: 12.0, minRem: 1.6 }},
    ],
    'worship-pp': [
      {{ id: 'current-main', baseRem: 13.5, minRem: 1.45 }},
      {{ id: 'next-main', baseRem: 11.0, minRem: 1.3 }},
    ],
  }};
  let stageLineLimit = {initial_line_limit};
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
  let clockTimer = null;

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

  const selectPrimary = (slide) => {{
    if (!slide) return '';
    const stage = (slide.stage || '').trim();
    return stage.length ? stage : (slide.main || '');
  }};

  const sanitizeSongTitle = (value) => {{
    if (value == null) return '';
    const input = String(value).trimStart();
    if (input.length >= 4 && /[0-9]/.test(input[0]) && /[0-9]/.test(input[1]) && /[0-9]/.test(input[2]) && /\s/.test(input[3])) {{
      return input.slice(4).trimStart();
    }}
    return input;
  }};

  const stageNoteText = (slide) => {{
    if (!slide) return '';
    const stage = (slide.stage || '').trim();
    return stage.length ? stage : '';
  }};

  const LINE_HEIGHT_FALLBACK_FACTOR = 1.12;
  const FIT_LINE_TARGET = 2;
  const FIT_LINE_TOLERANCE = 0.12;
  const MIN_FONT_PX = 22;
  const MIN_FONT_SCALE = 0.65;

  const getFitConfig = (layoutCode, elementId) => {{
    const entries = LYRIC_FIT_CONFIG[layoutCode];
    if (!entries) return null;
    return entries.find((entry) => entry.id === elementId) || null;
  }};

  const computeLyricsFontRem = (element, baseRem, minRem, rawContent, elementId) => {{
    if (!element) return null;
    const content = typeof rawContent === 'string' ? rawContent.trim() : '';
    if (!content) {{
      return null;
    }}

    const explicitLines = Math.max(1, content.split(/\r?\n/).length);
    const effectiveTarget = Math.max(FIT_LINE_TARGET, explicitLines);
    const maxLinesAllowed = effectiveTarget + FIT_LINE_TOLERANCE;

    const rootFontSize =
      parseFloat(window.getComputedStyle(document.documentElement).fontSize) || 16;
    let basePx = Math.max(MIN_FONT_PX, baseRem * rootFontSize);
    const configuredMinPx =
      typeof minRem === 'number' && minRem > 0 ? minRem * rootFontSize : null;
    let scaledBaseMin = basePx * MIN_FONT_SCALE;
    let baseMinPx = Math.max(MIN_FONT_PX, configuredMinPx != null ? configuredMinPx : 0, scaledBaseMin);

    // Measure available geometry using the nearest lyric row container, not the text box itself.
    const row = element.parentElement || element;
    const rowRect = row.getBoundingClientRect();
    const width = Math.max(1, rowRect && rowRect.width ? rowRect.width : (document.documentElement?.clientWidth || 640));

    const lyricsContainer = row.closest('.stage__lyrics');
    const lyricsRect = lyricsContainer ? lyricsContainer.getBoundingClientRect() : null;
    const viewportHeight =
      (window.visualViewport && window.visualViewport.height) ||
      window.innerHeight ||
      (document.documentElement && document.documentElement.clientHeight) ||
      1080;
    let layoutCode = null;
    if (document.body && document.body.dataset && document.body.dataset.layoutCode) {{
      layoutCode = document.body.dataset.layoutCode;
    }} else if (window.__presenterStageLayout) {{
      layoutCode = window.__presenterStageLayout;
    }}
    const lyricChildren = lyricsContainer
      ? Array.from(lyricsContainer.children).filter((child) => {{
          if (!(child instanceof HTMLElement)) return false;
          return child.classList.contains('stage__lyrics-current') || child.classList.contains('stage__lyrics-next');
        }})
      : [];
    const rowCount = lyricChildren.length > 0 ? lyricChildren.length : 1;
    const lyricsStyle = lyricsContainer ? window.getComputedStyle(lyricsContainer) : null;
    const lyricsPadTop = lyricsStyle ? parseFloat(lyricsStyle.paddingTop) || 0 : 0;
    const lyricsPadBottom = lyricsStyle ? parseFloat(lyricsStyle.paddingBottom) || 0 : 0;
    const rowGapPx = lyricsStyle ? (parseFloat((lyricsStyle).rowGap || (lyricsStyle).gap || '0') || 0) : 0;
    let stageInnerHeight = lyricsRect ? Math.max(0, lyricsRect.height - lyricsPadTop - lyricsPadBottom) : viewportHeight;
    if (!Number.isFinite(stageInnerHeight) || stageInnerHeight <= 0) {{
      stageInnerHeight = viewportHeight;
    }}
    stageInnerHeight = Math.min(stageInnerHeight, viewportHeight * 0.96);

    const totalGap = Math.max(0, (rowCount - 1)) * rowGapPx;
    const baselineRowHeight = Math.max(1, (stageInnerHeight - totalGap)) / Math.max(1, rowCount);
    const rowRectHeight = rowRect && Number.isFinite(rowRect.height) && rowRect.height > 0 ? rowRect.height : baselineRowHeight;
    const rowLowerBound = baselineRowHeight * 0.85;
    const rowUpperBound = baselineRowHeight * 1.2;
    let targetRowHeight = baselineRowHeight;
    if (rowRectHeight > 0) {{
      const clamped = Math.min(Math.max(rowRectHeight, rowLowerBound), rowUpperBound);
      targetRowHeight = clamped;
    }}
    targetRowHeight = Math.max(targetRowHeight, baselineRowHeight * 0.9, 220);
    targetRowHeight = Math.min(targetRowHeight, viewportHeight * 0.52);

    const referenceStyle = window.getComputedStyle(element);
    const computedFontSize = parseFloat(referenceStyle.fontSize) || basePx;
    const computedLineHeight = parseFloat(referenceStyle.lineHeight);
    let lineHeightRatio =
      Number.isFinite(computedLineHeight) && computedLineHeight > 0 && Number.isFinite(computedFontSize) && computedFontSize > 0
        ? computedLineHeight / computedFontSize
        : LINE_HEIGHT_FALLBACK_FACTOR;
    if (!Number.isFinite(lineHeightRatio) || lineHeightRatio <= 0) {{
      const rowStyle = window.getComputedStyle(row);
      const rowLineHeight = parseFloat(rowStyle.lineHeight);
      const rowFontSize = parseFloat(rowStyle.fontSize);
      if (Number.isFinite(rowLineHeight) && rowLineHeight > 0 && Number.isFinite(rowFontSize) && rowFontSize > 0) {{
        lineHeightRatio = rowLineHeight / rowFontSize;
      }}
    }}
    // Empirical fallback for fonts reporting 'normal' line-height
    if (!Number.isFinite(lineHeightRatio) || lineHeightRatio <= 0) {{
      try {{
        const probe = document.createElement('div');
        Object.assign(probe.style, {{ position: 'absolute', left: '-9999px', top: '-9999px', whiteSpace: 'pre-wrap', fontFamily: referenceStyle.fontFamily, fontWeight: referenceStyle.fontWeight, fontStyle: referenceStyle.fontStyle, fontVariant: referenceStyle.fontVariant, letterSpacing: referenceStyle.letterSpacing, textTransform: referenceStyle.textTransform, textAlign: referenceStyle.textAlign }});
        document.body.appendChild(probe);
        const sz = 100;
        probe.style.fontSize = `${{sz}}px`;
        probe.style.lineHeight = 'normal';
        probe.textContent = 'A\nB';
        const h = probe.scrollHeight;
        if (Number.isFinite(h) && h > 0) {{
          lineHeightRatio = (h / 2) / sz;
        }}
        probe.remove();
      }} catch (_e) {{}}
    }}

    let layoutHeightFill = 0.9;
    if (layoutCode === 'worship-pp' && (elementId === 'current-main' || elementId === 'next-main')) {{
      layoutHeightFill = 0.65;
    }}
  const occupancyTarget = 0.5;

    const occupancyMinPx = targetRowHeight > 0
      ? (targetRowHeight * occupancyTarget) / (lineHeightRatio * Math.max(1, effectiveTarget))
      : 0;
    let absoluteMinPx = baseMinPx;
    if (Number.isFinite(occupancyMinPx) && occupancyMinPx > baseMinPx) {{
      absoluteMinPx = Math.max(baseMinPx, occupancyMinPx);
    }}

    // Estimate extra vertical consumption inside the row (group labels, meta lines, child gaps, paddings)
    let extrasHeight = 0;
    try {{
      const children = Array.from(row.children || []);
      for (const child of children) {{
        if (child === element) continue;
        const cr = (child).getBoundingClientRect();
        if (cr && Number.isFinite(cr.height)) extrasHeight += Math.max(0, cr.height);
      }}
      const rs = window.getComputedStyle(row);
      const padT = parseFloat(rs.paddingTop) || 0;
      const padB = parseFloat(rs.paddingBottom) || 0;
      const rowGap = parseFloat((rs).rowGap || (rs).gap || '0') || 0;
      const gapsInsideRow = Math.max(0, children.length - 1) * rowGap;
      extrasHeight += padT + padB + gapsInsideRow;
    }} catch (_) {{ extrasHeight = 0; }}

    const measureLines = (fontPx) => {{
      const size = Math.max(fontPx, MIN_FONT_PX);
      const rowWidth = Math.max(1, width);
      const rowClone = row.cloneNode(true);
      if (rowClone instanceof HTMLElement) {{
        rowClone.removeAttribute('id');
        rowClone.setAttribute('data-role', 'stage-fit-row');
        Object.assign(rowClone.style, {{
          position: 'absolute',
          visibility: 'hidden',
          pointerEvents: 'none',
          left: '-99999px',
          top: '-99999px',
          width: `${{rowWidth}}px`,
          maxWidth: 'none',
        }});
      }}
      document.body.appendChild(rowClone);
      let targetEl = null;
      try {{
        if (typeof CSS !== 'undefined' && CSS && typeof CSS.escape === 'function') {{
          targetEl = rowClone.querySelector(`#${{CSS.escape(elementId)}}`);
        }} else {{
          targetEl = rowClone.querySelector(`#${{elementId.replace(/[^a-zA-Z0-9_-]/g, '')}}`);
        }}
      }} catch (_) {{
        targetEl = rowClone.querySelector(`#${{elementId}}`);
      }}
      if (!(targetEl instanceof HTMLElement)) {{
        targetEl = rowClone;
      }}
      targetEl.textContent = content;
      targetEl.style.fontSize = `${{size}}px`;
      targetEl.style.lineHeight = `${{lineHeightRatio}}`;
      targetEl.style.maxWidth = '100%';
      targetEl.style.whiteSpace = 'pre-wrap';
      targetEl.style.wordBreak = 'break-word';
      targetEl.style.overflowWrap = 'break-word';
      const computed = window.getComputedStyle(targetEl);
      const padTop = parseFloat(computed.paddingTop || '0') || 0;
      const padBottom = parseFloat(computed.paddingBottom || '0') || 0;
      const lineHeightPx = size * lineHeightRatio;
      const height = targetEl.scrollHeight;
      const rawLines = lineHeightPx > 0 ? Math.max(0, height - padTop - padBottom) / lineHeightPx : 0;
      let lines = rawLines;
      const minLines = explicitLines > 0 ? explicitLines : 0;
      if (lines < minLines) lines = minLines;
      const scrollWidth = targetEl.scrollWidth;
      const widthCoverage = rowWidth > 0 ? Math.min(1, scrollWidth / rowWidth) : 0;
      rowClone.remove();
      return {{ lines, rawLines, lineHeightPx, scrollWidth, widthCoverage }};
    }};
    // Character-per-line limit boost: if average characters per line exceed the configured
    // line limit, gently increase the starting font so fewer characters fit per line.
    // Global baseline scaling from configured characters-per-line limit: lower limit => larger text.
    const approxLinesForLimit = Math.max(1, effectiveTarget);
    const limitCpl = Math.max(10, Math.min(120, Number(stageLineLimit) || 32));
    const referenceCpl = 32; // design baseline
    const boost = Math.min(1.8, Math.max(0.8, referenceCpl / limitCpl));
    if (Number.isFinite(boost) && boost > 0 && boost !== 1) {{
      basePx = Math.max(MIN_FONT_PX, basePx * boost);
      scaledBaseMin = basePx * MIN_FONT_SCALE;
      baseMinPx = Math.max(MIN_FONT_PX, configuredMinPx != null ? configuredMinPx : 0, scaledBaseMin);
    }}

    const baseMeasure = measureLines(basePx);
    let finalPx = basePx;
    let finalMeasure = baseMeasure;
    if (baseMeasure.lines > maxLinesAllowed && absoluteMinPx > baseMinPx) {{
      absoluteMinPx = baseMinPx;
    }}
    if (baseMeasure.lines > maxLinesAllowed) {{
      let effectiveMinPx = Math.max(absoluteMinPx, MIN_FONT_PX);
      let low = effectiveMinPx;
      let high = Math.max(effectiveMinPx, basePx);
      let bestPx = low;
      let bestMeasure = measureLines(low);
      if (bestMeasure.lines > maxLinesAllowed) {{
        for (let decay = 0; decay < 6 && bestMeasure.lines > maxLinesAllowed && low > MIN_FONT_PX; decay += 1) {{
          const next = Math.max(MIN_FONT_PX, low * 0.85);
          if (next === low) break;
          low = next;
          bestMeasure = measureLines(low);
        }}
      }}
      if (bestMeasure.lines <= maxLinesAllowed) {{
        bestPx = low;
      }}
      for (let i = 0; i < 40 && high - low > 0.25; i += 1) {{
        const mid = (low + high) / 2;
        const measure = measureLines(mid);
        if (measure.lines <= maxLinesAllowed) {{
          bestPx = mid;
          bestMeasure = measure;
          low = mid;
        }} else {{
          high = mid;
        }}
      }}
      if (bestMeasure.lines > maxLinesAllowed) {{
        let attempts = 0;
        let nextPx = bestPx;
        let nextMeasure = bestMeasure;
        while (nextMeasure.lines > maxLinesAllowed && attempts < 36 && nextPx > MIN_FONT_PX) {{
          const ratio = Math.max(nextMeasure.lines / maxLinesAllowed, 1.0001);
          const candidate = Math.max(MIN_FONT_PX, nextPx / ratio);
          if (Math.abs(candidate - nextPx) < 0.5) {{
            nextPx = Math.max(MIN_FONT_PX, nextPx - Math.max(1, nextPx * 0.05));
          }} else {{
            nextPx = candidate;
          }}
          nextMeasure = measureLines(nextPx);
          attempts += 1;
        }}
        bestPx = nextPx;
        bestMeasure = nextMeasure;
      }}
      finalPx = bestPx;
      finalMeasure = bestMeasure;
    }} else if (finalMeasure.lines <= maxLinesAllowed) {{
      let growLow = finalPx;
      let growHigh = Math.max(finalPx, basePx * 1.2, absoluteMinPx * 1.2);
      let growBestPx = finalPx;
      let growBestMeasure = finalMeasure;
      for (let i = 0; i < 24 && growHigh - growLow > 0.25; i += 1) {{
        const mid = (growLow + growHigh) / 2;
        const measure = measureLines(mid);
        if (measure.lines <= maxLinesAllowed + 0.01) {{
          growBestPx = mid;
          growBestMeasure = measure;
          growLow = mid;
        }} else {{
          growHigh = mid;
        }}
      }}
      finalPx = growBestPx;
      finalMeasure = growBestMeasure;
    }}
    if (finalMeasure.lines <= maxLinesAllowed) {{
      const approxLines = Math.max(1, effectiveTarget);
      const perLineChars = content.length / approxLines;
      let coverageTarget = 0.58;
      if (approxLines >= 2) {{
        if (perLineChars >= 30) {{
          coverageTarget = 0.65;
        }} else if (perLineChars >= 22) {{
          coverageTarget = 0.6;
        }} else if (perLineChars >= 16) {{
          coverageTarget = 0.56;
        }} else {{
          coverageTarget = 0.52;
        }}
      }} else {{
        if (perLineChars >= 18) {{
          coverageTarget = 0.55;
        }} else if (perLineChars >= 12) {{
          coverageTarget = 0.5;
        }} else {{
          coverageTarget = 0.45;
        }}
      }}
      if (finalMeasure.widthCoverage < coverageTarget) {{
        let low = finalPx;
        let high = Math.max(finalPx, basePx * 1.6, absoluteMinPx * 1.6);
        let bestPx = finalPx;
        let bestMeasure = finalMeasure;
        for (let i = 0; i < 30 && high - low > 0.25; i += 1) {{
          const mid = (low + high) / 2;
          const measure = measureLines(mid);
          if (measure.lines <= maxLinesAllowed + 0.01 && measure.widthCoverage >= bestMeasure.widthCoverage) {{
            bestPx = mid;
            bestMeasure = measure;
            low = mid;
          }} else {{
            high = mid;
          }}
        }}
        finalPx = bestPx;
        finalMeasure = bestMeasure;
      }}
    }}
    // Strong safety: if lines still exceed the cap (e.g., long words/wrap variability),
    // keep shrinking with adaptive steps until we fit or hit the minimum.
    if (finalMeasure.lines > maxLinesAllowed) {{
      let attempts = 0;
      while (finalMeasure.lines > maxLinesAllowed && attempts < 48 && finalPx > MIN_FONT_PX) {{
        const step = Math.max(1.0, finalPx * 0.03);
        finalPx = Math.max(MIN_FONT_PX, finalPx - step);
        finalMeasure = measureLines(finalPx);
        attempts += 1;
      }}
    }}

    // Height-based cap to account for non-lyric siblings inside the row (PP has meta below)
    if (targetRowHeight > 0) {{
      const usable = Math.max(1, targetRowHeight - Math.max(0, extrasHeight));
      const capPx = usable / (lineHeightRatio * Math.max(1, effectiveTarget));
      if (Number.isFinite(capPx) && capPx > 0 && finalPx > capPx) {{
        finalPx = capPx;
        finalMeasure = measureLines(finalPx);
      }}
    }}

    // Enforce absolute floor to avoid tiny fonts when measurement jitter suggests >2 lines even at reasonable sizes.
    if (Number.isFinite(absoluteMinPx) && absoluteMinPx > 0 && finalPx < absoluteMinPx) {{
      finalPx = absoluteMinPx;
      finalMeasure = measureLines(finalPx);
    }}

    // Final gentle nudge for two-line cases to avoid borderline underfill in audits
    if (effectiveTarget >= 2 && finalMeasure.lines <= maxLinesAllowed) {{
      const bumped = Math.max(finalPx, finalPx * 1.0125);
      const bumpMeasure = measureLines(bumped);
      if (bumpMeasure.lines <= maxLinesAllowed + 0.005) {{
        finalPx = bumped;
        finalMeasure = bumpMeasure;
      }}
    }}

    const finalRem = finalPx / rootFontSize;

    if (window.PRESENTER_STAGE_TEST_CONFIG && window.PRESENTER_STAGE_TEST_CONFIG.traceFit) {{
      const log = window.__presenterStageFitLog || [];
      log.push({{
        elementId,
        basePx,
        baseMinPx,
        occupancyMinPx,
        absoluteMinPx,
        effectiveMinPx: absoluteMinPx,
        targetRowHeight,
        maxPx: null,
        effectiveTarget,
        baseLines: baseMeasure.lines,
        finalPx,
        finalLines: finalMeasure.lines,
        widthCoverage: finalMeasure.widthCoverage,
        contentLength: content.length,
        decision: 'fit',
      }});
      window.__presenterStageFitLog = log;
    }}

    return finalRem;
  }};

  const enforceActualLineLimit = (element, maxLinesAllowed) => {{
    if (!(element instanceof HTMLElement)) return;
    let attempts = 0;
    const tick = () => {{
      if (!(element instanceof HTMLElement)) return;
      const style = window.getComputedStyle(element);
      const fontPx = parseFloat(style.fontSize) || MIN_FONT_PX;
      let lineHeightPx = parseFloat(style.lineHeight) || 0;
      if (!Number.isFinite(lineHeightPx) || lineHeightPx <= 0) {{
        lineHeightPx = fontPx * LINE_HEIGHT_FALLBACK_FACTOR;
      }}
      const padTop = parseFloat(style.paddingTop || '0') || 0;
      const padBottom = parseFloat(style.paddingBottom || '0') || 0;
      const rawLines = lineHeightPx > 0 ? Math.max(0, element.scrollHeight - padTop - padBottom) / lineHeightPx : 0;
      if (!Number.isFinite(rawLines) || rawLines <= maxLinesAllowed + 0.01) return;
      const ratio = Math.max(rawLines / maxLinesAllowed, 1.0001);
      const candidatePx = Math.max(MIN_FONT_PX, fontPx / ratio);
      if (!Number.isFinite(candidatePx) || Math.abs(candidatePx - fontPx) < 0.25) return;
      const rootSize = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
      const candidateRem = candidatePx / rootSize;
      element.style.fontSize = `${{candidateRem}}rem`;
      element.dataset.fontRem = candidateRem.toFixed(4);
      attempts += 1;
      if (attempts < 8 && typeof window !== 'undefined') {{
        const raf = window.requestAnimationFrame || window.setTimeout;
        raf(() => tick());
      }}
    }};
    tick();
  }};

  const applyLyricText = (layoutCode, elementId, value) => {{
    const element = document.getElementById(elementId);
    if (!element) return;
    const textValue = value || '';
    if (!textValue.trim()) {{
      element.textContent = '';
      element.style.fontSize = '';
      delete element.dataset.fontRem;
      return;
    }}

    let config = getFitConfig(layoutCode, elementId);
    // Robust fallback: if config missing (shouldn't happen), derive base/min from computed style.
    if (!config) {{
      const rootSize = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
      const style = getComputedStyle(element);
      const computedPx = parseFloat(style.fontSize) || (14 * rootSize);
      const inferredBaseRem = computedPx / rootSize;
      let inferredMinRem = inferredBaseRem * 0.12; // small floor; clamped by absolute min px
      if (/current/.test(elementId)) inferredMinRem = Math.max(inferredMinRem, 1.8);
      if (/next/.test(elementId)) inferredMinRem = Math.max(inferredMinRem, 1.5);
      config = {{ id: elementId, baseRem: inferredBaseRem, minRem: inferredMinRem }};
    }}

    const finalRem = computeLyricsFontRem(
      element,
      config.baseRem,
      config.minRem,
      textValue,
      elementId,
    );
    if (Number.isFinite(finalRem) && finalRem > 0) {{
      element.style.fontSize = `${{finalRem}}rem`;
      if (/current-text/.test(elementId)) {{
        const len = (textValue || '').replace(/\s+/g, '').length;
        element.style.lineHeight = len < 20 ? '1.30' : '1.22';
      }}
      element.dataset.fontRem = finalRem.toFixed(4);
      enforceActualLineLimit(element, FIT_LINE_TARGET + FIT_LINE_TOLERANCE);
    }} else {{
      element.style.fontSize = '';
      delete element.dataset.fontRem;
    }}
    element.textContent = textValue;
  }};

  const refitVisibleLyrics = () => {{
    const entries = LYRIC_FIT_CONFIG[layout];
    if (!entries) return;
    entries.forEach((entry) => {{
      const element = document.getElementById(entry.id);
      if (!element) return;
      const currentText = element.textContent || '';
      if (!currentText.trim()) {{
        element.style.fontSize = '';
        delete element.dataset.fontRem;
        return;
      }}
      const finalRem = computeLyricsFontRem(
        element,
        entry.baseRem,
        entry.minRem,
        currentText,
        entry.id,
      );
      if (Number.isFinite(finalRem) && finalRem > 0) {{
        element.style.fontSize = `${{finalRem}}rem`;
        element.dataset.fontRem = finalRem.toFixed(4);
      }}
    }});
  }};

  const refreshLineLimit = () => {{
    try {{
      const controller = new AbortController();
      const timeout = window.setTimeout(() => controller.abort(), 4500);
      fetch('/settings/features', {{ headers: {{ Accept: 'application/json' }}, signal: controller.signal }})
        .then((response) => {{
          window.clearTimeout(timeout);
          if (!response || !response.ok) return null;
          return response.json();
        }})
        .then((data) => {{
          if (!data) return;
          const raw = Number(data.lineLimit ?? data.line_limit);
          if (!Number.isFinite(raw) || raw <= 0) return;
          const next = Math.min(Math.max(Math.round(raw), 10), 120);
          if (next !== stageLineLimit) {{
            stageLineLimit = next;
            refitVisibleLyrics();
          }}
        }})
        .catch(() => {{}});
    }} catch (_error) {{}}
  }};

  const scheduleLineLimitRefresh = () => {{
    const testConfig = window.PRESENTER_STAGE_TEST_CONFIG || {{}};
    if (testConfig.disableLineLimitFetch) return;
    refreshLineLimit();
    window.setInterval(refreshLineLimit, 60000);
  }};

  scheduleLineLimitRefresh();

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

  const updateClockElement = () => {{
    const el = document.getElementById('stage-clock');
    if (!el) return;
    el.textContent = clockFormatter.format(new Date());
  }};

  const ensureClockTimer = () => {{
    updateClockElement();
    if (clockTimer != null) return;
    clockTimer = window.setInterval(updateClockElement, 1000);
  }};

  const stopClockTimer = () => {{
    if (clockTimer != null) {{
      window.clearInterval(clockTimer);
      clockTimer = null;
    }}
  }};

  const reconcileClock = () => {{
    const hasClockLayout = layout === 'timer' || layout === 'preach';
    if (hasClockLayout) {{
      ensureClockTimer();
    }} else {{
      stopClockTimer();
      const clockEl = document.getElementById('stage-clock');
      if (clockEl) {{
        clockEl.textContent = '';
      }}
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
    reconcileClock();
  }};

  const renderPlaylist = (playlist) => {{
    const container = document.getElementById('stage-playlist');
    const titleEl = document.getElementById('stage-playlist-title');
    const listEl = document.getElementById('stage-playlist-items');
    if (!container || !titleEl || !listEl) return;
    listEl.innerHTML = '';
    if (!playlist || !Array.isArray(playlist.entries) || playlist.entries.length === 0) {{
      container.dataset.visible = 'false';
      titleEl.textContent = 'Playlist';
      return;
    }}
    container.dataset.visible = 'true';
    titleEl.textContent = playlist.name || 'Playlist';
    playlist.entries.forEach((entry) => {{
      const item = document.createElement('li');
      item.className = 'stage__playlist-item';
      const entryName = entry && entry.name ? sanitizeSongTitle(entry.name) : '';
      item.textContent = entryName;
      const isCurrent = Boolean(entry && (entry.isCurrent ?? entry.is_current));
      item.dataset.current = isCurrent ? 'true' : 'false';
      listEl.appendChild(item);
    }});
  }};

  const applyStage = (snapshot) => {{
    applyTimers(snapshot.timers);

    const rawSongName = snapshot.presentationName || snapshot.presentation_name || '';
    const songName = sanitizeSongTitle(rawSongName);

    if (layout === 'worship-snv') {{
      const current = snapshot.current;
      const next = snapshot.next;
      const currentText = selectPrimary(current);
      const nextText = selectPrimary(next);
      applyLyricText(layout, 'current-text', currentText || '');
      applyLyricText(layout, 'next-text', nextText || '');

      const currentGroup = current && current.group ? current.group : '';
      setText('current-group', currentGroup || '');
      setHidden('current-group', !currentGroup);

      setText('current-song', songName || '');
      setHidden('current-song', !songName);

      const nextGroup = next && next.group ? next.group : '';
      setText('next-group', nextGroup || '');
      setHidden('next-group', !nextGroup);

      renderPlaylist(null);
    }} else if (layout === 'worship-pp') {{
      const current = snapshot.current;
      const next = snapshot.next;
      const primary = selectPrimary(current);
      applyLyricText(layout, 'current-main', primary || '');
      const currentGroup = current && current.group ? current.group : '';
      setText('current-group', currentGroup || '');
      setHidden('current-group', !currentGroup);

      setText('current-song', songName || '');
      setHidden('current-song', !songName);

      const stageNote = stageNoteText(current);
      setText('current-stage', stageNote || '');
      setHidden('current-stage', !stageNote);

      const nextPrimary = selectPrimary(next);
      applyLyricText(layout, 'next-main', nextPrimary || '');
      const nextGroup = next && next.group ? next.group : '';
      setText('next-group', nextGroup || '');
      setHidden('next-group', !nextGroup);

      const playlistPayload = snapshot.playlist || snapshot.playlistSummary || snapshot.playlist_summary || null;
      renderPlaylist(playlistPayload);
    }} else {{
      renderPlaylist(null);
    }}
    reconcileClock();
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
  connectLive();

  document.addEventListener('visibilitychange', () => {{
    if (document.visibilityState === 'visible') {{
      refreshFromServer();
    }}
  }});

  // Do not refit visible lyrics on window resize to avoid on-screen size changes.
  // We only compute sizes when text changes.
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
  window.__presenterStageDebug = debugHelpers;
}})();
",
        snapshot_json = snapshot_json,
        heartbeat_config = heartbeat_config_literal,
        initial_line_limit = initial_line_limit
    );

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>{layout.name.clone()}</title>
                <style>{STAGE_STYLES}</style>
            </head>
            <body class="stage" data-layout-code={layout_code} data-output-stale="false">
                <main class="stage__body">{layout_view}</main>
                <div class="stage__status" id="stage-status">
                    <span class="stage__status-connection" id="stage-status-connection">Connecting...</span>
                    <span class="stage__status-latency" id="stage-status-latency" data-visible="false"></span>
                </div>
                <script>{script}</script>
            </body>
        </html>
    }
}

fn render_layout(snapshot: &StageDisplaySnapshot) -> AnyView {
    match snapshot.layout.code.as_str() {
        "worship-snv" => render_worship_snv(snapshot),
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
    let song_name = snapshot
        .presentation_name
        .as_ref()
        .map(|name| sanitize_display_title(name))
        .unwrap_or_default();
    let song_hidden = song_name.is_empty().to_string();

    view! {
        <section class="stage__lyrics">
            <div class="stage__lyrics-current">
                <div class="stage__group-slot stage__group-slot--current">
                    <span
                        id="current-group"
                        class="stage__group"
                        data-hidden={(current_group.is_empty()).to_string()}
                    >
                        {current_group.clone()}
                    </span>
                    <span
                        id="current-song"
                        class="stage__song"
                        data-hidden={song_hidden.clone()}
                    >
                        {song_name.clone()}
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
    let song_name = snapshot
        .presentation_name
        .as_ref()
        .map(|name| sanitize_display_title(name))
        .unwrap_or_default();
    let song_hidden = song_name.is_empty().to_string();
    let current_stage = snapshot
        .current
        .as_ref()
        .map(|slide| stage_text(slide))
        .unwrap_or_else(|| "".to_string());
    let current_stage_hidden = (current_stage.is_empty()).to_string();
    let next_text = snapshot.next.as_ref().map(primary_text).unwrap_or_default();
    let next_group = snapshot
        .next
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();
    let playlist = snapshot.playlist.clone();
    let playlist_visible = playlist
        .as_ref()
        .map(|summary| !summary.entries.is_empty())
        .unwrap_or(false)
        .to_string();
    let playlist_title = playlist
        .as_ref()
        .map(|summary| summary.name.clone())
        .unwrap_or_else(|| "Playlist".to_string());

    view! {
        <section class="stage__pp">
            <div class="stage__lyrics stage__lyrics--pp">
                <div class="stage__lyrics-current stage__lyrics-current--pp">
                    <div class="stage__group-slot stage__group-slot--current">
                        <span
                            id="current-group"
                            class="stage__group"
                            data-hidden={(current_group.is_empty()).to_string()}
                        >
                            {current_group.clone()}
                        </span>
                        <span
                            id="current-song"
                            class="stage__song"
                            data-hidden={song_hidden.clone()}
                        >
                            {song_name.clone()}
                        </span>
                    </div>
                    <p id="current-main">{current_text}</p>
                    <small
                        id="current-stage"
                        class="stage__meta stage__meta--stage"
                        data-hidden={current_stage_hidden.clone()}
                    >
                        {current_stage}
                    </small>
                </div>
                <div class="stage__lyrics-next stage__lyrics-next--pp">
                    <div class="stage__group-slot stage__group-slot--next">
                        <span
                            id="next-group"
                            class="stage__group stage__group--next"
                            data-hidden={(next_group.is_empty()).to_string()}
                        >
                            {next_group.clone()}
                        </span>
                    </div>
                    <p id="next-main">{next_text}</p>
                </div>
            </div>
            <aside
                class="stage__playlist"
                id="stage-playlist"
                data-visible={playlist_visible.clone()}
            >
                <div class="stage__playlist-header">
                    <h3 id="stage-playlist-title">{playlist_title}</h3>
                </div>
                <ul class="stage__playlist-list" id="stage-playlist-items"></ul>
            </aside>
        </section>
    }
    .into_any()
}

fn render_timer(snapshot: &StageDisplaySnapshot) -> AnyView {
    let countdown = snapshot.timers.countdown_to_start.seconds_remaining;
    let formatted = format_hms(countdown);
    let clock = Local::now().format("%H:%M").to_string();

    view! {
        <section class="stage__timer stage__timer--countdown">
            <div class="stage__timer-value" id="countdown-value">{formatted}</div>
            <p class="stage__timer-label">"Service Countdown"</p>
            <div class="stage__clock" id="stage-clock">{clock}</div>
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
    let clock = Local::now().format("%H:%M").to_string();

    view! {
        <section class="stage__timer stage__timer--preach">
            <div class="stage__timer-value" id="preach-value">{formatted}</div>
            <p class="stage__timer-label">"Preach Timer ("<span id="preach-status">{status}</span>")"</p>
            <div class="stage__clock stage__clock--preach" id="stage-clock">{clock}</div>
        </section>
    }
    .into_any()
}

fn sanitize_display_title(name: &str) -> String {
    let trimmed = name.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 4
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_whitespace()
    {
        trimmed[4..].trim_start().to_string()
    } else {
        trimmed.to_string()
    }
}

fn primary_text(slide: &StageDisplaySlide) -> String {
    if !slide.stage.trim().is_empty() {
        slide.stage.clone()
    } else {
        slide.main.clone()
    }
}

fn stage_text(slide: &StageDisplaySlide) -> String {
    if !slide.stage.trim().is_empty() {
        slide.stage.clone()
    } else if !slide.translation.trim().is_empty() {
        slide.translation.clone()
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
/* Lock stage to viewport: no scrollbars, exact 1:1 height */
html, body { height: 100%; }
html { width: 100vw; min-width: 100vw; overflow: hidden; }
body.stage { background: #000; color: #f8fafc; font-family: 'Inter', system-ui, sans-serif; margin: 0; height: 100vh; width: 100vw; overflow: hidden; display: flex; align-items: stretch; justify-content: stretch; padding: 0; }
/* SNV layout should maximize horizontal space: remove side padding on the body for SNV */
body.stage[data-layout-code="worship-snv"] { padding: 0; }
body.stage[data-output-stale="true"] .stage__body { opacity: 0.55; transition: opacity 0.25s ease; }
body.stage[data-output-stale="true"] .stage__status { box-shadow: 0 12px 32px -18px rgba(248, 113, 113, 0.55); }
body.stage[data-output-stale="true"] .stage__lyrics-current,
body.stage[data-output-stale="true"] .stage__lyrics-next,
body.stage[data-output-stale="true"] .stage__timer,
body.stage[data-output-stale="true"] .stage__split-main,
body.stage[data-output-stale="true"] .stage__split-sidebar { opacity: 0.65; transition: opacity 0.25s ease; }
body.stage[data-live-state="reconnecting"] .stage__status-connection { color: #fbbf24; }
body.stage[data-live-state="disconnected"] .stage__status-connection,
body.stage[data-live-state="error"] .stage__status-connection { color: #f87171; }
.stage__body { flex: 1; display: flex; align-items: stretch; justify-content: stretch; width: 100%; max-width: 100vw; height: 100vh; }
.stage__lyrics { display: grid; grid-template-rows: 1fr 1fr; grid-template-columns: 1fr; align-items: stretch; justify-items: stretch; text-align: center; width: 100%; height: 100vh; padding: 0 4vw; box-sizing: border-box; overflow: hidden; }
/* SNV: lock current/next to equal halves so text size never resizes the layout */
body.stage[data-layout-code="worship-snv"] .stage__lyrics { grid-template-rows: 1fr 1fr; height: 100vh; padding-left: 0; padding-right: 0; }
body.stage[data-layout-code="worship-snv"] .stage__lyrics-current,
body.stage[data-layout-code="worship-snv"] .stage__lyrics-next { overflow: hidden; display: flex; align-items: center; }
.stage__lyrics-current { font-size: 14rem; font-weight: 700; display: flex; flex-direction: column; gap: 0.8rem; align-items: stretch; justify-content: flex-start; letter-spacing: 0.04em; min-height: 0; }
.stage__lyrics-current p { margin: 0; line-height: 1.20; white-space: pre-wrap; text-transform: none; max-width: 100%; width: 100%; }
.stage__lyrics-next { font-size: 11.5rem; color: #cbd5f5; letter-spacing: 0.06em; display: flex; flex-direction: column; gap: 1rem; align-items: stretch; justify-content: center; padding-bottom: 4vh; }
.stage__lyrics-next p { margin: 0; white-space: pre-wrap; text-transform: none; line-height: 1.12; max-width: 100%; width: 100%; }
.stage__group-slot { min-height: 3.0rem; display: flex; align-items: center; justify-content: center; gap: 0.75rem; flex-wrap: wrap; text-align: center; width: 100%; }
.stage__group-slot--next { justify-content: center; }
.stage__group-slot--current { justify-content: space-between; align-items: center; flex-wrap: nowrap; gap: 1rem; width: 100%; }
.stage__group-slot--current .stage__group { margin-right: auto; }
.stage__group-slot--current .stage__song { margin-left: auto; }
.stage__split { display: grid; gap: 2rem; grid-template-columns: minmax(0, 2fr) minmax(0, 1fr); width: 100%; height: 100vh; overflow: hidden; }
.stage__split-main { background: rgba(15, 23, 42, 0.75); padding: 2rem; border-radius: 1rem; box-shadow: 0 20px 40px -30px rgba(15, 23, 42, 0.9); display: flex; flex-direction: column; gap: 1.25rem; }
.stage__split-main h2 { margin-top: 0; font-size: 1.5rem; letter-spacing: 0.1em; color: #38bdf8; }
.stage__split-main p { font-size: 3rem; margin: 0; white-space: pre-wrap; }
.stage__split-main small { color: #cbd5f5; }
.stage__split-sidebar { background: rgba(15, 23, 42, 0.55); padding: 1.5rem; border-radius: 1rem; display: flex; flex-direction: column; gap: 1rem; }
.stage__split-sidebar h3 { margin: 0; letter-spacing: 0.1em; color: #38bdf8; }
.stage__split-sidebar p { margin: 0; white-space: pre-wrap; }
.stage__split-main .stage__group-slot,
.stage__split-sidebar .stage__group-slot { justify-content: flex-start; }
.stage__pp { display: grid; gap: 2.5rem; grid-template-columns: minmax(0, 2.6fr) minmax(0, 1fr); width: 100%; align-items: stretch; height: 100vh; overflow: hidden; }
.stage__lyrics--pp { background: rgba(15, 23, 42, 0.75); padding: 2rem 3rem; border-radius: 1.25rem; display: grid; grid-template-rows: 1fr 1fr; grid-template-columns: 1fr; gap: 2rem; box-shadow: 0 24px 48px -32px rgba(15, 23, 42, 0.9); height: 100vh; overflow: hidden; }
.stage__lyrics-current--pp { align-items: stretch; gap: 1rem; font-size: 13.5rem; }
.stage__lyrics-next--pp { align-items: stretch; gap: 1rem; color: #cbd5f5; font-size: 11rem; }
.stage__lyrics-current--pp p { margin: 0; line-height: 1.06; white-space: pre-wrap; max-width: 100%; width: 100%; }
.stage__lyrics-next--pp p { margin: 0; line-height: 1.06; white-space: pre-wrap; max-width: 100%; width: 100%; }
.stage__lyrics-next--pp p { color: #cbd5f5; }
.stage__playlist { background: rgba(15, 23, 42, 0.55); padding: 1.8rem; border-radius: 1.25rem; display: flex; flex-direction: column; gap: 1.5rem; min-width: 0; box-shadow: 0 20px 40px -34px rgba(15, 23, 42, 0.9); }
.stage__playlist[data-visible="false"] { display: none; }
.stage__playlist-header h3 { margin: 0; letter-spacing: 0.18em; text-transform: uppercase; font-size: 1.1rem; color: #38bdf8; }
.stage__playlist-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.85rem; }
.stage__playlist-item { font-size: 1.35rem; letter-spacing: 0.14em; text-transform: uppercase; color: #94a3b8; opacity: 0.65; transition: opacity 0.2s ease, color 0.2s ease; }
.stage__playlist-item[data-current="true"] { color: #facc15; opacity: 1; font-weight: 600; }
.stage__timer { text-align: center; width: 100%; display: flex; flex-direction: column; align-items: center; gap: 1.4rem; }
.stage__timer-value { font-size: 10rem; font-weight: 700; letter-spacing: 0.1em; }
.stage__timer-label { font-size: 1.7rem; color: #94a3b8; letter-spacing: 0.28em; text-transform: uppercase; }
.stage__timer--preach .stage__timer-value { color: #34d399; }
.stage__timer--countdown .stage__timer-value { color: #38bdf8; }
.stage__clock { font-size: 2.6rem; letter-spacing: 0.22em; text-transform: uppercase; color: #94a3b8; }
.stage__clock--preach { color: #38bdf8; }
.stage__group { display: inline-flex; align-items: center; justify-content: center; padding: 0.5rem 1.8rem; background: rgba(56, 189, 248, 0.35); color: #38bdf8; border-radius: 999px; font-size: 1.8rem; letter-spacing: 0.14em; text-transform: uppercase; font-weight: 700; }
.stage__song { display: inline-flex; align-items: center; justify-content: center; padding: 0.4rem 1.2rem; border-radius: 999px; background: rgba(148, 163, 184, 0.2); color: #dbeafe; letter-spacing: 0.07em; text-transform: uppercase; font-weight: 600; font-size: 1.15rem; margin-left: auto; text-align: right; }
.stage__group[data-hidden="true"] { display: none; }
.stage__song[data-hidden="true"] { display: none; }
.stage__group--next { background: rgba(250, 204, 21, 0.3); color: #facc15; }
.stage__meta { color: #cbd5f5; display: block; margin-top: 0.5rem; }
.stage__meta--stage { font-size: 1.6rem; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.8; }
.stage__meta[data-hidden="true"] { display: none; }
.stage__empty { color: #94a3b8; font-size: 2rem; }
.stage__status { position: fixed; bottom: 2rem; right: 2.5rem; display: inline-flex; align-items: center; gap: 0.75rem; padding: 0.6rem 1.2rem; font-size: 0.85rem; letter-spacing: 0.12em; text-transform: uppercase; background: rgba(15, 23, 42, 0.7); border-radius: 999px; box-shadow: 0 12px 32px -24px rgba(15, 23, 42, 0.95); }
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
    use presenter_core::{PresentationId, SlideId, StageDisplayLayout, StageDisplaySlide};

    fn worship_layout() -> StageDisplayLayout {
        StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == "worship-snv")
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
        );

        let html = render_stage_display(snapshot, StageHeartbeatConfig::default_values(), 32).0;
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
            Some("Sample Library".into()),
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
        );

        let html = render_stage_display(snapshot, StageHeartbeatConfig::default_values(), 32).0;
        assert!(html.contains("Line A\nLine B"));
        assert!(html.contains("Verse"));
    }

    #[test]
    fn worship_stage_displays_sanitised_song_title() {
        let now = Utc::now();
        let layout = worship_layout();
        let slide = StageDisplaySlide {
            main: "Line A".to_string(),
            translation: String::new(),
            stage: String::new(),
            group: Some("Verse".to_string()),
        };
        let snapshot = StageDisplaySnapshot::new(
            layout,
            now,
            Some(PresentationId::new()),
            Some("001 Amazing Grace".into()),
            Some("New Level".into()),
            Some("001 Amazing Grace".into()),
            Some(SlideId::new()),
            Some(slide),
            None,
            None,
            presenter_core::timer::TimersOverview::demo(now),
            None,
            None,
            None,
            None,
        );

        let html = render_stage_display(snapshot, StageHeartbeatConfig::default_values(), 32).0;
        assert!(html.contains(">Amazing Grace</span>"));
        assert!(!html.contains(">001 Amazing Grace</span>"));
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
        );

        let html = render_stage_display(snapshot, StageHeartbeatConfig::default_values(), 32).0;
        assert!(html.contains("id=\"stage-status\""));
        assert!(html.contains("id=\"stage-status-connection\""));
        assert!(html.contains("id=\"stage-status-latency\""));
        assert!(html.contains("Connecting"));
    }
}
