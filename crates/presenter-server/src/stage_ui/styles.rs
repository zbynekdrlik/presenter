/// CSS styles for the stage display
pub const STAGE_STYLES: &str = r#"
* { box-sizing: border-box; }
body.stage { background: #000; color: #f8fafc; font-family: 'Inter', system-ui, sans-serif; margin: 0; min-height: 100vh; position: relative; overflow: hidden; }
body.stage[data-output-stale="true"] .stage__body { opacity: 0.55; transition: opacity 0.25s ease; }
body.stage[data-live-state="reconnecting"] .stage__status-connection { color: #fbbf24; }
body.stage[data-live-state="disconnected"] .stage__status-connection,
body.stage[data-live-state="error"] .stage__status-connection { color: #f87171; }
.stage__body { position: absolute; inset: 0; }
/* Design-driven absolute positioning for boxes */
.stage__box { position: absolute; display: flex; align-items: center; justify-content: center; overflow: hidden; }
.stage__box p { margin: 0; white-space: pre-wrap; line-height: 1.1; max-width: 100%; }
.stage__box--current-group { left: var(--box-current-group-x, 25%); top: var(--box-current-group-y, 2%); width: var(--box-current-group-w, 50%); height: var(--box-current-group-h, 6%); color: var(--box-current-group-color, #38bdf8); text-align: var(--box-current-group-align, center); font-weight: var(--box-current-group-weight, 700); }
.stage__box--current-group .stage__group { border-radius: 999px; padding: 0.25rem 1rem; letter-spacing: 0.18em; text-transform: uppercase; font-weight: inherit; }
.stage__box--current-slide { left: var(--box-current-slide-x, 2%); top: var(--box-current-slide-y, 10%); width: var(--box-current-slide-w, 96%); height: var(--box-current-slide-h, 45%); color: var(--box-current-slide-color, #f8fafc); text-align: var(--box-current-slide-align, center); font-weight: var(--box-current-slide-weight, 700); }
.stage__box--next-group { left: var(--box-next-group-x, 25%); top: var(--box-next-group-y, 58%); width: var(--box-next-group-w, 50%); height: var(--box-next-group-h, 5%); color: var(--box-next-group-color, #facc15); text-align: var(--box-next-group-align, center); font-weight: var(--box-next-group-weight, 700); }
.stage__box--next-group .stage__group { border-radius: 999px; padding: 0.25rem 1rem; letter-spacing: 0.18em; text-transform: uppercase; font-weight: inherit; }
.stage__box--next-slide { left: var(--box-next-slide-x, 2%); top: var(--box-next-slide-y, 64%); width: var(--box-next-slide-w, 96%); height: var(--box-next-slide-h, 28%); color: var(--box-next-slide-color, #cbd5f5); text-align: var(--box-next-slide-align, center); font-weight: var(--box-next-slide-weight, 700); }
.stage__box--clock { left: var(--box-clock-x, 2%); top: var(--box-clock-y, 92%); width: var(--box-clock-w, 20%); height: var(--box-clock-h, 6%); color: var(--box-clock-color, #38bdf8); text-align: var(--box-clock-align, left); font-weight: var(--box-clock-weight, 700); font-variant-numeric: tabular-nums; }
.stage__box--live-indicator { left: var(--box-live-indicator-x, 40%); top: var(--box-live-indicator-y, 92%); width: var(--box-live-indicator-w, 20%); height: var(--box-live-indicator-h, 6%); text-align: var(--box-live-indicator-align, center); font-weight: var(--box-live-indicator-weight, 700); letter-spacing: 0.12em; text-transform: uppercase; }
.stage__box--live-indicator .stage__live { padding: 0.4rem 1.5rem; border-radius: 999px; background: rgba(34, 197, 94, 0.9); color: #fff; box-shadow: 0 0 20px rgba(34, 197, 94, 0.5); transition: all 0.3s ease; }
.stage__box--live-indicator .stage__live[data-active="true"] { background: rgba(239, 68, 68, 0.95); box-shadow: 0 0 30px rgba(239, 68, 68, 0.7); animation: live-pulse 1.5s ease-in-out infinite; }
.stage__box--connection-status { left: var(--box-connection-status-x, 75%); top: var(--box-connection-status-y, 92%); width: var(--box-connection-status-w, 23%); height: var(--box-connection-status-h, 6%); color: var(--box-connection-status-color, #38bdf8); text-align: var(--box-connection-status-align, right); font-weight: var(--box-connection-status-weight, 600); letter-spacing: 0.1em; text-transform: uppercase; }
.stage__box--countdown-timer { left: var(--box-countdown-timer-x, 10%); top: var(--box-countdown-timer-y, 35%); width: var(--box-countdown-timer-w, 80%); height: var(--box-countdown-timer-h, 30%); color: var(--box-countdown-timer-color, #38bdf8); text-align: var(--box-countdown-timer-align, center); font-weight: var(--box-countdown-timer-weight, 700); letter-spacing: 0.1em; }
.stage__box--preach-timer { left: var(--box-preach-timer-x, 10%); top: var(--box-preach-timer-y, 35%); width: var(--box-preach-timer-w, 80%); height: var(--box-preach-timer-h, 30%); color: var(--box-preach-timer-color, #34d399); text-align: var(--box-preach-timer-align, center); font-weight: var(--box-preach-timer-weight, 700); letter-spacing: 0.1em; }
.stage__box[data-hidden="true"] { display: none; }
/* Timer label */
.stage__timer-label { font-size: 1.5rem; color: #94a3b8; letter-spacing: 0.3em; text-transform: uppercase; margin-top: 0.5rem; }
/* Group styling */
.stage__group[data-hidden="true"] { display: none; }
/* Empty state */
.stage__empty { color: #94a3b8; font-size: 2rem; }
/* Live pulse animation */
@keyframes live-pulse { 0%, 100% { box-shadow: 0 0 30px rgba(239, 68, 68, 0.7), 0 0 60px rgba(239, 68, 68, 0.4); } 50% { box-shadow: 0 0 50px rgba(239, 68, 68, 0.9), 0 0 80px rgba(239, 68, 68, 0.6); } }
/* Connection states */
body.stage[data-live-state="disconnected"] .stage__box--connection-status,
body.stage[data-live-state="error"] .stage__box--connection-status { color: #f87171; }
/* Status latency */
.stage__status-latency { font-variant-numeric: tabular-nums; opacity: 0.7; }
.stage__status-latency[data-visible="false"] { display: none; }
"#;
