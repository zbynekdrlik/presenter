//! Stage design models for visual box-based layout configuration.
//!
//! Replaces the abstract slider-based `StageAppearance` with a WYSIWYG approach
//! where content boxes are draggable/resizable on a stage canvas.

use serde::{Deserialize, Serialize};

/// Visual design configuration for a stage display layout.
/// Stores box positions and styling for each element.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDesign {
    pub layout_code: String,
    pub boxes: Vec<StageBox>,
    #[serde(default = "default_background_color")]
    pub background_color: String,
}

fn default_background_color() -> String {
    "#000000".to_string()
}

impl StageDesign {
    /// Create a new stage design for a layout with default boxes.
    pub fn default_for(layout_code: &str) -> Self {
        let boxes = default_boxes_for(layout_code);
        Self {
            layout_code: layout_code.to_string(),
            boxes,
            background_color: default_background_color(),
        }
    }

    /// Find a box by its ID.
    pub fn get_box(&self, id: &str) -> Option<&StageBox> {
        self.boxes.iter().find(|b| b.id == id)
    }

    /// Find a box by its ID mutably.
    pub fn get_box_mut(&mut self, id: &str) -> Option<&mut StageBox> {
        self.boxes.iter_mut().find(|b| b.id == id)
    }
}

/// A content box on the stage display.
/// Position and size are percentages (0-100) of the stage canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageBox {
    pub id: String,
    pub box_type: StageBoxType,
    /// X position as percentage (0-100)
    pub x: f32,
    /// Y position as percentage (0-100)
    pub y: f32,
    /// Width as percentage (0-100)
    pub width: f32,
    /// Height as percentage (0-100)
    pub height: f32,
    /// Text color (CSS color string)
    #[serde(default = "default_text_color")]
    pub text_color: String,
    /// Text alignment
    #[serde(default)]
    pub text_align: TextAlign,
    /// Font weight (100-900)
    #[serde(default = "default_font_weight")]
    pub font_weight: u16,
    /// Minimum font size in pixels
    #[serde(default = "default_min_font_px")]
    pub min_font_px: f32,
    /// Maximum font size in pixels
    #[serde(default = "default_max_font_px")]
    pub max_font_px: f32,
    /// Whether the box is visible
    #[serde(default = "default_visible")]
    pub visible: bool,
    /// Z-index for stacking order
    #[serde(default)]
    pub z_index: i32,
}

fn default_text_color() -> String {
    "#f8fafc".to_string()
}

fn default_font_weight() -> u16 {
    700
}

fn default_min_font_px() -> f32 {
    12.0
}

fn default_max_font_px() -> f32 {
    120.0
}

fn default_visible() -> bool {
    true
}

impl StageBox {
    /// Create a new box with default styling.
    pub fn new(
        id: impl Into<String>,
        box_type: StageBoxType,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Self {
        Self {
            id: id.into(),
            box_type,
            x,
            y,
            width,
            height,
            text_color: default_text_color(),
            text_align: TextAlign::default(),
            font_weight: default_font_weight(),
            min_font_px: default_min_font_px(),
            max_font_px: default_max_font_px(),
            visible: default_visible(),
            z_index: 0,
        }
    }

    /// Create a new box with specific styling.
    #[allow(clippy::too_many_arguments)]
    pub fn with_style(
        id: impl Into<String>,
        box_type: StageBoxType,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        text_color: impl Into<String>,
        text_align: TextAlign,
        font_weight: u16,
        max_font_px: f32,
    ) -> Self {
        Self {
            id: id.into(),
            box_type,
            x,
            y,
            width,
            height,
            text_color: text_color.into(),
            text_align,
            font_weight,
            min_font_px: default_min_font_px(),
            max_font_px,
            visible: true,
            z_index: 0,
        }
    }
}

/// Type of content displayed in a stage box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageBoxType {
    CurrentSlide,
    NextSlide,
    CurrentGroup,
    NextGroup,
    Playlist,
    CountdownTimer,
    PreachTimer,
    Clock,
    LiveIndicator,
    ConnectionStatus,
}

impl StageBoxType {
    /// Get a human-readable label for this box type.
    pub fn label(&self) -> &'static str {
        match self {
            Self::CurrentSlide => "Current Slide",
            Self::NextSlide => "Next Slide",
            Self::CurrentGroup => "Current Group",
            Self::NextGroup => "Next Group",
            Self::Playlist => "Playlist",
            Self::CountdownTimer => "Countdown Timer",
            Self::PreachTimer => "Preach Timer",
            Self::Clock => "Clock",
            Self::LiveIndicator => "Live Indicator",
            Self::ConnectionStatus => "Connection Status",
        }
    }

    /// Check if this box type is part of the shared status bar.
    /// Status bar boxes (Clock, LiveIndicator, ConnectionStatus) are synced
    /// from worship-snv to other layouts.
    pub fn is_status_bar(&self) -> bool {
        matches!(
            self,
            Self::Clock | Self::LiveIndicator | Self::ConnectionStatus
        )
    }
}

/// Text alignment within a stage box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

impl TextAlign {
    pub fn as_css(&self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

/// Generate default boxes for a layout.
fn default_boxes_for(layout_code: &str) -> Vec<StageBox> {
    match layout_code {
        "worship-snv" => worship_snv_default_boxes(),
        "worship-pp" => worship_pp_default_boxes(),
        "timer" => timer_default_boxes(),
        "preach" => preach_default_boxes(),
        _ => worship_snv_default_boxes(),
    }
}

fn worship_snv_default_boxes() -> Vec<StageBox> {
    vec![
        StageBox::with_style(
            "current-group",
            StageBoxType::CurrentGroup,
            25.0,
            2.0,
            50.0,
            6.0,
            "#38bdf8", // cyan
            TextAlign::Center,
            700,
            24.0,
        ),
        StageBox::with_style(
            "current-slide",
            StageBoxType::CurrentSlide,
            2.0,
            10.0,
            96.0,
            45.0,
            "#f8fafc", // white
            TextAlign::Center,
            700,
            120.0,
        ),
        StageBox::with_style(
            "next-group",
            StageBoxType::NextGroup,
            25.0,
            58.0,
            50.0,
            5.0,
            "#facc15", // yellow
            TextAlign::Center,
            700,
            20.0,
        ),
        StageBox::with_style(
            "next-slide",
            StageBoxType::NextSlide,
            2.0,
            64.0,
            96.0,
            28.0,
            "#cbd5f5", // light gray
            TextAlign::Center,
            700,
            80.0,
        ),
        // Status bar elements (bottom of screen)
        StageBox::with_style(
            "clock",
            StageBoxType::Clock,
            2.0,
            92.0,
            20.0,
            6.0,
            "#38bdf8",
            TextAlign::Left,
            700,
            48.0,
        ),
        StageBox::with_style(
            "live-indicator",
            StageBoxType::LiveIndicator,
            40.0,
            92.0,
            20.0,
            6.0,
            "#ef4444",
            TextAlign::Center,
            700,
            32.0,
        ),
        StageBox::with_style(
            "connection-status",
            StageBoxType::ConnectionStatus,
            75.0,
            92.0,
            23.0,
            6.0,
            "#38bdf8",
            TextAlign::Right,
            600,
            20.0,
        ),
    ]
}

fn worship_pp_default_boxes() -> Vec<StageBox> {
    vec![
        StageBox::with_style(
            "current-group",
            StageBoxType::CurrentGroup,
            2.0,
            2.0,
            45.0,
            6.0,
            "#38bdf8",
            TextAlign::Center,
            700,
            24.0,
        ),
        StageBox::with_style(
            "current-slide",
            StageBoxType::CurrentSlide,
            2.0,
            10.0,
            68.0,
            45.0,
            "#f8fafc",
            TextAlign::Center,
            700,
            100.0,
        ),
        StageBox::with_style(
            "next-group",
            StageBoxType::NextGroup,
            2.0,
            58.0,
            45.0,
            5.0,
            "#facc15",
            TextAlign::Center,
            700,
            20.0,
        ),
        StageBox::with_style(
            "next-slide",
            StageBoxType::NextSlide,
            2.0,
            64.0,
            68.0,
            28.0,
            "#cbd5f5",
            TextAlign::Center,
            700,
            64.0,
        ),
        StageBox::with_style(
            "playlist",
            StageBoxType::Playlist,
            72.0,
            2.0,
            26.0,
            90.0,
            "#94a3b8",
            TextAlign::Left,
            400,
            18.0,
        ),
        // Status bar elements
        StageBox::with_style(
            "clock",
            StageBoxType::Clock,
            2.0,
            92.0,
            20.0,
            6.0,
            "#38bdf8",
            TextAlign::Left,
            700,
            48.0,
        ),
        StageBox::with_style(
            "live-indicator",
            StageBoxType::LiveIndicator,
            28.0,
            92.0,
            20.0,
            6.0,
            "#ef4444",
            TextAlign::Center,
            700,
            32.0,
        ),
        StageBox::with_style(
            "connection-status",
            StageBoxType::ConnectionStatus,
            50.0,
            92.0,
            20.0,
            6.0,
            "#38bdf8",
            TextAlign::Right,
            600,
            20.0,
        ),
    ]
}

fn timer_default_boxes() -> Vec<StageBox> {
    vec![
        StageBox::with_style(
            "countdown-timer",
            StageBoxType::CountdownTimer,
            10.0,
            35.0,
            80.0,
            30.0,
            "#38bdf8",
            TextAlign::Center,
            700,
            200.0,
        ),
        // Status bar elements
        StageBox::with_style(
            "clock",
            StageBoxType::Clock,
            2.0,
            92.0,
            20.0,
            6.0,
            "#38bdf8",
            TextAlign::Left,
            700,
            48.0,
        ),
        StageBox::with_style(
            "live-indicator",
            StageBoxType::LiveIndicator,
            40.0,
            92.0,
            20.0,
            6.0,
            "#ef4444",
            TextAlign::Center,
            700,
            32.0,
        ),
        StageBox::with_style(
            "connection-status",
            StageBoxType::ConnectionStatus,
            75.0,
            92.0,
            23.0,
            6.0,
            "#38bdf8",
            TextAlign::Right,
            600,
            20.0,
        ),
    ]
}

fn preach_default_boxes() -> Vec<StageBox> {
    vec![
        StageBox::with_style(
            "preach-timer",
            StageBoxType::PreachTimer,
            10.0,
            35.0,
            80.0,
            30.0,
            "#34d399",
            TextAlign::Center,
            700,
            200.0,
        ),
        // Status bar elements
        StageBox::with_style(
            "clock",
            StageBoxType::Clock,
            2.0,
            92.0,
            20.0,
            6.0,
            "#38bdf8",
            TextAlign::Left,
            700,
            48.0,
        ),
        StageBox::with_style(
            "live-indicator",
            StageBoxType::LiveIndicator,
            40.0,
            92.0,
            20.0,
            6.0,
            "#ef4444",
            TextAlign::Center,
            700,
            32.0,
        ),
        StageBox::with_style(
            "connection-status",
            StageBoxType::ConnectionStatus,
            75.0,
            92.0,
            23.0,
            6.0,
            "#38bdf8",
            TextAlign::Right,
            600,
            20.0,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_design_for_worship_snv_has_all_boxes() {
        let design = StageDesign::default_for("worship-snv");
        assert_eq!(design.layout_code, "worship-snv");
        assert!(design.get_box("current-slide").is_some());
        assert!(design.get_box("next-slide").is_some());
        assert!(design.get_box("current-group").is_some());
        assert!(design.get_box("next-group").is_some());
        assert!(design.get_box("clock").is_some());
        assert!(design.get_box("live-indicator").is_some());
        assert!(design.get_box("connection-status").is_some());
    }

    #[test]
    fn default_design_for_worship_pp_has_playlist() {
        let design = StageDesign::default_for("worship-pp");
        assert_eq!(design.layout_code, "worship-pp");
        assert!(design.get_box("playlist").is_some());
    }

    #[test]
    fn default_design_for_timer_has_countdown() {
        let design = StageDesign::default_for("timer");
        assert!(design.get_box("countdown-timer").is_some());
    }

    #[test]
    fn default_design_for_preach_has_preach_timer() {
        let design = StageDesign::default_for("preach");
        assert!(design.get_box("preach-timer").is_some());
    }

    #[test]
    fn stage_box_serializes_correctly() {
        let box_item = StageBox::new("test", StageBoxType::CurrentSlide, 10.0, 20.0, 80.0, 60.0);
        let json = serde_json::to_string(&box_item).expect("serialize");
        assert!(json.contains("\"boxType\":\"current_slide\""));
        assert!(json.contains("\"x\":10"));
    }

    #[test]
    fn text_align_css() {
        assert_eq!(TextAlign::Left.as_css(), "left");
        assert_eq!(TextAlign::Center.as_css(), "center");
        assert_eq!(TextAlign::Right.as_css(), "right");
    }

    #[test]
    fn is_status_bar_identifies_correct_types() {
        assert!(StageBoxType::Clock.is_status_bar());
        assert!(StageBoxType::LiveIndicator.is_status_bar());
        assert!(StageBoxType::ConnectionStatus.is_status_bar());
        assert!(!StageBoxType::CurrentSlide.is_status_bar());
        assert!(!StageBoxType::NextSlide.is_status_bar());
        assert!(!StageBoxType::CountdownTimer.is_status_bar());
        assert!(!StageBoxType::PreachTimer.is_status_bar());
        assert!(!StageBoxType::Playlist.is_status_bar());
    }
}
