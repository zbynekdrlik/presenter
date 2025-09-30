use axum::response::Html;
use leptos::prelude::*;
use leptos::prelude::{AnyView, IntoAny};
use presenter_core::{StageDisplaySlide, StageDisplaySnapshot, TimerState};
use reactive_graph::owner::Owner;
use serde_json::to_string;

pub fn render_stage_display(snapshot: StageDisplaySnapshot) -> Html<String> {
    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! { <StageDisplayDocument snapshot /> }
            .into_view()
            .to_html()
    });
    Html(format!("<!DOCTYPE html>{html}"))
}

#[component]
fn StageDisplayDocument(snapshot: StageDisplaySnapshot) -> impl IntoView {
    let layout = snapshot.layout.clone();
    let layout_view = render_layout(&snapshot);
    let layout_code = layout.code.clone();
    let snapshot_json = to_string(&snapshot).unwrap_or_else(|_| "{}".to_string());
    let script = format!(
        r#"(function() {{
  const initial = {snapshot_json};
  let currentSnapshot = initial;
  const layout = initial.layout.code;
  const statusEls = {{
    container: document.getElementById('stage-status'),
    connection: document.getElementById('stage-status-connection'),
    latency: document.getElementById('stage-status-latency'),
  }};
  let lastGeneratedAt = initial.generated_at ? Date.parse(initial.generated_at) : null;

  const renderLatency = (value) => {{
    if (!statusEls.latency) return;
    if (value == null || Number.isNaN(value)) {{
      statusEls.latency.textContent = '';
      statusEls.latency.dataset.visible = 'false';
      return;
    }}
    if (value >= 1000) {{
      statusEls.latency.textContent = `${{(value / 1000).toFixed(1)}} s`;
      statusEls.latency.dataset.visible = 'true';
      return;
    }}
    statusEls.latency.textContent = `${{Math.max(0, Math.round(value))}} ms`;
    statusEls.latency.dataset.visible = 'true';
  }};

  const updateLatency = (generatedAt) => {{
    if (!generatedAt) {{
      lastGeneratedAt = null;
      renderLatency(null);
      return;
    }}
    const timestamp = Date.parse(generatedAt);
    if (Number.isNaN(timestamp)) {{
      return;
    }}
    lastGeneratedAt = timestamp;
    renderLatency(Date.now() - timestamp);
  }};

  if (lastGeneratedAt != null) {{
    renderLatency(Date.now() - lastGeneratedAt);
  }}

  setInterval(() => {{
    if (lastGeneratedAt != null) {{
      renderLatency(Date.now() - lastGeneratedAt);
    }}
  }}, 2000);


  const connectionLabels = {{
    connecting: 'Connecting',
    connected: 'Connected',
    disconnected: 'Disconnected',
    error: 'Error',
  }};

  const setConnectionState = (state) => {{
    document.body.dataset.liveState = state;
    window.__presenterStageConnectionState = state;
    if (statusEls.connection) {{
      statusEls.connection.textContent = connectionLabels[state] || state;
    }}
    if (state !== 'connected') {{
      renderLatency(null);
    }}
  }};

  const fitTextElement = (element, baseRem, minRem = 2.4, maxLines = 2) => {{
    if (!element) return;
    let size = baseRem;
    const applySize = (value) => {{
      size = Math.max(value, minRem);
      element.style.fontSize = `${{size}}rem`;
    }};
    applySize(baseRem);

    const measureLines = () => {{
      const style = window.getComputedStyle(element);
      const lineHeight = parseFloat(style.lineHeight || '0');
      if (!lineHeight) {{
        return 0;
      }}
      return Math.ceil(element.scrollHeight / lineHeight);
    }};

    let lines = measureLines();
    let iterations = 0;
    while (lines > maxLines && size > minRem && iterations < 40) {{
      applySize(size - 0.15);
      lines = measureLines();
      iterations += 1;
    }}
  }};

  const fitLyrics = (snapshotLayout) => {{
    window.requestAnimationFrame(() => {{
      if (snapshotLayout === 'worship-snv') {{
        fitTextElement(document.getElementById('current-text'), 6.5, 3.2);
        fitTextElement(document.getElementById('next-text'), 5.2, 2.6);
      }} else if (snapshotLayout === 'worship-pp') {{
        fitTextElement(document.getElementById('current-main'), 5.4, 3.0);
        fitTextElement(document.getElementById('next-main'), 4.0, 2.4);
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
    updateLatency(snapshot.generated_at);
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
      setText('current-main', current ? current.main : '');
      const currentGroup = current && current.group ? current.group : '';
      setText('current-group', currentGroup || '');
      setHidden('current-group', !currentGroup);

      const stage = preferredStage(current);
      setText('current-stage', stage || '');
      setHidden('current-stage', !stage);

      setText('next-main', next ? next.main : '');
      const nextGroup = next && next.group ? next.group : '';
      setText('next-group', nextGroup || '');
      setHidden('next-group', !nextGroup);
    }}
    fitLyrics(layout);
  }};

  const layoutEndpoint = `/stage/snapshots/${{layout}}`;
  let liveSocket = null;
  let reconnectAttempts = 0;
  let reconnectTimer = null;

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
    if (reconnectTimer) return;
    const delay = Math.min(500 * Math.pow(2, reconnectAttempts), 5000);
    reconnectAttempts += 1;
    reconnectTimer = setTimeout(() => {{
      reconnectTimer = null;
      connectLive();
    }}, delay);
  }};

  const connectLive = () => {{
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
      setConnectionState('connected');
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
        }}
      }} catch (error) {{
        console.error('Presenter live event error', error);
      }}
    }});

    const handleClose = () => {{
      setConnectionState('disconnected');
      scheduleReconnect();
    }};

    ws.addEventListener('close', handleClose);
    ws.addEventListener('error', (event) => {{
      console.warn('Presenter stage socket error', event);
      setConnectionState('error');
      try {{ ws.close(); }} catch (_) {{}}
    }});
  }};

  applyStage(currentSnapshot);
  connectLive();

  document.addEventListener('visibilitychange', () => {{
    if (document.visibilityState === 'visible') {{
      refreshFromServer();
    }}
  }});

  window.addEventListener('resize', () => fitLyrics(layout));
  window.addEventListener('focus', refreshFromServer);
  window.__presenterStageRefresh = refreshFromServer;
  window.__presenterStageReconnect = connectLive;
}})();
"#,
        snapshot_json = snapshot_json
    );

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>{layout.name.clone()}</title>
                <style>{STAGE_STYLES}</style>
            </head>
            <body class="stage" data-layout-code={layout_code}>
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
        .map(|slide| slide.main.clone())
        .unwrap_or_default();
    let current_group = snapshot
        .current
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();
    let current_stage = snapshot
        .current
        .as_ref()
        .map(|slide| stage_text(slide))
        .unwrap_or_else(|| "".to_string());
    let current_stage_hidden = (current_stage.is_empty()).to_string();
    let current_stage_text = current_stage.clone();
    let next_main = snapshot
        .next
        .as_ref()
        .map(|slide| slide.main.clone())
        .unwrap_or_default();
    let next_group = snapshot
        .next
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();

    view! {
        <section class="stage__split">
            <div class="stage__split-main">
                <h2>"Current"</h2>
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
                <small
                    id="current-stage"
                    class="stage__meta"
                    data-hidden={current_stage_hidden}
                >
                    {current_stage_text}
                </small>
            </div>
            <aside class="stage__split-sidebar">
                <h3>"Next"</h3>
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
            </aside>
        </section>
    }
    .into_any()
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
body.stage { background: #000; color: #f8fafc; font-family: 'Inter', system-ui, sans-serif; margin: 0; min-height: 100vh; display: flex; align-items: stretch; justify-content: center; padding: 4vh 6vw; }
.stage__body { flex: 1; display: flex; align-items: stretch; justify-content: center; width: 100%; }
.stage__lyrics { display: flex; flex-direction: column; justify-content: space-between; gap: 2.5rem; text-align: center; width: 100%; height: 100%; padding: 2vh 4vw; box-sizing: border-box; }
.stage__lyrics-current { font-size: 6.5rem; font-weight: 700; display: flex; flex-direction: column; gap: 1rem; align-items: center; justify-content: flex-start; letter-spacing: 0.04em; min-height: 0; }
.stage__lyrics-current p { margin: 0; line-height: 1.06; white-space: pre-wrap; text-transform: none; max-width: 100%; }
.stage__lyrics-next { font-size: 5.2rem; color: #cbd5f5; letter-spacing: 0.06em; display: flex; flex-direction: column; gap: 1.4rem; align-items: center; justify-content: center; padding-bottom: 4vh; }
.stage__lyrics-next p { margin: 0; white-space: pre-wrap; text-transform: none; line-height: 1.1; max-width: 100%; }
.stage__group-slot { min-height: 3.8rem; display: flex; align-items: center; justify-content: center; }
.stage__group-slot--next { justify-content: center; }
.stage__split { display: grid; gap: 2rem; grid-template-columns: minmax(0, 2fr) minmax(0, 1fr); width: 100%; }
.stage__split-main { background: rgba(15, 23, 42, 0.75); padding: 2rem; border-radius: 1rem; box-shadow: 0 20px 40px -30px rgba(15, 23, 42, 0.9); display: flex; flex-direction: column; gap: 1.25rem; }
.stage__split-main h2 { margin-top: 0; font-size: 1.5rem; letter-spacing: 0.1em; color: #38bdf8; }
.stage__split-main p { font-size: 3rem; margin: 0; white-space: pre-wrap; }
.stage__split-main small { color: #cbd5f5; }
.stage__split-sidebar { background: rgba(15, 23, 42, 0.55); padding: 1.5rem; border-radius: 1rem; display: flex; flex-direction: column; gap: 1rem; }
.stage__split-sidebar h3 { margin: 0; letter-spacing: 0.1em; color: #38bdf8; }
.stage__split-sidebar p { margin: 0; white-space: pre-wrap; }
.stage__split-main .stage__group-slot,
.stage__split-sidebar .stage__group-slot { justify-content: flex-start; }
.stage__timer { text-align: center; width: 100%; }
.stage__timer-value { font-size: 8rem; font-weight: 700; letter-spacing: 0.1em; }
.stage__timer-label { font-size: 1.5rem; color: #94a3b8; letter-spacing: 0.3em; text-transform: uppercase; }
.stage__timer--preach .stage__timer-value { color: #34d399; }
.stage__timer--countdown .stage__timer-value { color: #38bdf8; }
.stage__group { display: inline-flex; align-items: center; justify-content: center; padding: 0.75rem 2.8rem; background: rgba(56, 189, 248, 0.35); color: #38bdf8; border-radius: 999px; font-size: 2.4rem; letter-spacing: 0.18em; text-transform: uppercase; font-weight: 700; }
.stage__group[data-hidden="true"] { display: none; }
.stage__group--next { background: rgba(250, 204, 21, 0.3); color: #facc15; }
.stage__meta { color: #cbd5f5; display: block; margin-top: 0.5rem; }
.stage__meta[data-hidden="true"] { display: none; }
.stage__empty { color: #94a3b8; font-size: 2rem; }
.stage__status { position: fixed; bottom: 2rem; right: 2.5rem; display: inline-flex; align-items: center; gap: 0.75rem; padding: 0.6rem 1.2rem; font-size: 0.85rem; letter-spacing: 0.12em; text-transform: uppercase; background: rgba(15, 23, 42, 0.7); border-radius: 999px; box-shadow: 0 12px 32px -24px rgba(15, 23, 42, 0.95); }
.stage__status span { display: inline-flex; align-items: center; }
.stage__status-connection { color: #38bdf8; font-weight: 600; }
.stage__status-latency { font-variant-numeric: tabular-nums; color: #e2e8f0; }
.stage__status-latency[data-visible="false"] { display: none; }
body.stage[data-live-state="disconnected"] .stage__status-connection,
body.stage[data-live-state="error"] .stage__status-connection { color: #f87171; }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use presenter_core::{StageDisplayLayout, StageDisplaySlide};

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
            presenter_core::timer::TimersOverview::demo(now),
        );

        let html = render_stage_display(snapshot).0;
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
            Some(presenter_core::SlideId::new()),
            Some(slide),
            None,
            None,
            presenter_core::timer::TimersOverview::demo(now),
        );

        let html = render_stage_display(snapshot).0;
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
            presenter_core::timer::TimersOverview::demo(now),
        );

        let html = render_stage_display(snapshot).0;
        assert!(html.contains("id=\"stage-status\""));
        assert!(html.contains("id=\"stage-status-connection\""));
        assert!(html.contains("id=\"stage-status-latency\""));
        assert!(html.contains("Connecting"));
    }
}
