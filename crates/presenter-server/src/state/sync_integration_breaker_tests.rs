//! #558 systemic-failure circuit-breaker tests — split out of
//! `sync_integration_tests.rs` when that file crossed the 1000-line hard
//! cap (mirrors the earlier `sync_tests.rs` → `sync_trash_tests.rs` split in
//! `presenter-persistence`). Tests moved verbatim; the shared `client()`
//! helper is duplicated locally (it's a 3-line `reqwest::Client` builder,
//! not worth threading a shared-test-support module for).
use crate::state::sync::{run_sync_cycle, run_sync_cycle_with_clients};
use crate::state::AppState;

fn client() -> reqwest::Client {
    reqwest::Client::builder().build().unwrap()
}

#[tokio::test]
async fn three_consecutive_transport_failures_trip_the_breaker_and_abort_the_cycle() {
    // #558 V7, narrowed by W2: the systemic-failure circuit breaker must
    // trip on a GENUINE peer-unreachable signal — connection refused/reset
    // or a request timeout — never on a per-song APPLICATION-level failure
    // (the peer answered, just badly, for one song). The original V7 test
    // simulated "peer unreachable" with bare 500 responses, which W2
    // identified as the WRONG signal (a 500 status is application-level,
    // not evidence the peer itself is down) — see the companion test below,
    // which proves 500s never trip the breaker. This test simulates a
    // GENUINE transport failure instead: a short client timeout racing a
    // delayed mock response, producing a real `reqwest::Error::is_timeout()`
    // for every per-song content fetch.
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;
    let b = AppState::in_memory().await.unwrap();

    let now = chrono::Utc::now();
    let entries: Vec<serde_json::Value> = (0..5)
        .map(|i| {
            serde_json::json!({
                "syncId": format!("sid-{i}"),
                "libraryName": "Songs",
                "name": format!("Song {i}"),
                "updatedAt": now,
                "deletedAt": null,
            })
        })
        .collect();

    Mock::given(method("GET"))
        .and(path("/sync/manifest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(entries))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/sync/presentations/.+$"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(300)))
        .mount(&mock_server)
        .await;

    // A short client timeout turns the delayed response into a genuine
    // transport-shaped `reqwest::Error` — the ONLY signal the breaker
    // counts (#558 W2).
    //
    // #558 X7: this 30ms budget must apply ONLY to the per-song content
    // fetches this test intends to time out — routing the (never-delayed)
    // manifest fetch through the SAME razor-thin client made it flaky on a
    // loaded runner, where even an un-delayed mock response can
    // occasionally take a few milliseconds longer than 30ms, timing out
    // the manifest fetch itself and failing the cycle for the WRONG
    // reason. Two clients: a normal one for the always-fast manifest, the
    // tight one for the content fetches actually under test.
    let tight_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(30))
        .build()
        .unwrap();
    let result =
        run_sync_cycle_with_clients(&b, &mock_server.uri(), &client(), &tight_client).await;
    assert!(
        result.is_err(),
        "3 consecutive TRANSPORT (timeout) failures must abort the cycle with an error"
    );

    let content_fetch_count = mock_server
        .received_requests()
        .await
        .expect("wiremock request log")
        .iter()
        .filter(|req| req.url.path().starts_with("/sync/presentations/"))
        .count();
    assert_eq!(
        content_fetch_count, 3,
        "the breaker must stop at exactly 3 attempts, never burning a timeout on entries 4 and 5"
    );
}

#[tokio::test]
async fn adjacent_per_song_application_failures_never_trip_the_breaker() {
    // #558 W2: a per-song APPLICATION-level failure (the peer is up and
    // answers, just with an error status for THIS one song) must stay
    // isolated forever (#558 round-4 U1(b)) and must never contribute to
    // the systemic-failure breaker — otherwise 3 adjacent, individually
    // harmless broken songs would abort the WHOLE cycle, starving every
    // healthy song after them. Three adjacent 500s, then two healthy songs:
    // all 5 must be attempted, and the two healthy ones must still apply.
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;
    let b = AppState::in_memory().await.unwrap();

    let now = chrono::Utc::now();
    let entries: Vec<serde_json::Value> = (0..5)
        .map(|i| {
            serde_json::json!({
                "syncId": format!("sid-{i}"),
                "libraryName": "Songs",
                "name": format!("Song {i}"),
                "updatedAt": now,
                "deletedAt": null,
            })
        })
        .collect();

    Mock::given(method("GET"))
        .and(path("/sync/manifest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(entries))
        .mount(&mock_server)
        .await;
    // Songs 0-2: application-level 500 (peer is up, just errors for them).
    for i in 0..3 {
        Mock::given(method("GET"))
            .and(path(format!("/sync/presentations/sid-{i}")))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;
    }
    // Songs 3-4: healthy content.
    for i in 3..5 {
        Mock::given(method("GET"))
            .and(path(format!("/sync/presentations/sid-{i}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "syncId": format!("sid-{i}"),
                "libraryName": "Songs",
                "name": format!("Song {i}"),
                "updatedAt": now,
                "deletedAt": null,
                "slides": [],
            })))
            .mount(&mock_server)
            .await;
    }

    let (pulled, applied, errors) = run_sync_cycle(&b, &mock_server.uri(), &client())
        .await
        .unwrap();
    assert_eq!(pulled, 5, "every manifest entry must be attempted");
    assert_eq!(errors, 3, "the three broken songs are counted as errors");
    assert_eq!(
        applied, 2,
        "the two healthy songs after the broken run must still apply — the breaker \
         must never trip on application-level (status-code) failures"
    );
}

#[tokio::test]
async fn an_application_level_failure_resets_the_transport_failure_streak() {
    // #558 X2: an application-level failure (the peer answers, just badly,
    // for ONE song) PROVES the peer is reachable — exactly like a genuine
    // success does — so it must reset the transport-failure streak, not
    // merely fail to trip it. Two transport (timeout) failures, then ONE
    // application (500) failure, then two more transport failures: the
    // transport failures are never 3 CONSECUTIVE (the 500 proves
    // reachability in between), so the breaker must NEVER trip and all 5
    // entries must be attempted. Pre-X2 code left the streak untouched on
    // the application failure, so it accumulated 2 (entries 0-1) + 1
    // (entry 3) = 3 at entry 3 and aborted before entry 4 was ever fetched.
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;
    let b = AppState::in_memory().await.unwrap();

    let now = chrono::Utc::now();
    let entries: Vec<serde_json::Value> = (0..5)
        .map(|i| {
            serde_json::json!({
                "syncId": format!("sid-{i}"),
                "libraryName": "Songs",
                "name": format!("Song {i}"),
                "updatedAt": now,
                "deletedAt": null,
            })
        })
        .collect();

    Mock::given(method("GET"))
        .and(path("/sync/manifest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(entries))
        .mount(&mock_server)
        .await;
    // Songs 0, 1, 3, 4: delayed content — a genuine transport (timeout)
    // failure under the tight content-fetch client below.
    for i in [0, 1, 3, 4] {
        Mock::given(method("GET"))
            .and(path(format!("/sync/presentations/sid-{i}")))
            .respond_with(
                ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(300)),
            )
            .mount(&mock_server)
            .await;
    }
    // Song 2: immediate application-level 500 — the peer IS reachable,
    // just errors for this one song.
    Mock::given(method("GET"))
        .and(path("/sync/presentations/sid-2"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    // #558 X7 two-client split: a normal client for the always-fast
    // manifest, the razor-thin client for the content fetches under test.
    let tight_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(30))
        .build()
        .unwrap();
    let (pulled, applied, errors) =
        run_sync_cycle_with_clients(&b, &mock_server.uri(), &client(), &tight_client)
            .await
            .expect(
                "the breaker must never trip — the application failure at sid-2 resets \
                 the streak before 3 transport failures ever accumulate consecutively",
            );
    assert_eq!(pulled, 5, "every manifest entry must be attempted");
    assert_eq!(
        errors, 5,
        "all 5 entries fail (4 transport + 1 application)"
    );
    assert_eq!(applied, 0, "nothing succeeds in this cycle");

    let content_fetch_count = mock_server
        .received_requests()
        .await
        .expect("wiremock request log")
        .iter()
        .filter(|req| req.url.path().starts_with("/sync/presentations/"))
        .count();
    assert_eq!(
        content_fetch_count, 5,
        "every entry must be attempted — the breaker must never trip when a reachability-\
         proving application failure interrupts otherwise-consecutive transport failures"
    );
}
