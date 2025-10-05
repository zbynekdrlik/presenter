# Companion Button Reference

This table mirrors the default profile export (`presenter-companion-profile.json`). Adjust the payloads or labels if you customise the profile for local workflows.

> **Prerequisite:** enable “Companion WebSocket” under Presenter’s **Settings → Services** and note the assigned port. The Companion module must target the same host/port pair.

## Page 1 – Timers

| Button | Label | Command | Payload | Feedback |
|--------|-------|---------|---------|----------|
| 1 | `Countdown Start` | `timer.start_countdown` | `{}` | Background green when `timer_countdown_state === "running"` |
| 2 | `Countdown Pause` | `timer.pause_countdown` | `{}` | Amber when `timer_countdown_state === "paused"` |
| 3 | `Countdown Reset` | `timer.reset_countdown` | `{}` | Red when `timer_countdown_state === "idle"` |
| 4 | `Countdown +5` | `timer.set_countdown_target` | Macro recalculates the remaining time (+5 min) and sends HH:MM | Updates `timer_countdown_target` and `timer_countdown_remaining_hhmm` |
| 5 | `Countdown -5` | `timer.set_countdown_target` | Macro recalculates remaining time (-5 min) and sends HH:MM | Same as above |
| 6 | `Set HH:MM` | `timer.set_countdown_target` | Macro forwards the button text (HH:MM) | Displays `timer_countdown_target` |
| 7 | `Preach Start` | `timer.start_preach` | `{}` | Green when `timer_preach_state === "running"` |
| 8 | `Preach Reset` | `timer.reset_preach` | `{}` | Red when idle |
| 9 | `Status` | no command | — | Reads `timer_countdown_remaining_hhmm` as text + progress |

## Page 2 – Stage & Bible

| Button | Label | Command | Payload | Feedback |
|--------|-------|---------|---------|----------|
| 1 | `Lyrics SNV` | `stage.layout.worship-snv` | `—` | Highlights green when `stage_layout_code === "worship-snv"` |
| 2 | `Lyrics PP` | `stage.layout.worship-pp` | `—` | Highlights when active |
| 3 | `Timer` | `stage.layout.timer` | `—` | Highlights when active |
| 4 | `Preach` | `stage.layout.preach` | `—` | Highlights when active |
| 5 | `Bible Trigger` | `bible.trigger` | `{ "translation": "KJV", "book": "John", "chapter": 3, "verseStart": 16 }` | Shows `bible_reference` |
| 6 | `Bible Clear` | `bible.clear` | `{}` | Dims when `bible_reference` empty |

The export ships example macros (`macro_presenter_alert`, `macro_presenter_offset_plus`, `macro_presenter_offset_minus`) that manipulate payloads and trigger alerts; review them in Companion if you extend the layout.

> **Note:** The offset buttons rely on a Companion script that reads `timer_countdown_remaining_seconds`, applies the delta, converts it to an `HH:MM` duration, and reissues `timer.set_countdown_target`. Presenter converts that duration into the absolute countdown target internally.
