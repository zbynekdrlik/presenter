use super::*;
use crate::slide::SlideGroup;

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
                presentation_id: first_presentation.id,
                midi_binding: Some(crate::playlist::MidiBinding::new(1).unwrap()),
            },
            PlaylistEntry {
                presentation_id: second_presentation.id,
                midi_binding: Some(crate::playlist::MidiBinding::new(2).unwrap()),
            },
        ],
    )
    .unwrap();

    assert_eq!(playlist.entries.len(), 2);
    assert!(library.get(first_presentation.id).is_some());
}
