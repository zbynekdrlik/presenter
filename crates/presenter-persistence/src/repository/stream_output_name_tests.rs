//! #842: output names are capped like scene names. The editor can now create
//! and rename outputs, and a pasted paragraph must not end up in the output
//! select and the Companion catalog. Kept in its own file (`stream_tests.rs` is
//! near the file-size cap).
use super::stream_tests::{as_repo_error, repo};
use super::RepositoryError;
use presenter_core::stream::STREAM_SCENE_NAME_MAX;

#[tokio::test]
async fn an_output_name_longer_than_the_cap_is_refused() {
    let repo = repo().await;
    let too_long = "x".repeat(STREAM_SCENE_NAME_MAX + 1);
    let err = repo
        .create_stream_output("name-cap-842", &too_long)
        .await
        .expect_err("a 101-character name is refused");
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));

    let at_cap = "é".repeat(STREAM_SCENE_NAME_MAX);
    let created = repo
        .create_stream_output("name-cap-842", &at_cap)
        .await
        .expect("100 characters (not bytes) are accepted");
    assert_eq!(created.name, at_cap);

    let err = repo
        .rename_stream_output("name-cap-842", &too_long)
        .await
        .expect_err("a rename to 101 characters is refused");
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
}
