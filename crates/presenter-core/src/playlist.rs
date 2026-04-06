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
                },
            },
            PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Presentation {
                    presentation_id: PresentationId::new(),
                    midi_binding: Some(MidiBinding::new(1).unwrap()),
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
                    },
                },
                PlaylistEntry {
                    id: PlaylistEntryId::new(),
                    kind: PlaylistEntryKind::Presentation {
                        presentation_id: PresentationId::new(),
                        midi_binding: None,
                    },
                },
            ],
        )
        .unwrap();

        assert_eq!(playlist.position_for_midi(10), Some(0));
        assert_eq!(playlist.position_for_midi(11), None);
    }
}
