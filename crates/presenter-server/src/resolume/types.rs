use std::borrow::Cow;
use tokio::time::Instant;

#[derive(Debug, Clone)]
pub(super) struct ResolvedEndpoint {
    pub(super) base_url: String,
    pub(super) host_header: Option<String>,
    pub(super) resolved_at: Instant,
}

impl ResolvedEndpoint {
    pub(super) fn new(base_url: String, host_header: Option<String>) -> Self {
        Self {
            base_url,
            host_header,
            resolved_at: Instant::now(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(super) struct SlotState {
    main_next: LaneTarget,
    translation_next: LaneTarget,
    bible_next: LaneTarget,
    bible_translation_next: LaneTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum LaneTarget {
    #[default]
    A,
    B,
}

impl LaneTarget {
    pub(super) fn label(&self) -> &'static str {
        match self {
            LaneTarget::A => "A",
            LaneTarget::B => "B",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum SlotKind {
    Main,
    Translation,
    Bible,
    BibleTranslation,
}

impl SlotState {
    pub(super) fn current(&self, slot: SlotKind) -> LaneTarget {
        match slot {
            SlotKind::Main => self.main_next,
            SlotKind::Translation => self.translation_next,
            SlotKind::Bible => self.bible_next,
            SlotKind::BibleTranslation => self.bible_translation_next,
        }
    }

    pub(super) fn flip(&mut self, slot: SlotKind) {
        let target = match slot {
            SlotKind::Main => &mut self.main_next,
            SlotKind::Translation => &mut self.translation_next,
            SlotKind::Bible => &mut self.bible_next,
            SlotKind::BibleTranslation => &mut self.bible_translation_next,
        };
        *target = match *target {
            LaneTarget::A => LaneTarget::B,
            LaneTarget::B => LaneTarget::A,
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipTarget {
    pub clip_id: i64,
    pub text_param_id: Option<i64>,
    pub transforms: Vec<TextTransform>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextTransform {
    Uppercase,
    RemoveLineBreaks,
}

pub(super) fn select_lane_targets<'a>(
    lane: LaneTarget,
    lane_a: &'a [ClipTarget],
    lane_b: &'a [ClipTarget],
) -> (&'a [ClipTarget], &'a [ClipTarget]) {
    match lane {
        LaneTarget::A => (lane_a, lane_b),
        LaneTarget::B => (lane_b, lane_a),
    }
}

pub(super) fn apply_transforms<'a>(text: &'a str, transforms: &[TextTransform]) -> Cow<'a, str> {
    if transforms.is_empty() {
        return Cow::Borrowed(text);
    }

    let mut output: Cow<'a, str> = Cow::Borrowed(text);
    for transform in transforms {
        match transform {
            TextTransform::Uppercase => {
                output = Cow::Owned(output.to_uppercase());
            }
            TextTransform::RemoveLineBreaks => {
                let collapsed = output.split_whitespace().collect::<Vec<_>>().join(" ");
                output = Cow::Owned(collapsed);
            }
        }
    }
    output
}
