//! Operator-header Resolume connection indicator (#564): one colored chip
//! per ENABLED Resolume host, fed by `/integrations/resolume/status` on the
//! same 5s poll cadence the settings-page status cards use (#546). Renders
//! nothing when there are no enabled hosts — a deployment with no Resolume
//! configured (e.g. the dev box) shows a clean absent state, never an empty
//! placeholder or a console error.
//!
//! Placement (#573): mounted in the operator header's TOP brand row, inside
//! `.operator__brand-nav`, immediately after `<SurfaceNav />` — connection
//! status belongs next to the Stage/Camera/Tablet/Timer surface-nav pills,
//! never next to the Stage Output select in `operator__header-right`.

use leptos::prelude::*;

use crate::api::settings::{get_resolume_connection_status, ResolumeConnectionStatusDto};

const RESOLUME_STATUS_REFRESH_MS: u32 = 5_000;
/// #564 design: "red = failing >2 min" — below this, an erroring host still
/// reads yellow ("retrying"), not red ("down").
pub(crate) const FAILING_RED_AFTER_SECS: i64 = 120;

/// Which of the three dot colors a host's chip shows right now. Keyed off
/// the SERVER's authoritative connection state (never re-derived from raw
/// timestamps client-side) plus the server-computed `failing_for_secs`.
pub(crate) fn chip_dot_state(state: &str, failing_for_secs: Option<i64>) -> &'static str {
    match state {
        "connected" => "green",
        "error" if failing_for_secs.unwrap_or(0) >= FAILING_RED_AFTER_SECS => "red",
        "error" => "yellow",
        // "connecting" (first attempt in flight) or anything unrecognised —
        // no confirmed failure yet, but no confirmed success either.
        _ => "yellow",
    }
}

/// Plain-language cause for the tooltip — never a raw wire token like
/// `connect_refused`.
pub(crate) fn error_kind_label(kind: Option<&str>) -> &'static str {
    match kind {
        Some("timeout") => "timed out",
        Some("connect_refused") => "connection refused",
        Some("connect_other") => "could not connect",
        Some("reset") => "connection reset",
        Some("other") => "request failed",
        _ => "not reachable",
    }
}

/// The drift note ("Port drifted: 8090 → 8091 (auto-detected)"), only when a
/// port-drift probe actually found the host somewhere other than its
/// configured port. `None` while dialing the configured port as usual.
pub(crate) fn port_drift_note(configured_port: u16, active_port: Option<u16>) -> Option<String> {
    active_port.map(|p| format!("Port drifted: {configured_port} \u{2192} {p} (auto-detected)"))
}

/// The full tooltip text (native `title` attribute — no custom popover
/// needed for this MVP): the host's FULL label first (the visible chip text
/// ellipsizes past 12ch), then connection cause + retry countdown while
/// erroring, the configured-vs-active port drift note, and any missing
/// composition clips (#563g).
pub(crate) fn chip_tooltip(dto: &ResolumeConnectionStatusDto) -> String {
    let mut lines = vec![dto.label.clone()];
    match dto.state.as_str() {
        "connected" => lines.push("Connected".to_string()),
        "error" => {
            lines.push(format!(
                "Not reachable — {}",
                error_kind_label(dto.last_error_kind.as_deref())
            ));
            if let Some(secs) = dto.next_retry_in_secs {
                lines.push(format!("Retrying in {secs}s"));
            }
        }
        _ => lines.push("Connecting…".to_string()),
    }
    lines.push(format!("Configured port: {}", dto.configured_port));
    if let Some(note) = port_drift_note(dto.configured_port, dto.active_port) {
        lines.push(note);
    }
    if !dto.missing_clips.is_empty() {
        lines.push(format!(
            "Composition missing: {}",
            dto.missing_clips.join(", ")
        ));
    }
    lines.join("\n")
}

#[component]
pub fn ResolumeStatusChips() -> impl IntoView {
    let hosts = RwSignal::new(Vec::<ResolumeConnectionStatusDto>::new());

    let poll = move || {
        leptos::task::spawn_local(async move {
            // A failed poll leaves the last-known chips in place rather than
            // flickering to empty — same "don't blank on a blip" discipline
            // as the settings-page status cards (#546).
            if let Ok(list) = get_resolume_connection_status().await {
                hosts.set(list.into_iter().filter(|h| h.is_enabled).collect());
            }
        });
    };
    poll();

    // `forget()` — the timer dies with page navigation (no client-side
    // router), same as every other operator-header/settings poller. Not
    // `on_cleanup`: `gloo_timers::Interval` is not `Send`, which the host
    // (non-wasm) `cargo test --lib` build of this crate requires.
    let interval = gloo_timers::callback::Interval::new(RESOLUME_STATUS_REFRESH_MS, move || {
        poll();
    });
    interval.forget();

    view! {
        <div class="operator__resolume-chips" data-role="resolume-status-chips">
            <For
                each=move || hosts.get()
                // Keyed on host identity, NOT on the live state — the dot,
                // label and tooltip below read `hosts` through a `Memo` by
                // id, so they update on every poll without destroying and
                // rebuilding the chip (see the presenter-ui skill's
                // "Per-row live state in a <For>" gotcha).
                key=|h: &ResolumeConnectionStatusDto| h.host_id.clone()
                children=move |host: ResolumeConnectionStatusDto| {
                    let host_id = host.host_id.clone();
                    let status = Memo::new(move |_| {
                        hosts.with(|list| list.iter().find(|h| h.host_id == host_id).cloned())
                    });
                    let dot_state = move || {
                        status
                            .get()
                            .map(|h| chip_dot_state(&h.state, h.failing_for_secs))
                            .unwrap_or("yellow")
                    };
                    let dot_class = move || {
                        format!("operator__resolume-dot operator__resolume-dot--{}", dot_state())
                    };
                    // The visible text is the HOST'S NAME (its configured
                    // label) — with several walls configured, "Connected"
                    // alone never says WHICH Resolume the chip is. State
                    // lives in the dot color + tooltip. Read through the
                    // `status` Memo (not a one-shot capture) so a rename
                    // refreshes on the next poll like the dot does.
                    let label = move || status.get().map(|h| h.label).unwrap_or_default();
                    let tooltip = move || status.get().map(|h| chip_tooltip(&h)).unwrap_or_default();
                    view! {
                        <span
                            class="operator__resolume-chip"
                            data-role="resolume-status-chip"
                            data-host-label=label
                            data-state=dot_state
                            title=tooltip
                        >
                            <span class=dot_class aria-hidden="true"></span>
                            <span class="operator__resolume-chip-label">{label}</span>
                        </span>
                    }
                }
            />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dto(state: &str, failing_for_secs: Option<i64>) -> ResolumeConnectionStatusDto {
        ResolumeConnectionStatusDto {
            host_id: "h1".to_string(),
            label: "resolume-pp".to_string(),
            is_enabled: true,
            configured_port: 8090,
            active_port: None,
            port_drifted: false,
            state: state.to_string(),
            last_success: None,
            last_latency_ms: None,
            last_error: None,
            last_error_kind: None,
            consecutive_failures: 0,
            next_retry_in_secs: None,
            failing_for_secs,
            missing_clips: Vec::new(),
        }
    }

    #[test]
    fn connected_is_green() {
        assert_eq!(chip_dot_state("connected", None), "green");
    }

    #[test]
    fn erroring_under_two_minutes_is_yellow_not_red() {
        assert_eq!(chip_dot_state("error", Some(119)), "yellow");
    }

    #[test]
    fn erroring_past_two_minutes_is_red() {
        assert_eq!(chip_dot_state("error", Some(120)), "red");
        assert_eq!(chip_dot_state("error", Some(600)), "red");
    }

    #[test]
    fn an_error_with_no_failing_duration_yet_defaults_to_yellow_not_red() {
        // Freshly transitioned into Error — error_since was just set, so the
        // server hasn't had a chance to report a non-zero failing_for_secs.
        assert_eq!(chip_dot_state("error", None), "yellow");
    }

    #[test]
    fn connecting_is_yellow() {
        assert_eq!(chip_dot_state("connecting", None), "yellow");
    }

    #[test]
    fn an_unrecognised_state_degrades_to_yellow_not_a_panic_or_green() {
        assert_eq!(chip_dot_state("wat", None), "yellow");
    }

    #[test]
    fn no_drift_means_no_port_note() {
        assert_eq!(port_drift_note(8090, None), None);
    }

    #[test]
    fn a_drift_names_both_ports_with_an_auto_detected_label() {
        assert_eq!(
            port_drift_note(8090, Some(8091)),
            Some("Port drifted: 8090 \u{2192} 8091 (auto-detected)".to_string())
        );
    }

    #[test]
    fn error_kind_labels_are_plain_language_never_raw_wire_tokens() {
        assert_eq!(error_kind_label(Some("timeout")), "timed out");
        assert_eq!(
            error_kind_label(Some("connect_refused")),
            "connection refused"
        );
        assert_eq!(error_kind_label(Some("connect_other")), "could not connect");
        assert_eq!(error_kind_label(Some("reset")), "connection reset");
        assert_eq!(error_kind_label(Some("other")), "request failed");
        assert_eq!(error_kind_label(None), "not reachable");
    }

    #[test]
    fn tooltip_for_a_healthy_host_has_no_error_or_drift_lines() {
        let text = chip_tooltip(&dto("connected", None));
        assert!(text.contains("Connected"));
        assert!(!text.contains("Not reachable"));
        assert!(!text.contains("Port drifted"));
    }

    #[test]
    fn tooltip_opens_with_the_full_host_label() {
        // The visible chip text ellipsizes past 12ch — the tooltip is where
        // a long name stays fully readable, so it must lead with the label.
        let text = chip_tooltip(&dto("connected", None));
        assert!(text.starts_with("resolume-pp\n"), "{text}");
    }

    #[test]
    fn tooltip_for_an_erroring_host_names_the_cause_and_retry_countdown() {
        let mut d = dto("error", Some(30));
        d.last_error_kind = Some("connect_refused".to_string());
        d.next_retry_in_secs = Some(8);
        let text = chip_tooltip(&d);
        assert!(text.contains("connection refused"), "{text}");
        assert!(text.contains("Retrying in 8s"), "{text}");
    }

    #[test]
    fn tooltip_includes_the_drift_note_when_active_port_differs() {
        let mut d = dto("connected", None);
        d.active_port = Some(8091);
        let text = chip_tooltip(&d);
        assert!(text.contains("Port drifted: 8090 \u{2192} 8091"), "{text}");
    }

    #[test]
    fn tooltip_lists_missing_clips_when_present() {
        let mut d = dto("connected", None);
        d.missing_clips = vec!["#timer".to_string(), "#song-name".to_string()];
        let text = chip_tooltip(&d);
        assert!(
            text.contains("Composition missing: #timer, #song-name"),
            "{text}"
        );
    }
}
