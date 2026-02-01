use super::styles;
use super::utils::{format_seconds_compact, json_safe};
use crate::state::AppState;
use axum::response::Html;
use leptos::prelude::*;
use reactive_graph::owner::Owner;

pub async fn render_timer_overlay(state: &AppState) -> anyhow::Result<Html<String>> {
    let overview = state.timers_overview().await?;
    let initial_seconds = overview.countdown_to_start.seconds_remaining;
    let initial_display = format_seconds_compact(initial_seconds);
    let timers_json = json_safe(&overview);

    let script = format!(
        r"(function() {{
  const initial = {timers_json};
  let overview = initial || {{}};
  let countdown = overview.countdown_to_start || overview.countdownToStart || {{}};
  let remaining = Number(countdown.seconds_remaining ?? countdown.secondsRemaining ?? 0);
  let state = String(countdown.state ?? 'idle').toLowerCase();
  const valueEl = document.getElementById('timer-value');

  const coerceTargetEpoch = (value) => {{
    if (typeof value !== 'string') return null;
    const parsed = Date.parse(value);
    return Number.isNaN(parsed) ? null : parsed;
  }};

  let targetEpochMs = coerceTargetEpoch(
    countdown.target ?? countdown.targetUtc ?? countdown.targetUTC ?? null
  );

  const clampNumber = (value) => (Number.isFinite(value) ? value : 0);
  const format = (seconds) => {{
    const total = Math.max(0, Math.floor(clampNumber(seconds)));
    if (total < 60) {{
      return String(total);
    }}
    const minutes = Math.floor(total / 60);
    const secs = total % 60;
    return `${{String(minutes).padStart(2, '0')}}:${{String(secs).padStart(2, '0')}}`;
  }};

  const remainingFromTarget = () => {{
    if (!Number.isFinite(targetEpochMs)) return null;
    return Math.max(0, Math.round((targetEpochMs - Date.now()) / 1000));
  }};

  const publishState = () => {{
    window.__presenterTimerOverlayState = {{ remaining, state }};
  }};

  const render = () => {{
    if (valueEl) {{
      valueEl.textContent = format(remaining);
    }}
    publishState();
  }};

  const applyOverview = (nextOverview) => {{
    if (!nextOverview) return;
    const nextCountdown =
      nextOverview.countdown_to_start ||
      nextOverview.countdownToStart ||
      {{}};
    const rawSeconds = Number(
      nextCountdown.seconds_remaining ??
        nextCountdown.secondsRemaining ??
        remaining
    );
    if (Number.isFinite(rawSeconds)) {{
      remaining = Math.floor(rawSeconds);
    }}
    if (typeof nextCountdown.state === 'string') {{
      state = nextCountdown.state.toLowerCase();
    }}
    const candidateTarget =
      nextCountdown.target ??
      nextCountdown.targetUtc ??
      nextCountdown.targetUTC ??
      null;
    const parsedTarget = coerceTargetEpoch(candidateTarget);
    if (parsedTarget !== null) {{
      targetEpochMs = parsedTarget;
    }}
    render();
  }};

  applyOverview(overview);

  window.setInterval(() => {{
    if (state === 'running') {{
      const derived = remainingFromTarget();
      if (derived !== null) {{
        remaining = derived;
      }} else {{
        remaining = Math.max(0, remaining - 1);
      }}
      render();
    }}
  }}, 1000);

  const connect = () => {{
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(`${{protocol}}//${{window.location.host}}/live/ws`);
    window.__presenterTimerOverlaySocket = socket;

    socket.addEventListener('message', (event) => {{
      try {{
        const data = JSON.parse(event.data);
        if (data.type === 'timers') {{
          overview = data.overview || overview;
          applyOverview(overview);
        }}
      }} catch (error) {{
        console.warn('timer overlay parse error', error);
      }}
    }});

    const scheduleReconnect = () => {{
      window.setTimeout(connect, 1500);
    }};

    socket.addEventListener('close', scheduleReconnect);
    socket.addEventListener('error', () => {{
      try {{ socket.close(); }} catch (_) {{}}
    }});
  }};

  connect();
}})();",
        timers_json = timers_json
    );

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! {
            <html lang="en">
                <head>
                    <meta charset="utf-8" />
                    <title>"Presenter Timer Overlay"</title>
                    <style>{styles::TIMER_OVERLAY}</style>
                </head>
                <body class="overlay overlay--timer">
                    <div class="overlay__timer" id="timer-value">{initial_display}</div>
                    <script>{script}</script>
                </body>
            </html>
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}
