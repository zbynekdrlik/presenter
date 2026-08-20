//! Stream-graphics domain model — typed element props + pure validation, the
//! single source of truth shared by the persistence layer (#705), the REST
//! write paths (#706+) and the WASM operator editor/output pages
//! (`presenter-core` compiles into `presenter-ui`).
//!
//! Stream-graphics replaces Resolume Arena for the STREAM output: a nameable,
//! transparent web page (an OBS browser source) renders one exclusive BASE
//! scene plus any set of OVERLAY scenes, each composed of typed elements
//! (image / countdown / lyrics / verse). See the epic architecture (issue
//! #718, comment "Stream graphics architecture", §4) and ADR
//! `docs/adr/0009-stream-graphics.md` §4 — this module is the concrete
//! `StreamElementProps` (serde `tag = "kind"`, matching the
//! `stream_elements.kind` column), the style value types, the shared def / show
//! DTOs, and the validation functions those sections specify.
//!
//! No `unwrap`/`expect`/`panic!` — every failure is a typed
//! [`StreamValidationError`].

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fixed v1 font whitelist (arch §4). Custom-font upload is deferred to v2.
pub const STREAM_FONT_FAMILIES: &[&str] = &["system-ui", "Arial", "Inter", "Bebas Neue", "Oswald"];

/// Slugs that collide with the static route prefixes `/stream/api` and
/// `/stream/assets` (arch §9) — never usable as an output slug.
pub const RESERVED_STREAM_SLUGS: &[&str] = &["api", "assets"];

/// Maximum scene-name length, in characters.
pub const STREAM_SCENE_NAME_MAX: usize = 100;
/// Maximum output-slug length (`^[a-z0-9-]{1,64}$`).
pub const STREAM_SLUG_MAX: usize = 64;
/// Upper bound for a content-transition fade, in milliseconds.
pub const STREAM_TRANSITION_MAX_MS: u32 = 10_000;
/// Default content-transition fade, in milliseconds (arch §4: `Fade{300}`).
pub const STREAM_DEFAULT_FADE_MS: u32 = 300;

/// A typed validation failure for a stream slug, scene name, or element props.
/// Carries enough detail for a 422 body; the persistence layer maps it to
/// [`crate`]-external `RepositoryError::Invalid` (#705).
#[derive(Debug, Clone, PartialEq, Error)]
pub enum StreamValidationError {
    #[error("invalid slug {slug:?}: must match ^[a-z0-9-]{{1,64}}$")]
    InvalidSlug { slug: String },
    #[error("reserved slug {slug:?}: not allowed")]
    ReservedSlug { slug: String },
    #[error("scene name must be non-empty and at most 100 characters")]
    InvalidSceneName,
    #[error("{field} percentage {value} out of range (expected 0..=100)")]
    PctOutOfRange { field: &'static str, value: f32 },
    #[error("frame {field} must be greater than 0 (got {value})")]
    NonPositiveFrameSize { field: &'static str, value: f32 },
    #[error("opacity {value} out of range (expected 0.0..=1.0)")]
    OpacityOutOfRange { value: f32 },
    #[error("invalid color {color:?}: expected #rrggbb or #rrggbbaa")]
    InvalidColor { color: String },
    #[error("font weight {value} out of range (expected 1..=1000)")]
    WeightOutOfRange { value: u16 },
    #[error("line height {value} out of range (expected 0.5..=3.0)")]
    LineHeightOutOfRange { value: f32 },
    #[error("transition duration {value}ms out of range (expected <=10000)")]
    TransitionTooLong { value: u32 },
    #[error("unknown font family {family:?}")]
    UnknownFontFamily { family: String },
    #[error("{ref_kind} reference must be a positive id (got {value})")]
    InvalidRef { ref_kind: &'static str, value: i64 },
}

/// Horizontal text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// How an image element fills its [`Frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    Contain,
    Cover,
    Stretch,
}

/// A base or overlay scene's kind. The base scene is exclusive (one active per
/// output, stored in `stream_outputs.active_scene_id`); overlays form an
/// independent active set (`stream_scenes.is_active`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneKind {
    Base,
    Overlay,
}

impl SceneKind {
    /// The stored `stream_scenes.kind` column value.
    pub fn as_str(&self) -> &'static str {
        match self {
            SceneKind::Base => "base",
            SceneKind::Overlay => "overlay",
        }
    }

    /// Parse a stored `stream_scenes.kind` column value. `None` for an
    /// unrecognised value (a corrupt row) — callers decide how to degrade.
    pub fn from_db(value: &str) -> Option<SceneKind> {
        match value {
            "base" => Some(SceneKind::Base),
            "overlay" => Some(SceneKind::Overlay),
            _ => None,
        }
    }
}

/// A rectangle expressed as percentages of the fixed 16:9 output canvas
/// (`0..=100`; the OBS source is 1920×1080 so `%` maps 1:1 to CSS `vw`/`vh`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub x_pct: f32,
    pub y_pct: f32,
    pub w_pct: f32,
    pub h_pct: f32,
}

/// A CSS text/box shadow. `color` is `#rrggbb` or `#rrggbbaa`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub x_px: f32,
    pub y_px: f32,
    pub blur_px: f32,
    pub color: String,
}

/// Typographic style for a text element. `size_pct` is a percentage of canvas
/// height (⇒ CSS `vh`); `color` is `#rrggbb`/`#rrggbbaa`; `weight` is a CSS
/// font weight `1..=1000`; `line_height` is a unitless multiplier `0.5..=3.0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub font_family: String,
    pub size_pct: f32,
    pub color: String,
    pub weight: u16,
    pub align: TextAlign,
    pub line_height: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
}

/// How element CONTENT changes are animated (a lyric line change, a new verse,
/// a countdown tick). Distinct from the scene-switch transition. Defaults to
/// `Fade{300}` (arch §4) so an omitted `content_transition` deserialises to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ContentTransition {
    Cut,
    Fade { duration_ms: u32 },
}

impl Default for ContentTransition {
    fn default() -> Self {
        ContentTransition::Fade {
            duration_ms: STREAM_DEFAULT_FADE_MS,
        }
    }
}

/// The typed, serde-tagged element props stored in `stream_elements.props`.
/// The serde tag `kind` MUST equal the `stream_elements.kind` column — #705
/// enforces `tag == kind` on every write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamElementProps {
    Image {
        asset_id: i64,
        fit: ImageFit,
        frame: Frame,
        opacity: f32,
    },
    Countdown {
        timer_id: i64,
        style: TextStyle,
        frame: Frame,
        #[serde(default)]
        content_transition: ContentTransition,
    },
    Lyrics {
        show_main: bool,
        show_translation: bool,
        main_style: TextStyle,
        translation_style: TextStyle,
        frame: Frame,
        #[serde(default)]
        content_transition: ContentTransition,
    },
    Verse {
        show_secondary: bool,
        text_style: TextStyle,
        secondary_style: TextStyle,
        reference_style: TextStyle,
        frame: Frame,
        #[serde(default)]
        content_transition: ContentTransition,
    },
}

impl StreamElementProps {
    /// The serde tag string for this variant — must equal the stored
    /// `stream_elements.kind` column (#705's `tag == kind` check).
    pub fn kind_str(&self) -> &'static str {
        match self {
            StreamElementProps::Image { .. } => "image",
            StreamElementProps::Countdown { .. } => "countdown",
            StreamElementProps::Lyrics { .. } => "lyrics",
            StreamElementProps::Verse { .. } => "verse",
        }
    }
}

// ---------------------------------------------------------------------------
// Wire DTOs — the `/def` payload family, shared with the WASM client (#706+).
// ---------------------------------------------------------------------------

/// The persisted activation snapshot for one output — the payload of the
/// `StreamState` LiveEvent and the StreamManager's in-memory cache. `base` is
/// exclusive (`Option`); overlays are an independent set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamShowState {
    pub active_scene_id: Option<i64>,
    pub active_overlay_ids: Vec<i64>,
    pub config_revision: u64,
}

/// A lightweight output row (no scenes) — returned by output CRUD + list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamOutputSummary {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub default_transition_ms: u32,
    pub active_scene_id: Option<i64>,
    pub config_revision: u64,
}

/// One element within a [`StreamSceneDef`], ordered by `z_order`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamElementDef {
    pub id: i64,
    pub z_order: i32,
    pub props: StreamElementProps,
}

/// One scene within a [`StreamOutputDef`], with its ordered elements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamSceneDef {
    pub id: i64,
    pub name: String,
    pub kind: SceneKind,
    pub position: i32,
    pub is_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_ms: Option<u32>,
    pub elements: Vec<StreamElementDef>,
}

/// The full output definition served by `GET /stream/api/outputs/{slug}/def`
/// — the cold-load payload for the WASM output page and the editor. Scenes are
/// ordered by (kind, position); each scene's elements are ordered by `z_order`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamOutputDef {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub default_transition_ms: u32,
    pub active_scene_id: Option<i64>,
    pub config_revision: u64,
    pub scenes: Vec<StreamSceneDef>,
}

/// Metadata for an uploaded, sha256-addressed image asset — listed by the
/// editor and referenced by `StreamElementProps::Image { asset_id }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamAsset {
    pub id: i64,
    pub sha256: String,
    pub original_filename: String,
    pub mime: String,
    pub size_bytes: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

// ---------------------------------------------------------------------------
// Validation — pure, unit-tested, no panics.
// ---------------------------------------------------------------------------

/// Validate an output slug: `^[a-z0-9-]{1,64}$` and not a reserved slug.
pub fn validate_slug(slug: &str) -> Result<(), StreamValidationError> {
    let well_formed = !slug.is_empty()
        && slug.len() <= STREAM_SLUG_MAX
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !well_formed {
        return Err(StreamValidationError::InvalidSlug {
            slug: slug.to_string(),
        });
    }
    if RESERVED_STREAM_SLUGS.contains(&slug) {
        return Err(StreamValidationError::ReservedSlug {
            slug: slug.to_string(),
        });
    }
    Ok(())
}

/// Validate a scene name: non-empty after trimming, at most 100 characters.
pub fn validate_scene_name(name: &str) -> Result<(), StreamValidationError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > STREAM_SCENE_NAME_MAX {
        return Err(StreamValidationError::InvalidSceneName);
    }
    Ok(())
}

/// Validate typed element props: percentage ranges, colors, font family,
/// weight, line height, transition duration, and positive asset/timer refs.
pub fn validate_props(props: &StreamElementProps) -> Result<(), StreamValidationError> {
    match props {
        StreamElementProps::Image {
            asset_id,
            frame,
            opacity,
            ..
        } => {
            validate_ref("asset", *asset_id)?;
            validate_frame(frame)?;
            validate_opacity(*opacity)?;
        }
        StreamElementProps::Countdown {
            timer_id,
            style,
            frame,
            content_transition,
        } => {
            validate_ref("timer", *timer_id)?;
            validate_text_style(style)?;
            validate_frame(frame)?;
            validate_transition(content_transition)?;
        }
        StreamElementProps::Lyrics {
            main_style,
            translation_style,
            frame,
            content_transition,
            ..
        } => {
            validate_text_style(main_style)?;
            validate_text_style(translation_style)?;
            validate_frame(frame)?;
            validate_transition(content_transition)?;
        }
        StreamElementProps::Verse {
            text_style,
            secondary_style,
            reference_style,
            frame,
            content_transition,
            ..
        } => {
            validate_text_style(text_style)?;
            validate_text_style(secondary_style)?;
            validate_text_style(reference_style)?;
            validate_frame(frame)?;
            validate_transition(content_transition)?;
        }
    }
    Ok(())
}

fn validate_pct(field: &'static str, value: f32) -> Result<(), StreamValidationError> {
    if !(0.0..=100.0).contains(&value) {
        return Err(StreamValidationError::PctOutOfRange { field, value });
    }
    Ok(())
}

fn validate_frame(frame: &Frame) -> Result<(), StreamValidationError> {
    validate_pct("frame.x_pct", frame.x_pct)?;
    validate_pct("frame.y_pct", frame.y_pct)?;
    validate_pct("frame.w_pct", frame.w_pct)?;
    validate_pct("frame.h_pct", frame.h_pct)?;
    if frame.w_pct <= 0.0 {
        return Err(StreamValidationError::NonPositiveFrameSize {
            field: "w_pct",
            value: frame.w_pct,
        });
    }
    if frame.h_pct <= 0.0 {
        return Err(StreamValidationError::NonPositiveFrameSize {
            field: "h_pct",
            value: frame.h_pct,
        });
    }
    Ok(())
}

fn validate_opacity(value: f32) -> Result<(), StreamValidationError> {
    if !(0.0..=1.0).contains(&value) {
        return Err(StreamValidationError::OpacityOutOfRange { value });
    }
    Ok(())
}

fn validate_text_style(style: &TextStyle) -> Result<(), StreamValidationError> {
    if !STREAM_FONT_FAMILIES.contains(&style.font_family.as_str()) {
        return Err(StreamValidationError::UnknownFontFamily {
            family: style.font_family.clone(),
        });
    }
    validate_pct("size_pct", style.size_pct)?;
    validate_color(&style.color)?;
    if !(1..=1000).contains(&style.weight) {
        return Err(StreamValidationError::WeightOutOfRange {
            value: style.weight,
        });
    }
    if !(0.5..=3.0).contains(&style.line_height) {
        return Err(StreamValidationError::LineHeightOutOfRange {
            value: style.line_height,
        });
    }
    if let Some(shadow) = &style.shadow {
        validate_color(&shadow.color)?;
    }
    Ok(())
}

fn validate_color(color: &str) -> Result<(), StreamValidationError> {
    let well_formed = (color.len() == 7 || color.len() == 9)
        && color.starts_with('#')
        && color[1..].bytes().all(|b| b.is_ascii_hexdigit());
    if !well_formed {
        return Err(StreamValidationError::InvalidColor {
            color: color.to_string(),
        });
    }
    Ok(())
}

fn validate_transition(transition: &ContentTransition) -> Result<(), StreamValidationError> {
    if let ContentTransition::Fade { duration_ms } = transition {
        if *duration_ms > STREAM_TRANSITION_MAX_MS {
            return Err(StreamValidationError::TransitionTooLong {
                value: *duration_ms,
            });
        }
    }
    Ok(())
}

fn validate_ref(ref_kind: &'static str, value: i64) -> Result<(), StreamValidationError> {
    if value <= 0 {
        return Err(StreamValidationError::InvalidRef { ref_kind, value });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ok_text_style() -> TextStyle {
        TextStyle {
            font_family: "Inter".to_string(),
            size_pct: 8.0,
            color: "#ffffff".to_string(),
            weight: 700,
            align: TextAlign::Center,
            line_height: 1.2,
            shadow: None,
        }
    }

    fn ok_frame() -> Frame {
        Frame {
            x_pct: 10.0,
            y_pct: 20.0,
            w_pct: 50.0,
            h_pct: 30.0,
        }
    }

    fn image_props() -> StreamElementProps {
        StreamElementProps::Image {
            asset_id: 7,
            fit: ImageFit::Contain,
            frame: ok_frame(),
            opacity: 1.0,
        }
    }

    fn countdown_props() -> StreamElementProps {
        StreamElementProps::Countdown {
            timer_id: 3,
            style: ok_text_style(),
            frame: ok_frame(),
            content_transition: ContentTransition::Cut,
        }
    }

    fn lyrics_props() -> StreamElementProps {
        StreamElementProps::Lyrics {
            show_main: true,
            show_translation: false,
            main_style: ok_text_style(),
            translation_style: ok_text_style(),
            frame: ok_frame(),
            content_transition: ContentTransition::Fade { duration_ms: 250 },
        }
    }

    fn verse_props() -> StreamElementProps {
        StreamElementProps::Verse {
            show_secondary: true,
            text_style: ok_text_style(),
            secondary_style: ok_text_style(),
            reference_style: ok_text_style(),
            frame: ok_frame(),
            content_transition: ContentTransition::default(),
        }
    }

    fn assert_round_trips(props: &StreamElementProps) {
        let json = serde_json::to_string(props).expect("serialize");
        let back: StreamElementProps = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, props);
    }

    #[test]
    fn all_four_kinds_round_trip() {
        assert_round_trips(&image_props());
        assert_round_trips(&countdown_props());
        assert_round_trips(&lyrics_props());
        assert_round_trips(&verse_props());
    }

    #[test]
    fn image_props_wire_shape_is_exact() {
        // Exact wire shape (single-line fixture keeps braces balanced for the
        // fn-length gate's brace counter).
        let wire = r#"{"kind":"image","asset_id":7,"fit":"contain","frame":{"x_pct":10.0,"y_pct":20.0,"w_pct":50.0,"h_pct":30.0},"opacity":1.0}"#;
        let parsed: StreamElementProps = serde_json::from_str(wire).expect("parse");
        assert_eq!(parsed, image_props());
        assert_eq!(
            serde_json::to_value(image_props()).unwrap(),
            serde_json::from_str::<serde_json::Value>(wire).unwrap()
        );
    }

    #[test]
    fn countdown_and_verse_tags_are_snake_case() {
        let cd = serde_json::to_value(countdown_props()).unwrap();
        assert_eq!(cd["kind"], json!("countdown"));
        assert_eq!(cd["content_transition"], json!({"mode":"cut"}));
        let verse = serde_json::to_value(verse_props()).unwrap();
        assert_eq!(verse["kind"], json!("verse"));
        // Default content transition serialises as Fade{300}.
        assert_eq!(
            verse["content_transition"],
            json!({"mode":"fade","duration_ms":300})
        );
    }

    #[test]
    fn omitted_content_transition_defaults_to_fade_300() {
        let wire = r##"{"kind":"lyrics","show_main":true,"show_translation":false,"main_style":{"font_family":"Inter","size_pct":8.0,"color":"#ffffff","weight":700,"align":"center","line_height":1.2},"translation_style":{"font_family":"Inter","size_pct":8.0,"color":"#ffffff","weight":700,"align":"center","line_height":1.2},"frame":{"x_pct":10.0,"y_pct":20.0,"w_pct":50.0,"h_pct":30.0}}"##;
        let parsed: StreamElementProps = serde_json::from_str(wire).expect("parse");
        // The wire fixture omits `content_transition`; it must deserialise to
        // the Fade{300} default (arch §4), so the whole value equals this.
        let expected = StreamElementProps::Lyrics {
            show_main: true,
            show_translation: false,
            main_style: ok_text_style(),
            translation_style: ok_text_style(),
            frame: ok_frame(),
            content_transition: ContentTransition::Fade { duration_ms: 300 },
        };
        assert_eq!(parsed, expected);
    }

    #[test]
    fn kind_str_matches_serde_tag() {
        assert_eq!(image_props().kind_str(), "image");
        assert_eq!(countdown_props().kind_str(), "countdown");
        assert_eq!(lyrics_props().kind_str(), "lyrics");
        assert_eq!(verse_props().kind_str(), "verse");
    }

    #[test]
    fn scene_kind_round_trips_db_strings() {
        assert_eq!(SceneKind::Base.as_str(), "base");
        assert_eq!(SceneKind::Overlay.as_str(), "overlay");
        assert_eq!(SceneKind::from_db("base"), Some(SceneKind::Base));
        assert_eq!(SceneKind::from_db("overlay"), Some(SceneKind::Overlay));
        assert_eq!(SceneKind::from_db("bogus"), None);
    }

    #[test]
    fn valid_slugs_accepted() {
        for slug in ["stream", "event-1", "a", "abc-123"] {
            assert!(validate_slug(slug).is_ok(), "{slug} should be valid");
        }
    }

    #[test]
    fn bad_slugs_rejected() {
        assert!(matches!(
            validate_slug(""),
            Err(StreamValidationError::InvalidSlug { .. })
        ));
        assert!(matches!(
            validate_slug("Stream"),
            Err(StreamValidationError::InvalidSlug { .. })
        ));
        assert!(matches!(
            validate_slug("has space"),
            Err(StreamValidationError::InvalidSlug { .. })
        ));
        assert!(matches!(
            validate_slug(&"x".repeat(65)),
            Err(StreamValidationError::InvalidSlug { .. })
        ));
    }

    #[test]
    fn reserved_slugs_rejected() {
        assert!(matches!(
            validate_slug("api"),
            Err(StreamValidationError::ReservedSlug { .. })
        ));
        assert!(matches!(
            validate_slug("assets"),
            Err(StreamValidationError::ReservedSlug { .. })
        ));
    }

    #[test]
    fn scene_name_bounds_enforced() {
        assert!(validate_scene_name("Base").is_ok());
        assert!(validate_scene_name("  Chvály  ").is_ok());
        assert!(matches!(
            validate_scene_name("   "),
            Err(StreamValidationError::InvalidSceneName)
        ));
        assert!(matches!(
            validate_scene_name(&"n".repeat(101)),
            Err(StreamValidationError::InvalidSceneName)
        ));
    }

    #[test]
    fn valid_props_accepted() {
        assert!(validate_props(&image_props()).is_ok());
        assert!(validate_props(&countdown_props()).is_ok());
        assert!(validate_props(&lyrics_props()).is_ok());
        assert!(validate_props(&verse_props()).is_ok());
    }

    #[test]
    fn out_of_range_pct_rejected() {
        let props = StreamElementProps::Image {
            asset_id: 1,
            fit: ImageFit::Cover,
            frame: Frame {
                x_pct: 150.0,
                y_pct: 0.0,
                w_pct: 10.0,
                h_pct: 10.0,
            },
            opacity: 1.0,
        };
        assert!(matches!(
            validate_props(&props),
            Err(StreamValidationError::PctOutOfRange { .. })
        ));
    }

    #[test]
    fn zero_size_frame_rejected() {
        let props = StreamElementProps::Image {
            asset_id: 1,
            fit: ImageFit::Cover,
            frame: Frame {
                x_pct: 0.0,
                y_pct: 0.0,
                w_pct: 0.0,
                h_pct: 10.0,
            },
            opacity: 1.0,
        };
        assert!(matches!(
            validate_props(&props),
            Err(StreamValidationError::NonPositiveFrameSize { .. })
        ));
    }

    #[test]
    fn bad_opacity_rejected() {
        let props = StreamElementProps::Image {
            asset_id: 1,
            fit: ImageFit::Cover,
            frame: ok_frame(),
            opacity: 1.5,
        };
        assert!(matches!(
            validate_props(&props),
            Err(StreamValidationError::OpacityOutOfRange { .. })
        ));
    }

    #[test]
    fn non_positive_ref_rejected() {
        let props = StreamElementProps::Image {
            asset_id: 0,
            fit: ImageFit::Cover,
            frame: ok_frame(),
            opacity: 1.0,
        };
        assert!(matches!(
            validate_props(&props),
            Err(StreamValidationError::InvalidRef { .. })
        ));
    }

    #[test]
    fn bad_color_rejected() {
        let mut style = ok_text_style();
        style.color = "ffffff".to_string();
        let props = StreamElementProps::Countdown {
            timer_id: 1,
            style,
            frame: ok_frame(),
            content_transition: ContentTransition::Cut,
        };
        assert!(matches!(
            validate_props(&props),
            Err(StreamValidationError::InvalidColor { .. })
        ));
    }

    #[test]
    fn eight_digit_color_accepted() {
        let mut style = ok_text_style();
        style.color = "#11223344".to_string();
        style.shadow = Some(Shadow {
            x_px: 1.0,
            y_px: 1.0,
            blur_px: 2.0,
            color: "#000000ff".to_string(),
        });
        assert!(validate_text_style(&style).is_ok());
    }

    #[test]
    fn unknown_font_rejected() {
        let mut style = ok_text_style();
        style.font_family = "Comic Sans".to_string();
        assert!(matches!(
            validate_text_style(&style),
            Err(StreamValidationError::UnknownFontFamily { .. })
        ));
    }

    #[test]
    fn bad_weight_and_line_height_rejected() {
        let mut style = ok_text_style();
        style.weight = 0;
        assert!(matches!(
            validate_text_style(&style),
            Err(StreamValidationError::WeightOutOfRange { .. })
        ));
        let mut style = ok_text_style();
        style.line_height = 4.0;
        assert!(matches!(
            validate_text_style(&style),
            Err(StreamValidationError::LineHeightOutOfRange { .. })
        ));
    }

    #[test]
    fn long_transition_rejected() {
        let props = StreamElementProps::Countdown {
            timer_id: 1,
            style: ok_text_style(),
            frame: ok_frame(),
            content_transition: ContentTransition::Fade {
                duration_ms: 20_000,
            },
        };
        assert!(matches!(
            validate_props(&props),
            Err(StreamValidationError::TransitionTooLong { .. })
        ));
    }
}
