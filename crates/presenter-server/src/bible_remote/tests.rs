//! #826: the NLT parser on real API samples (a few verses, `fixtures/`), the
//! client against a mock API (an external network service — the one thing
//! tests may mock), the chunk cache and the book table.

use super::books::nlt_book_abbreviation;
use super::cache::{ChunkCache, ChunkKey};
use super::client::{api_reference, chunk_ranges, MAX_VERSES_PER_FETCH, MAX_VERSES_PER_REQUEST};
use super::parse::{decode_entities, parse_verses, verse_text};
use super::*;
use presenter_core::bible::{canonical_book_by_code, canonical_book_by_number};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const FIXTURE_1JN: &str = include_str!("fixtures/nlt_1jn_1_1-3.html");
const FIXTURE_PS3: &str = include_str!("fixtures/nlt_ps_3_1-3.html");
const FIXTURE_JOHN3: &str = include_str!("fixtures/nlt_john_3_20-23.html");

// --- parser -------------------------------------------------------------

#[test]
fn parses_each_verse_export_with_its_chapter_and_number() {
    let verses = parse_verses(FIXTURE_1JN);
    let numbers: Vec<(u16, u16)> = verses.iter().map(|v| (v.chapter, v.verse)).collect();
    assert_eq!(numbers, vec![(1, 1), (1, 2), (1, 3)]);
}

#[test]
fn chapter_heading_section_heading_footnote_and_verse_number_are_stripped() {
    let verses = parse_verses(FIXTURE_1JN);
    assert_eq!(
        verses[0].text,
        "We proclaim to you the one who existed from the beginning, whom we have heard \
         and seen. We saw him with our own eyes and touched him with our own hands. He is \
         the Word of life."
    );
    assert!(verses[1].text.starts_with("This one who is life itself"));
    for verse in &verses {
        for leaked in ["Introduction", "1 John", "Greek", "*", "<", ">"] {
            assert!(
                !verse.text.contains(leaked),
                "verse {} leaked {leaked:?}: {}",
                verse.verse,
                verse.text
            );
        }
    }
}

#[test]
fn psalm_title_is_dropped_poetry_lines_join_and_the_divine_name_is_capitalised() {
    let verses = parse_verses(FIXTURE_PS3);
    assert_eq!(verses.len(), 3);
    assert_eq!(
        verses[0].text,
        "O LORD, I have so many enemies; so many are against me."
    );
    assert_eq!(
        verses[1].text,
        "So many are saying, “God will never rescue him!” Interlude"
    );
    assert_eq!(
        verses[2].text,
        "But you, O LORD, are a shield around me; you are my glory, the one who holds my \
         head high."
    );
}

#[test]
fn a_section_heading_inside_the_range_is_not_verse_text() {
    let verses = parse_verses(FIXTURE_JOHN3);
    let numbers: Vec<u16> = verses.iter().map(|v| v.verse).collect();
    assert_eq!(numbers, vec![20, 21, 22, 23]);
    assert!(verses.iter().all(|v| v.chapter == 3));
    assert_eq!(
        verses[1].text,
        "But those who do what is right come to the light so others can see that they are \
         doing what God wants.”"
    );
    assert_eq!(
        verses[2].text,
        "Then Jesus and his disciples left Jerusalem and went into the Judean countryside. \
         Jesus spent some time with them there, baptizing people."
    );
}

#[test]
fn a_page_without_verses_parses_to_nothing() {
    assert!(parse_verses("").is_empty());
    assert!(parse_verses("<html><body><div id=\"bibletext\"></div></body></html>").is_empty());
}

#[test]
fn html_entities_are_decoded() {
    assert_eq!(
        decode_entities(
            "God&#8217;s &amp; &quot;x&quot; &#x201C;y&#x201D; &ldquo;z&rdquo; 5 &lt; 6 \
             &unknown; & done"
        ),
        "God’s & \"x\" “y” “z” 5 < 6 &unknown; & done"
    );
    assert_eq!(decode_entities("a&nbsp;b"), "a\u{a0}b");
}

#[test]
fn verse_text_decodes_entities_and_collapses_whitespace() {
    // Synthetic markup (not NLT text) in the API's shape.
    let inner = "\n<p class=\"body\"><span class=\"vn\">7</span>Synthetic&nbsp;line &#8212; \
                 with   gaps</p>\n<p class=\"poet2\">and the second&#8217;s line.</p>\n";
    assert_eq!(
        verse_text(inner),
        "Synthetic line — with gaps and the second’s line."
    );
}

// --- book table, ranges, cache -------------------------------------------

#[test]
fn every_canonical_book_has_a_distinct_api_abbreviation() {
    let mut seen = HashSet::new();
    for number in 1..=66u16 {
        let canon = canonical_book_by_number(number).expect("canonical book");
        let abbreviation = nlt_book_abbreviation(canon.code)
            .unwrap_or_else(|| panic!("no NLT abbreviation for {}", canon.code));
        assert!(seen.insert(abbreviation), "duplicate {abbreviation}");
    }
    // The live API rejects the OSIS `1Thess` / `1John` spellings.
    assert_eq!(nlt_book_abbreviation("1th"), Some("1Thes"));
    assert_eq!(nlt_book_abbreviation("1JN"), Some("1Jn"));
    assert_eq!(nlt_book_abbreviation("PSA"), Some("Ps"));
    assert_eq!(nlt_book_abbreviation("TOB"), None);
}

#[test]
fn ranges_split_into_chunks_of_at_most_fifty_verses() {
    assert_eq!(
        chunk_ranges(1, 60, MAX_VERSES_PER_REQUEST),
        vec![(1, 50), (51, 60)]
    );
    assert_eq!(chunk_ranges(1, 50, MAX_VERSES_PER_REQUEST), vec![(1, 50)]);
    assert_eq!(chunk_ranges(3, 3, MAX_VERSES_PER_REQUEST), vec![(3, 3)]);
    assert_eq!(
        chunk_ranges(1, 176, MAX_VERSES_PER_REQUEST),
        vec![(1, 50), (51, 100), (101, 150), (151, 176)]
    );
    assert!(chunk_ranges(5, 4, MAX_VERSES_PER_REQUEST).is_empty());
    assert_eq!(
        chunk_ranges(u16::MAX - 1, u16::MAX, MAX_VERSES_PER_REQUEST),
        vec![(u16::MAX - 1, u16::MAX)]
    );
}

#[test]
fn api_reference_uses_the_single_verse_form_for_one_verse() {
    assert_eq!(api_reference("1Jn", 1, 1, 3), "1Jn.1.1-3");
    assert_eq!(api_reference("Ps", 23, 4, 4), "Ps.23.4");
}

fn cache_key(verse: u16) -> ChunkKey {
    ChunkKey {
        book_code: "1JN".to_string(),
        chapter: 1,
        verse_start: verse,
        verse_end: verse,
    }
}

fn cached_verses(verse: u16) -> Arc<Vec<NltVerse>> {
    Arc::new(vec![NltVerse {
        chapter: 1,
        verse,
        text: format!("synthetic {verse}"),
    }])
}

#[test]
fn the_cache_evicts_the_least_recently_used_chunk() {
    let mut cache = ChunkCache::new(2);
    cache.insert(cache_key(1), cached_verses(1));
    cache.insert(cache_key(2), cached_verses(2));
    // Reading 1 makes 2 the least recently used one.
    assert_eq!(cache.get(&cache_key(1)), Some(cached_verses(1)));
    cache.insert(cache_key(3), cached_verses(3));
    assert!(cache.contains(&cache_key(1)));
    assert!(!cache.contains(&cache_key(2)));
    assert!(cache.contains(&cache_key(3)));
}

#[test]
fn nlt_passages_carry_the_given_book_name_and_the_canonical_code() {
    let canon = canonical_book_by_code("1JN").expect("1JN");
    let verse = NltVerse {
        chapter: 1,
        verse: 2,
        text: "synthetic".to_string(),
    };
    let passages = nlt_passages(&[verse], "1 John", canon);
    assert_eq!(passages.len(), 1);
    let reference = &passages[0].reference;
    assert_eq!(reference.book, "1 John");
    assert_eq!(reference.book_code.as_deref(), Some("1JN"));
    assert_eq!(reference.book_number, Some(62));
    assert_eq!(
        (
            reference.chapter,
            reference.verse_start,
            reference.verse_end
        ),
        (1, 2, 2)
    );
    assert_eq!(passages[0].translation.code, NLT_CODE);
    assert_eq!(passages[0].text, "synthetic");
}

#[test]
fn only_eng_nlt_is_remote_and_it_borrows_the_kjv_structure() {
    assert!(is_remote_translation("eng-nlt"));
    assert!(is_remote_translation(" ENG-NLT "));
    assert!(!is_remote_translation("eng-kjv"));
    assert_eq!(structure_source("eng-nlt"), "eng-kjv");
    assert_eq!(structure_source("slk-seb"), "slk-seb");
}

// --- client against a mock API ------------------------------------------

/// A synthetic API page (NOT NLT text) holding `start..=end` of `chapter`.
fn synthetic_page(chapter: u16, start: u16, end: u16) -> String {
    let mut html = String::from("<div id=\"bibletext\"><section>");
    for verse in start..=end {
        html.push_str(&format!(
            "<verse_export orig=\"x\" bk=\"x\" ch=\"{chapter}\" vn=\"{verse}\">\
             <span class=\"vn\">{verse}</span>Synthetic verse {verse}. </verse_export>"
        ));
    }
    html.push_str("</section></div>");
    html
}

/// Answers every `ref=<Book>.<ch>.<start>-<end>` with a synthetic page of
/// exactly that range, like the live API for a chapter long enough.
struct EchoRange;

impl Respond for EchoRange {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let reference = request
            .url
            .query_pairs()
            .find(|(name, _)| name == "ref")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        let mut parts = reference.split('.').skip(1);
        let chapter: u16 = parts.next().and_then(|c| c.parse().ok()).unwrap_or(1);
        let range = parts.next().unwrap_or("1");
        let (start, end) = range.split_once('-').unwrap_or((range, range));
        let start: u16 = start.parse().unwrap_or(1);
        let end: u16 = end.parse().unwrap_or(start);
        ResponseTemplate::new(200).set_body_string(synthetic_page(chapter, start, end))
    }
}

async fn mount_page(server: &MockServer, reference: &str, body: String) {
    Mock::given(method("GET"))
        .and(path("/api/passages"))
        .and(query_param("ref", reference))
        .and(query_param("version", "NLT"))
        .and(query_param("key", "TEST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .expect(1)
        .mount(server)
        .await;
}

fn mock_client(server: &MockServer) -> NltClient {
    NltClient::for_test(&server.uri(), Duration::from_secs(5))
}

#[tokio::test]
async fn a_long_range_is_requested_in_fifty_verse_chunks() {
    let server = MockServer::start().await;
    mount_page(&server, "Ps.119.1-50", synthetic_page(119, 1, 50)).await;
    mount_page(&server, "Ps.119.51-60", synthetic_page(119, 51, 60)).await;

    let verses = mock_client(&server)
        .fetch_verses("PSA", 119, 1, 60)
        .await
        .expect("verses");

    let numbers: Vec<u16> = verses.iter().map(|v| v.verse).collect();
    assert_eq!(numbers, (1..=60).collect::<Vec<u16>>());
    assert_eq!(verses[59].text, "Synthetic verse 60.");
    server.verify().await;
}

#[tokio::test]
async fn a_cached_range_is_not_requested_again() {
    let server = MockServer::start().await;
    mount_page(&server, "1Jn.1.1-3", FIXTURE_1JN.to_string()).await;
    let client = mock_client(&server);

    let first = client.fetch_verses("1JN", 1, 1, 3).await.expect("first");
    let second = client.fetch_verses("1jn", 1, 1, 3).await.expect("second");

    assert_eq!(first.len(), 3);
    assert_eq!(first, second);
    // `expect(1)`: the second fetch was served from the cache.
    server.verify().await;
}

#[tokio::test]
async fn verses_past_the_end_of_the_chapter_are_empty_not_an_error() {
    // The live API clamps `1Jn.1.12-15` to verse 10, the chapter's last.
    let server = MockServer::start().await;
    mount_page(&server, "1Jn.1.12-15", synthetic_page(1, 10, 10)).await;

    let verses = mock_client(&server)
        .fetch_verses("1JN", 1, 12, 15)
        .await
        .expect("no error");

    assert!(verses.is_empty());
    server.verify().await;
}

#[tokio::test]
async fn a_range_past_the_chapter_end_stops_at_the_first_empty_chunk() {
    // 1-120 of a 10-verse chapter: chunk 51-100 comes back clamped to verse
    // 10, so chunk 101-120 is never requested.
    let server = MockServer::start().await;
    mount_page(&server, "1Jn.1.1-50", synthetic_page(1, 1, 10)).await;
    mount_page(&server, "1Jn.1.51-100", synthetic_page(1, 10, 10)).await;
    Mock::given(query_param("ref", "1Jn.1.101-120"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let verses = mock_client(&server)
        .fetch_verses("1JN", 1, 1, 120)
        .await
        .expect("verses");

    assert_eq!(verses.len(), 10);
    server.verify().await;
}

#[tokio::test]
async fn a_nonsense_range_is_capped_to_a_bounded_number_of_requests() {
    let server = MockServer::start().await;
    Mock::given(path("/api/passages"))
        .respond_with(EchoRange)
        .mount(&server)
        .await;

    let verses = mock_client(&server)
        .fetch_verses("PSA", 119, 1, 9999)
        .await
        .expect("verses");

    assert_eq!(verses.len(), usize::from(MAX_VERSES_PER_FETCH));
    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(
        requests.len(),
        usize::from(MAX_VERSES_PER_FETCH / MAX_VERSES_PER_REQUEST)
    );
}

#[tokio::test]
async fn an_http_error_status_is_a_typed_bad_gateway_error() {
    let server = MockServer::start().await;
    Mock::given(path("/api/passages"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let err = mock_client(&server)
        .fetch_verses("1JN", 1, 1, 3)
        .await
        .expect_err("a 500 must fail");

    assert!(
        matches!(err, RemoteBibleError::Status { status: 500, .. }),
        "{err:?}"
    );
    assert!(!err.is_unreachable());
    assert!(err.to_string().starts_with("NLT nedostupné"), "{err}");
}

#[tokio::test]
async fn an_empty_page_is_an_error_not_an_empty_passage() {
    // An unknown ref makes the live API answer 200 with an empty page.
    let server = MockServer::start().await;
    Mock::given(path("/api/passages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let err = mock_client(&server)
        .fetch_verses("1JN", 1, 1, 3)
        .await
        .expect_err("no verses must fail");

    assert!(matches!(err, RemoteBibleError::NoVerses { .. }), "{err:?}");
    assert!(!err.is_unreachable());
}

#[tokio::test]
async fn a_slow_api_is_a_typed_timeout() {
    let server = MockServer::start().await;
    Mock::given(path("/api/passages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE_1JN)
                .set_delay(Duration::from_secs(3)),
        )
        .mount(&server)
        .await;
    let client = NltClient::for_test(&server.uri(), Duration::from_millis(200));

    let err = client
        .fetch_verses("1JN", 1, 1, 3)
        .await
        .expect_err("timeout");

    assert!(matches!(err, RemoteBibleError::Timeout { .. }), "{err:?}");
    assert!(err.is_unreachable());
}

#[tokio::test]
async fn an_unreachable_api_is_a_typed_network_error_that_never_shows_the_key() {
    let client = NltClient::for_test("http://127.0.0.1:1", Duration::from_secs(5));

    let err = client
        .fetch_verses("1JN", 1, 1, 3)
        .await
        .expect_err("connection refused");

    let RemoteBibleError::Network { detail, .. } = &err else {
        panic!("expected a network error, got {err:?}");
    };
    // The cause chain names the failed connect; the URL (with the key) is gone.
    assert!(detail.to_lowercase().contains("connect"), "{detail}");
    assert!(err.is_unreachable());
    assert!(!err.to_string().contains("key="), "{err}");
    assert!(!err.to_string().contains("127.0.0.1"), "{err}");
}

#[tokio::test]
async fn a_failed_request_is_not_cached() {
    let server = MockServer::start().await;
    Mock::given(path("/api/passages"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    mount_page(&server, "1Jn.1.1-3", FIXTURE_1JN.to_string()).await;
    let client = mock_client(&server);

    assert!(client.fetch_verses("1JN", 1, 1, 3).await.is_err());
    let verses = client.fetch_verses("1JN", 1, 1, 3).await.expect("retry");

    assert_eq!(verses.len(), 3);
    server.verify().await;
}

#[tokio::test]
async fn a_book_outside_the_canon_needs_no_request() {
    let server = MockServer::start().await;
    Mock::given(path("/api/passages"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let verses = mock_client(&server)
        .fetch_verses("TOB", 1, 1, 3)
        .await
        .expect("no error");

    assert!(verses.is_empty());
    server.verify().await;
}
