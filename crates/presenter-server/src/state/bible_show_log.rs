//! One INFO line per Bible show (#850 spec §8): target
//! `presenter::bible::show` with the source endpoint, the device, the client
//! IP and the references that went to the wall.

use presenter_core::BibleSlideOutput;

/// The endpoint a Bible show came from. B2a adds the console, A2b Companion
/// and the AI tools when the legacy trigger is rerouted here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BibleShowSource {
    /// `POST /bible/trigger-slide`.
    Api,
    /// `POST /bible/presentations/{id}/slides/{slide_id}/trigger`.
    Prepared,
}

impl BibleShowSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Prepared => "prepared",
        }
    }
}

/// Who asked for a show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibleShowOrigin {
    pub source: BibleShowSource,
    /// `touch` / `desktop` once the console sends it (#850 B2a).
    pub device: Option<String>,
    pub client_ip: String,
}

/// The fields of one `presenter::bible::show` line; `-` for a missing value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BibleShowLine<'a> {
    pub(crate) source: &'static str,
    pub(crate) device: &'a str,
    pub(crate) client_ip: &'a str,
    pub(crate) reference: &'a str,
    pub(crate) secondary: &'a str,
}

impl<'a> BibleShowLine<'a> {
    pub(crate) fn new(origin: &'a BibleShowOrigin, output: &'a BibleSlideOutput) -> Self {
        Self {
            source: origin.source.as_str(),
            device: origin.device.as_deref().unwrap_or("-"),
            client_ip: &origin.client_ip,
            reference: &output.main_reference,
            secondary: if output.secondary_reference.is_empty() {
                "-"
            } else {
                &output.secondary_reference
            },
        }
    }
}

pub(crate) fn log_bible_show(origin: &BibleShowOrigin, output: &BibleSlideOutput) {
    let line = BibleShowLine::new(origin, output);
    tracing::info!(
        target: "presenter::bible::show",
        source = line.source,
        device = line.device,
        client_ip = line.client_ip,
        reference = line.reference,
        secondary = line.secondary,
        "bible show"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn output(secondary_reference: &str) -> BibleSlideOutput {
        BibleSlideOutput::new(
            "Viera je podstatou toho, v čo dúfame",
            "Hebrejom 11:1 (SEB)",
            "",
            secondary_reference,
            Utc::now(),
        )
    }

    #[test]
    fn the_line_carries_source_device_ip_and_both_references() {
        let origin = BibleShowOrigin {
            source: BibleShowSource::Prepared,
            device: Some("touch".to_string()),
            client_ip: "192.0.2.7".to_string(),
        };
        let output = output("Hebrews 11:1 (NLT)");
        assert_eq!(
            BibleShowLine::new(&origin, &output),
            BibleShowLine {
                source: "prepared",
                device: "touch",
                client_ip: "192.0.2.7",
                reference: "Hebrejom 11:1 (SEB)",
                secondary: "Hebrews 11:1 (NLT)",
            }
        );
    }

    #[test]
    fn a_missing_device_and_secondary_are_dashes() {
        let origin = BibleShowOrigin {
            source: BibleShowSource::Api,
            device: None,
            client_ip: "anonymous".to_string(),
        };
        let output = output("");
        let line = BibleShowLine::new(&origin, &output);
        assert_eq!(
            (line.source, line.device, line.secondary),
            ("api", "-", "-")
        );
    }
}
