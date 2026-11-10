//! The microphones (#852 C1a, spec §5.6): the `app_settings` row
//! `listen.mics`, a JSON list edited through `GET|PUT /listen/mics` (the C1b
//! Settings card). An empty list = listening idle, no socket (PP, C-S8).
//!
//! A mic arrives over VBAN (the default) or Dante (#852, dstack's `dlink`). A
//! VBAN mic is selected by its stream name (exact, 1–16 printable ASCII) and
//! optionally the sender IP; two mics that could both receive one packet are
//! refused. A Dante mic is one RX channel of Presenter's own Dante device,
//! named like Dante names it, optionally subscribed to `channel@device`. The
//! server is the only validator.

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
/// A Dante channel or device name (dstack `device::MAX_NAME`).
pub const DANTE_NAME_MAX_CHARS: usize = 31;

/// How the mic's audio reaches Presenter. Absent on the wire = `vban` (every
/// list stored before #852's Dante slice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicTransport {
    #[default]
    Vban,
    Dante,
}

impl MicTransport {
    /// Serde's `skip_serializing_if`: a VBAN mic is written as before.
    pub fn is_vban(&self) -> bool {
        *self == Self::Vban
    }
}

/// A Dante mic's input: one RX channel of Presenter's Dante device.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DanteInput {
    /// Our RX channel's name, as Dante Controller lists it.
    pub channel: String,
    /// `channel@device` it subscribes to; `None` = set in Dante Controller.
    #[serde(default)]
    pub source: Option<String>,
}

impl DanteInput {
    /// The source split into (channel, device); `None` without one (a
    /// validated list holds only valid sources).
    pub fn source_parts(&self) -> Option<(&str, &str)> {
        self.source
            .as_deref()
            .and_then(|source| source.split_once('@'))
    }
}

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
    /// The VBAN stream; empty (and not written) for a Dante mic.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stream_name: String,
    /// `None` = any sender.
    #[serde(default)]
    pub sender_ip: Option<String>,
    pub channel: MicChannel,
    pub language: MicLanguage,
    pub role: MicRole,
    #[serde(default, skip_serializing_if = "MicTransport::is_vban")]
    pub transport: MicTransport,
    /// A Dante mic's channel (and source); `None` for a VBAN mic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dante: Option<DanteInput>,
}

impl MicConfig {
    pub fn is_dante(&self) -> bool {
        self.transport == MicTransport::Dante
    }

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
    #[error("Mikrofón „{0}“: názov kanála Dante môže mať 1–31 znakov ASCII, bez „=“, „.“ a „@“.")]
    BadDanteChannel(String),
    #[error("Mikrofón „{0}“: zdroj Dante má tvar kanál@zariadenie (zariadenie: 1–31 znakov A–Z, a–z, 0–9 a „-“).")]
    BadDanteSource(String),
    #[error("Mikrofóny „{0}“ a „{1}“ majú ten istý kanál Dante.")]
    DuplicateDanteChannel(String, String),
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
        if let Some(other) = valid.iter().find(|other| dante_overlap(other, &mic)) {
            return Err(MicsValidationError::DuplicateDanteChannel(
                other.id.clone(),
                mic.id,
            ));
        }
        valid.push(mic);
    }
    Ok(valid)
}

/// The id of a new mic (#852 C1b: the Settings card never asks for one; the id
/// names the mic's recording files and never changes): its role's short name —
/// `guest`, `interp`, `preacher`, the spec §5.6 ids — then `-2`, `-3` … while
/// that is taken. Always a valid id, never a taken one: of the first
/// `taken.len() + 1` candidates at least one is free.
pub fn mic_id_for_role(role: MicRole, taken: &[&str]) -> String {
    let base = match role {
        MicRole::Guest => "guest",
        MicRole::Interpreter => "interp",
        MicRole::Preacher => "preacher",
    };
    (1..=taken.len() + 1)
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
    match mic.transport {
        MicTransport::Vban => normalize_vban(mic),
        MicTransport::Dante => normalize_dante(mic),
    }
}

/// A Dante mic: a valid RX channel name, an optional valid source, no VBAN
/// fields; one mono channel, so `channel` is `left`.
fn normalize_dante(mut mic: MicConfig) -> Result<MicConfig, MicsValidationError> {
    let Some(mut dante) = mic.dante.take() else {
        return Err(MicsValidationError::BadDanteChannel(mic.id));
    };
    dante.channel = dante.channel.trim().to_string();
    if !is_valid_dante_channel_name(&dante.channel) {
        return Err(MicsValidationError::BadDanteChannel(mic.id));
    }
    dante.source = dante
        .source
        .as_deref()
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .map(str::to_string);
    if let Some(source) = dante.source.as_deref() {
        let valid = source.split_once('@').is_some_and(|(channel, device)| {
            is_valid_dante_channel_name(channel) && is_valid_dante_device_name(device)
        });
        if !valid {
            return Err(MicsValidationError::BadDanteSource(mic.id));
        }
    }
    mic.dante = Some(dante);
    mic.stream_name = String::new();
    mic.sender_ip = None;
    mic.channel = MicChannel::Left;
    Ok(mic)
}

fn normalize_vban(mut mic: MicConfig) -> Result<MicConfig, MicsValidationError> {
    mic.dante = None;
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

pub(crate) fn is_valid_mic_id(id: &str) -> bool {
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

/// A Dante channel name (dstack `device::valid_channel_name`): 1–31 printable
/// ASCII characters, none of `=`, `.` and `@`.
pub fn is_valid_dante_channel_name(name: &str) -> bool {
    (1..=DANTE_NAME_MAX_CHARS).contains(&name.len())
        && name.trim() == name
        && name
            .bytes()
            .all(|b| (0x20..=0x7e).contains(&b) && !matches!(b, b'=' | b'.' | b'@'))
}

/// A Dante device name (dstack `device::valid_device_name`): 1–31 of A–Z,
/// a–z, 0–9 and `-`, no `-` at either end.
pub fn is_valid_dante_device_name(name: &str) -> bool {
    (1..=DANTE_NAME_MAX_CHARS).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

/// Two Dante mics on one RX channel (Dante compares names case aside).
fn dante_overlap(a: &MicConfig, b: &MicConfig) -> bool {
    match (&a.dante, &b.dante) {
        (Some(x), Some(y)) => x.channel.eq_ignore_ascii_case(&y.channel),
        _ => false,
    }
}

/// Whether one packet could match both mics (VBAN mics only).
fn streams_overlap(a: &MicConfig, b: &MicConfig) -> bool {
    !a.is_dante()
        && !b.is_dante()
        && a.stream_name == b.stream_name
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
            transport: MicTransport::Vban,
            dante: None,
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
        let taken = [
            "guest", "guest-2", "guest-3", "guest-4", "guest-5", "guest-7",
        ];
        let id = mic_id_for_role(MicRole::Guest, &taken);
        assert!(!taken.contains(&id.as_str()), "{id} is taken");
        assert_eq!(id, "guest-6");
    }

    fn dante_mic(id: &str, channel: &str, source: Option<&str>) -> MicConfig {
        MicConfig {
            transport: MicTransport::Dante,
            dante: Some(DanteInput {
                channel: channel.to_string(),
                source: source.map(str::to_string),
            }),
            ..mic(id, "", None)
        }
    }

    #[test]
    fn a_stored_vban_list_reads_and_writes_exactly_as_before() {
        let parsed: Vec<MicConfig> = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        assert!(parsed
            .iter()
            .all(|m| m.transport == MicTransport::Vban && m.dante.is_none()));
        let back = serde_json::to_value(validate_mics(parsed).unwrap()).unwrap();
        let text = back.to_string();
        assert!(
            !text.contains("transport") && !text.contains("dante"),
            "a VBAN mic writes no new field: {text}"
        );
    }

    #[test]
    fn a_dante_mic_round_trips_without_vban_fields() {
        let json = r#"[{"id":"guest","name":"Hosť","channel":"mix","language":"en",
            "role":"guest","transport":"dante","streamName":"LEFTOVER","senderIp":"192.0.2.1",
            "dante":{"channel":" Hosť EN ","source":" Mic 1@stagebox-1 "}}]"#
            .replace("Hosť EN", "Guest EN");
        let mics = validate_mics(serde_json::from_str(&json).unwrap()).unwrap();
        let m = &mics[0];
        assert_eq!(m.transport, MicTransport::Dante);
        assert_eq!(
            m.dante,
            Some(DanteInput {
                channel: "Guest EN".into(),
                source: Some("Mic 1@stagebox-1".into())
            })
        );
        assert_eq!(
            (m.stream_name.as_str(), m.sender_ip.as_deref(), m.channel),
            ("", None, MicChannel::Left)
        );
        assert_eq!(
            m.dante.as_ref().unwrap().source_parts(),
            Some(("Mic 1", "stagebox-1"))
        );
        let back = serde_json::to_value(&mics).unwrap();
        assert_eq!(back[0]["transport"], "dante");
        assert!(back[0].get("streamName").is_none() && back[0]["senderIp"].is_null());
        let again: Vec<MicConfig> = serde_json::from_value(back).unwrap();
        assert_eq!(again, mics);
    }

    #[test]
    fn dante_mics_have_their_own_refusals() {
        use MicsValidationError::*;
        let mut missing = dante_mic("guest", "Mic", None);
        missing.dante = None;
        let cases = [
            (vec![missing], BadDanteChannel("guest".into())),
            (
                vec![dante_mic("guest", "", None)],
                BadDanteChannel("guest".into()),
            ),
            (
                vec![dante_mic("guest", "a.b", None)],
                BadDanteChannel("guest".into()),
            ),
            (
                vec![dante_mic("guest", "a@b", None)],
                BadDanteChannel("guest".into()),
            ),
            (
                vec![dante_mic("guest", &"a".repeat(32), None)],
                BadDanteChannel("guest".into()),
            ),
            (
                vec![dante_mic("guest", "Hosť", None)],
                BadDanteChannel("guest".into()),
            ),
            (
                vec![dante_mic("guest", "Mic", Some("Mic 1"))],
                BadDanteSource("guest".into()),
            ),
            (
                vec![dante_mic("guest", "Mic", Some("Mic 1@-box"))],
                BadDanteSource("guest".into()),
            ),
            (
                vec![dante_mic("guest", "Mic", Some("Mic=1@box"))],
                BadDanteSource("guest".into()),
            ),
            (
                vec![dante_mic("guest", "Mic", Some("@box"))],
                BadDanteSource("guest".into()),
            ),
            (
                vec![
                    dante_mic("guest", "Mic 1", None),
                    dante_mic("interp", "MIC 1", None),
                ],
                DuplicateDanteChannel("guest".into(), "interp".into()),
            ),
        ];
        for (mics, expected) in cases {
            assert_eq!(validate_mics(mics.clone()), Err(expected), "{mics:?}");
        }
        assert!(
            validate_mics(vec![dante_mic("guest", &"a".repeat(31), Some("Mic 1@box"))]).is_ok(),
            "31 chars and a source"
        );
        assert!(
            validate_mics(vec![dante_mic("guest", "Mic", Some("   "))]).is_ok_and(|m| m[0]
                .dante
                .as_ref()
                .unwrap()
                .source
                .is_none()),
            "a blank source = set in Dante Controller"
        );
    }

    #[test]
    fn a_dante_mic_never_collides_with_a_vban_stream() {
        let mixed = vec![
            mic("guest", "GUEST_EN", None),
            dante_mic("interp", "GUEST_EN", None),
            mic("preacher", "PASTOR", None),
        ];
        let mics = validate_mics(mixed).unwrap();
        assert_eq!(mics.iter().filter(|m| m.is_dante()).count(), 1);
        let mut vban_with_dante = mic("guest", "GUEST_EN", None);
        vban_with_dante.dante = Some(DanteInput {
            channel: "x".into(),
            source: None,
        });
        assert_eq!(validate_mics(vec![vban_with_dante]).unwrap()[0].dante, None);
        assert!(
            MicsValidationError::DuplicateDanteChannel("a".into(), "b".into())
                .to_string()
                .contains("Dante")
        );
    }
}
