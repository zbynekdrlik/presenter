use super::*;
use crate::{
    id::PlaylistEntryId,
    playlist::PlaylistEntryKind,
    search::{fold_query, normalise_for_search, query_tokens},
    slide::SlideGroup,
};

fn sample_slide(order: u32, group: Option<&str>) -> Slide {
    Slide::new(
        order,
        SlideContent::new(
            SlideText::new(format!("Slide {order}")).unwrap(),
            SlideText::new("Translation").unwrap(),
            SlideText::new("Stage").unwrap(),
            group.map(SlideGroup::new),
        ),
    )
}

#[test]
fn library_and_playlist_share_ids_without_collision() {
    let first_presentation = Presentation::new("Test", vec![sample_slide(0, None)]).unwrap();
    let second_presentation = Presentation::new("Second", vec![sample_slide(0, None)]).unwrap();

    let mut library = Library::new("Lib", vec![first_presentation.clone()]).unwrap();
    library
        .add_presentation(second_presentation.clone())
        .unwrap();

    let playlist = Playlist::new(
        "Set",
        vec![
            PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Presentation {
                    presentation_id: first_presentation.id,
                    midi_binding: Some(crate::playlist::MidiBinding::new(1).unwrap()),
                    presentation_name: None,
                },
            },
            PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Presentation {
                    presentation_id: second_presentation.id,
                    midi_binding: Some(crate::playlist::MidiBinding::new(2).unwrap()),
                    presentation_name: None,
                },
            },
        ],
    )
    .unwrap();

    assert_eq!(playlist.entries.len(), 2);
    assert!(library.get(first_presentation.id).is_some());
}

#[test]
fn fold_query_strips_accents_and_punctuation() {
    let folded = fold_query("Ježiš, ja!");
    assert_eq!(folded, "jezis ja");
}

#[test]
fn query_tokens_split_on_commas_and_spaces() {
    let tokens = query_tokens("jezis, ja tymy");
    assert_eq!(tokens, vec!["jezis", "ja", "tymy"]);
}

#[test]
fn normalise_for_search_handles_mixed_scripts() {
    assert_eq!(
        normalise_for_search("Žalm 23 – PÁSTIER"),
        "zalm 23 – pastier"
    );
    assert_eq!(normalise_for_search("Čítanie"), "citanie");
    assert_eq!(normalise_for_search(" ТЕСТ "), " тест ");
}
