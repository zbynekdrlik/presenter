//! Field accessors + sensible defaults for the property-form draft (#714).
//!
//! The property form (`element_form.rs` + `text_style_form.rs`) edits ONE
//! working copy per selected element — a `RwSignal<StreamElementProps>` — and
//! each input reads/writes a single field of that tagged enum through these
//! helpers, instead of a wall of per-field signals (the design comment's
//! rejected alternative). `Frame` is shared by all four kinds (one `|`-pattern);
//! the six `TextStyle` occurrences are addressed by a [`TsSlot`] selector so the
//! shared `TextStyleForm` component works for every kind.

use presenter_core::{
    AnimationPreset, ContentTransition, Frame, ImageFit, Shadow, StreamElementProps, TextAlign,
    TextBox, TextStyle,
};

/// Which of an element's `TextStyle` fields a `TextStyleForm` edits. Copy so it
/// can be threaded into several input closures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsSlot {
    CountdownStyle,
    LyricsMain,
    LyricsTranslation,
    VerseText,
    VerseSecondary,
    VerseReference,
    LowerThirdPrimary,
    LowerThirdSecondary,
}

/// Read the `TextStyle` at `slot` from the props (clone), or `None` if the slot
/// does not apply to this kind (never happens: a form only renders the slots
/// its kind has).
pub fn read_ts(props: &StreamElementProps, slot: TsSlot) -> Option<TextStyle> {
    match (props, slot) {
        (StreamElementProps::Countdown { style, .. }, TsSlot::CountdownStyle) => {
            Some(style.clone())
        }
        (StreamElementProps::Lyrics { main_style, .. }, TsSlot::LyricsMain) => {
            Some(main_style.clone())
        }
        (
            StreamElementProps::Lyrics {
                translation_style, ..
            },
            TsSlot::LyricsTranslation,
        ) => Some(translation_style.clone()),
        (StreamElementProps::Verse { text_style, .. }, TsSlot::VerseText) => {
            Some(text_style.clone())
        }
        (
            StreamElementProps::Verse {
                secondary_style, ..
            },
            TsSlot::VerseSecondary,
        ) => Some(secondary_style.clone()),
        (
            StreamElementProps::Verse {
                reference_style, ..
            },
            TsSlot::VerseReference,
        ) => Some(reference_style.clone()),
        (StreamElementProps::LowerThird { primary_style, .. }, TsSlot::LowerThirdPrimary) => {
            Some(primary_style.clone())
        }
        (
            StreamElementProps::LowerThird {
                secondary_style, ..
            },
            TsSlot::LowerThirdSecondary,
        ) => Some(secondary_style.clone()),
        _ => None,
    }
}

/// Mutate the `TextStyle` at `slot` in place (no-op if the slot does not apply).
pub fn with_ts_mut(props: &mut StreamElementProps, slot: TsSlot, f: impl FnOnce(&mut TextStyle)) {
    match (props, slot) {
        (StreamElementProps::Countdown { style, .. }, TsSlot::CountdownStyle) => f(style),
        (StreamElementProps::Lyrics { main_style, .. }, TsSlot::LyricsMain) => f(main_style),
        (
            StreamElementProps::Lyrics {
                translation_style, ..
            },
            TsSlot::LyricsTranslation,
        ) => f(translation_style),
        (StreamElementProps::Verse { text_style, .. }, TsSlot::VerseText) => f(text_style),
        (
            StreamElementProps::Verse {
                secondary_style, ..
            },
            TsSlot::VerseSecondary,
        ) => f(secondary_style),
        (
            StreamElementProps::Verse {
                reference_style, ..
            },
            TsSlot::VerseReference,
        ) => f(reference_style),
        (StreamElementProps::LowerThird { primary_style, .. }, TsSlot::LowerThirdPrimary) => {
            f(primary_style)
        }
        (
            StreamElementProps::LowerThird {
                secondary_style, ..
            },
            TsSlot::LowerThirdSecondary,
        ) => f(secondary_style),
        _ => {}
    }
}

/// The `ContentTransition` of a kind that has one (countdown / lyrics / verse);
/// `None` for image.
pub fn read_transition(props: &StreamElementProps) -> Option<ContentTransition> {
    match props {
        StreamElementProps::Countdown {
            content_transition, ..
        }
        | StreamElementProps::Lyrics {
            content_transition, ..
        }
        | StreamElementProps::Verse {
            content_transition, ..
        } => Some(content_transition.clone()),
        // LowerThird has no content_transition — its enter/leave IS the animation.
        StreamElementProps::Image { .. }
        | StreamElementProps::Color { .. }
        | StreamElementProps::LowerThird { .. } => None,
    }
}

/// Mutate the `ContentTransition` in place (no-op for image).
pub fn with_transition_mut(props: &mut StreamElementProps, f: impl FnOnce(&mut ContentTransition)) {
    match props {
        StreamElementProps::Countdown {
            content_transition, ..
        }
        | StreamElementProps::Lyrics {
            content_transition, ..
        }
        | StreamElementProps::Verse {
            content_transition, ..
        } => f(content_transition),
        StreamElementProps::Image { .. }
        | StreamElementProps::Color { .. }
        | StreamElementProps::LowerThird { .. } => {}
    }
}

/// Every kind carries a `Frame` — read it (clone) via one `|`-pattern.
pub fn read_frame(props: &StreamElementProps) -> Frame {
    match props {
        StreamElementProps::Image { frame, .. }
        | StreamElementProps::Countdown { frame, .. }
        | StreamElementProps::Lyrics { frame, .. }
        | StreamElementProps::Verse { frame, .. }
        | StreamElementProps::Color { frame, .. }
        | StreamElementProps::LowerThird { frame, .. } => frame.clone(),
    }
}

/// Mutate the (kind-agnostic) `Frame` in place.
pub fn with_frame_mut(props: &mut StreamElementProps, f: impl FnOnce(&mut Frame)) {
    match props {
        StreamElementProps::Image { frame, .. }
        | StreamElementProps::Countdown { frame, .. }
        | StreamElementProps::Lyrics { frame, .. }
        | StreamElementProps::Verse { frame, .. }
        | StreamElementProps::Color { frame, .. }
        | StreamElementProps::LowerThird { frame, .. } => f(frame),
    }
}

/// The `opacity` (0..=1) of a kind that has one (image / color); `None` for the
/// text kinds, which express transparency through their `TextStyle` color's
/// alpha byte instead. Used by the shared percent-input control (#776).
pub fn read_opacity(props: &StreamElementProps) -> Option<f32> {
    match props {
        StreamElementProps::Image { opacity, .. } | StreamElementProps::Color { opacity, .. } => {
            Some(*opacity)
        }
        // LowerThird's transparency control is `bar_opacity` (a distinct field),
        // not this top-level `opacity`, so the shared PercentInput never edits it.
        StreamElementProps::Countdown { .. }
        | StreamElementProps::Lyrics { .. }
        | StreamElementProps::Verse { .. }
        | StreamElementProps::LowerThird { .. } => None,
    }
}

/// Mutate the (0..=1) `opacity` in place (no-op for a kind without one).
pub fn with_opacity_mut(props: &mut StreamElementProps, f: impl FnOnce(&mut f32)) {
    match props {
        StreamElementProps::Image { opacity, .. } | StreamElementProps::Color { opacity, .. } => {
            f(opacity)
        }
        StreamElementProps::Countdown { .. }
        | StreamElementProps::Lyrics { .. }
        | StreamElementProps::Verse { .. }
        | StreamElementProps::LowerThird { .. } => {}
    }
}

/// Split a stored color (`#rrggbb` or `#rrggbbaa`) into the `<input type=color>`
/// value (`#rrggbb`) + an alpha byte `0..=255` for the separate alpha field. A
/// malformed value degrades to opaque black.
pub fn split_color(color: &str) -> (String, u8) {
    if color.len() == 9 && color.starts_with('#') {
        let alpha = u8::from_str_radix(&color[7..9], 16).unwrap_or(255);
        (color[..7].to_string(), alpha)
    } else if color.len() == 7 && color.starts_with('#') {
        (color.to_string(), 255)
    } else {
        ("#000000".to_string(), 255)
    }
}

/// Recombine a `#rrggbb` + alpha byte into the stored color: 6-digit when fully
/// opaque, else 8-digit `#rrggbbaa`.
pub fn join_color(rgb: &str, alpha: u8) -> String {
    let rgb6 = if rgb.len() == 7 && rgb.starts_with('#') {
        rgb
    } else {
        "#000000"
    };
    if alpha >= 255 {
        rgb6.to_string()
    } else {
        format!("{rgb6}{alpha:02x}")
    }
}

/// A sensible default `TextStyle` for a freshly-added text element.
pub fn default_text_style() -> TextStyle {
    TextStyle {
        font_family: "Inter".to_string(),
        size_pct: 8.0,
        color: "#ffffff".to_string(),
        weight: 700,
        align: TextAlign::Center,
        line_height: 1.2,
        shadow: None,
        letter_spacing_em: None,
    }
}

/// A sensible default `Frame` (a wide band in the lower third) for a new element.
pub fn default_frame() -> Frame {
    Frame {
        x_pct: 10.0,
        y_pct: 40.0,
        w_pct: 80.0,
        h_pct: 20.0,
    }
}

/// A default `Shadow` used when the shadow toggle is first enabled.
pub fn default_shadow() -> Shadow {
    Shadow {
        x_px: 2.0,
        y_px: 2.0,
        blur_px: 4.0,
        color: "#000000".to_string(),
    }
}

/// A default countdown background `TextBox` (#785), used when the "Pozadie"
/// toggle is first enabled: a dark, semi-transparent rounded card.
pub fn default_text_box() -> TextBox {
    TextBox {
        color: "#0f172a".to_string(),
        opacity: 0.6,
        padding_pct: 2.0,
        radius_pct: 1.5,
    }
}

/// Default props for a freshly-added element of `kind`. The image asset_id
/// starts at 1 (a valid positive ref that passes core validation); the operator
/// then picks a real uploaded asset via the picker (#715).
pub fn default_element_props(kind: &str) -> StreamElementProps {
    match kind {
        "image" => StreamElementProps::Image {
            asset_id: 1,
            fit: ImageFit::Contain,
            frame: default_frame(),
            opacity: 1.0,
        },
        "countdown" => StreamElementProps::Countdown {
            timer_id: 1,
            style: default_text_style(),
            frame: default_frame(),
            content_transition: ContentTransition::default(),
            r#box: None,
        },
        "lyrics" => StreamElementProps::Lyrics {
            show_main: true,
            show_translation: false,
            main_style: default_text_style(),
            translation_style: default_text_style(),
            frame: default_frame(),
            content_transition: ContentTransition::default(),
        },
        // A static solid fill (#753): opaque black by default in a lower-third
        // band; the operator drags opacity down for a semi-transparent
        // background and picks the color.
        "color" => StreamElementProps::Color {
            color: "#000000".to_string(),
            opacity: 1.0,
            frame: default_frame(),
        },
        // A lower-third "menovka" (#779): a dark bar with an accent stripe in the
        // lower third, a bold primary line + a lighter secondary line, sliding in.
        "lower_third" => StreamElementProps::LowerThird {
            frame: Frame {
                x_pct: 6.0,
                y_pct: 74.0,
                w_pct: 46.0,
                h_pct: 14.0,
            },
            bar_color: "#0f172a".to_string(),
            bar_opacity: 0.85,
            accent_color: "#38bdf8".to_string(),
            accent_width_pct: 2.5,
            primary_style: default_text_style(),
            secondary_style: TextStyle {
                size_pct: 4.0,
                weight: 400,
                ..default_text_style()
            },
            padding_pct: 3.0,
            animation: AnimationPreset::SlideLeft,
            in_ms: 500,
            out_ms: 400,
            auto_hide_s: 0,
        },
        // "verse" (the only remaining valid kind) + a defensive default — the
        // panel's add-buttons only ever pass the five valid kind strings.
        _ => StreamElementProps::Verse {
            show_secondary: false,
            text_style: default_text_style(),
            secondary_style: default_text_style(),
            reference_style: default_text_style(),
            frame: default_frame(),
            content_transition: ContentTransition::default(),
        },
    }
}
