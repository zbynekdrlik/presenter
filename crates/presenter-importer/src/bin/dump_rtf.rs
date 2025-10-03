use std::path::PathBuf;

use anyhow::{Context, Result};
use presenter_importer::proto::Presentation;
use presenter_importer::{decode_rtf, proto};
use prost::Message;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .context("usage: dump_rtf <presentation.pro>")?;
    let bytes =
        std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let raw = Presentation::decode(bytes.as_slice())
        .with_context(|| format!("failed to decode presentation {}", path.display()))?;

    for cue in &raw.cues {
        for action in &cue.actions {
            if matches!(
                proto::action::ActionType::from_i32(action.r#type),
                Some(proto::action::ActionType::PresentationSlide)
            ) {
                if let Some(proto::action::ActionTypeData::Slide(slide_type)) =
                    &action.action_type_data
                {
                    if let Some(proto::action::slide_type::Slide::Presentation(slide)) =
                        &slide_type.slide
                    {
                        if let Some(base) = slide.base_slide.as_ref() {
                            for element in &base.elements {
                                if let Some(graphic) = &element.element {
                                    if let Some(text) = &graphic.text {
                                        if text.rtf_data.is_empty() {
                                            continue;
                                        }
                                        let raw_str = String::from_utf8_lossy(&text.rtf_data);
                                        let decoded = decode_rtf(&text.rtf_data)?;
                                        println!("==== {} ====", graphic.name);
                                        println!("RAW:\n{}", raw_str);
                                        println!("DECODED:\n{}\n", decoded);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
