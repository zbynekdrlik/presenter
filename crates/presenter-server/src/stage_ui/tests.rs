use super::*;
use crate::stage_connections::StageHeartbeatConfig;
use chrono::Utc;
use presenter_core::{StageDisplayLayout, StageDisplaySlide, DEFAULT_STAGE_LAYOUT_CODE};

fn worship_layout() -> StageDisplayLayout {
    StageDisplayLayout::built_in()
        .into_iter()
        .find(|layout| layout.code == DEFAULT_STAGE_LAYOUT_CODE)
        .expect("worship layout")
}

#[test]
fn worship_stage_cleared_snapshot_has_no_placeholders() {
    let now = Utc::now();
    let snapshot = presenter_core::StageDisplaySnapshot::new(
        worship_layout(),
        now,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        presenter_core::timer::TimersOverview::demo(now),
        None,
        None,
        None,
        None,
        None,
        None,
    );

    let html = render_stage_display(snapshot, StageHeartbeatConfig::default_values()).0;
    assert!(!html.contains("No next slide"));
    assert!(!html.contains("No active slide"));
}

#[test]
fn worship_stage_preserves_line_breaks() {
    let now = Utc::now();
    let layout = worship_layout();
    let slide = StageDisplaySlide {
        main: "Line A\nLine B".to_string(),
        translation: String::new(),
        stage: String::new(),
        group: Some("Verse".to_string()),
    };
    let snapshot = presenter_core::StageDisplaySnapshot::new(
        layout,
        now,
        Some(presenter_core::PresentationId::new()),
        Some("Sample".into()),
        None,
        Some("Sample Song".into()),
        Some(presenter_core::SlideId::new()),
        Some(slide),
        None,
        None,
        presenter_core::timer::TimersOverview::demo(now),
        None,
        None,
        None,
        None,
        None,
        None,
    );

    let html = render_stage_display(snapshot, StageHeartbeatConfig::default_values()).0;
    assert!(html.contains("Line A\nLine B"));
    assert!(html.contains("Verse"));
}

#[test]
fn stage_status_overlay_is_rendered() {
    let now = Utc::now();
    let snapshot = presenter_core::StageDisplaySnapshot::new(
        worship_layout(),
        now,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        presenter_core::timer::TimersOverview::demo(now),
        None,
        None,
        None,
        None,
        None,
        None,
    );

    let html = render_stage_display(snapshot, StageHeartbeatConfig::default_values()).0;
    assert!(html.contains("id=\"stage-status-bar\""));
    assert!(html.contains("id=\"stage-clock\""));
    assert!(html.contains("id=\"stage-live\""));
    assert!(html.contains("id=\"stage-status\""));
    assert!(html.contains("id=\"stage-status-connection\""));
    assert!(html.contains("id=\"stage-status-latency\""));
    assert!(html.contains("Connecting"));
}
