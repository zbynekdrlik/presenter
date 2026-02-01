use presenter_core::TimerState;
use serde::Serialize;

/// Serialize a value to JSON and escape `</script>` tags for safe embedding
/// in HTML `<script>` blocks.
pub fn json_safe<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</script>", r"<\/script>")
}

/// Escape `</script>` tags in a pre-serialized JSON string for safe HTML embedding.
pub fn escape_script_tag(json: &str) -> String {
    json.replace("</script>", r"<\/script>")
}

pub fn format_timer_state(state: TimerState) -> &'static str {
    match state {
        TimerState::Idle => "Idle",
        TimerState::Running => "Running",
        TimerState::Paused => "Paused",
        TimerState::Completed => "Completed",
    }
}

pub fn format_seconds(total_seconds: i64) -> String {
    let total = total_seconds.max(0);
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

pub fn format_seconds_compact(total_seconds: i64) -> String {
    let total = total_seconds.max(0);
    if total < 60 {
        total.to_string()
    } else {
        let minutes = total / 60;
        let seconds = total % 60;
        format!("{minutes:02}:{seconds:02}")
    }
}
