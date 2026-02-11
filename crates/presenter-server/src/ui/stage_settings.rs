use crate::state::AppState;
use axum::response::Html;
use leptos::prelude::*;
use reactive_graph::owner::Owner;

use super::styles;
use super::utils::json_safe;

/// Render the stage appearance settings page.
pub async fn render_stage_settings_ui(state: &AppState) -> anyhow::Result<Html<String>> {
    let layouts = ["worship-snv", "worship-pp", "timer", "preach"];
    let mut appearances = Vec::new();
    for layout in &layouts {
        let appearance = state.get_stage_appearance(layout).await?;
        appearances.push((*layout, appearance));
    }
    let appearances_json = json_safe(
        &appearances
            .iter()
            .map(|(code, app)| serde_json::json!({ "code": code, "appearance": app }))
            .collect::<Vec<_>>(),
    );

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! {
            <StageSettingsDocument appearances_json=appearances_json />
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}

#[component]
fn StageSettingsDocument(appearances_json: String) -> impl IntoView {
    let script = format!(
        r#"(function() {{
  const layouts = {appearances_json};
  const layoutMap = {{}};
  for (const entry of layouts) {{
    layoutMap[entry.code] = entry.appearance;
  }}

  const defaults = {{
    'worship-snv': {{
      bodyPaddingV: 1, bodyPaddingH: 2, currentMaxFont: 120, nextMaxFont: 80,
      nextRatio: 0.8, groupFontSize: 1.6, lyricsGap: 0.5, nextPaddingBottom: 2,
      baseChars: 25, minFont: 12, playlistFontSize: 1.3, playlistHeaderSize: 1.1,
      playlistPadding: 1, slidesPlaylistRatio: '7fr 3fr'
    }},
    'worship-pp': {{
      bodyPaddingV: 1, bodyPaddingH: 2, currentMaxFont: 100, nextMaxFont: 64,
      nextRatio: 0.8, groupFontSize: 1.6, lyricsGap: 0.5, nextPaddingBottom: 2,
      baseChars: 25, minFont: 12, playlistFontSize: 1.3, playlistHeaderSize: 1.1,
      playlistPadding: 1, slidesPlaylistRatio: '7fr 3fr'
    }},
    'timer': {{
      bodyPaddingV: 1, bodyPaddingH: 2, currentMaxFont: 120, nextMaxFont: 80,
      nextRatio: 0.8, groupFontSize: 1.6, lyricsGap: 0.5, nextPaddingBottom: 2,
      baseChars: 25, minFont: 12, playlistFontSize: 1.3, playlistHeaderSize: 1.1,
      playlistPadding: 1, slidesPlaylistRatio: '7fr 3fr'
    }},
    'preach': {{
      bodyPaddingV: 1, bodyPaddingH: 2, currentMaxFont: 120, nextMaxFont: 80,
      nextRatio: 0.8, groupFontSize: 1.6, lyricsGap: 0.5, nextPaddingBottom: 2,
      baseChars: 25, minFont: 12, playlistFontSize: 1.3, playlistHeaderSize: 1.1,
      playlistPadding: 1, slidesPlaylistRatio: '7fr 3fr'
    }}
  }};

  let activeLayout = 'worship-snv';

  const showTab = (code) => {{
    activeLayout = code;
    document.querySelectorAll('[data-role="layout-tab"]').forEach(btn => {{
      btn.dataset.active = btn.dataset.layout === code ? 'true' : 'false';
    }});
    document.querySelectorAll('[data-role="layout-section"]').forEach(section => {{
      section.dataset.visible = section.dataset.layout === code ? 'true' : 'false';
    }});
  }};

  const populateSliders = (code) => {{
    const cfg = layoutMap[code] || defaults[code] || defaults['worship-snv'];
    const section = document.querySelector(`[data-role="layout-section"][data-layout="${{code}}"]`);
    if (!section) return;
    section.querySelectorAll('[data-param]').forEach(input => {{
      const key = input.dataset.param;
      if (key in cfg) {{
        input.value = cfg[key];
        const display = input.closest('.sa__slider-row')?.querySelector('[data-role="value-display"]');
        if (display) display.textContent = cfg[key];
      }}
    }});
  }};

  const collectValues = (code) => {{
    const section = document.querySelector(`[data-role="layout-section"][data-layout="${{code}}"]`);
    if (!section) return {{}};
    const result = {{}};
    section.querySelectorAll('[data-param]').forEach(input => {{
      const key = input.dataset.param;
      if (input.type === 'text') {{
        result[key] = input.value;
      }} else {{
        result[key] = parseFloat(input.value);
      }}
    }});
    return result;
  }};

  const showToast = (message, isError) => {{
    const toast = document.querySelector('[data-role="toast"]');
    if (!toast) return;
    toast.textContent = message;
    toast.dataset.visible = 'true';
    toast.dataset.state = isError ? 'error' : 'success';
    clearTimeout(toast._timer);
    toast._timer = setTimeout(() => {{ toast.dataset.visible = 'false'; }}, 3000);
  }};

  const saveCurrent = async () => {{
    const code = activeLayout;
    const values = collectValues(code);
    try {{
      const response = await fetch(`/stage/appearance/${{code}}`, {{
        method: 'PUT',
        headers: {{ 'Content-Type': 'application/json' }},
        body: JSON.stringify(values),
      }});
      if (!response.ok) throw new Error(await response.text());
      layoutMap[code] = values;
      showToast('Saved ' + code + ' appearance', false);
    }} catch (err) {{
      showToast('Save failed: ' + err.message, true);
    }}
  }};

  const resetCurrent = async () => {{
    const code = activeLayout;
    const def = defaults[code] || defaults['worship-snv'];
    try {{
      const response = await fetch(`/stage/appearance/${{code}}`, {{
        method: 'PUT',
        headers: {{ 'Content-Type': 'application/json' }},
        body: JSON.stringify(def),
      }});
      if (!response.ok) throw new Error(await response.text());
      layoutMap[code] = {{ ...def }};
      populateSliders(code);
      showToast('Reset ' + code + ' to defaults', false);
    }} catch (err) {{
      showToast('Reset failed: ' + err.message, true);
    }}
  }};

  // Wire up tabs
  document.querySelectorAll('[data-role="layout-tab"]').forEach(btn => {{
    btn.addEventListener('click', () => {{
      showTab(btn.dataset.layout);
    }});
  }});

  // Wire up sliders
  document.querySelectorAll('[data-param]').forEach(input => {{
    input.addEventListener('input', () => {{
      const display = input.closest('.sa__slider-row')?.querySelector('[data-role="value-display"]');
      if (display) display.textContent = input.value;
    }});
  }});

  // Wire up save/reset buttons
  document.querySelectorAll('[data-role="save"]').forEach(btn => {{
    btn.addEventListener('click', saveCurrent);
  }});
  document.querySelectorAll('[data-role="reset"]').forEach(btn => {{
    btn.addEventListener('click', resetCurrent);
  }});

  // Initialize
  for (const code of Object.keys(layoutMap)) {{
    populateSliders(code);
  }}
  showTab('worship-snv');
}})();"#,
        appearances_json = appearances_json
    );

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>"Stage Appearance Settings"</title>
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <style>{styles::SETTINGS}</style>
                <style>{STAGE_SETTINGS_STYLES}</style>
            </head>
            <body class="settings">
                <header class="settings__header">
                    <div class="settings__header-title">
                        <h1>"Stage Appearance Settings"</h1>
                        <p>"Adjust visual parameters per stage layout. Changes are applied live."</p>
                    </div>
                    <nav class="settings__header-nav">
                        <a href="/ui/settings" class="settings__link">"← Back to settings"</a>
                    </nav>
                </header>
                <main class="settings__main">
                    <nav class="sa__tabs" data-role="layout-tabs">
                        <button data-role="layout-tab" data-layout="worship-snv" data-active="true" class="sa__tab">"worship-snv"</button>
                        <button data-role="layout-tab" data-layout="worship-pp" data-active="false" class="sa__tab">"worship-pp"</button>
                        <button data-role="layout-tab" data-layout="timer" data-active="false" class="sa__tab">"timer"</button>
                        <button data-role="layout-tab" data-layout="preach" data-active="false" class="sa__tab">"preach"</button>
                    </nav>
                    {render_layout_section("worship-snv", true)}
                    {render_layout_section("worship-pp", false)}
                    {render_layout_section("timer", false)}
                    {render_layout_section("preach", false)}
                </main>
                <div class="settings__toast" data-role="toast" data-visible="false"></div>
                <script>{script}</script>
            </body>
        </html>
    }
}

fn render_layout_section(layout_code: &str, visible: bool) -> impl IntoView {
    let code = layout_code.to_string();
    let vis = if visible { "true" } else { "false" };
    let is_pp = layout_code == "worship-pp";

    view! {
        <section class="settings__card" data-role="layout-section" data-layout={code.clone()} data-visible={vis}>
            {slider_row("bodyPaddingV", "Body Padding V", "vh", 0.0, 10.0, 0.5)}
            {slider_row("bodyPaddingH", "Body Padding H", "vw", 0.0, 10.0, 0.5)}
            {slider_row("currentMaxFont", "Current Max Font", "px", 20.0, 250.0, 1.0)}
            {slider_row("nextMaxFont", "Next Max Font", "px", 20.0, 200.0, 1.0)}
            {slider_row("nextRatio", "Next Ratio", "", 0.3, 1.0, 0.05)}
            {slider_row("groupFontSize", "Group Font Size", "rem", 0.5, 4.0, 0.1)}
            {slider_row("lyricsGap", "Lyrics Gap", "rem", 0.0, 3.0, 0.1)}
            {slider_row("nextPaddingBottom", "Next Padding Bottom", "vh", 0.0, 10.0, 0.5)}
            {slider_row("baseChars", "Base Chars", "", 10.0, 60.0, 1.0)}
            {slider_row("minFont", "Min Font", "px", 6.0, 40.0, 1.0)}
            {if is_pp {
                Some(view! {
                    <>
                        {slider_row("playlistFontSize", "Playlist Font Size", "rem", 0.5, 4.0, 0.1)}
                        {slider_row("playlistHeaderSize", "Playlist Header Size", "rem", 0.5, 3.0, 0.1)}
                        {slider_row("playlistPadding", "Playlist Padding", "rem", 0.0, 4.0, 0.1)}
                        <div class="sa__slider-row">
                            <label class="sa__slider-label">"Slides/Playlist Ratio"</label>
                            <input type="text" data-param="slidesPlaylistRatio" value="7fr 3fr" class="sa__text-input" />
                        </div>
                    </>
                })
            } else {
                None
            }}
            <div class="sa__actions">
                <button data-role="save" class="settings__button settings__button--primary">"Save"</button>
                <button data-role="reset" class="settings__button settings__button--ghost">"Reset to Defaults"</button>
            </div>
        </section>
    }
}

fn slider_row(
    param: &str,
    label: &str,
    unit: &str,
    min: f32,
    max: f32,
    step: f32,
) -> impl IntoView {
    let min_str = format_number(min);
    let max_str = format_number(max);
    let step_str = format_number(step);
    let label_with_unit = if unit.is_empty() {
        label.to_string()
    } else {
        format!("{label} ({unit})")
    };

    view! {
        <div class="sa__slider-row">
            <label class="sa__slider-label">{label_with_unit}</label>
            <input
                type="range"
                data-param={param.to_string()}
                min={min_str.clone()}
                max={max_str.clone()}
                step={step_str.clone()}
                class="sa__slider"
            />
            <span data-role="value-display" class="sa__value-display">"0"</span>
        </div>
    }
}

fn format_number(value: f32) -> String {
    if value == value.floor() {
        format!("{}", value as i32)
    } else {
        format!("{value}")
    }
}

const STAGE_SETTINGS_STYLES: &str = r#"
.sa__tabs { display: flex; gap: 0.5rem; margin-bottom: 1.5rem; }
.sa__tab { padding: 0.6rem 1.2rem; border: 1px solid #334155; border-radius: 0.5rem; background: transparent; color: #94a3b8; cursor: pointer; font-size: 0.9rem; transition: all 0.15s; }
.sa__tab[data-active="true"] { background: #1e40af; color: #fff; border-color: #1e40af; }
.sa__tab:hover { border-color: #60a5fa; }
[data-role="layout-section"][data-visible="false"] { display: none; }
.sa__slider-row { display: flex; align-items: center; gap: 1rem; padding: 0.5rem 0; }
.sa__slider-label { min-width: 200px; font-size: 0.9rem; color: #cbd5e1; }
.sa__slider { flex: 1; accent-color: #3b82f6; }
.sa__value-display { min-width: 60px; text-align: right; font-variant-numeric: tabular-nums; color: #e2e8f0; font-size: 0.9rem; }
.sa__text-input { flex: 1; padding: 0.4rem 0.8rem; background: #1e293b; border: 1px solid #334155; border-radius: 0.4rem; color: #e2e8f0; font-size: 0.9rem; }
.sa__actions { display: flex; gap: 0.75rem; margin-top: 1.5rem; padding-top: 1rem; border-top: 1px solid #1e293b; }
"#;
