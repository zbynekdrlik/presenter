use rosc::{OscMessage, OscType};

use super::handler::ListenerConfig;

#[derive(Debug)]
pub(super) enum Extracted {
    Complete(u8, u8),
    Pending {
        channel: String,
        note: Option<u8>,
        velocity: Option<u8>,
    },
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SplitKind {
    Note,
    Velocity,
}

pub(super) fn extract_note_velocity(message: &OscMessage, config: &ListenerConfig) -> Extracted {
    if let Some((note, velocity)) = parse_compound_message(message, config) {
        return Extracted::Complete(note, velocity);
    }
    if let Some((channel, kind, value)) = parse_split_message(message) {
        return match kind {
            SplitKind::Note => Extracted::Pending {
                channel,
                note: Some(value),
                velocity: None,
            },
            SplitKind::Velocity => Extracted::Pending {
                channel,
                note: None,
                velocity: Some(value),
            },
        };
    }
    Extracted::Ignore
}

fn parse_compound_message(message: &OscMessage, config: &ListenerConfig) -> Option<(u8, u8)> {
    let addr = message.addr.to_ascii_lowercase();
    let pattern = config.address_pattern.to_ascii_lowercase();
    if addr != pattern {
        return None;
    }
    let mut numeric = Vec::new();
    for arg in &message.args {
        match arg {
            OscType::Int(value) => numeric.push(*value as f64),
            OscType::Float(value) => numeric.push(*value as f64),
            _ => {}
        }
    }
    if numeric.len() >= 3 {
        let note = numeric[numeric.len() - 2];
        let velocity = numeric[numeric.len() - 1];
        return normalise_note_velocity(note, velocity);
    }
    if numeric.len() >= 2 {
        return normalise_note_velocity(numeric[0], numeric[1]);
    }
    None
}

fn parse_split_message(message: &OscMessage) -> Option<(String, SplitKind, u8)> {
    if message.args.is_empty() {
        return None;
    }
    let value = match &message.args[0] {
        OscType::Int(value) => *value as f64,
        OscType::Float(value) => *value as f64,
        _ => return None,
    };
    let value = normalise_single(value)?;

    let addr = message.addr.to_ascii_lowercase();
    if !addr.starts_with('/') {
        return None;
    }
    let mut prefix = String::new();
    let mut digits = String::new();
    for ch in addr.chars().skip(1) {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            prefix.push(ch);
        }
    }
    let kind = match prefix.as_str() {
        "note" => SplitKind::Note,
        "velocity" => SplitKind::Velocity,
        _ => return None,
    };
    let channel = if digits.is_empty() {
        "0".to_string()
    } else {
        digits
    };
    Some((channel, kind, value))
}

fn normalise_note_velocity(note: f64, velocity: f64) -> Option<(u8, u8)> {
    let note = normalise_single(note)?;
    let velocity = normalise_single(velocity)?;
    Some((note, velocity))
}

fn normalise_single(value: f64) -> Option<u8> {
    let rounded = value.round();
    if !(0.0..=127.0).contains(&rounded) {
        return None;
    }
    Some(rounded as u8)
}
