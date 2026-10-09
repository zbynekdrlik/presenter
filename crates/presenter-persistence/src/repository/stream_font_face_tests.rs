//! #830 re-derived font faces: listing a family's faces and rewriting a face's
//! weight/italic. Kept in its own file (`stream_tests.rs` is at the file-size
//! cap).
use super::stream_tests::{new_font, repo};

#[tokio::test]
async fn faces_of_a_family_are_listed_and_their_weight_updated() {
    let repo = repo().await;
    let light = repo
        .insert_or_get_stream_font(new_font("s830-l", "Nexa", 400, false))
        .await
        .unwrap();
    let black = repo
        .insert_or_get_stream_font(new_font("s830-b", "Nexa", 400, false))
        .await
        .unwrap();
    repo.insert_or_get_stream_font(new_font("s830-o", "Other", 400, false))
        .await
        .unwrap();
    let ids: Vec<i64> = repo
        .stream_fonts_of_family("Nexa")
        .await
        .unwrap()
        .iter()
        .map(|f| f.id)
        .collect();
    assert_eq!(
        ids,
        vec![light.id, black.id],
        "only the family, oldest first"
    );

    assert!(repo
        .update_stream_font_face(black.id, 900, true)
        .await
        .unwrap());
    let row = repo.get_stream_font(black.id).await.unwrap();
    assert_eq!((row.weight, row.italic), (900, true));
    assert_eq!(row.sha256, "s830-b", "only weight + italic change");
    let untouched = repo.get_stream_font(light.id).await.unwrap();
    assert_eq!((untouched.weight, untouched.italic), (400, false));
}

#[tokio::test]
async fn updating_a_face_deleted_meanwhile_reports_nothing_updated() {
    // A delete can race the re-derive (which holds only its own lock): the
    // update must report "no row", not fail the whole pass.
    let repo = repo().await;
    assert!(!repo
        .update_stream_font_face(999_999, 700, false)
        .await
        .unwrap());
}
