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
    ContentTransition, Frame, ImageFit, Shadow, StreamElementProps, TextAlign, TextStyle,
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
        StreamElementProps::Image { .. } | StreamElementProps::Color { .. } => None,
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
        StreamElementProps::Image { .. } | StreamElementProps::Color { .. } => {}
    }
}

/// Every kind carries a `Frame` — read it (clone) via one `|`-pattern.
pub fn read_frame(props: &StreamElementProps) -> Frame {
    match props {
        StreamElementProps::Image { frame, .. }
        | StreamElementProps::Countdown { frame, .. }
        | StreamElementProps::Lyrics { frame, .. }
        | StreamElementProps::Verse { frame, .. }
        | StreamElementProps::Color { frame, .. } => frame.clone(),
    }
}

/// Mutate the (kind-agnostic) `Frame` in place.
pub fn with_frame_mut(props: &mut StreamElementProps, f: impl FnOnce(&mut Frame)) {
    match props {
        StreamElementProps::Image { frame, .. }
        | StreamElementProps::Countdown { frame, .. }
        | StreamElementProps::Lyrics { frame, .. }
        | StreamElementProps::Verse { frame, .. }
        | StreamElementProps::Color { frame, .. } => f(frame),
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
