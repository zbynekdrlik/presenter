use crate::state::AppState;
use axum::response::Html;
use leptos::prelude::*;
use reactive_graph::owner::Owner;

use super::styles;
use super::utils::json_safe;

/// Render the visual stage design editor page.
pub async fn render_stage_design_ui(state: &AppState) -> anyhow::Result<Html<String>> {
    let layouts = ["worship-snv", "worship-pp", "timer", "preach"];
    let mut designs = Vec::new();
    for layout in &layouts {
        let design = state.get_stage_design(layout).await?;
        designs.push(design);
    }
    let designs_json = json_safe(&designs);

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! {
            <StageDesignDocument designs_json=designs_json />
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}

#[component]
fn StageDesignDocument(designs_json: String) -> impl IntoView {
    let script = format!(
        r#"(function() {{
  'use strict';

  const designs = {designs_json};
  const designMap = {{}};
  for (const d of designs) {{
    designMap[d.layoutCode] = d;
  }}

  let activeLayout = 'worship-snv';
  let selectedBoxId = null;
  let isDragging = false;
  let isResizing = false;
  let resizeHandle = null;
  let startX = 0, startY = 0;
  let startBoxX = 0, startBoxY = 0, startBoxW = 0, startBoxH = 0;

  const canvas = () => document.getElementById('design-canvas');

  const boxTypeLabels = {{
    current_slide: 'Current Slide',
    next_slide: 'Next Slide',
    current_group: 'Current Group',
    next_group: 'Next Group',
    playlist: 'Playlist',
    countdown_timer: 'Countdown',
    preach_timer: 'Preach Timer',
    clock: 'Clock',
    live_indicator: 'LIVE',
    connection_status: 'Connection'
  }};

  const defaultSampleContent = {{
    current_slide: 'Hosana, hosana\\nHosana v najvyssich',
    next_slide: 'Hosana, hosana\\nHosana v najvyssich',
    current_group: 'Verse 1',
    next_group: 'Chorus',
    playlist: '1. Amazing Grace\\n2. How Great Thou Art\\n3. Be Thou My Vision',
    countdown_timer: '00:15:30',
    preach_timer: '00:23:45',
    clock: '10:30:00',
    live_indicator: 'LIVE',
    connection_status: 'Connected'
  }};
  const customSampleContent = {{}};

  const getSampleContent = (boxType) => {{
    return customSampleContent[boxType] ?? defaultSampleContent[boxType] ?? boxTypeLabels[boxType] ?? '';
  }};

  const setSampleContent = (boxType, text) => {{
    customSampleContent[boxType] = text;
    renderCanvas();
    updateLivePreview();
  }};

  const showTab = (code) => {{
    activeLayout = code;
    selectedBoxId = null;
    document.querySelectorAll('[data-role="layout-tab"]').forEach(btn => {{
      btn.dataset.active = btn.dataset.layout === code ? 'true' : 'false';
    }});
    renderCanvas();
    updatePropertiesPanel();
    refreshLivePreview();
  }};

  const getDesign = () => designMap[activeLayout];

  const getBox = (id) => {{
    const design = getDesign();
    return design?.boxes?.find(b => b.id === id);
  }};

  const renderCanvas = () => {{
    const c = canvas();
    if (!c) return;
    const design = getDesign();
    if (!design) return;

    c.innerHTML = '';
    for (const box of design.boxes) {{
      if (!box.visible) continue;
      const isLocked = isStatusBarBox(box.boxType) && activeLayout !== 'worship-snv';
      const el = document.createElement('div');
      el.className = 'sd__box';
      el.dataset.boxId = box.id;
      el.dataset.boxType = box.boxType;
      el.dataset.selected = box.id === selectedBoxId ? 'true' : 'false';
      el.dataset.locked = isLocked ? 'true' : 'false';
      el.style.left = box.x + '%';
      el.style.top = box.y + '%';
      el.style.width = box.width + '%';
      el.style.height = box.height + '%';
      el.style.color = box.textColor;
      el.style.fontWeight = box.fontWeight;
      el.style.textAlign = box.textAlign;
      el.style.zIndex = box.zIndex;

      const content = document.createElement('div');
      content.className = 'sd__box-content';
      content.textContent = getSampleContent(box.boxType);
      el.appendChild(content);

      const label = document.createElement('div');
      label.className = 'sd__box-label';
      label.textContent = (isLocked ? '🔗 ' : '') + (boxTypeLabels[box.boxType] || box.id);
      el.appendChild(label);

      // Resize handles (not shown for locked boxes)
      if (!isLocked) {{
        const handles = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
        for (const h of handles) {{
          const handle = document.createElement('div');
          handle.className = 'sd__resize-handle sd__resize-handle--' + h;
          handle.dataset.handle = h;
          el.appendChild(handle);
        }}
      }}

      el.addEventListener('pointerdown', onBoxPointerDown);
      c.appendChild(el);
    }}
  }};

  const onBoxPointerDown = (e) => {{
    const boxEl = e.target.closest('.sd__box');
    if (!boxEl) return;

    const handle = e.target.dataset.handle;
    const boxId = boxEl.dataset.boxId;
    const box = getBox(boxId);
    if (!box) return;

    selectedBoxId = boxId;
    renderCanvas();
    updatePropertiesPanel();

    // Don't allow dragging/resizing locked status bar boxes
    const isLockedStatusBar = isStatusBarBox(box.boxType) && activeLayout !== 'worship-snv';
    if (isLockedStatusBar) return;

    if (handle) {{
      isResizing = true;
      resizeHandle = handle;
    }} else if (!e.target.classList.contains('sd__resize-handle')) {{
      isDragging = true;
    }} else {{
      return;
    }}

    startX = e.clientX;
    startY = e.clientY;
    startBoxX = box.x;
    startBoxY = box.y;
    startBoxW = box.width;
    startBoxH = box.height;

    e.preventDefault();
    e.stopPropagation();
    document.addEventListener('pointermove', onPointerMove);
    document.addEventListener('pointerup', onPointerUp);
  }};

  const onPointerMove = (e) => {{
    if (!isDragging && !isResizing) return;

    const c = canvas();
    if (!c) return;
    const rect = c.getBoundingClientRect();
    const deltaXPct = ((e.clientX - startX) / rect.width) * 100;
    const deltaYPct = ((e.clientY - startY) / rect.height) * 100;

    const box = getBox(selectedBoxId);
    if (!box) return;

    if (isDragging) {{
      box.x = Math.max(0, Math.min(100 - box.width, startBoxX + deltaXPct));
      box.y = Math.max(0, Math.min(100 - box.height, startBoxY + deltaYPct));
    }} else if (isResizing) {{
      const h = resizeHandle;
      let newX = startBoxX, newY = startBoxY, newW = startBoxW, newH = startBoxH;

      if (h.includes('w')) {{
        newX = Math.max(0, Math.min(startBoxX + startBoxW - 5, startBoxX + deltaXPct));
        newW = startBoxW - (newX - startBoxX);
      }}
      if (h.includes('e')) {{
        newW = Math.max(5, Math.min(100 - startBoxX, startBoxW + deltaXPct));
      }}
      if (h.includes('n')) {{
        newY = Math.max(0, Math.min(startBoxY + startBoxH - 5, startBoxY + deltaYPct));
        newH = startBoxH - (newY - startBoxY);
      }}
      if (h.includes('s')) {{
        newH = Math.max(5, Math.min(100 - startBoxY, startBoxH + deltaYPct));
      }}

      box.x = newX;
      box.y = newY;
      box.width = newW;
      box.height = newH;
    }}

    renderCanvas();
    updatePropertiesPanel();
  }};

  const onPointerUp = () => {{
    if (isDragging || isResizing) {{
      updateLivePreview();
    }}
    isDragging = false;
    isResizing = false;
    resizeHandle = null;
    document.removeEventListener('pointermove', onPointerMove);
    document.removeEventListener('pointerup', onPointerUp);
  }};

  const STATUS_BAR_TYPES = ['clock', 'live_indicator', 'connection_status'];
  const isStatusBarBox = (boxType) => STATUS_BAR_TYPES.includes(boxType);

  const updatePropertiesPanel = () => {{
    const panel = document.getElementById('properties-panel');
    if (!panel) return;

    if (!selectedBoxId) {{
      panel.innerHTML = '<p class="sd__hint">Select a box to edit its properties</p>';
      return;
    }}

    const box = getBox(selectedBoxId);
    if (!box) {{
      panel.innerHTML = '<p class="sd__hint">Box not found</p>';
      return;
    }}

    const isStatusBar = isStatusBarBox(box.boxType);
    const isLockedStatusBar = isStatusBar && activeLayout !== 'worship-snv';
    const disabledAttr = isLockedStatusBar ? 'disabled' : '';

    const currentSampleText = getSampleContent(box.boxType).replace(/\\n/g, '\\n');
    const statusBarNote = isLockedStatusBar ? `
      <div class="sd__status-bar-note">
        <span class="sd__note-icon">&#x1F517;</span>
        <span>Synced from worship-snv. Edit there to change.</span>
      </div>
    ` : '';

    panel.innerHTML = `
      <div class="sd__prop-header">${{boxTypeLabels[box.boxType] || box.id}}</div>
      ${{statusBarNote}}
      <div class="sd__prop-group">
        <label class="sd__prop-group-label">Sample Text</label>
        <textarea data-sample-text="${{box.boxType}}" class="sd__sample-text" rows="3">${{currentSampleText}}</textarea>
      </div>
      <div class="sd__prop-row">
        <label>X</label>
        <input type="number" data-prop="x" value="${{box.x.toFixed(1)}}" step="0.5" min="0" max="100" ${{disabledAttr}} />
        <span>%</span>
      </div>
      <div class="sd__prop-row">
        <label>Y</label>
        <input type="number" data-prop="y" value="${{box.y.toFixed(1)}}" step="0.5" min="0" max="100" ${{disabledAttr}} />
        <span>%</span>
      </div>
      <div class="sd__prop-row">
        <label>Width</label>
        <input type="number" data-prop="width" value="${{box.width.toFixed(1)}}" step="0.5" min="1" max="100" ${{disabledAttr}} />
        <span>%</span>
      </div>
      <div class="sd__prop-row">
        <label>Height</label>
        <input type="number" data-prop="height" value="${{box.height.toFixed(1)}}" step="0.5" min="1" max="100" ${{disabledAttr}} />
        <span>%</span>
      </div>
      <div class="sd__prop-row">
        <label>Text Color</label>
        <input type="color" data-prop="textColor" value="${{box.textColor}}" ${{disabledAttr}} />
      </div>
      <div class="sd__prop-row">
        <label>Align</label>
        <select data-prop="textAlign" ${{disabledAttr}}>
          <option value="left" ${{box.textAlign === 'left' ? 'selected' : ''}}>Left</option>
          <option value="center" ${{box.textAlign === 'center' ? 'selected' : ''}}>Center</option>
          <option value="right" ${{box.textAlign === 'right' ? 'selected' : ''}}>Right</option>
        </select>
      </div>
      <div class="sd__prop-row">
        <label>Max Font</label>
        <input type="number" data-prop="maxFontPx" value="${{box.maxFontPx}}" step="1" min="8" max="300" ${{disabledAttr}} />
        <span>px</span>
      </div>
      <div class="sd__prop-row">
        <label>Visible</label>
        <input type="checkbox" data-prop="visible" ${{box.visible ? 'checked' : ''}} ${{disabledAttr}} />
      </div>
    `;

    panel.querySelectorAll('[data-prop]').forEach(input => {{
      input.addEventListener('change', onPropertyChange);
      input.addEventListener('input', onPropertyChange);
    }});

    const sampleTextarea = panel.querySelector('[data-sample-text]');
    if (sampleTextarea) {{
      sampleTextarea.addEventListener('input', (e) => {{
        const boxType = e.target.dataset.sampleText;
        setSampleContent(boxType, e.target.value);
      }});
    }}
  }};

  const onPropertyChange = (e) => {{
    const prop = e.target.dataset.prop;
    const box = getBox(selectedBoxId);
    if (!box || !prop) return;

    let value;
    if (e.target.type === 'checkbox') {{
      value = e.target.checked;
    }} else if (e.target.type === 'number') {{
      value = parseFloat(e.target.value) || 0;
    }} else {{
      value = e.target.value;
    }}

    box[prop] = value;
    renderCanvas();
    updateLivePreview();
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

  const saveDesign = async () => {{
    const design = getDesign();
    if (!design) return;
    try {{
      const response = await fetch(`/stage/design/${{activeLayout}}`, {{
        method: 'PUT',
        headers: {{ 'Content-Type': 'application/json' }},
        body: JSON.stringify(design),
      }});
      if (!response.ok) throw new Error(await response.text());
      showToast('Design saved for ' + activeLayout, false);
    }} catch (err) {{
      showToast('Save failed: ' + err.message, true);
    }}
  }};

  const resetDesign = async () => {{
    try {{
      const response = await fetch(`/stage/design/${{activeLayout}}/reset`, {{
        method: 'POST',
      }});
      if (!response.ok) throw new Error(await response.text());
      const newDesign = await response.json();
      designMap[activeLayout] = newDesign;
      selectedBoxId = null;
      renderCanvas();
      updatePropertiesPanel();
      showToast('Reset ' + activeLayout + ' to defaults', false);
    }} catch (err) {{
      showToast('Reset failed: ' + err.message, true);
    }}
  }};

  const openPreview = () => {{
    window.open('/stage', '_blank');
  }};

  const updateLivePreview = () => {{
    const iframe = document.getElementById('live-preview');
    if (!iframe?.contentWindow) return;
    const design = getDesign();
    if (!design) return;
    // Send design update to iframe
    iframe.contentWindow.postMessage({{ type: 'presenter:design-update', design }}, '*');
  }};

  const refreshLivePreview = () => {{
    const iframe = document.getElementById('live-preview');
    if (!iframe) return;
    iframe.src = '/stage?layout=' + activeLayout + '&t=' + Date.now();
  }};

  // Listen for iframe ready
  window.addEventListener('message', (e) => {{
    if (e.data?.type === 'presenter:stage-ready') {{
      setTimeout(() => updateLivePreview(), 100);
    }}
  }});

  // Wire up tabs
  document.querySelectorAll('[data-role="layout-tab"]').forEach(btn => {{
    btn.addEventListener('click', () => showTab(btn.dataset.layout));
  }});

  // Wire up action buttons
  document.querySelector('[data-role="save"]')?.addEventListener('click', saveDesign);
  document.querySelector('[data-role="reset"]')?.addEventListener('click', resetDesign);
  document.querySelector('[data-role="preview"]')?.addEventListener('click', openPreview);

  // Click on canvas background deselects
  canvas()?.addEventListener('click', (e) => {{
    if (e.target === canvas()) {{
      selectedBoxId = null;
      renderCanvas();
      updatePropertiesPanel();
    }}
  }});

  // Initialize
  showTab('worship-snv');
}})();"#,
        designs_json = designs_json
    );

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>"Stage Design Editor"</title>
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <style>{styles::SETTINGS}</style>
                <style>{STAGE_DESIGN_STYLES}</style>
            </head>
            <body class="settings">
                <header class="settings__header">
                    <div class="settings__header-title">
                        <h1>"Stage Design Editor"</h1>
                        <p>"Drag and resize boxes to design your stage layout visually."</p>
                    </div>
                    <nav class="settings__header-nav">
                        <a href="/ui/settings" class="settings__link">"Back to settings"</a>
                    </nav>
                </header>
                <main class="sd__main">
                    <div class="sd__toolbar">
                        <nav class="sd__tabs" data-role="layout-tabs">
                            <button data-role="layout-tab" data-layout="worship-snv" data-active="true" class="sd__tab">"worship-snv"</button>
                            <button data-role="layout-tab" data-layout="worship-pp" data-active="false" class="sd__tab">"worship-pp"</button>
                            <button data-role="layout-tab" data-layout="timer" data-active="false" class="sd__tab">"timer"</button>
                            <button data-role="layout-tab" data-layout="preach" data-active="false" class="sd__tab">"preach"</button>
                        </nav>
                        <div class="sd__actions">
                            <button data-role="preview" class="settings__button settings__button--ghost">"Preview"</button>
                            <button data-role="reset" class="settings__button settings__button--ghost">"Reset"</button>
                            <button data-role="save" class="settings__button settings__button--primary">"Save"</button>
                        </div>
                    </div>
                    <div class="sd__workspace">
                        <div class="sd__canvas-wrapper">
                            <div class="sd__canvas" id="design-canvas"></div>
                        </div>
                        <aside class="sd__properties" id="properties-panel">
                            <p class="sd__hint">"Select a box to edit its properties"</p>
                        </aside>
                    </div>
                    <div class="sd__live-preview-section">
                        <h3 class="sd__section-title">"Live Preview"</h3>
                        <div class="sd__live-preview-wrapper">
                            <iframe id="live-preview" class="sd__live-preview" src="/stage?layout=worship-snv"></iframe>
                        </div>
                    </div>
                </main>
                <div class="settings__toast" data-role="toast" data-visible="false"></div>
                <script>{script}</script>
            </body>
        </html>
    }
}

const STAGE_DESIGN_STYLES: &str = r#"
.sd__main { display: flex; flex-direction: column; gap: 1rem; padding: 1.5rem; }
.sd__toolbar { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 1rem; }
.sd__tabs { display: flex; gap: 0.5rem; }
.sd__tab { padding: 0.6rem 1.2rem; border: 1px solid #334155; border-radius: 0.5rem; background: transparent; color: #94a3b8; cursor: pointer; font-size: 0.9rem; transition: all 0.15s; }
.sd__tab[data-active="true"] { background: #1e40af; color: #fff; border-color: #1e40af; }
.sd__tab:hover { border-color: #60a5fa; }
.sd__actions { display: flex; gap: 0.5rem; }
.sd__workspace { display: grid; grid-template-columns: 1fr 280px; gap: 1.5rem; }
@media (max-width: 900px) { .sd__workspace { grid-template-columns: 1fr; } }
.sd__canvas-wrapper { background: #0f172a; border-radius: 0.75rem; padding: 1rem; display: flex; align-items: center; justify-content: center; }
.sd__canvas { position: relative; width: 100%; aspect-ratio: 16 / 9; background: #000; border-radius: 0.5rem; overflow: hidden; box-shadow: 0 0 0 2px #1e293b; }
.sd__box { position: absolute; border: 2px solid rgba(255, 255, 255, 0.3); border-radius: 0.25rem; cursor: move; transition: border-color 0.15s; display: flex; flex-direction: column; overflow: hidden; }
.sd__box[data-selected="true"] { border-color: #3b82f6; box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.3); }
.sd__box:hover { border-color: rgba(255, 255, 255, 0.6); }
.sd__box[data-locked="true"] { border-style: dashed; cursor: default; opacity: 0.8; }
.sd__box[data-locked="true"]:hover { border-color: rgba(255, 255, 255, 0.4); }
.sd__box-content { flex: 1; display: flex; align-items: center; justify-content: center; padding: 4px; font-size: 14px; white-space: pre-wrap; text-align: inherit; overflow: hidden; }
.sd__box-label { position: absolute; top: 2px; left: 4px; font-size: 9px; color: rgba(255, 255, 255, 0.5); text-transform: uppercase; letter-spacing: 0.05em; pointer-events: none; }
.sd__resize-handle { position: absolute; width: 10px; height: 10px; background: #3b82f6; border-radius: 2px; opacity: 0; transition: opacity 0.15s; }
.sd__box[data-selected="true"] .sd__resize-handle { opacity: 1; }
.sd__resize-handle--nw { top: -5px; left: -5px; cursor: nw-resize; }
.sd__resize-handle--n { top: -5px; left: 50%; transform: translateX(-50%); cursor: n-resize; }
.sd__resize-handle--ne { top: -5px; right: -5px; cursor: ne-resize; }
.sd__resize-handle--e { top: 50%; right: -5px; transform: translateY(-50%); cursor: e-resize; }
.sd__resize-handle--se { bottom: -5px; right: -5px; cursor: se-resize; }
.sd__resize-handle--s { bottom: -5px; left: 50%; transform: translateX(-50%); cursor: s-resize; }
.sd__resize-handle--sw { bottom: -5px; left: -5px; cursor: sw-resize; }
.sd__resize-handle--w { top: 50%; left: -5px; transform: translateY(-50%); cursor: w-resize; }
.sd__properties { background: #1e293b; border-radius: 0.75rem; padding: 1rem; }
.sd__hint { color: #64748b; font-size: 0.875rem; text-align: center; margin: 2rem 0; }
.sd__prop-header { font-size: 1rem; font-weight: 600; color: #e2e8f0; margin-bottom: 1rem; padding-bottom: 0.5rem; border-bottom: 1px solid #334155; }
.sd__prop-row { display: flex; align-items: center; gap: 0.5rem; margin-bottom: 0.75rem; }
.sd__prop-row label { flex: 0 0 70px; font-size: 0.8rem; color: #94a3b8; }
.sd__prop-row input[type="number"], .sd__prop-row input[type="text"] { flex: 1; padding: 0.4rem 0.6rem; background: #0f172a; border: 1px solid #334155; border-radius: 0.3rem; color: #e2e8f0; font-size: 0.85rem; }
.sd__prop-row input[type="color"] { width: 40px; height: 28px; padding: 0; border: 1px solid #334155; border-radius: 0.3rem; cursor: pointer; }
.sd__prop-row select { flex: 1; padding: 0.4rem 0.6rem; background: #0f172a; border: 1px solid #334155; border-radius: 0.3rem; color: #e2e8f0; font-size: 0.85rem; }
.sd__prop-row input[type="checkbox"] { width: 18px; height: 18px; accent-color: #3b82f6; }
.sd__prop-row span { font-size: 0.8rem; color: #64748b; min-width: 20px; }
.sd__prop-group { margin-bottom: 1rem; }
.sd__prop-group-label { display: block; font-size: 0.8rem; color: #94a3b8; margin-bottom: 0.4rem; }
.sd__sample-text { width: 100%; padding: 0.5rem; background: #0f172a; border: 1px solid #334155; border-radius: 0.3rem; color: #e2e8f0; font-size: 0.85rem; resize: vertical; font-family: inherit; }
.sd__live-preview-section { margin-top: 1.5rem; }
.sd__section-title { font-size: 1rem; font-weight: 600; color: #e2e8f0; margin-bottom: 0.75rem; }
.sd__live-preview-wrapper { background: #0f172a; border-radius: 0.75rem; padding: 1rem; }
.sd__live-preview { width: 100%; aspect-ratio: 16 / 9; border: none; border-radius: 0.5rem; background: #000; }
.sd__status-bar-note { display: flex; align-items: center; gap: 0.5rem; padding: 0.6rem 0.8rem; margin-bottom: 1rem; background: #1e3a5f; border-radius: 0.4rem; font-size: 0.8rem; color: #93c5fd; border: 1px solid #3b82f6; }
.sd__note-icon { font-size: 1rem; }
.sd__properties input:disabled, .sd__properties select:disabled { opacity: 0.5; cursor: not-allowed; }
"#;
