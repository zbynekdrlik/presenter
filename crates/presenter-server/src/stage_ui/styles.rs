/// CSS styles for the stage display
pub const STAGE_STYLES: &str = r#"
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
.stage__status-bar { position: fixed; bottom: 0; left: 0; right: 0; display: flex; align-items: center; justify-content: space-between; padding: 1.5rem 2.5rem; background: linear-gradient(to top, rgba(0, 0, 0, 0.85) 0%, transparent 100%); }
.stage__clock { font-size: 4rem; font-weight: 700; font-variant-numeric: tabular-nums; color: #38bdf8; padding: 0.5rem 1.5rem; background: rgba(15, 23, 42, 0.8); border-radius: 999px; letter-spacing: 0.05em; }
.stage__live { font-size: 2.2rem; font-weight: 700; padding: 0.6rem 2rem; border-radius: 999px; letter-spacing: 0.12em; text-transform: uppercase; transition: all 0.3s ease; background: rgba(34, 197, 94, 0.9); color: #fff; box-shadow: 0 0 20px rgba(34, 197, 94, 0.5), 0 0 40px rgba(34, 197, 94, 0.25); }
.stage__live[data-active="true"] { background: rgba(239, 68, 68, 0.95); color: #fff; box-shadow: 0 0 30px rgba(239, 68, 68, 0.7), 0 0 60px rgba(239, 68, 68, 0.4); animation: live-pulse 1.5s ease-in-out infinite; }
@keyframes live-pulse { 0%, 100% { box-shadow: 0 0 30px rgba(239, 68, 68, 0.7), 0 0 60px rgba(239, 68, 68, 0.4); } 50% { box-shadow: 0 0 50px rgba(239, 68, 68, 0.9), 0 0 80px rgba(239, 68, 68, 0.6); } }
.stage__status { display: inline-flex; align-items: center; gap: 0.75rem; padding: 0.8rem 1.5rem; font-size: 1.3rem; letter-spacing: 0.12em; text-transform: uppercase; background: rgba(15, 23, 42, 0.8); border-radius: 999px; box-shadow: 0 12px 32px -24px rgba(15, 23, 42, 0.95); }
.stage__status span { display: inline-flex; align-items: center; }
.stage__status-connection { color: #38bdf8; font-weight: 600; }
.stage__status-latency { font-variant-numeric: tabular-nums; color: #e2e8f0; min-width: 7ch; white-space: pre; text-align: right; display: inline-flex; justify-content: flex-end; text-transform: none; letter-spacing: normal; }
.stage__status-latency[data-visible="false"] { display: none; }
body.stage[data-live-state="disconnected"] .stage__status-connection,
body.stage[data-live-state="error"] .stage__status-connection { color: #f87171; }
"#;
