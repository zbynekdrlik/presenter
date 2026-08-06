//! #655 F10: `AppState`-level integration tests for the AbleSet mismatch
//! acknowledgement HTTP/persistence surface — a gap left uncovered by
//! `ableset_mismatch.rs`'s pure comparison-logic unit tests. Same pattern as
//! `sync_integration_tests.rs`: real `AppState`, real (in-memory) DB, a real
//! wiremock-backed AbleSet bridge where the pipeline genuinely needs one.

use std::time::Duration;

use presenter_core::AbleSetSettingsDraft;
use presenter_persistence::SettingsAuditSource;

use super::ableset_ack::AbleSetAckRefusal;
use super::AppState;

/// Apply AbleSet settings with a given `song_prefix_length`, disabled (no
/// tracker/wiremock needed) — enough for the pure validation tests below,
/// which only need `AppState::ableset_bridge`'s cached `song_prefix_length`.
async fn set_prefix_length(state: &AppState, song_prefix_length: u8) {
    let draft = AbleSetSettingsDraft {
        enabled: false,
        host: "ableset.invalid".to_string(),
        osc_port: 39051,
        http_port: 80,
        library_name: "Hymnal".to_string(),
        song_prefix_length,
    };
    state
        .update_ableset_settings(draft, SettingsAuditSource::HttpSetter, "test")
        .await
        .unwrap();
}

fn expect_ack_refusal(err: anyhow::Error) -> AbleSetAckRefusal {
    match err.downcast::<AbleSetAckRefusal>() {
        Ok(refusal) => refusal,
        Err(err) => panic!("expected a typed AbleSetAckRefusal, got: {err:?}"),
    }
}

/// #655 F9d: a malformed number (wrong length here) must be refused before
/// ever touching the ack store.
#[tokio::test]
async fn invalid_number_ack_is_refused() {
    let state = AppState::in_memory().await.unwrap();
    set_prefix_length(&state, 3).await;

    let err = state
        .acknowledge_ableset_mismatch("17", "A title", "B title")
        .await
        .unwrap_err();
    assert!(
        matches!(
            expect_ack_refusal(err),
            AbleSetAckRefusal::InvalidNumber { .. }
        ),
        "a number of the wrong length must be refused as InvalidNumber"
    );
}

/// #655 F9d: unacknowledge validates the number too, for the same reason.
#[tokio::test]
async fn invalid_number_unack_is_refused() {
    let state = AppState::in_memory().await.unwrap();
    set_prefix_length(&state, 3).await;

    let err = state
        .unacknowledge_ableset_mismatch("abc")
        .await
        .unwrap_err();
    assert!(matches!(
        expect_ack_refusal(err),
        AbleSetAckRefusal::InvalidNumber { .. }
    ));
}

/// #655 F4: a one-sided structural gap (one title empty) can never be
/// acknowledged away.
#[tokio::test]
async fn gap_ack_is_refused_as_structural() {
    let state = AppState::in_memory().await.unwrap();
    set_prefix_length(&state, 3).await;

    let err = state
        .acknowledge_ableset_mismatch("017", "", "017 Real Title")
        .await
        .unwrap_err();
    assert!(
        matches!(expect_ack_refusal(err), AbleSetAckRefusal::StructuralGap),
        "a one-sided gap ack must be refused as StructuralGap, never accepted"
    );
}

/// #655 F9c: the ack store is capped — the entry PAST the cap is refused.
#[tokio::test]
async fn ack_store_full_refuses_a_new_entry_past_the_cap() {
    let state = AppState::in_memory().await.unwrap();
    set_prefix_length(&state, 3).await;

    for i in 0..500u32 {
        let number = format!("{i:03}");
        state
            .acknowledge_ableset_mismatch(&number, "A", "B")
            .await
            .unwrap_or_else(|err| panic!("seeding ack {number} must succeed: {err:?}"));
    }

    // A number ALREADY in the store must still be updatable (not blocked by
    // the cap it is already counted in).
    state
        .acknowledge_ableset_mismatch("000", "A2", "B2")
        .await
        .expect("updating an EXISTING ack must not be refused by the cap");

    // A genuinely NEW number, with the store already full, must be refused.
    let err = state
        .acknowledge_ableset_mismatch("999", "A", "B")
        .await
        .unwrap_err();
    assert!(matches!(
        expect_ack_refusal(err),
        AbleSetAckRefusal::StoreFull
    ));
}

/// #655 F9: a corrupt persisted ack blob must degrade to empty, never panic
/// or propagate an error to the caller.
#[tokio::test]
async fn corrupt_ack_blob_degrades_to_empty_without_erroring() {
    let state = AppState::in_memory().await.unwrap();
    state
        .repository()
        .set_app_setting("ableset_mismatch_acks", "not valid json {{{")
        .await
        .unwrap();

    let acks = state
        .load_ableset_mismatch_acks()
        .await
        .expect("a corrupt blob must not error the caller");
    assert!(
        acks.is_empty(),
        "a corrupt ack blob must degrade to empty, not panic or propagate"
    );
}

/// #655 F5c/F8/F9e end-to-end: a genuine title disagreement is reported,
/// acknowledging it silences it via the LIGHTWEIGHT recompute (no full DB
/// rescan), and revoking the ack re-raises it.
#[tokio::test]
async fn ack_persists_and_silences_mismatch_then_unack_restores_it() {
    let state = AppState::in_memory().await.unwrap();
    let library = state.create_library("Hymnal").await.unwrap();
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "017 Real Title", None)
        .await
        .unwrap();

    let mock_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/setlist"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "activeSongId": null,
                "songs": [{ "id": "s1", "meta": { "name": "017 Different Title" } }]
            })),
        )
        .mount(&mock_server)
        .await;
    let addr = mock_server.address();

    let draft = AbleSetSettingsDraft {
        enabled: true,
        host: addr.ip().to_string(),
        osc_port: 39051,
        http_port: addr.port(),
        library_name: "Hymnal".to_string(),
        song_prefix_length: 3,
    };
    state
        .update_ableset_settings(draft, SettingsAuditSource::HttpSetter, "test")
        .await
        .unwrap();

    // Wait for the live tracker to actually poll the mock AbleSet server.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if !state.ableset_bridge.setlist_song_titles().await.is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "AbleSet setlist never populated from the mock server"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let resolved = state.resolve_ableset_presentation("017").await.unwrap();
    assert_eq!(resolved, Some(presentation.id));

    let before_ack = state.ableset_status_snapshot().await;
    assert_eq!(
        before_ack.mismatch_count, 1,
        "the genuine title disagreement must be reported before any ack"
    );

    state
        .acknowledge_ableset_mismatch("017", "017 Different Title", "017 Real Title")
        .await
        .expect("acknowledging a genuine (non-gap) mismatch must succeed");

    let after_ack = state.ableset_status_snapshot().await;
    assert_eq!(
        after_ack.mismatch_count, 0,
        "an acknowledged mismatch must be silenced immediately by the lightweight recompute"
    );

    state
        .unacknowledge_ableset_mismatch("017")
        .await
        .expect("unacknowledging must succeed");

    let after_unack = state.ableset_status_snapshot().await;
    assert_eq!(
        after_unack.mismatch_count, 1,
        "revoking the ack must re-raise the warning on the next recompute"
    );
}
