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

  const setConnectionState = (state) => {{
    document.body.dataset.liveState = state;
    window.__presenterStageConnectionState = state;
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
                <span
                    id="current-group"
                    class="stage__group"
                    data-hidden={(current_group.is_empty()).to_string()}
                >
                    {current_group.clone()}
                </span>
                <p id="current-text">{current_text}</p>
            </div>
            <div class="stage__lyrics-next">
                <span>"NEXT"</span>
                <span
                    id="next-group"
                    class="stage__group stage__group--next"
                    data-hidden={(next_group.is_empty()).to_string()}
                >
                    {next_group.clone()}
                </span>
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
                <span
                    id="current-group"
                    class="stage__group"
                    data-hidden={(current_group.is_empty()).to_string()}
                >
                    {current_group.clone()}
                </span>
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
                <span
                    id="next-group"
                    class="stage__group stage__group--next"
                    data-hidden={(next_group.is_empty()).to_string()}
                >
                    {next_group.clone()}
                </span>
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
.stage__lyrics { display: grid; gap: 3rem; text-align: center; width: 100%; align-content: center; }
.stage__lyrics-current { font-size: 6.5rem; font-weight: 700; display: flex; flex-direction: column; gap: 1rem; align-items: center; letter-spacing: 0.04em; }
.stage__lyrics-current p { margin: 0; line-height: 1.06; white-space: pre-wrap; text-transform: none; max-width: 100%; }
.stage__lyrics-next { font-size: 3rem; color: #cbd5f5; letter-spacing: 0.06em; display: flex; flex-direction: column; gap: 0.75rem; align-items: center; }
.stage__lyrics-next span { display: block; font-size: 1.2rem; color: #38bdf8; letter-spacing: 0.2em; text-transform: uppercase; }
.stage__lyrics-next p { margin: 0; white-space: pre-wrap; text-transform: none; line-height: 1.1; max-width: 100%; }
.stage__split { display: grid; gap: 2rem; grid-template-columns: minmax(0, 2fr) minmax(0, 1fr); width: 100%; }
.stage__split-main { background: rgba(15, 23, 42, 0.75); padding: 2rem; border-radius: 1rem; box-shadow: 0 20px 40px -30px rgba(15, 23, 42, 0.9); display: flex; flex-direction: column; gap: 1.25rem; }
.stage__split-main h2 { margin-top: 0; font-size: 1.5rem; letter-spacing: 0.1em; color: #38bdf8; }
.stage__split-main p { font-size: 3rem; margin: 0; white-space: pre-wrap; }
.stage__split-main small { color: #cbd5f5; }
.stage__split-sidebar { background: rgba(15, 23, 42, 0.55); padding: 1.5rem; border-radius: 1rem; display: flex; flex-direction: column; gap: 1rem; }
.stage__split-sidebar h3 { margin: 0; letter-spacing: 0.1em; color: #38bdf8; }
.stage__split-sidebar p { margin: 0; white-space: pre-wrap; }
.stage__timer { text-align: center; width: 100%; }
.stage__timer-value { font-size: 8rem; font-weight: 700; letter-spacing: 0.1em; }
.stage__timer-label { font-size: 1.5rem; color: #94a3b8; letter-spacing: 0.3em; text-transform: uppercase; }
.stage__timer--preach .stage__timer-value { color: #34d399; }
.stage__timer--countdown .stage__timer-value { color: #38bdf8; }
.stage__group { display: inline-block; padding: 0.35rem 1.5rem; background: rgba(56, 189, 248, 0.25); color: #38bdf8; border-radius: 999px; font-size: 1.2rem; letter-spacing: 0.08em; text-transform: uppercase; }
.stage__group[data-hidden="true"] { display: none; }
.stage__group--next { background: rgba(250, 204, 21, 0.3); color: #facc15; }
.stage__meta { color: #cbd5f5; display: block; margin-top: 0.5rem; }
.stage__meta[data-hidden="true"] { display: none; }
.stage__empty { color: #94a3b8; font-size: 2rem; }
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
}
