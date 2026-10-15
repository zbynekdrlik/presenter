//! The microphones (#852 C1a, spec §5.6): the `app_settings` row
//! `listen.mics`, a JSON list edited through `GET|PUT /listen/mics` (the C1b
//! Settings card). An empty list = listening idle, no socket (PP, C-S8).
//!
//! A mic is selected by its VBAN stream name (exact, 1–16 printable ASCII) and
//! optionally the sender IP; two mics that could both receive one packet are
//! refused. The server is the only validator.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::IpAddr;
use thiserror::Error;

/// The `app_settings` key of the list.
pub const LISTEN_MICS_KEY: &str = "listen.mics";
/// Two are planned (guest + interpreter); room for a spare and a preacher.
pub const LISTEN_MAX_MICS: usize = 4;
pub const MIC_ID_MAX_CHARS: usize = 20;
pub const MIC_NAME_MAX_CHARS: usize = 40;
/// = the VBAN stream-name field.
pub const MIC_STREAM_NAME_MAX_CHARS: usize = 16;

/// Which channel of the stream the mic is (spec §5.4): `left` = channel 1,
/// `right` = channel 2 (channel 1 of a mono stream), `mix` = the mean of all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicChannel {
    Left,
    Right,
    Mix,
}

/// What is spoken into the mic (owner: per mic); `off` = not transcribed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicLanguage {
    En,
    Sk,
    Off,
}

/// Who speaks into the mic (spec §5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicRole {
    Guest,
    Interpreter,
    Preacher,
}

/// One microphone of `listen.mics`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicConfig {
    pub id: String,
    pub name: String,
    pub stream_name: String,
    /// `None` = any sender.
    #[serde(default)]
    pub sender_ip: Option<String>,
    pub channel: MicChannel,
    pub language: MicLanguage,
    pub role: MicRole,
}

impl MicConfig {
    /// The sender IP (a validated list holds only valid ones). The receiver
    /// calls it once per saved list, never per packet.
    pub fn sender_ip_addr(&self) -> Option<IpAddr> {
        self.sender_ip.as_deref().and_then(|ip| ip.parse().ok())
    }
}

/// Why a list was refused (HTTP 422, shown as it is).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MicsValidationError {
    #[error("Najviac 4 mikrofóny.")]
    TooMany,
    #[error("Identifikátor mikrofónu „{0}“ môže mať 1–20 znakov a–z, 0–9 alebo „-“.")]
    BadId(String),
    #[error("Identifikátor „{0}“ je použitý dvakrát.")]
    DuplicateId(String),
    #[error("Mikrofón „{0}“: názov môže mať 1–40 znakov.")]
    BadName(String),
    #[error("Mikrofón „{0}“: názov VBAN streamu môže mať 1–16 znakov ASCII bez medzery na kraji.")]
    BadStreamName(String),
    #[error("Mikrofón „{0}“: neplatná IP adresa odosielateľa.")]
    BadSenderIp(String),
    #[error("Mikrofóny „{0}“ a „{1}“ by dostávali ten istý VBAN stream.")]
    Ambiguous(String, String),
}

/// Validate and normalize a whole list: names trimmed, the sender IP in its
/// canonical form (an IPv4-mapped IPv6 address as plain IPv4, the form the
/// receiver compares the canonicalised peer address with), a blank IP = any
/// sender.
pub fn validate_mics(mics: Vec<MicConfig>) -> Result<Vec<MicConfig>, MicsValidationError> {
    if mics.len() > LISTEN_MAX_MICS {
        return Err(MicsValidationError::TooMany);
    }
    let mut ids = HashSet::new();
    let mut valid: Vec<MicConfig> = Vec::with_capacity(mics.len());
    for mic in mics {
        let mic = normalize_mic(mic)?;
        if !ids.insert(mic.id.clone()) {
            return Err(MicsValidationError::DuplicateId(mic.id));
        }
        if let Some(other) = valid.iter().find(|other| streams_overlap(other, &mic)) {
            return Err(MicsValidationError::Ambiguous(other.id.clone(), mic.id));
        }
        valid.push(mic);
    }
    Ok(valid)
}

/// The id of a new mic (#852 C1b: the Settings card never asks for one; the id
/// names the mic's recording files and never changes): its role's short name —
/// `guest`, `interp`, `preacher`, the spec §5.6 ids — then `-2`, `-3` … while
/// that is taken. Always a valid id.
pub fn mic_id_for_role(role: MicRole, taken: &[&str]) -> String {
    let base = match role {
        MicRole::Guest => "guest",
        MicRole::Interpreter => "interp",
        MicRole::Preacher => "preacher",
    };
    (1..=LISTEN_MAX_MICS + 1)
        .map(|n| match n {
            1 => base.to_string(),
            n => format!("{base}-{n}"),
        })
        .find(|id| !taken.contains(&id.as_str()))
        .unwrap_or_else(|| format!("{base}-{}", taken.len() + 1))
}

fn normalize_mic(mut mic: MicConfig) -> Result<MicConfig, MicsValidationError> {
    if !is_valid_mic_id(&mic.id) {
        return Err(MicsValidationError::BadId(mic.id));
    }
    let name = mic.name.trim();
    let chars = name.chars().count();
    if chars == 0 || chars > MIC_NAME_MAX_CHARS || name.chars().any(char::is_control) {
        return Err(MicsValidationError::BadName(mic.id));
    }
    mic.name = name.to_string();
    if !is_valid_stream_name(&mic.stream_name) {
        return Err(MicsValidationError::BadStreamName(mic.id));
    }
    let sender_ip = match mic
        .sender_ip
        .as_deref()
        .map(str::trim)
        .filter(|ip| !ip.is_empty())
    {
        None => None,
        Some(raw) => match raw.parse::<IpAddr>() {
            Ok(ip) => Some(ip.to_canonical().to_string()),
            Err(_) => return Err(MicsValidationError::BadSenderIp(mic.id)),
        },
    };
    mic.sender_ip = sender_ip;
    Ok(mic)
}

fn is_valid_mic_id(id: &str) -> bool {
    (1..=MIC_ID_MAX_CHARS).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_valid_stream_name(name: &str) -> bool {
    (1..=MIC_STREAM_NAME_MAX_CHARS).contains(&name.len())
        && name.bytes().all(|b| (0x20..0x7F).contains(&b))
        && name.trim() == name
}

/// Whether one packet could match both mics.
fn streams_overlap(a: &MicConfig, b: &MicConfig) -> bool {
    a.stream_name == b.stream_name
        && match (&a.sender_ip, &b.sender_ip) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec §5.6 example, verbatim.
    const SPEC_EXAMPLE: &str = r#"[{ "id": "guest", "name": "Hosť", "streamName": "GUEST_EN", "senderIp": null, "channel": "left",
   "language": "en", "role": "guest" },
 { "id": "interp", "name": "Tlmočník", "streamName": "INTERP_SK", "senderIp": null, "channel": "left",
   "language": "sk", "role": "interpreter" }]"#;

    fn mic(id: &str, stream: &str, ip: Option<&str>) -> MicConfig {
        MicConfig {
            id: id.to_string(),
            name: id.to_string(),
            stream_name: stream.to_string(),
            sender_ip: ip.map(str::to_string),
            channel: MicChannel::Left,
            language: MicLanguage::En,
            role: MicRole::Guest,
        }
    }

    #[test]
    fn the_spec_example_is_valid_and_round_trips() {
        let parsed: Vec<MicConfig> = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        let mics = validate_mics(parsed).unwrap();
        assert_eq!(mics.len(), 2);
        assert_eq!(
            (mics[1].language, mics[1].role, mics[1].channel),
            (MicLanguage::Sk, MicRole::Interpreter, MicChannel::Left)
        );
        let back = serde_json::to_value(&mics).unwrap();
        let spec: serde_json::Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        assert_eq!(back, spec);
        assert_eq!(
            validate_mics(Vec::new()),
            Ok(Vec::new()),
            "an empty list = listening idle"
        );
    }

    #[test]
    fn names_and_sender_ips_are_normalized() {
        let mut raw = mic("guest", "GUEST_EN", Some(" 192.0.2.10 "));
        raw.name = "  Hosť ".to_string();
        let mut v6 = mic("interp", "INTERP_SK", Some("2001:db8:0:0:0:0:0:1"));
        v6.name = "Tlmočník".to_string();
        let mut any = mic("pastor", "PASTOR", Some("   "));
        any.role = MicRole::Preacher;
        let mics = validate_mics(vec![raw, v6, any]).unwrap();
        assert_eq!(mics[0].name, "Hosť");
        assert_eq!(mics[0].sender_ip.as_deref(), Some("192.0.2.10"));
        assert_eq!(mics[1].sender_ip.as_deref(), Some("2001:db8::1"));
        assert_eq!(mics[2].sender_ip, None, "blank = any sender");
    }

    #[test]
    fn each_rule_has_its_own_refusal() {
        use MicsValidationError::*;
        let long = |n: usize| "a".repeat(n);
        let named = |name: &str| MicConfig {
            name: name.to_string(),
            ..mic("guest", "GUEST_EN", None)
        };
        let cases = [
            (
                (0..5)
                    .map(|k| mic(&format!("m{k}"), &format!("S{k}"), None))
                    .collect::<Vec<_>>(),
                TooMany,
            ),
            (vec![mic("Guest", "GUEST_EN", None)], BadId("Guest".into())),
            (vec![mic("", "GUEST_EN", None)], BadId(String::new())),
            (vec![mic(&long(21), "GUEST_EN", None)], BadId(long(21))),
            (vec![mic("a_b", "GUEST_EN", None)], BadId("a_b".into())),
            (
                vec![mic("guest", "A", None), mic("guest", "B", None)],
                DuplicateId("guest".into()),
            ),
            (vec![named("   ")], BadName("guest".into())),
            (vec![named(&"ž".repeat(41))], BadName("guest".into())),
            (vec![named("a\u{7}b")], BadName("guest".into())),
            (vec![mic("guest", "", None)], BadStreamName("guest".into())),
            (
                vec![mic("guest", "ABCDEFGHIJKLMNOPQ", None)],
                BadStreamName("guest".into()),
            ),
            (
                vec![mic("guest", "HOSŤ", None)],
                BadStreamName("guest".into()),
            ),
            (
                vec![mic("guest", " GUEST", None)],
                BadStreamName("guest".into()),
            ),
            (
                vec![mic("guest", "GUEST_EN", Some("10.0.0"))],
                BadSenderIp("guest".into()),
            ),
            (
                vec![
                    mic("guest", "GUEST_EN", None),
                    mic("interp", "GUEST_EN", Some("192.0.2.10")),
                ],
                Ambiguous("guest".into(), "interp".into()),
            ),
            (
                vec![
                    mic("guest", "GUEST_EN", Some("192.0.2.10")),
                    mic("interp", "GUEST_EN", Some("192.0.2.10")),
                ],
                Ambiguous("guest".into(), "interp".into()),
            ),
        ];
        for (mics, expected) in cases {
            assert_eq!(validate_mics(mics.clone()), Err(expected), "{mics:?}");
        }
        let two_senders = vec![
            mic("guest", "GUEST_EN", Some("192.0.2.10")),
            mic("interp", "GUEST_EN", Some("198.51.100.7")),
        ];
        assert!(
            validate_mics(two_senders).is_ok(),
            "one name from two fixed senders is fine"
        );
        assert!(
            validate_mics(vec![named(&"ž".repeat(40))]).is_ok(),
            "40 chars, not bytes"
        );
    }

    #[test]
    fn the_upper_edges_are_allowed() {
        // each refusal in `each_rule_has_its_own_refusal` is one past these:
        let four: Vec<MicConfig> = (0..4)
            .map(|k| mic(&format!("m{k}"), &format!("S{k}"), None))
            .collect();
        assert_eq!(validate_mics(four).map(|mics| mics.len()), Ok(4), "4 mics");
        assert!(
            validate_mics(vec![mic(&"a".repeat(20), "ABCDEFGHIJKLMNOP", None)]).is_ok(),
            "a 20-char id and a 16-char stream name"
        );
    }

    #[test]
    fn an_ipv4_mapped_sender_is_saved_as_ipv4_and_is_the_same_sender() {
        let mics =
            validate_mics(vec![mic("guest", "GUEST_EN", Some("::ffff:192.0.2.10"))]).unwrap();
        assert_eq!(mics[0].sender_ip.as_deref(), Some("192.0.2.10"));
        assert_eq!(
            mics[0].sender_ip_addr(),
            Some("192.0.2.10".parse().unwrap())
        );
        assert_eq!(
            validate_mics(vec![
                mic("guest", "GUEST_EN", Some("::ffff:192.0.2.10")),
                mic("interp", "GUEST_EN", Some("192.0.2.10")),
            ]),
            Err(MicsValidationError::Ambiguous(
                "guest".into(),
                "interp".into()
            ))
        );
    }

    #[test]
    fn the_messages_are_slovak_and_name_the_mic() {
        assert_eq!(
            MicsValidationError::TooMany.to_string(),
            "Najviac 4 mikrofóny."
        );
        let text = MicsValidationError::BadStreamName("guest".into()).to_string();
        assert!(text.contains("„guest“") && text.contains("16"), "{text}");
        let text = MicsValidationError::Ambiguous("guest".into(), "interp".into()).to_string();
        assert!(
            text.contains("„guest“") && text.contains("„interp“"),
            "{text}"
        );
    }

    #[test]
    fn the_sender_ip_of_a_validated_list_parses() {
        let mics = validate_mics(vec![
            mic("guest", "GUEST_EN", Some(" 192.0.2.10 ")),
            mic("interp", "INTERP_SK", None),
        ])
        .unwrap();
        assert_eq!(
            mics[0].sender_ip_addr(),
            Some("192.0.2.10".parse().unwrap())
        );
        assert_eq!(mics[1].sender_ip_addr(), None);
    }

    #[test]
    fn unknown_enum_values_are_refused_by_serde() {
        let json = SPEC_EXAMPLE.replacen(r#""channel": "left""#, r#""channel": "center""#, 1);
        assert!(serde_json::from_str::<Vec<MicConfig>>(&json).is_err());
    }

    #[test]
    fn a_new_mics_id_comes_from_its_role_and_is_never_taken_twice() {
        assert_eq!(mic_id_for_role(MicRole::Guest, &[]), "guest");
        assert_eq!(mic_id_for_role(MicRole::Interpreter, &[]), "interp");
        assert_eq!(mic_id_for_role(MicRole::Preacher, &[]), "preacher");
        assert_eq!(mic_id_for_role(MicRole::Guest, &["guest"]), "guest-2");
        assert_eq!(
            mic_id_for_role(MicRole::Guest, &["guest", "guest-2", "guest-3", "guest-4"]),
            "guest-5",
            "a full list of 4 still gets a free id"
        );
        for id in ["guest", "interp", "preacher", "guest-5"] {
            assert!(is_valid_mic_id(id), "{id}");
        }
        let spec = validate_mics(serde_json::from_str(SPEC_EXAMPLE).unwrap()).unwrap();
        assert_eq!(
            (spec[0].id.as_str(), spec[1].id.as_str()),
            (
                mic_id_for_role(MicRole::Guest, &[]).as_str(),
                mic_id_for_role(MicRole::Interpreter, &[]).as_str()
            ),
            "the spec §5.6 ids"
        );
    }

    #[test]
    fn a_new_mics_id_is_free_even_past_the_list_limit() {
        // Not reachable through a valid list (≤ 4 mics), but the fallback must
        // never hand out a taken id whatever the caller passes.
        let taken = ["guest", "guest-2", "guest-3", "guest-4", "guest-5", "guest-7"];
        let id = mic_id_for_role(MicRole::Guest, &taken);
        assert!(!taken.contains(&id.as_str()), "{id} is taken");
        assert_eq!(id, "guest-6");
    }
}
