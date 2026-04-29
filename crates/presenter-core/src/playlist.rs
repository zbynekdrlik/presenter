#![allow(clippy::collapsible_match)]

use crate::id::{PlaylistEntryId, PlaylistId, PresentationId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlaylistError {
    #[error("midi note {0} out of supported range 0-127")]
    MidiOutOfRange(u8),
    #[error("duplicate midi binding for note {0}")]
    DuplicateMidiBinding(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiBinding {
    note: u8,
}

impl MidiBinding {
    pub fn new(note: u8) -> Result<Self, PlaylistError> {
        if note > 127 {
            return Err(PlaylistError::MidiOutOfRange(note));
        }
        Ok(Self { note })
    }

    pub fn note(&self) -> u8 {
        self.note
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
    pub id: PlaylistEntryId,
    #[serde(flatten)]
    pub kind: PlaylistEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum PlaylistEntryKind {
    Presentation {
        presentation_id: PresentationId,
        #[serde(skip_serializing_if = "Option::is_none")]
        midi_binding: Option<MidiBinding>,
        /// Display name resolved by the server when serializing the playlist
        /// for the client. Stored as None internally; populated only on the
        /// response path. See `enrich_playlist_with_names` in the server.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        presentation_name: Option<String>,
    },
    Separator {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    #[serde(default)]
    pub show_in_dashboard: bool,
    pub entries: Vec<PlaylistEntry>,
}

impl Playlist {
    pub fn new<T: Into<String>>(
        name: T,
        entries: Vec<PlaylistEntry>,
    ) -> Result<Self, PlaylistError> {
        let playlist = Self {
            id: PlaylistId::new(),
            name: name.into(),
            show_in_dashboard: false,
            entries,
        };
        playlist.ensure_unique_midi()?;
        Ok(playlist)
    }

    pub fn with_id(mut self, id: PlaylistId) -> Self {
        self.id = id;
        self
    }

    pub fn from_parts(
        id: PlaylistId,
        name: String,
        show_in_dashboard: bool,
        entries: Vec<PlaylistEntry>,
    ) -> Result<Self, PlaylistError> {
        let playlist = Self {
            id,
            name,
            show_in_dashboard,
            entries,
        };
        playlist.ensure_unique_midi()?;
        Ok(playlist)
    }

    pub fn position_for_midi(&self, note: u8) -> Option<usize> {
        self.entries.iter().position(|entry| match &entry.kind {
            PlaylistEntryKind::Presentation { midi_binding, .. } => {
                midi_binding.map(|binding| binding.note()) == Some(note)
            }
            PlaylistEntryKind::Separator { .. } => false,
        })
    }

    #[allow(clippy::collapsible_match)]
    fn ensure_unique_midi(&self) -> Result<(), PlaylistError> {
        let mut seen = [false; 128];
        for entry in &self.entries {
            if let PlaylistEntryKind::Presentation { midi_binding, .. } = &entry.kind {
                if let Some(binding) = midi_binding {
                    let note = binding.note() as usize;
                    if seen[note] {
                        return Err(PlaylistError::DuplicateMidiBinding(binding.note()));
                    }
                    seen[note] = true;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod presentation_name_tests {
    use super::*;
    use crate::id::{PlaylistEntryId, PresentationId};

    #[test]
    fn presentation_entry_round_trips_presentation_name() {
        let id = PlaylistEntryId::new();
        let pres_id = PresentationId::new();
        let entry = PlaylistEntry {
            id,
            kind: PlaylistEntryKind::Presentation {
                presentation_id: pres_id,
                midi_binding: None,
                presentation_name: Some("My Song".to_string()),
            },
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        // serde serializes struct variant fields with the rename_all from the enum,
        // but flattening means field names come through as snake_case
        assert!(
            json.contains("\"presentation_name\":\"My Song\""),
            "expected presentation_name field in JSON; got: {json}"
        );
        let parsed: PlaylistEntry = serde_json::from_str(&json).expect("deserialize");
        match parsed.kind {
            PlaylistEntryKind::Presentation {
                presentation_name, ..
            } => assert_eq!(presentation_name.as_deref(), Some("My Song")),
            _ => panic!("expected Presentation"),
        }
    }

    #[test]
    fn presentation_entry_omits_name_when_none() {
        let entry = PlaylistEntry {
            id: PlaylistEntryId::new(),
            kind: PlaylistEntryKind::Presentation {
                presentation_id: PresentationId::new(),
                midi_binding: None,
                presentation_name: None,
            },
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        assert!(
            !json.contains("presentation_name"),
            "None should be skip_serializing_if; got: {json}"
        );
    }

    #[test]
    fn deserializing_legacy_payload_without_name_works() {
        // Fields use snake_case (serde flatten with tagged enum preserves snake_case)
        let json = r#"{"id":"00000000-0000-0000-0000-000000000001","type":"presentation","presentation_id":"00000000-0000-0000-0000-000000000002"}"#;
        let parsed: PlaylistEntry = serde_json::from_str(json).expect("deserialize");
        match parsed.kind {
            PlaylistEntryKind::Presentation {
                presentation_name, ..
            } => assert_eq!(presentation_name, None),
            _ => panic!("expected Presentation"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::PresentationId;

    #[test]
    fn midi_binding_range_check() {
        let binding = MidiBinding::new(127).unwrap();
        assert_eq!(binding.note(), 127);

        let binding_zero = MidiBinding::new(0).unwrap();
        assert_eq!(binding_zero.note(), 0);

        assert_eq!(
            MidiBinding::new(128).unwrap_err(),
            PlaylistError::MidiOutOfRange(128)
        );

        assert_eq!(
            MidiBinding::new(255).unwrap_err(),
            PlaylistError::MidiOutOfRange(255)
        );
    }

    #[test]
    fn playlist_rejects_duplicate_midi_binding() {
        let presentation_id = PresentationId::new();
        let entries = vec![
            PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Presentation {
                    presentation_id,
                    midi_binding: Some(MidiBinding::new(1).unwrap()),
                    presentation_name: None,
                },
            },
            PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Presentation {
                    presentation_id: PresentationId::new(),
                    midi_binding: Some(MidiBinding::new(1).unwrap()),
                    presentation_name: None,
                },
            },
        ];
        let err = Playlist::new("Set", entries).unwrap_err();
        assert_eq!(err, PlaylistError::DuplicateMidiBinding(1));
    }

    #[test]
    fn playlist_reports_position_for_midi_note() {
        let first = PresentationId::new();
        let playlist = Playlist::new(
            "Set",
            vec![
                PlaylistEntry {
                    id: PlaylistEntryId::new(),
                    kind: PlaylistEntryKind::Presentation {
                        presentation_id: first,
                        midi_binding: Some(MidiBinding::new(10).unwrap()),
                        presentation_name: None,
                    },
                },
                PlaylistEntry {
                    id: PlaylistEntryId::new(),
                    kind: PlaylistEntryKind::Presentation {
                        presentation_id: PresentationId::new(),
                        midi_binding: None,
                        presentation_name: None,
                    },
                },
            ],
        )
        .unwrap();

        assert_eq!(playlist.position_for_midi(10), Some(0));
        assert_eq!(playlist.position_for_midi(11), None);
    }
}
