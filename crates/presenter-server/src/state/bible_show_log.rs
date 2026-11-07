//! One INFO line per Bible show (#850 spec §8): target
//! `presenter::bible::show` with the source endpoint, the device, the client
//! IP and the references that went to the wall.

use presenter_core::BibleSlideOutput;

/// The endpoint a Bible show came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BibleShowSource {
    /// `POST /bible/trigger-slide`.
    Api,
    /// `POST /bible/presentations/{id}/slides/{slide_id}/trigger`.
    Prepared,
    /// The same endpoint on a sermon's slide: logged as `prepared`, but its text
    /// is the preacher's cut, so "Táto bohoslužba" leaves it out (#850 B2d).
    Sermon,
    /// `POST /bible/trigger-slide` from the verse console (#850 B2a).
    Console,
    /// The legacy reference trigger `POST /bible/trigger` (#850 A2b).
    Trigger,
    /// Companion's `bible.trigger` command (#850 A2b).
    Companion,
    /// The AI `trigger_bible_verse` tool (#850 A2b).
    Ai,
}

impl BibleShowSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Prepared | Self::Sermon => "prepared",
            Self::Console => "console",
            Self::Trigger => "trigger",
            Self::Companion => "companion",
            Self::Ai => "ai",
        }
    }

    /// `/bible/trigger-slide`'s source: `console` when the client says so,
    /// otherwise the plain API.
    pub fn of_trigger_slide(claimed: Option<&str>) -> Self {
        if claimed == Some("console") {
            Self::Console
        } else {
            Self::Api
        }
    }
}

/// The device a client names, only when it is one the log knows (`touch`,
/// `desktop`); anything else (a client-controlled string) is dropped.
pub fn client_device(claimed: Option<&str>) -> Option<String> {
    claimed
        .filter(|device| matches!(*device, "touch" | "desktop"))
        .map(str::to_string)
}

/// Who asked for a show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibleShowOrigin {
    pub source: BibleShowSource,
    /// `touch` / `desktop` once the console sends it (#850 B2a).
    pub device: Option<String>,
    pub client_ip: String,
}

impl BibleShowOrigin {
    /// A show with no HTTP client of its own (Companion's socket, the AI
    /// tool): no device, client IP `-`.
    pub fn internal(source: BibleShowSource) -> Self {
        Self {
            source,
            device: None,
            client_ip: "-".to_string(),
        }
    }
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
    fn only_the_console_claims_a_source_and_only_known_devices_are_kept() {
        assert_eq!(
            BibleShowSource::of_trigger_slide(Some("console")),
            BibleShowSource::Console
        );
        assert_eq!(
            BibleShowSource::of_trigger_slide(Some("prepared")),
            BibleShowSource::Api
        );
        assert_eq!(
            BibleShowSource::of_trigger_slide(None),
            BibleShowSource::Api
        );
        assert_eq!(client_device(Some("touch")).as_deref(), Some("touch"));
        assert_eq!(client_device(Some("desktop")).as_deref(), Some("desktop"));
        assert_eq!(client_device(Some("x\" injected=\"1")), None);
        assert_eq!(client_device(None), None);
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
