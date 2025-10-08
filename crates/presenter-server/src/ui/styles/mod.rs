//! CSS style bundles for Presenter UI pages.

pub const OPERATOR: &str = r#"
:root {
    --operator-bg: #f5f6f8;
    --operator-panel: #ffffff;
    --operator-border: #d7d9e0;
    --operator-text: #191a1d;
    --operator-muted: #6b6f7b;
    --operator-accent: #3b7cff;
    --operator-accent-dark: #2554c1;
    --operator-radius: 12px;
    --shadow-soft: 0 12px 28px rgba(15, 23, 42, 0.08);
    --shadow-inner: inset 0 0 0 1px rgba(15, 23, 42, 0.04);
}

.sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
}

body.operator {
    margin: 0;
    min-height: 100vh;
    height: 100vh;
    display: flex;
    flex-direction: column;
    font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    background: var(--operator-bg);
    color: var(--operator-text);
    overflow: hidden;
    --operator-line-limit-ch: 32;
    --operator-line-line-height: 1.35;
}

.operator__header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1rem 1.5rem;
    background: linear-gradient(90deg, #111827, #1f2937);
    color: #ffffff;
    box-shadow: var(--shadow-soft);
    position: sticky;
    top: 0;
    z-index: 10;
}

.operator__header h1 {
    margin: 0;
    font-size: 1.25rem;
    font-weight: 600;
}

.operator__header-left {
    display: flex;
    align-items: center;
    gap: 1.5rem;
}

.operator__header-center {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    position: relative;
    margin: 0 1.5rem;
}

.operator__search {
    width: min(100%, 420px);
    background: rgba(255, 255, 255, 0.12);
    border-radius: 999px;
    display: flex;
    align-items: center;
    padding: 0.35rem 0.75rem;
    gap: 0.5rem;
    border: 1px solid rgba(255, 255, 255, 0.18);
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.05);
}

.operator__search input {
    flex: 1;
    border: none;
    background: transparent;
    color: #ffffff;
    font-size: 0.85rem;
    outline: none;
}

.operator__search input::placeholder {
    color: rgba(255, 255, 255, 0.6);
}

.operator__search button {
    border: none;
    background: transparent;
    color: rgba(255, 255, 255, 0.7);
    font-size: 1rem;
    cursor: pointer;
    padding: 0;
}

.operator__search button:hover {
    color: #ffffff;
}

.operator__search-icon {
    width: 1rem;
    height: 1rem;
    border-radius: 50%;
    border: 2px solid rgba(255, 255, 255, 0.6);
    position: relative;
}

.operator__search [data-role="global-search-clear"] {
    border: none;
    background: transparent;
    color: rgba(248, 250, 252, 0.75);
    cursor: pointer;
    padding: 0;
    margin: 0;
    font-size: 1.1rem;
    line-height: 1;
    transition: color 0.2s ease;
}

.operator__search [data-role="global-search-clear"]:hover {
    color: #ffffff;
}

.operator__search [data-role="global-search-clear"][hidden] {
    display: none;
}

.operator__search-icon::after {
    content: '';
    position: absolute;
    width: 0.55rem;
    height: 0.15rem;
    background: rgba(255, 255, 255, 0.6);
    top: 0.75rem;
    left: 0.55rem;
    transform: rotate(45deg);
    border-radius: 999px;
}

.operator__search-results {
    position: absolute;
    top: 3.2rem;
    left: 50%;
    transform: translateX(-50%);
    width: min(100%, 520px);
    background: #ffffff;
    color: var(--operator-text);
    border-radius: 14px;
    border: 1px solid rgba(15, 23, 42, 0.12);
    box-shadow: 0 18px 38px rgba(15, 23, 42, 0.18);
    max-height: 420px;
    overflow-y: auto;
    display: none;
    z-index: 20;
}

.operator__search-results[data-visible="true"] {
    display: block;
}

.operator__search-group {
    padding: 0.75rem 1rem;
}

.operator__search-group + .operator__search-group {
    border-top: 1px solid rgba(15, 23, 42, 0.08);
}

.operator__search-group h3 {
    margin: 0 0 0.35rem 0;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--operator-muted);
}

.operator__search-result {
    list-style: none;
    margin: 0;
    padding: 0;
}

.operator__search-result button {
    width: 100%;
    border: none;
    background: transparent;
    text-align: left;
    padding: 0.4rem 0.55rem;
    border-radius: 10px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
}

.operator__search-result button:hover {
    background: rgba(59, 124, 255, 0.12);
}

.operator__search-result-title {
    font-weight: 600;
    font-size: 0.9rem;
    color: #0f172a;
}

.operator__search-result-meta {
    font-size: 0.75rem;
    color: var(--operator-muted);
}

.operator__search-result-snippet {
    font-size: 0.75rem;
    color: rgba(15, 23, 42, 0.72);
}

.operator__search-empty {
    margin: 0;
    font-size: 0.8rem;
    color: var(--operator-muted);
}

.operator__view-nav {
    font-size: 0.7rem;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: #cbd5f5;
    opacity: 0.75;
}

.operator__mode-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 999px;
    padding: 0.25rem;
}

.operator__view-nav button,
.operator__mode-toggle button {
    border: none;
    background: transparent;
    color: inherit;
    padding: 0.45rem 0.9rem;
    border-radius: 999px;
    font-size: 0.85rem;
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
}

.operator__view-nav button[data-active="true"],
.operator__mode-toggle button[data-active="true"] {
    background: #ffffff;
    color: #1f2937;
    box-shadow: 0 6px 12px rgba(15, 23, 42, 0.15);
}

.operator__main {
    flex: 1;
    display: flex;
    position: relative;
    overflow: hidden;
    min-height: 0;
}

.operator__worship {
    flex: 1;
    display: flex;
    gap: 1.5rem;
    min-height: 0;
}

.operator__sidebar {
    flex: 0 0 280px;
    background: var(--operator-panel);
    border: 1px solid var(--operator-border);
    border-radius: var(--operator-radius);
    padding: 1rem 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
    overflow-y: auto;
    max-height: calc(100vh - 5.5rem);
    position: sticky;
    top: calc(4.75rem);
}

.operator__group-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 0.75rem;
}

.operator__presentations-header h2 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--operator-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}

.operator__group h2 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--operator-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}

.operator__group-controls {
    display: flex;
    align-items: center;
    gap: 0.45rem;
}

.operator__group-controls [data-role$="create"] {
    font-size: 0.85rem;
    padding: 0.3rem 0.7rem;
    border-radius: 999px;
    border: none;
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-accent-dark);
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
}

.operator__group-controls [data-role$="create"]:hover {
    background: rgba(59, 124, 255, 0.28);
    color: #ffffff;
}

.operator__group-count {
    border: 1px solid rgba(59, 124, 255, 0.35);
    background: rgba(59, 124, 255, 0.12);
    color: var(--operator-accent-dark);
    border-radius: 999px;
    padding: 0.25rem 0.65rem;
    font-size: 0.85rem;
    cursor: pointer;
    min-width: 2.5rem;
    text-align: center;
    transition: background 0.2s ease, color 0.2s ease;
}

.operator__group-count:hover {
    background: rgba(59, 124, 255, 0.24);
    color: #ffffff;
}

.operator__group-count[disabled] {
    opacity: 0.55;
    cursor: default;
}

.operator__group-count[data-empty="true"] {
    opacity: 0.6;
}

.operator__list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
}

.operator__list-item {
    display: flex;
    align-items: center;
    gap: 0.35rem;
}

.operator__favorites-empty {
    color: var(--operator-muted);
    font-size: 0.9rem;
    margin: 0.4rem 0 0;
}

.operator__list-button {
    width: 100%;
    text-align: left;
    display: flex;
    align-items: center;
    gap: 0.4rem;
    background: rgba(99, 102, 241, 0.08);
    border: 1px solid transparent;
    border-radius: 10px;
    padding: 0.55rem 0.75rem;
    font-size: 0.9rem;
    color: var(--operator-text);
    cursor: pointer;
    transition: background 0.2s ease, border 0.2s ease;
}

.operator__list-favorite {
    border: none;
    background: transparent;
    color: rgba(59, 124, 255, 0.45);
    font-size: 1rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    cursor: pointer;
    transition: color 0.2s ease, transform 0.2s ease;
}

.operator__list-favorite[aria-pressed="true"] {
    color: #f59e0b;
    transform: scale(1.1);
}

.operator__list-favorite:focus-visible {
    outline: 2px solid rgba(59, 124, 255, 0.6);
    outline-offset: 2px;
}

.operator__list-favorite--inline {
    width: 1.75rem;
    height: 1.75rem;
    font-size: 0.95rem;
    margin-right: 0.25rem;
}

.operator__list-label {
    flex: 1;
}

.operator__list-meta {
    font-size: 0.75rem;
    color: var(--operator-muted);
    background: rgba(59, 124, 255, 0.16);
    border-radius: 999px;
    padding: 0.1rem 0.4rem;
}

.operator__list-button:hover {
    border-color: rgba(59, 124, 255, 0.45);
}

.operator__list-button[data-active="true"] {
    background: rgba(59, 124, 255, 0.18);
    border-color: rgba(59, 124, 255, 0.6);
    font-weight: 600;
}

.operator__list-row {
    display: flex;
    align-items: center;
    gap: 0.35rem;
}

.operator__list-row--modal {
    padding: 0.1rem 0;
}

.operator__list-row > .operator__list-button {
    flex: 1;
}

.operator__list-actions {
    display: flex;
    gap: 0.25rem;
    align-items: center;
}

.operator__list-action {
    border: 1px solid transparent;
    border-radius: 8px;
    background: rgba(148, 163, 184, 0.12);
    color: var(--operator-muted);
    font-size: 0.75rem;
    padding: 0.35rem 0.55rem;
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
}

.operator__list-action--icon {
    width: 2.1rem;
    height: 2.1rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    font-size: 1rem;
}

.operator__list-action:hover {
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-text);
}

.operator__list-action--danger {
    background: rgba(239, 68, 68, 0.12);
    color: rgb(239, 68, 68);
}

.operator__list-action--danger:hover {
    background: rgba(239, 68, 68, 0.24);
    color: rgb(248, 113, 113);
}

.operator__list-action--menu {
    color: rgba(100, 116, 139, 0.9);
    background: transparent;
}

.operator__list-action--menu:hover {
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-accent-dark);
}

.operator__playlist-modal-body ul {
    list-style: none;
    margin: 0;
    padding: 0;
}

.operator__playlist-modal-body li + li {
    margin-top: 0.4rem;
}

.operator__workspace {
    flex: 1;
    display: flex;
    gap: 1.5rem;
    padding: 0;
    overflow: hidden;
    min-height: 0;
}

.operator__presentations {
    flex: 0 0 320px;
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    border: 1px solid var(--operator-border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
}

.operator__presentations header {
    padding: 0.9rem 1rem;
    border-bottom: 1px solid rgba(15, 23, 42, 0.06);
}

.operator__presentation-list {
    list-style: none;
    margin: 0;
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    overflow-y: auto;
}

.operator__presentation-list[data-dropzone="append"] {
    background: rgba(59, 124, 255, 0.08);
    outline: 2px dashed rgba(59, 124, 255, 0.5);
    outline-offset: -6px;
}

.operator__catalog-bottom[data-dropzone="append"] {
    background: rgba(59, 124, 255, 0.04);
}

.operator__presentation-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.65rem;
    background: rgba(15, 23, 42, 0.05);
    border-radius: 10px;
    padding: 0.55rem 0.75rem;
    border: 1px solid transparent;
    cursor: pointer;
    transition: background 0.2s ease, border 0.2s ease;
}

.operator__presentation-item[data-drop-position] {
    position: relative;
}

.operator__presentation-item[data-drop-position="before"]::before,
.operator__presentation-item[data-drop-position="after"]::after {
    content: '';
    position: absolute;
    left: 12px;
    right: 12px;
    border-top: 3px solid rgba(59, 124, 255, 0.85);
    border-radius: 999px;
    pointer-events: none;
}

.operator__presentation-item[data-drop-position="before"]::before {
    top: -6px;
}

.operator__presentation-item[data-drop-position="after"]::after {
    bottom: -6px;
}

.operator__presentation-item.is-active {
    background: rgba(59, 124, 255, 0.2);
    border-color: rgba(59, 124, 255, 0.5);
}

.operator__presentation-item.is-stage-active {
    box-shadow: 0 0 0 2px rgba(59, 124, 255, 0.35);
}

.operator__presentation-meta {
    font-size: 0.75rem;
    color: var(--operator-muted);
    margin-left: auto;
    margin-right: 0.35rem;
}

.operator__presentation-actions {
    display: inline-flex;
    gap: 0.35rem;
}

.operator__presentation-actions button {
    border: none;
    background: rgba(15, 23, 42, 0.12);
    color: var(--operator-muted);
    border-radius: 999px;
    padding: 0.1rem 0.45rem;
    cursor: pointer;
}

.operator__presentation-actions button:hover {
    background: rgba(59, 124, 255, 0.2);
    color: var(--operator-accent-dark);
}

.operator__slides-panel {
    flex: 1;
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    border: 1px solid var(--operator-border);
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
}

.operator__slides-toolbar {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid rgba(15, 23, 42, 0.06);
}

.operator__line-limit {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.78rem;
    color: var(--operator-muted);
    transition: opacity 0.2s ease;
}

.operator__line-limit[hidden] {
    display: none !important;
}

.operator__line-limit input {
    width: 3.5rem;
    border-radius: 8px;
    border: 1px solid rgba(15, 23, 42, 0.2);
    padding: 0.35rem 0.45rem;
    font-size: 0.85rem;
    text-align: center;
}

.operator__line-limit[data-disabled="true"] {
    opacity: 0.35;
}

.operator__line-limit[data-disabled="true"] input {
    pointer-events: none;
}

body.operator[data-mode="live"] .operator__line-limit {
    display: none !important;
}

.operator__slides-actions button {
    border: none;
    border-radius: 8px;
    padding: 0.45rem 0.85rem;
    background: var(--operator-accent);
    color: #ffffff;
    font-weight: 500;
    cursor: pointer;
    box-shadow: 0 10px 18px rgba(59, 124, 255, 0.28);
}

.operator__slides-clear:hover {
    background: #dc2626;
}

.operator__header-right {
    display: flex;
    align-items: center;
    gap: 1.5rem;
}

.operator__stage-preview {
    position: relative;
    display: inline-flex;
    align-items: stretch;
    gap: 1rem;
    padding: 0.65rem 1rem;
    background: #101828;
    border: 1px solid rgba(148, 163, 184, 0.25);
    color: #f8fafc;
    min-width: 0;
    border-radius: 14px;
    box-shadow: inset 0 0 0 1px rgba(15, 23, 42, 0.25);
}

.operator__stage-preview[data-active="false"] {
    opacity: 0.6;
}

.operator__stage-monitor {
    position: absolute;
    right: 0.35rem;
    bottom: 0.25rem;
    padding: 0;
    border: none;
    background: none;
    color: #e2e8f0;
    font-size: 0.78rem;
    font-weight: 700;
    display: inline-flex;
    align-items: baseline;
    gap: 0.2rem;
    cursor: pointer;
    font-variant-numeric: tabular-nums;
    text-shadow: 0 0 6px rgba(15, 23, 42, 0.85);
}

.operator__stage-monitor:hover {
    color: #38bdf8;
}

.operator__stage-monitor:focus-visible {
    outline: 2px solid rgba(56, 189, 248, 0.65);
    outline-offset: 2px;
}

.operator__stage-monitor--alert {
    color: #f87171;
}

.operator__stage-monitor-separator {
    opacity: 0.6;
}

.operator__stage-monitor-count {
    font-variant-numeric: tabular-nums;
    min-width: 1.4ch;
    text-align: right;
    display: inline-block;
}

.operator__stage-monitor-count--connected {
    color: #4ade80;
}

.operator__stage-monitor-count--issues {
    color: #64748b;
    transition: color 0.2s ease;
}

.operator__stage-monitor--alert .operator__stage-monitor-count--issues {
    color: #f87171;
    font-size: 1.15rem;
    font-weight: 800;
    animation: operatorStageMonitorPulse 1s ease-in-out infinite;
    text-shadow: 0 0 8px rgba(248, 113, 113, 0.45);
}

@keyframes operatorStageMonitorPulse {
    0%, 100% {
        opacity: 1;
    }
    50% {
        opacity: 0.35;
    }
}

.operator__stage-preview-stack {
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
    gap: 0.5rem;
    min-width: 12rem;
    align-items: center;
}

.operator__stage-preview-song {
    font-size: 0.82rem;
    font-weight: 400;
    letter-spacing: 0.01em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
    text-align: center;
}

.operator__stage-preview-actions {
    display: flex;
    gap: 0.5rem;
    justify-content: center;
}

.operator__stage-toggle {
    border: 1px solid rgba(148, 163, 184, 0.35);
    border-radius: 8px;
    background: rgba(15, 23, 42, 0.6);
    color: #f1f5f9;
    padding: 0.35rem 0.7rem;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.2s ease, border-color 0.2s ease;
}

.operator__stage-toggle[data-state="off"] {
    background: rgba(15, 23, 42, 0.25);
    color: rgba(226, 232, 240, 0.75);
    border-color: rgba(148, 163, 184, 0.25);
}

.operator__stage-toggle:disabled {
    opacity: 0.55;
    cursor: not-allowed;
}

.operator__stage-preview-panel {
    width: 180px;
    min-height: 70px;
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    font-size: 0.95rem;
    line-height: 1.3;
    padding: 0.35rem 0.5rem;
    background: rgba(15, 23, 42, 0.82);
    border: 1px solid rgba(148, 163, 184, 0.3);
    border-radius: 10px;
}

.operator__stage-preview-panel--current {
    background: rgba(59, 124, 255, 0.28);
    font-weight: 600;
}

.operator__stage-preview-panel--next {
    min-height: 3.5rem;
    font-size: 0.82rem;
    padding: 0.45rem 0.6rem;
}

.operator__clear-button {
    position: absolute;
    top: -0.45rem;
    right: -0.45rem;
    width: 2.1rem;
    height: 2.1rem;
    border-radius: 999px;
    border: 1px solid rgba(148, 163, 184, 0.45);
    background: rgba(15, 23, 42, 0.92);
    color: rgba(226, 232, 240, 0.92);
    font-size: 1.1rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: background 0.2s ease, transform 0.2s ease;
}

.operator__clear-button:hover {
    background: rgba(59, 124, 255, 0.6);
    transform: translateY(-1px);
}

.operator__clear-button[disabled] {
    opacity: 0.45;
    cursor: default;
    transform: none;
}

.operator__mode-toggle {
    display: inline-flex;
    flex-direction: column;
    align-items: stretch;
    gap: 0.4rem;
    background: rgba(15, 23, 42, 0.12);
    padding: 0.45rem 0.5rem;
    border-radius: 18px;
}

.operator__mode-toggle button {
    border: none;
    background: transparent;
    color: rgba(226, 232, 240, 0.75);
    padding: 0.35rem 1.1rem;
    border-radius: 12px;
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
    text-transform: uppercase;
    font-size: 0.75rem;
    letter-spacing: 0.08em;
}

.operator__mode-toggle button[data-active="true"] {
    background: rgba(59, 124, 255, 0.25);
    color: #ffffff;
}

.operator__slides-add {
    border: none;
    border-radius: 8px;
    padding: 0.35rem 0.75rem;
    background: var(--operator-accent);
    color: #ffffff;
    font-weight: 600;
    cursor: pointer;
    box-shadow: 0 10px 18px rgba(59, 124, 255, 0.28);
    transition: background 0.2s ease;
}

.operator__slides-add:hover {
    background: var(--operator-accent-dark);
}

.operator__group-count--static {
    cursor: default;
    border: 1px solid rgba(59, 124, 255, 0.2);
}

.operator__group-count--static:hover {
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-accent-dark);
}

.operator__slides {
    flex: 1;
    overflow-y: auto;
    padding: 0.35rem;
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 0.9rem;
    min-height: 0;
}

.operator__slide-card {
    background: #ffffff;
    border-radius: 12px;
    border: 1px solid rgba(15, 23, 42, 0.08);
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    box-shadow: var(--shadow-inner);
    transition: border-color 0.2s ease, box-shadow 0.2s ease;
}

.operator__slide-card.is-active {
    border-color: rgba(59, 124, 255, 0.6);
    box-shadow: 0 0 0 3px rgba(59, 124, 255, 0.18);
}

.operator__slide-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
}

.operator__slide-header-left {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
}

.operator__slide-handle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    border-radius: 0.6rem;
    border: 1px solid rgba(15, 23, 42, 0.12);
    background: rgba(15, 23, 42, 0.04);
    color: var(--operator-muted);
    font-size: 0.95rem;
    cursor: grab;
    transition: background 0.2s ease, border-color 0.2s ease, color 0.2s ease;
}

.operator__slide-handle:hover {
    background: rgba(59, 124, 255, 0.12);
    border-color: rgba(59, 124, 255, 0.35);
    color: var(--operator-accent-dark);
}

.operator__slide-handle:active {
    cursor: grabbing;
    background: rgba(59, 124, 255, 0.2);
}

.operator__slide-index {
    font-size: 0.75rem;
    color: var(--operator-muted);
    font-weight: 500;
}

.operator__slide-warning-dot {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin-left: 0.35rem;
    font-size: 0.7rem;
    color: #fb923c;
}

.operator__slide-controls {
    display: inline-flex;
    gap: 0.35rem;
}

.operator__slide-controls button {
    border: none;
    background: rgba(15, 23, 42, 0.06);
    color: var(--operator-muted);
    padding: 0.35rem 0.55rem;
    border-radius: 8px;
    cursor: pointer;
    font-size: 0.75rem;
}

.operator__slide-bodies {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    min-height: 9.5rem;
}

.operator__slide-text {
    white-space: pre-wrap;
    line-height: 1.45;
    text-align: center;
    padding: 0.35rem 0.5rem;
}

.operator__slide-overflow {
    color: #ef4444;
    font-weight: 600;
}

.operator__slide-overflow[data-overflow-line="true"] {
    display: inline;
}

.operator__slide-text--main {
    font-weight: 600;
    font-size: 1rem;
    color: #0f172a;
}

.operator__slide-text--translation {
    color: #1d4ed8;
    font-style: italic;
}

.operator__slide-text--stage {
    color: #0f766e;
    font-family: 'IBM Plex Mono', 'SFMono-Regular', Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace;
    font-size: 0.95rem;
}

.operator__slide-group {
    font-size: 0.68rem;
    color: var(--operator-muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    text-align: center;
    margin-top: auto;
    min-height: 1rem;
    display: flex;
    align-items: flex-end;
    justify-content: center;
}

.operator__slide-group[data-hidden="true"] {
    visibility: hidden;
}

.operator__slide-text.is-warning {
    color: #dc2626;
}

.operator__slide-card[data-warning="true"] {
    box-shadow: 0 0 0 2px rgba(220, 38, 38, 0.12);
}

.operator__slide-warning {
    font-size: 0.75rem;
    color: #dc2626;
    text-align: center;
    margin-top: -0.1rem;
    display: none;
}

.operator__slide-warning[data-visible="true"] {
    display: block;
}

.operator__slide-editor {
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
}

.operator__slide-editor label {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    font-size: 0.8rem;
    color: var(--operator-muted);
}

.operator__slide-editor textarea,
.operator__slide-editor input {
    border-radius: 8px;
    border: 1px solid rgba(15, 23, 42, 0.16);
    padding: 0.4rem 0.55rem;
    font-family: inherit;
    font-size: 0.9rem;
    width: min(100%, calc(var(--operator-line-limit-ch, 32) * 1ch + 1.75rem));
    margin-inline: auto;
}

.operator__slide-editor input::placeholder {
    font-style: italic;
    color: rgba(15, 23, 42, 0.45);
}

.operator__slide-editor textarea {
    line-height: var(--operator-line-line-height, 1.35);
    min-height: calc(var(--operator-line-line-height, 1.35) * 2em + 0.6rem);
    max-height: calc(var(--operator-line-line-height, 1.35) * 2em + 0.6rem);
    height: calc(var(--operator-line-line-height, 1.35) * 2em + 0.6rem);
    overflow-y: auto;
    resize: none;
}



body.operator[data-mode="edit"] .operator__slide-editor textarea,
body.operator[data-mode="edit"] .operator__slide-editor input {
    text-align: center;
}

.operator__slide-editor textarea[data-warning="true"] {
    border-color: #dc2626;
    background: #fef2f2;
}

body.operator[data-mode="live"] .operator__slide-editor {
    display: none;
}

body.operator[data-mode="edit"] .operator__slide-text {
    display: none;
}

body.operator[data-mode="live"] .operator__slide-controls {
    display: none;
}

body.operator[data-mode="edit"] .operator__slide-group {
    display: none;
}

.operator__slide-group {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-accent-dark);
    border-radius: 999px;
    padding: 0.15rem 0.6rem;
    align-self: center;
}

.operator__slide-group.is-inherited {
    background: rgba(15, 23, 42, 0.1);
    color: var(--operator-muted);
}

.operator__panel {
    position: absolute;
    inset: 0;
    background: var(--operator-bg);
    display: none;
    padding: 1.5rem;
}

body.operator[data-view="worship"] [data-view-panel="worship"] {
    display: flex;
}

body.operator[data-view="bible"] [data-view-panel="bible"],
body.operator[data-view="timers"] [data-view-panel="timers"] {
    display: block;
}

body.operator[data-view="settings"] [data-view-panel="settings"] {
    display: block;
}

.operator__panel--settings {
    padding: 0;
}

.operator__settings-frame {
    width: 100%;
    height: 100%;
    border: none;
    border-radius: var(--operator-radius);
    box-shadow: var(--shadow-soft);
    background: #ffffff;
}

.operator__panel--bible iframe {
    width: 100%;
    height: 100%;
    border: none;
    border-radius: var(--operator-radius);
    background: #ffffff;
    box-shadow: var(--shadow-soft);
}

.operator__timers {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    margin-bottom: 1.25rem;
}

.operator__timer-card {
    flex: 1 1 220px;
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    padding: 1rem 1.2rem;
    box-shadow: var(--shadow-soft);
}

.operator__timer-state {
    display: inline-block;
    font-size: 0.75rem;
    color: var(--operator-muted);
    margin-left: 0.5rem;
    padding: 0.125rem 0.5rem;
    border-radius: 999px;
    background: rgba(59, 124, 255, 0.12);
}

.operator__timer-primary {
    margin: 0.35rem 0 0.1rem;
    font-size: 1.75rem;
    font-variant-numeric: tabular-nums;
}

.operator__timer-actions {
    display: flex;
    gap: 1.5rem;
}

.operator__timer-group {
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    padding: 1rem;
    flex: 1 1 240px;
    box-shadow: var(--shadow-soft);
}

.operator__timer-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-bottom: 0.75rem;
}

.operator__timer-field input {
    border-radius: 8px;
    border: 1px solid rgba(15, 23, 42, 0.12);
    padding: 0.5rem 0.6rem;
    font-size: 0.9rem;
    max-width: 160px;
}

.operator__timer-help {
    margin: -0.35rem 0 0.85rem;
    font-size: 0.75rem;
    color: var(--operator-muted);
}

.operator__timer-buttons {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
}

.operator__timer-buttons button {
    flex: 1;
    border-radius: 8px;
    border: none;
    background: rgba(59, 124, 255, 0.1);
    color: var(--operator-accent-dark);
    padding: 0.45rem 0.5rem;
    cursor: pointer;
}

.operator__timer-links {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.75rem;
    flex-wrap: wrap;
}

.operator__timer-links button {
    flex: 1;
    border-radius: 8px;
    border: 1px solid rgba(59, 124, 255, 0.4);
    background: rgba(59, 124, 255, 0.08);
    color: var(--operator-accent-dark);
    padding: 0.45rem 0.5rem;
    cursor: pointer;
}

.operator__toast {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    background: var(--operator-text);
    color: #ffffff;
    padding: 0.75rem 1rem;
    border-radius: 10px;
    box-shadow: var(--shadow-soft);
    opacity: 0;
    transform: translateY(8px);
    transition: opacity 0.2s ease, transform 0.2s ease;
    pointer-events: none;
}

.operator__toast[data-visible="true"] {
    opacity: 1;
    transform: translateY(0);
}

.operator__library-modal,
.operator__playlist-modal {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(15, 23, 42, 0.65);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.2s ease;
    padding: 1.5rem;
    z-index: 1200;
}

.operator__library-modal[data-open="true"],
.operator__playlist-modal[data-open="true"] {
    opacity: 1;
    pointer-events: auto;
}

.operator__library-modal-panel,
.operator__playlist-modal-panel {
    width: min(520px, 90vw);
    max-height: 80vh;
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    border: 1px solid var(--operator-border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: var(--shadow-elevated);
}

.operator__library-modal-header,
.operator__playlist-modal-header {
    padding: 1rem 1.25rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid var(--operator-border);
}

.operator__library-modal-header h3,
.operator__playlist-modal-header h3 {
    margin: 0;
    font-size: 1.05rem;
}

.operator__library-modal-close,
.operator__playlist-modal-close {
    border: none;
    background: transparent;
    color: var(--operator-muted);
    font-size: 1.3rem;
    cursor: pointer;
}

.operator__library-modal-body,
.operator__playlist-modal-body {
    padding: 1rem 1.25rem;
    overflow-y: auto;
}

.operator__library-edit {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(15, 23, 42, 0.65);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.25s ease;
    padding: 1.5rem;
    z-index: 1300;
}

.operator__library-edit[data-open="true"] {
    opacity: 1;
    pointer-events: auto;
}

.operator__library-edit-panel {
    width: min(420px, 92vw);
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    border: 1px solid var(--operator-border);
    box-shadow: var(--shadow-elevated);
}

.operator__library-edit-form {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
    padding: 1.5rem;
}

.operator__library-edit-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.operator__library-edit-header h3 {
    margin: 0;
    font-size: 1.15rem;
}

.operator__library-edit-body {
    display: flex;
    flex-direction: column;
    gap: 1rem;
}

.operator__library-edit-body label {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    font-size: 0.9rem;
    color: var(--operator-muted);
}

.operator__library-edit-body input[type="text"] {
    border-radius: 10px;
    border: 1px solid rgba(15, 23, 42, 0.12);
    padding: 0.6rem 0.7rem;
    font-size: 1rem;
    color: var(--operator-text);
    background: rgba(255, 255, 255, 0.95);
}

.operator__library-edit-body input[type="text"]:focus {
    outline: none;
    border-color: rgba(59, 124, 255, 0.65);
    box-shadow: 0 0 0 3px rgba(59, 124, 255, 0.15);
}

.operator__library-edit-favorite {
    flex-direction: row;
    align-items: center;
    gap: 0.6rem;
    cursor: pointer;
    color: var(--operator-text);
}

.operator__library-edit-favorite input {
    width: 1.15rem;
    height: 1.15rem;
}

.operator__library-edit-body select {
    border-radius: 10px;
    border: 1px solid rgba(15, 23, 42, 0.12);
    padding: 0.55rem 0.7rem;
    font-size: 1rem;
    color: var(--operator-text);
    background: rgba(255, 255, 255, 0.95);
}

.operator__library-edit-body select:focus {
    outline: none;
    border-color: rgba(59, 124, 255, 0.65);
    box-shadow: 0 0 0 3px rgba(59, 124, 255, 0.15);
}

.operator__library-edit-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
}

.operator__library-edit[data-mode="create"] [data-role="library-edit-delete"] {
    display: none;
}

.operator__library-edit-delete {
    border: 1px solid rgba(239, 68, 68, 0.4);
    background: rgba(239, 68, 68, 0.12);
    color: rgb(239, 68, 68);
    border-radius: 8px;
    padding: 0.5rem 0.85rem;
    cursor: pointer;
}

.operator__library-edit-delete:hover {
    background: rgba(239, 68, 68, 0.22);
}

.operator__library-edit-actions {
    display: flex;
    gap: 0.75rem;
}

.operator__library-edit-actions button {
    border: none;
    border-radius: 8px;
    padding: 0.5rem 0.85rem;
    font-weight: 600;
    cursor: pointer;
}

.operator__library-edit-actions [data-role="library-edit-cancel"] {
    background: rgba(148, 163, 184, 0.18);
    color: var(--operator-muted);
}

.operator__library-edit-actions [data-role="library-edit-save"] {
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-accent-dark);
}

.operator__library-edit-form[data-submitting="true"] button {
    pointer-events: none;
    opacity: 0.6;
}

.operator__presentation-list .empty,
.operator__slides .empty {
    color: var(--operator-muted);
    font-size: 0.9rem;
}


.operator__catalog {
    --catalog-top-size: 320px;
    flex: 0 0 320px;
    display: flex;
    flex-direction: column;
    background: var(--operator-panel);
    border: 1px solid var(--operator-border);
    border-radius: var(--operator-radius);
    padding: 1rem 1.25rem;
    gap: 0;
    max-height: calc(100vh - 5.5rem);
    position: sticky;
    top: calc(4.75rem);
}

.operator__catalog-top {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    overflow-y: auto;
    padding-right: 0.35rem;
    flex: 0 0 var(--catalog-top-size);
    min-height: 0;
}

.operator__catalog-resizer {
    flex: 0 0 10px;
    cursor: row-resize;
    margin: 0 -1.25rem;
    background: linear-gradient(90deg, rgba(15, 23, 42, 0) 0%, rgba(15, 23, 42, 0.12) 50%, rgba(15, 23, 42, 0) 100%);
    border-radius: 999px;
}

.operator__catalog-bottom {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    border-top: 1px solid rgba(15, 23, 42, 0.08);
    padding-top: 0.85rem;
    overflow-y: auto;
    padding-right: 0.35rem;
    margin-top: 0.85rem;
}

.operator__presentations-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    margin-bottom: 0.75rem;
}

.operator__presentations-heading h2 {
    margin: 0;
    font-size: 0.95rem;
}

.operator__presentations-count {
    font-size: 0.75rem;
    color: var(--operator-muted);
}

.operator__presentations-actions {
    display: inline-flex;
    gap: 0.45rem;
}

.operator__presentations-actions button {
    font-size: 0.75rem;
    padding: 0.3rem 0.75rem;
    border-radius: 8px;
    border: 1px solid rgba(59, 124, 255, 0.3);
    background: rgba(59, 124, 255, 0.12);
    color: var(--operator-accent-dark);
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease, border 0.2s ease;
}

.operator__presentations-actions button:hover:enabled {
    background: rgba(59, 124, 255, 0.24);
    color: #ffffff;
}

.operator__presentations-actions button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
    border-color: rgba(107, 111, 123, 0.24);
    background: rgba(107, 111, 123, 0.12);
    color: var(--operator-muted);
}

.operator__slides-column {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: var(--operator-panel);
    border: 1px solid var(--operator-border);
    border-radius: var(--operator-radius);
    padding: 1.2rem 1.4rem;
    gap: 1rem;
    min-height: 0;
}

.operator__slides-heading {
    display: flex;
    align-items: stretch;
    justify-content: space-between;
    gap: 1rem;
    width: 100%;
}

.operator__slides {
    flex: 1;
    overflow-y: auto;
    padding: 0.35rem;
    display: grid;
    grid-template-columns: repeat(var(--operator-slide-columns, 3), minmax(0, 1fr));
    gap: 0.9rem;
    align-content: start;
}

.operator__slides[data-size="compact"] {
    --operator-slide-columns: 4;
}

.operator__slides[data-size="medium"] {
    --operator-slide-columns: 3;
}

.operator__slides[data-size="expanded"] {
    --operator-slide-columns: 2;
}

.operator__slide-card {
    padding: 0.85rem;
}

.operator__list-button {
    font-size: 0.85rem;
    padding: 0.5rem 0.7rem;
}

.operator__presentation-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
}

.operator__presentation-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    border: 1px solid rgba(15, 23, 42, 0.08);
    border-radius: 10px;
    padding: 0.55rem 0.75rem;
    background: #ffffff;
    font-size: 0.84rem;
    cursor: pointer;
    transition: border 0.2s ease, box-shadow 0.2s ease, background 0.2s ease;
}

.operator__presentation-item[data-drop-position] {
    position: relative;
}

.operator__presentation-item[data-drop-position="before"]::before,
.operator__presentation-item[data-drop-position="after"]::after {
    content: '';
    position: absolute;
    left: 10px;
    right: 10px;
    border-top: 3px solid rgba(59, 124, 255, 0.85);
    border-radius: 999px;
    pointer-events: none;
}

.operator__presentation-item[data-drop-position="before"]::before {
    top: -6px;
}

.operator__presentation-item[data-drop-position="after"]::after {
    bottom: -6px;
}

.operator__presentation-item.is-active {
    border-color: rgba(59, 124, 255, 0.55);
    box-shadow: 0 0 0 2px rgba(59, 124, 255, 0.2);
}

.operator__presentation-item.is-stage-active {
    background: rgba(59, 124, 255, 0.1);
}

.operator__presentation-item[data-type="separator"] {
    background: rgba(15, 23, 42, 0.06);
    border-style: dashed;
    font-style: italic;
    cursor: default;
}

.operator__presentation-item[data-type="separator"] span {
    opacity: 0.85;
}
.settings__form--osc {
    margin-bottom: 1.5rem;
}

.settings__osc-status {
    border-top: 1px solid rgba(15, 23, 42, 0.08);
    padding-top: 1rem;
}

.settings__status-line {
    display: flex;
    align-items: center;
    margin-bottom: 0.75rem;
}

.settings__status-list {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 0.8rem 1.2rem;
    margin: 0 0 0.75rem 0;
    padding: 0;
}

.settings__status-list dt {
    font-size: 0.75rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: rgba(255, 255, 255, 0.65);
    margin: 0 0 0.2rem 0;
}

.settings__status-list dd {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 500;
}

"#;

pub const TABLET: &str = r#"
body.tablet {
    margin: 0;
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    background: linear-gradient(180deg, #0f172a 0%, #1e293b 100%);
    color: #f8fafc;
}

.tablet-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1.25rem 1.75rem;
    background: rgba(12, 20, 35, 0.9);
    box-shadow: 0 14px 32px rgba(15, 23, 42, 0.55);
}

.tablet-header h1 {
    margin: 0;
    font-size: 1.35rem;
}

.tablet-header__subtitle {
    margin: 0.4rem 0 0;
    font-size: 0.9rem;
    color: #cbd5f5;
}

.tablet-header__modes {
    display: inline-flex;
    gap: 0.4rem;
    background: rgba(148, 163, 184, 0.18);
    padding: 0.25rem;
    border-radius: 999px;
}

.tablet-header__modes button {
    border: none;
    border-radius: 999px;
    padding: 0.45rem 0.9rem;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font-size: 0.85rem;
}

.tablet-header__modes button[data-active="true"] {
    background: #38bdf8;
    color: #0f172a;
    box-shadow: 0 10px 22px rgba(56, 189, 248, 0.4);
}

.tablet-layout {
    flex: 1;
    display: flex;
    overflow: hidden;
}

.tablet-sidebar {
    width: 260px;
    padding: 1.25rem;
    background: rgba(15, 23, 42, 0.72);
    border-right: 1px solid rgba(148, 163, 184, 0.25);
    display: flex;
    flex-direction: column;
    gap: 1.1rem;
}

.tablet-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
}

.tablet-main__header {
    padding: 1.2rem 1.6rem 0.75rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
}

.tablet-main__header h2 {
    margin: 0;
    font-size: 1.05rem;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: #a5b4fc;
}

.tablet-panel h2 {
    margin: 0 0 0.8rem;
    font-size: 0.95rem;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: #94a3b8;
}

.tablet-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
}

.tablet-list-item {
    display: flex;
    align-items: center;
    gap: 0.45rem;
}

.tablet-list-actions {
    display: flex;
    gap: 0.3rem;
}

.tablet-list-action {
    border: 1px solid transparent;
    border-radius: 10px;
    background: rgba(148, 163, 184, 0.22);
    color: #e2e8f0;
    font-size: 0.78rem;
    padding: 0.35rem 0.55rem;
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
}

.tablet-list-action:hover {
    background: rgba(56, 189, 248, 0.28);
    color: #0f172a;
}

.tablet-list-action--danger {
    background: rgba(239, 68, 68, 0.24);
    color: #fecaca;
}

.tablet-list-action--danger:hover {
    background: rgba(239, 68, 68, 0.38);
    color: #0f172a;
}

.tablet-button {
    border: none;
    text-align: left;
    background: rgba(148, 163, 184, 0.18);
    color: #f8fafc;
    padding: 0.55rem 0.75rem;
    border-radius: 10px;
    font-size: 0.95rem;
    cursor: pointer;
    transition: transform 0.2s ease, background 0.2s ease;
    display: flex;
    align-items: center;
    gap: 0.55rem;
}

.tablet-button:hover {
    transform: translateY(-1px);
}

.tablet-button[data-active="true"] {
    background: rgba(56, 189, 248, 0.3);
    box-shadow: 0 12px 26px rgba(56, 189, 248, 0.35);
}

.tablet-button__label {
    flex: 1;
}

.tablet-button__meta {
    font-size: 0.78rem;
    color: #cbd5f5;
    background: rgba(56, 189, 248, 0.25);
    border-radius: 999px;
    padding: 0.05rem 0.45rem;
}

.tablet-slides {
    flex: 1;
    padding: 1.5rem;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 1.25rem;
    overflow-y: auto;
}

.tablet-slides__empty {
    color: #94a3b8;
    font-size: 0.95rem;
}

.tablet-slide {
    background: rgba(15, 23, 42, 0.8);
    border-radius: 16px;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    border: 1px solid transparent;
    cursor: pointer;
    transition: border-color 0.2s ease, box-shadow 0.2s ease, transform 0.2s ease;
}

.tablet-slide:hover {
    transform: translateY(-2px);
}

.tablet-slide.is-active {
    border-color: rgba(56, 189, 248, 0.8);
    box-shadow: 0 14px 30px rgba(56, 189, 248, 0.38);
}

.tablet-slide header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    color: #cbd5f5;
    font-size: 0.85rem;
}

.tablet-slide__group {
    background: rgba(56, 189, 248, 0.18);
    padding: 0.1rem 0.45rem;
    border-radius: 999px;
}

.tablet-slide__body p {
    margin: 0;
    white-space: pre-wrap;
    line-height: 1.45;
}

.tablet-slide__translation {
    color: #93c5fd;
    font-size: 0.9rem;
}

.tablet-editor {
    position: fixed;
    inset: 0;
    background: rgba(12, 20, 35, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.2s ease;
}

.tablet-editor[data-open="true"] {
    opacity: 1;
    pointer-events: auto;
}

.tablet-editor__content {
    background: #0f172a;
    border-radius: 18px;
    width: min(520px, 92vw);
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
    box-shadow: 0 30px 60px rgba(15, 23, 42, 0.6);
}

.tablet-editor__content textarea,
.tablet-editor__content input {
    border-radius: 10px;
    border: 1px solid rgba(148, 163, 184, 0.2);
    padding: 0.7rem 0.8rem;
    font-family: inherit;
    font-size: 0.95rem;
    background: rgba(15, 23, 42, 0.6);
    color: #f8fafc;
}

.tablet-editor__content textarea {
    min-height: 110px;
    resize: vertical;
}

.tablet-editor__error {
    margin: 0;
    color: #fca5a5;
    font-size: 0.85rem;
    display: none;
}

.tablet-editor__error[data-visible="true"] {
    display: block;
}

.tablet-editor__actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
}

.tablet-editor__actions button {
    border: none;
    border-radius: 10px;
    padding: 0.5rem 1.1rem;
    font-size: 0.9rem;
    cursor: pointer;
}

.tablet-editor__actions button[data-role="editor-save"] {
    background: #38bdf8;
    color: #0f172a;
}

.tablet-editor__actions button[data-role="editor-cancel"] {
    background: rgba(148, 163, 184, 0.28);
    color: #f8fafc;
}

.tablet-toast {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    background: rgba(15, 23, 42, 0.88);
    color: #f8fafc;
    padding: 0.7rem 1rem;
    border-radius: 12px;
    box-shadow: 0 12px 26px rgba(15, 23, 42, 0.55);
    opacity: 0;
    transform: translateY(8px);
    transition: opacity 0.2s ease, transform 0.2s ease;
    pointer-events: none;
}

.tablet-toast[data-visible="true"] {
    opacity: 1;
    transform: translateY(0);
}
"#;

pub const BIBLE: &str = r#"
body.bible {
    margin: 0;
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    background: #f8fafc;
    color: #0f172a;
}

.bible__header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1.5rem 2rem;
    background: #0f172a;
    color: #f8fafc;
    box-shadow: 0 14px 24px rgba(15, 23, 42, 0.25);
}

.bible__header h1 {
    margin: 0;
    font-size: 1.4rem;
}

.bible__header p {
    margin: 0.4rem 0 0;
    color: #cbd5f5;
}

.bible__clear {
    border: none;
    background: rgba(99, 102, 241, 0.2);
    color: #eef2ff;
    padding: 0.6rem 1.2rem;
    border-radius: 10px;
    cursor: pointer;
    font-size: 0.95rem;
}

.bible__search {
    display: grid;
    gap: 1rem;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    padding: 1.5rem 2rem;
    background: #eef2ff;
}

.bible__search label {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    font-size: 0.85rem;
    color: #4c51bf;
}

.bible__search select,
.bible__search input {
    border-radius: 10px;
    border: 1px solid rgba(79, 70, 229, 0.35);
    padding: 0.65rem 0.75rem;
    font-size: 0.95rem;
    font-family: inherit;
    background: #ffffff;
}

.bible__search-button {
    align-self: end;
    border: none;
    border-radius: 10px;
    background: #4f46e5;
    color: #eef2ff;
    padding: 0.65rem 1.25rem;
    font-size: 0.95rem;
    cursor: pointer;
    box-shadow: 0 12px 24px rgba(79, 70, 229, 0.3);
}

.bible__active {
    padding: 1.5rem 2rem;
}

.bible__active-card {
    background: #ffffff;
    border-radius: 16px;
    padding: 1.25rem 1.4rem;
    box-shadow: 0 14px 30px rgba(15, 23, 42, 0.12);
}

.bible__active-card--empty {
    background: rgba(248, 250, 252, 0.6);
    border: 1px dashed rgba(148, 163, 184, 0.45);
    box-shadow: none;
}

.bible__active-card header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 1rem;
    margin-bottom: 0.8rem;
}

.bible__active-translation {
    font-size: 0.85rem;
    color: #6366f1;
}

.bible__active-card p {
    margin: 0;
    white-space: pre-wrap;
    line-height: 1.6;
}

.bible__results {
    padding: 0 2rem 2.5rem;
    display: grid;
    gap: 1rem;
}

.bible__result {
    background: #ffffff;
    border-radius: 14px;
    padding: 1rem 1.2rem;
    border: 1px solid rgba(148, 163, 184, 0.25);
    box-shadow: 0 8px 20px rgba(15, 23, 42, 0.08);
}

.bible__result header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 1rem;
    margin-bottom: 0.6rem;
}

.bible__result-actions button {
    border: none;
    background: #38bdf8;
    color: #0f172a;
    border-radius: 8px;
    padding: 0.45rem 0.85rem;
    font-size: 0.85rem;
    cursor: pointer;
}

.bible__result p {
    margin: 0;
    white-space: pre-wrap;
    line-height: 1.5;
}

.bible__empty {
    color: #64748b;
    font-size: 0.95rem;
}

.bible__toast {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    background: #0f172a;
    color: #f8fafc;
    padding: 0.7rem 1rem;
    border-radius: 10px;
    box-shadow: 0 12px 24px rgba(15, 23, 42, 0.28);
    opacity: 0;
    transform: translateY(6px);
    transition: opacity 0.2s ease, transform 0.2s ease;
    pointer-events: none;
}

.bible__toast[data-visible="true"] {
    opacity: 1;
    transform: translateY(0);
}

@media (max-width: 720px) {
    .bible__header {
        flex-direction: column;
        align-items: flex-start;
        gap: 0.75rem;
    }

    .bible__search {
        grid-template-columns: 1fr;
    }
}
"#;

pub const SETTINGS: &str = r#"
:root {
    color-scheme: light;
}

body.settings {
    margin: 0;
    background: #f8fafc;
    color: #0f172a;
    font-family: 'Inter', system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
}

.settings__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 24px 40px;
    background: #ffffff;
    border-bottom: 1px solid #e2e8f0;
}

.settings__header-title h1 {
    margin: 0;
    font-size: 1.75rem;
    font-weight: 600;
}

.settings__header-title p {
    margin: 8px 0 0;
    color: #475569;
}

.settings__header-nav .settings__link {
    text-decoration: none;
    color: #3b82f6;
    font-weight: 600;
}

.settings__header-nav .settings__link:hover {
    text-decoration: underline;
}

.settings__main {
    max-width: 1000px;
    margin: 32px auto;
    padding: 0 32px 48px;
    display: flex;
    flex-direction: column;
    gap: 32px;
}

.settings__card {
    background: #ffffff;
    border-radius: 20px;
    box-shadow: 0 12px 40px rgba(15, 23, 42, 0.08);
    padding: 32px;
    display: flex;
    flex-direction: column;
    gap: 24px;
}

.settings__card-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 24px;
}

.settings__card-header h2 {
    margin: 0;
    font-size: 1.5rem;
    font-weight: 600;
}

.settings__card-header p {
    margin: 8px 0 0;
    color: #475569;
    max-width: 460px;
}

.settings__badge-group {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
}

.settings__badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 48px;
    padding: 6px 12px;
    border-radius: 999px;
    background: #eef2ff;
    color: #312e81;
    font-weight: 600;
    font-size: 0.95rem;
}

.settings__badge-label {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: #64748b;
}

.settings__form {
    display: flex;
    flex-direction: column;
    gap: 20px;
    background: #f8fafc;
    border: 1px solid #e2e8f0;
    border-radius: 16px;
    padding: 24px;
}

.settings__form-header h3 {
    margin: 0;
    font-size: 1.2rem;
    font-weight: 600;
}

.settings__form-header p {
    margin: 6px 0 0;
    color: #64748b;
}

.settings__form-row {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
}

.settings__form-row--single {
    justify-content: flex-start;
}

.settings__form-row label {
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex: 1 1 220px;
    font-weight: 600;
    color: #0f172a;
}

.settings__form-row label span {
    font-size: 0.9rem;
}

.settings__form-row input[type="text"],
.settings__form-row input[type="number"] {
    padding: 10px 12px;
    border: 1px solid #cbd5f5;
    border-radius: 10px;
    font-size: 1rem;
    background: #ffffff;
    color: #0f172a;
    transition: border-color 0.2s ease, box-shadow 0.2s ease;
}

.settings__form-row input:focus {
    outline: none;
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.12);
}

.settings__form-control--small {
    flex: 0 1 120px;
}

.settings__form-checkbox {
    flex: 0 1 auto;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    padding-top: 28px;
    font-weight: 600;
}

.settings__form-checkbox--block {
    padding-top: 0;
}

.settings__form-checkbox input {
    width: 18px;
    height: 18px;
}

.settings__form-actions {
    display: flex;
    gap: 12px;
    align-items: center;
}

.settings__form-checkbox--inline {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    font-weight: 600;
    color: #0f172a;
}

.settings__form-checkbox--inline input {
    width: 18px;
    height: 18px;
}

.settings__button {
    border: none;
    border-radius: 10px;
    font-size: 0.95rem;
    font-weight: 600;
    padding: 10px 18px;
    cursor: pointer;
    transition: transform 0.15s ease, box-shadow 0.15s ease;
}

.settings__button:disabled {
    opacity: 0.6;
    cursor: wait;
}

.settings__button--primary {
    background: #4f46e5;
    color: #ffffff;
    box-shadow: 0 12px 24px rgba(79, 70, 229, 0.22);
}

.settings__button--primary:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 14px 28px rgba(79, 70, 229, 0.26);
}

.settings__button--ghost {
    background: transparent;
    color: #475569;
    border: 1px solid #cbd5f5;
}

.settings__button--ghost:hover {
    background: #e2e8f0;
}

.settings__button--danger {
    background: #dc2626;
    color: #ffffff;
    box-shadow: 0 10px 24px rgba(220, 38, 38, 0.25);
}

.settings__button--danger:hover {
    transform: translateY(-1px);
}

body.settings[data-mode="create"] [data-role="host-reset"] {
    display: none;
}

.settings__form-status {
    min-height: 1.2rem;
    font-size: 0.9rem;
    margin: 0;
}

.settings__form-status[data-state="error"] {
    color: #dc2626;
}

.settings__form-status[data-state="success"] {
    color: #16a34a;
}

.settings__list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
}

.settings__list-item {
    display: flex;
    justify-content: space-between;
    gap: 24px;
    padding: 20px 24px;
    border: 1px solid #e2e8f0;
    border-radius: 16px;
    background: #ffffff;
    box-shadow: 0 10px 24px rgba(15, 23, 42, 0.04);
}

.settings__list-item[data-enabled="false"] {
    opacity: 0.75;
}

.settings__list-primary {
    display: flex;
    flex-direction: column;
    gap: 8px;
}

.settings__list-title {
    display: flex;
    align-items: center;
    gap: 12px;
}

.settings__host-label {
    font-size: 1.1rem;
    font-weight: 600;
}

.settings__status {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 4px 10px;
    border-radius: 999px;
}

.settings__status--enabled {
    background: #dcfcef;
    color: #047857;
}

.settings__status--connecting {
    background: #bfdbfe;
    color: #1d4ed8;
}

.settings__status--disabled {
    background: #fee2e2;
    color: #b91c1c;
}

.settings__status--error {
    background: #fee2e2;
    color: #b91c1c;
}

.settings__list-line {
    margin: 0;
    font-family: 'JetBrains Mono', 'Fira Mono', monospace;
    font-size: 0.95rem;
    color: #0f172a;
}

.settings__list-meta {
    margin: 0;
    color: #64748b;
    font-size: 0.85rem;
}

.settings__list-meta--warning {
    color: #b91c1c;
    font-weight: 600;
}

.settings__list-actions {
    display: flex;
    gap: 10px;
    align-items: flex-start;
}

.settings__list-empty {
    padding: 32px;
    border: 1px dashed #cbd5f5;
    border-radius: 16px;
    text-align: center;
    color: #64748b;
    background: #f8fafc;
    font-weight: 500;
}

.settings__toast {
    position: fixed;
    right: 28px;
    bottom: 28px;
    padding: 14px 20px;
    background: #1e293b;
    color: #f8fafc;
    border-radius: 12px;
    box-shadow: 0 18px 40px rgba(15, 23, 42, 0.35);
    opacity: 0;
    pointer-events: none;
    transform: translateY(20px);
    transition: opacity 0.2s ease, transform 0.2s ease;
}

.settings__toast[data-visible="true"] {
    opacity: 1;
    pointer-events: auto;
    transform: translateY(0);
}

.settings__toast[data-state="success"] {
    background: #0f766e;
}

.settings__toast[data-state="error"] {
    background: #b91c1c;
}

body.settings[data-embedded="true"] {
    background: transparent;
}

body.settings[data-embedded="true"] .settings__header {
    display: none;
}

body.settings[data-embedded="true"] .settings__main {
    margin: 0;
    padding: 16px 24px 32px;
}

body.settings[data-embedded="true"] .settings__card {
    box-shadow: none;
}

.settings__legend {
    margin-top: 1.75rem;
    background: rgba(148, 163, 184, 0.08);
    border: 1px solid rgba(148, 163, 184, 0.2);
    border-radius: 14px;
    padding: 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
}

.settings__legend-note {
    margin: 0;
    color: #cbd5f5;
    font-size: 0.85rem;
    line-height: 1.4;
}

.settings__legend h3 {
    margin: 0;
    font-size: 1.05rem;
    font-weight: 600;
}

.settings__legend dl {
    margin: 0;
    display: grid;
    gap: 0.25rem 1.25rem;
    grid-template-columns: minmax(160px, auto) 1fr;
}

.settings__legend dt {
    font-weight: 600;
    color: #cbd5f5;
}

.settings__legend dd {
    margin: 0;
    color: #e2e8f0;
}

@media (max-width: 840px) {
    .settings__card {
        padding: 24px;
    }

    .settings__card-header {
        flex-direction: column;
        align-items: flex-start;
    }

    .settings__badge-group {
        flex-direction: row;
        align-items: center;
        gap: 12px;
    }

    .settings__list-item {
        flex-direction: column;
    }

    .settings__list-actions {
        align-self: flex-end;
    }
}
"#;

pub const HOME: &str = r#"
body.home {
    margin: 0;
    min-height: 100vh;
    font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    background: linear-gradient(180deg, #111827 0%, #1f2937 100%);
    color: #f8fafc;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 4rem 1.5rem;
}

.home__container {
    width: min(960px, 100%);
    display: flex;
    flex-direction: column;
    gap: 2rem;
}

.home__cta-row {
    display: flex;
    justify-content: flex-start;
}

.home__cta-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0.9rem 1.6rem;
    border-radius: 999px;
    background: #3b82f6;
    color: #0f172a;
    font-weight: 600;
    font-size: 1rem;
    text-decoration: none;
    box-shadow: 0 18px 36px rgba(59, 130, 246, 0.35);
    transition: transform 0.2s ease, box-shadow 0.2s ease;
}

.home__cta-button:hover {
    transform: translateY(-2px);
    box-shadow: 0 24px 42px rgba(59, 130, 246, 0.45);
}

.home__header h1 {
    margin: 0 0 0.5rem;
    font-size: 2rem;
}

.home__header p {
    margin: 0;
    color: #cbd5f5;
}

.home__section h2 {
    margin: 0 0 0.6rem;
    font-size: 1.15rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: #93c5fd;
}

.home__links {
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    margin: 0;
    padding: 0;
}

.home__links a {
    display: inline-flex;
    align-items: center;
    background: rgba(148, 163, 184, 0.18);
    color: #f8fafc;
    padding: 0.75rem 1.2rem;
    border-radius: 12px;
    text-decoration: none;
    font-size: 0.95rem;
    transition: background 0.2s ease, transform 0.2s ease;
}

.home__links a:hover {
    background: rgba(59, 130, 246, 0.3);
    transform: translateY(-2px);
}
"#;

pub const TIMER_OVERLAY: &str = r#"
body.overlay {
    margin: 0;
    min-height: 100vh;
    background: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    color: #f8fafc;
}

.overlay__timer {
    font-size: 12vw;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-align: center;
    text-shadow: 0 12px 40px rgba(15, 23, 42, 0.55);
    font-variant-numeric: tabular-nums;
    font-feature-settings: 'tnum' 1;
    text-rendering: optimizeLegibility;
    -webkit-font-smoothing: antialiased;
    font-smooth: always;
}

@media (max-width: 720px) {
    .overlay__timer {
        font-size: 18vw;
        letter-spacing: 0.06em;
    }
}
"#;
