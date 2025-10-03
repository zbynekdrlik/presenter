# Companion Button Reference

This table mirrors the default profile export (`presenter-companion-profile.json`). Adjust the payloads or labels if you customise the profile for local workflows.

## Page 1 – Timers

| Button | Label | Command | Payload | Feedback |
|--------|-------|---------|---------|----------|
| 1 | `Countdown Start` | `timer.start_countdown` | `{}` | Background green when `timer_countdown_state === "running"` |
| 2 | `Countdown Pause` | `timer.pause_countdown` | `{}` | Amber when `timer_countdown_state === "paused"` |
| 3 | `Countdown Reset` | `timer.reset_countdown` | `{}` | Red when `timer_countdown_state === "idle"` |
| 4 | `Countdown +5` | `timer.set_countdown_target` | Macro computes new ISO timestamp (adds +5 min) | Shows resulting target time via `presenter_target_label` variable |
| 5 | `Countdown -5` | `timer.set_countdown_target` | Macro computes new ISO timestamp (subtracts 5 min) | Same as above |
| 6 | `Set HH:MM` | `timer.set_countdown_target` | Macro converts button text to ISO timestamp | Displays `timer_countdown_target_label` |
| 7 | `Preach Start` | `timer.start_preach` | `{}` | Green when `timer_pteach_state === "running"` |
| 8 | `Preach Reset` | `timer.reset_preach` | `{}` | Red when idle |
| 9 | `Status` | no command | — | Reads `stage_countdown_remaining_seconds` as text + progress |

## Page 2 – Stage & Bible

| Button | Label | Command | Payload | Feedback |
|--------|-------|---------|---------|----------|
| 1 | `Stage Clear` | `stage.set` | `{ "presentationId": null, "currentSlideId": null }` | Turns red when current slide is non-empty |
| 2 | `Next Slide` | `stage.set` | `{ "presentationId": $(presenter:active_presentation_id), "currentSlideId": $(presenter:next_slide_id) }` | Shows `stage_next_main` |
| 3 | `Blank Outputs` | `stage.set` | `{ "presentationId": null, "currentSlideId": null, "blank": true }` | Pulses red via macro when active |
| 4 | `Panic` | macro | macro triggers blank + alert | Flashes red when `live_ws_connected` is false |
| 5 | `Bible Trigger` | `bible.trigger` | `{ "translation": "KJV", "book": "John", "chapter": 3, "verseStart": 16 }` | Shows `bible_reference` |
| 6 | `Bible Clear` | `bible.clear` | `{}` | Dims when `bible_reference` empty |

The export ships example macros (`macro_presenter_alert`, `macro_presenter_offset_plus`, `macro_presenter_offset_minus`) that manipulate payloads and trigger alerts; review them in Companion if you extend the layout.

> **Note:** The offset buttons rely on a small script inside Companion that reads `stage_countdown_remaining_seconds`, converts it to an absolute target, and re-sends `timer.set_countdown_target` with the adjusted ISO timestamp. Presenter does not currently support direct offset payloads; this macro approach keeps compatibility with other clients.
