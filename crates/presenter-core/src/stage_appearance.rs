use serde::{Deserialize, Serialize};

/// Visual appearance settings for a stage display layout.
///
/// Each layout stores its own copy; unknown fields are ignored on
/// deserialization so old clients survive schema additions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StageAppearance {
    pub body_padding_v: f32,
    pub body_padding_h: f32,
    pub current_max_font: f32,
    pub next_max_font: f32,
    pub next_ratio: f32,
    pub group_font_size: f32,
    pub lyrics_gap: f32,
    pub next_padding_bottom: f32,
    pub base_chars: u32,
    pub min_font: f32,
    // worship-pp specific
    pub playlist_font_size: f32,
    pub playlist_header_size: f32,
    pub playlist_padding: f32,
    pub slides_playlist_ratio: String,
}

impl Default for StageAppearance {
    fn default() -> Self {
        Self {
            body_padding_v: 1.0,
            body_padding_h: 2.0,
            current_max_font: 120.0,
            next_max_font: 80.0,
            next_ratio: 0.8,
            group_font_size: 1.6,
            lyrics_gap: 0.5,
            next_padding_bottom: 2.0,
            base_chars: 25,
            min_font: 12.0,
            playlist_font_size: 1.3,
            playlist_header_size: 1.1,
            playlist_padding: 1.0,
            slides_playlist_ratio: "7fr 3fr".to_string(),
        }
    }
}

impl StageAppearance {
    /// Return sensible defaults per layout code.
    pub fn default_for(layout: &str) -> Self {
        match layout {
            "worship-pp" => Self {
                current_max_font: 100.0,
                next_max_font: 64.0,
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}
