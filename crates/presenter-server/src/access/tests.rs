//! #915: who a request is — the verified Access token, the remembered device,
//! or nobody.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{header::COOKIE, HeaderMap, HeaderValue};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::jwt::{KeyTiming, TokenRefusal};
use super::test_support::{self, claims, forge, sign, token, AUD, TEAM};
use super::*;

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(
            axum::http::HeaderName::from_bytes(name.as_bytes()).expect("name"),
            HeaderValue::from_str(value).expect("value"),
        );
    }
    headers
}

fn facts(headers: &HeaderMap, from_tunnel: bool) -> RequestFacts<'_> {
    RequestFacts {
        headers,
        from_tunnel,
        https: true,
        now: test_support::now(),
    }
}

// ---- the token ---------------------------------------------------------------

#[tokio::test]
async fn a_token_cloudflare_signed_for_this_app_gives_its_email() {
    let email = test_support::verifier()
        .verify(&token("  Preacher@Example.org "))
        .await;
    assert_eq!(email, Ok("preacher@example.org".to_string()));
}

#[tokio::test]
async fn an_expired_token_is_refused() {
    let mut expired = claims("preacher@example.org");
    expired["exp"] = json!(test_support::now() - 3600);
    let refused = test_support::verifier().verify(&sign(&expired)).await;
    assert_eq!(
        refused,
        Err(TokenRefusal::Invalid("ExpiredSignature".into()))
    );
}

#[tokio::test]
async fn a_token_of_another_access_app_or_team_is_refused() {
    let mut other_app = claims("preacher@example.org");
    other_app["aud"] = json!(["someotherapp"]);
    let refused = test_support::verifier().verify(&sign(&other_app)).await;
    assert_eq!(
        refused,
        Err(TokenRefusal::Invalid("InvalidAudience".into()))
    );

    let mut other_team = claims("preacher@example.org");
    other_team["iss"] = json!("https://evil.cloudflareaccess.com");
    let refused = test_support::verifier().verify(&sign(&other_team)).await;
    assert_eq!(refused, Err(TokenRefusal::Invalid("InvalidIssuer".into())));
}

#[tokio::test]
async fn a_forged_or_malformed_token_or_one_without_email_is_refused() {
    let verifier = test_support::verifier();
    let forged = verifier
        .verify(&forge(&claims("preacher@example.org")))
        .await;
    assert_eq!(
        forged,
        Err(TokenRefusal::Invalid("InvalidSignature".into()))
    );
    assert_eq!(
        verifier.verify("not.a.token").await,
        Err(TokenRefusal::Malformed)
    );
    let mut service = claims("x@example.org");
    service["email"] = json!(null);
    assert_eq!(
        verifier.verify(&sign(&service)).await,
        Err(TokenRefusal::NoEmail)
    );
    let mut not_before = claims("preacher@example.org");
    not_before["nbf"] = json!(test_support::now() + 3600);
    assert_eq!(
        verifier.verify(&sign(&not_before)).await,
        Err(TokenRefusal::Invalid("ImmatureSignature".into()))
    );
}

#[test]
fn the_config_needs_a_team_host_and_an_alphanumeric_aud() {
    let config =
        JwtConfig::parse(Some(" Team.Example.com "), Some("abc, def ,"), None).expect("ok");
    assert_eq!(config.team_domain(), "team.example.com");
    assert_eq!(config.audiences(), ["abc".to_string(), "def".to_string()]);
    assert_eq!(
        JwtConfig::parse(None, Some("abc"), None),
        Err("no team domain")
    );
    assert_eq!(
        JwtConfig::parse(Some("t.example.com"), Some(" "), None),
        Err("no application AUD")
    );
    assert_eq!(
        JwtConfig::parse(Some("t.example.com/x"), Some("abc"), None),
        Err("the team domain is no host name")
    );
    assert_eq!(
        JwtConfig::parse(Some("t.example.com"), Some("ab\"c"), None),
        Err("an application AUD is not alphanumeric")
    );
}

// ---- the keys from Cloudflare ------------------------------------------------

fn remote(url: &str, timing: KeyTiming) -> Arc<JwtVerifier> {
    let config = JwtConfig::parse(Some(TEAM), Some(AUD), Some(url)).expect("config");
    Arc::new(JwtVerifier::new(config, timing).expect("client"))
}

fn no_wait() -> KeyTiming {
    KeyTiming {
        ttl: Duration::ZERO,
        min_refetch: Duration::ZERO,
    }
}

/// The published keys with every key id replaced (a rotation's old set).
fn certs_with_kid(kid: &str) -> serde_json::Value {
    let mut certs = test_support::certs_json();
    certs["keys"][0]["kid"] = json!(kid);
    certs
}

#[tokio::test]
async fn hs256_none_or_kid_less_tokens_are_refused_before_any_key_is_used() {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    let verifier = test_support::verifier();
    let mut hs256 = Header::new(Algorithm::HS256);
    hs256.kid = Some("test-signing".into());
    let symmetric = encode(
        &hs256,
        &claims("boss@example.org"),
        &EncodingKey::from_secret(b"public key bytes"),
    )
    .expect("hs256");
    assert_eq!(
        verifier.verify(&symmetric).await,
        Err(TokenRefusal::NotRs256)
    );

    let none_header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
    let body = URL_SAFE_NO_PAD.encode(claims("boss@example.org").to_string());
    assert_eq!(
        verifier.verify(&format!("{none_header}.{body}.")).await,
        Err(TokenRefusal::Malformed)
    );

    let kid_less = sign_without_kid(&claims("boss@example.org"));
    assert_eq!(
        verifier.verify(&kid_less).await,
        Err(TokenRefusal::NotRs256)
    );
}

fn sign_without_kid(claims: &serde_json::Value) -> String {
    // A token signed by the trusted key, with the kid removed from its header.
    let token = sign(claims);
    let (_, rest) = token.split_once('.').expect("header");
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#);
    format!("{header}.{rest}")
}

#[tokio::test]
async fn a_rotated_key_is_fetched_when_a_token_names_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(certs_with_kid("old-key")))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(test_support::certs_json()))
        .expect(1)
        .mount(&server)
        .await;
    let verifier = remote(&format!("{}/certs", server.uri()), no_wait());
    assert_eq!(
        verifier.verify(&token("preacher@example.org")).await,
        Err(TokenRefusal::UnknownKey),
        "the first fetch only has the old key"
    );
    assert_eq!(
        verifier.verify(&token("preacher@example.org")).await,
        Ok("preacher@example.org".into()),
        "an unknown kid fetches again and finds the new key"
    );
}

#[tokio::test]
async fn an_unknown_kid_fetches_at_most_once_a_minute() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(certs_with_kid("old-key")))
        .expect(1)
        .mount(&server)
        .await;
    let verifier = remote(&format!("{}/certs", server.uri()), KeyTiming::default());
    for _ in 0..3 {
        assert_eq!(
            verifier.verify(&token("preacher@example.org")).await,
            Err(TokenRefusal::UnknownKey)
        );
    }
}

#[tokio::test]
async fn concurrent_first_logins_wait_for_one_fetch_and_all_verify() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(test_support::certs_json())
                .set_delay(Duration::from_millis(300)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let verifier = remote(&format!("{}/certs", server.uri()), KeyTiming::default());
    let token = token("preacher@example.org");
    let results = futures_util::future::join_all((0..8).map(|_| verifier.verify(&token))).await;
    assert!(
        results
            .iter()
            .all(|result| result.as_deref() == Ok("preacher@example.org")),
        "{results:?}"
    );
}

#[tokio::test]
async fn stale_keys_answer_at_once_and_refresh_in_the_background() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(test_support::certs_json()))
        .expect(3)
        .mount(&server)
        .await;
    let verifier = remote(
        &format!("{}/certs", server.uri()),
        KeyTiming {
            ttl: Duration::ZERO,
            min_refetch: Duration::from_secs(60),
        },
    );
    let token = token("preacher@example.org");
    assert!(verifier.verify(&token).await.is_ok());
    // Stale (ttl 0) but cached: answered from the cache. The background refresh
    // waits for the minute since the last fetch, so nothing is fetched yet.
    assert!(verifier.verify(&token).await.is_ok());
    assert_eq!(server.received_requests().await.expect("requests").len(), 1);

    let verifier = remote(&format!("{}/certs", server.uri()), no_wait());
    assert!(verifier.verify(&token).await.is_ok());
    // Two fetches so far (one per verifier); the background refresh is the third.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    assert!(
        verifier.verify(&token).await.is_ok(),
        "a stale key still answers"
    );
    while server.received_requests().await.expect("requests").len() < 3 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no background refresh"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_keys_are_fetched_once_and_a_failed_fetch_is_not_repeated_at_once() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cdn-cgi/access/certs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(test_support::certs_json()))
        .expect(1)
        .mount(&server)
        .await;
    let url = format!("{}/cdn-cgi/access/certs", server.uri());
    let verifier = remote(&url, KeyTiming::default());
    for _ in 0..3 {
        assert_eq!(
            verifier.verify(&token("preacher@example.org")).await,
            Ok("preacher@example.org".into())
        );
    }

    let down = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&down)
        .await;
    let url = format!("{}/certs", down.uri());
    let verifier = remote(&url, KeyTiming::default());
    for _ in 0..3 {
        assert_eq!(
            verifier.verify(&token("preacher@example.org")).await,
            Err(TokenRefusal::KeysUnavailable)
        );
    }
}

// ---- the resolution ---------------------------------------------------------

#[tokio::test]
async fn a_verified_login_identifies_and_remembers_the_device() {
    let identity = test_support::identity();
    let token = token("preacher@example.org");
    let request = headers(&[("cf-access-jwt-assertion", &token)]);
    let resolved = identity.resolve(facts(&request, true)).await;
    assert_eq!(
        resolved.identity,
        Some(Identity {
            email: "preacher@example.org".into(),
            source: IdentitySource::Jwt
        })
    );
    let cookie = resolved.set_cookie.expect("a device cookie");
    assert!(cookie.contains("; Secure"), "{cookie}");

    // The same browser through the church bypass: no token, only the cookie.
    let pair = cookie.split(';').next().expect("pair");
    let at_church = headers(&[("cookie", pair)]);
    let resolved = identity.resolve(facts(&at_church, true)).await;
    assert_eq!(
        resolved.identity,
        Some(Identity {
            email: "preacher@example.org".into(),
            source: IdentitySource::DeviceCookie
        })
    );
    assert_eq!(resolved.set_cookie, None);

    // Logged in again with a current cookie: no new cookie.
    let both = headers(&[("cf-access-jwt-assertion", &token), ("cookie", pair)]);
    assert_eq!(identity.resolve(facts(&both, true)).await.set_cookie, None);
    // Another person logs in on that device: the cookie follows the login.
    let other = token_for_other();
    let switched = headers(&[("cf-access-jwt-assertion", &other), ("cookie", pair)]);
    let resolved = identity.resolve(facts(&switched, true)).await;
    assert_eq!(resolved.identity.expect("id").email, "other@example.org");
    assert!(resolved
        .set_cookie
        .expect("new cookie")
        .contains("presenter_device="));
}

fn token_for_other() -> String {
    token("other@example.org")
}

#[tokio::test]
async fn the_bare_email_header_a_forged_token_or_a_lan_token_identify_nobody() {
    let identity = test_support::identity();
    let header_only = headers(&[("cf-access-authenticated-user-email", "boss@example.org")]);
    let resolved = identity.resolve(facts(&header_only, true)).await;
    assert_eq!((resolved.identity, resolved.set_cookie), (None, None));

    let forged = forge(&claims("boss@example.org"));
    let request = headers(&[("cf-access-jwt-assertion", &forged)]);
    let resolved = identity.resolve(facts(&request, true)).await;
    assert_eq!((resolved.identity, resolved.set_cookie), (None, None));

    let good = token("boss@example.org");
    let lan = headers(&[("cf-access-jwt-assertion", &good)]);
    let resolved = identity.resolve(facts(&lan, false)).await;
    assert_eq!((resolved.identity, resolved.set_cookie), (None, None));
}

#[tokio::test]
async fn without_access_config_a_token_identifies_nobody_but_a_cookie_still_does() {
    let device = test_support::device_key();
    let cookie = device.issue("preacher@example.org", test_support::now(), false);
    let identity = AccessIdentity::new(None, Some(device));
    let token = token("preacher@example.org");
    let request = headers(&[("cf-access-jwt-assertion", &token)]);
    let resolved = identity.resolve(facts(&request, true)).await;
    assert_eq!((resolved.identity, resolved.set_cookie), (None, None));

    let mut with_cookie = HeaderMap::new();
    let pair = cookie.split(';').next().expect("pair");
    with_cookie.insert(COOKIE, HeaderValue::from_str(pair).expect("cookie"));
    let resolved = identity.resolve(facts(&with_cookie, false)).await;
    assert_eq!(
        resolved.identity.map(|id| (id.email, id.source)),
        Some(("preacher@example.org".into(), IdentitySource::DeviceCookie))
    );
}

#[tokio::test]
async fn a_login_re_issues_a_cookie_older_than_thirty_days_and_a_refused_cookie_is_removed() {
    let identity = test_support::identity();
    let now = test_support::now();
    let old = test_support::device_key().issue("preacher@example.org", now - 31 * 24 * 3600, true);
    let old_pair = old.split(';').next().expect("pair").to_string();
    let token = token("preacher@example.org");
    let request = headers(&[("cf-access-jwt-assertion", &token), ("cookie", &old_pair)]);
    let renewed = identity
        .resolve(facts(&request, true))
        .await
        .set_cookie
        .expect("renewed");
    assert!(
        renewed.starts_with("presenter_device=") && renewed.contains("Max-Age=31536000"),
        "{renewed}"
    );

    // A cookie of a rotated key, at church (no token): nobody, and it is removed.
    let rotated =
        DeviceKey::from_bytes([9; 32])
            .expect("key")
            .issue("preacher@example.org", now, true);
    let rotated_pair = rotated.split(';').next().expect("pair").to_string();
    let at_church = headers(&[("cookie", &rotated_pair)]);
    let resolved = identity.resolve(facts(&at_church, true)).await;
    assert_eq!(resolved.identity, None);
    let removal = resolved.set_cookie.expect("removal");
    assert!(
        removal.starts_with("presenter_device=; Max-Age=0"),
        "{removal}"
    );
}

#[test]
fn the_device_key_lives_next_to_the_database_file() {
    assert_eq!(
        device_key_location(None, "sqlite:///opt/presenter/presenter.db?mode=rwc"),
        Some(PathBuf::from("/opt/presenter/access-device.key"))
    );
    assert_eq!(
        device_key_location(None, "sqlite://presenter.db"),
        Some(PathBuf::from("./access-device.key"))
    );
    assert_eq!(device_key_location(None, "sqlite::memory:"), None);
    assert_eq!(device_key_location(None, ""), None);
    assert_eq!(
        device_key_location(Some(OsString::from("/etc/k")), "sqlite::memory:"),
        Some(PathBuf::from("/etc/k"))
    );
    assert_eq!(
        device_key_location(Some(OsString::new()), "sqlite:///a/b.db"),
        Some(PathBuf::from("/a/access-device.key"))
    );
}

#[test]
fn emails_are_trimmed_lower_cased_and_bounded() {
    assert_eq!(normalize_email(" A@B.org "), Some("a@b.org".into()));
    assert_eq!(normalize_email("nobody"), None);
    assert_eq!(
        normalize_email(&format!("{}@example.org", "a".repeat(250))),
        None
    );
}
