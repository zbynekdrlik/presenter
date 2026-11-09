//! Test-only Access tokens (#915): a verifier that trusts a key pair generated
//! for this test process, and tokens signed with it (or with a second,
//! untrusted "forger" key under the same key id). No private key is committed.

use std::sync::{Arc, LazyLock};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::rand_core::OsRng;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde_json::{json, Value};

use super::{AccessIdentity, DeviceKey, JwtConfig, JwtVerifier};

pub(crate) const TEAM: &str = "team.example.cloudflareaccess.com";
pub(crate) const AUD: &str = "testaud0001";
const KID: &str = "test-signing";

struct TestKey {
    encoding: EncodingKey,
    /// The public key as Cloudflare publishes it (`n`, `e`).
    jwk: Value,
}

impl TestKey {
    fn generate() -> Self {
        let key = RsaPrivateKey::new(&mut OsRng, 2048).expect("rsa key");
        let der = key.to_pkcs1_der().expect("der");
        Self {
            encoding: EncodingKey::from_rsa_der(der.as_bytes()),
            jwk: json!({
                "kid": KID,
                "kty": "RSA",
                "alg": "RS256",
                "use": "sig",
                "n": URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),
                "e": URL_SAFE_NO_PAD.encode(key.e().to_bytes_be()),
            }),
        }
    }
}

static SIGNING: LazyLock<TestKey> = LazyLock::new(TestKey::generate);
static FORGER: LazyLock<TestKey> = LazyLock::new(TestKey::generate);

/// The team's key document, as `/cdn-cgi/access/certs` serves it.
pub(crate) fn certs_json() -> Value {
    json!({ "keys": [SIGNING.jwk.clone()], "public_cert": { "kid": KID, "cert": "-" } })
}

pub(crate) fn certs() -> JwkSet {
    serde_json::from_value(certs_json()).expect("certs")
}

pub(crate) fn config() -> JwtConfig {
    JwtConfig::parse(Some(TEAM), Some(AUD), None).expect("config")
}

pub(crate) fn verifier() -> Arc<JwtVerifier> {
    Arc::new(JwtVerifier::with_keys(config(), &certs()))
}

pub(crate) fn device_key() -> DeviceKey {
    DeviceKey::from_bytes([42; 32]).expect("device key")
}

/// The identity service of a site whose Access app is the test one.
pub(crate) fn identity() -> AccessIdentity {
    AccessIdentity::new(
        Some(JwtVerifier::with_keys(config(), &certs())),
        Some(device_key()),
    )
}

pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Claims Cloudflare would put in a token for `email`, valid for an hour.
pub(crate) fn claims(email: &str) -> Value {
    let now = now();
    json!({
        "aud": [AUD],
        "email": email,
        "exp": now + 3600,
        "iat": now,
        "nbf": now,
        "iss": format!("https://{TEAM}"),
        "type": "app",
        "sub": "00000000-0000-0000-0000-000000000000",
    })
}

pub(crate) fn sign(claims: &Value) -> String {
    sign_with(claims, &SIGNING)
}

/// Signed with a key the site does not trust, under the trusted key id.
pub(crate) fn forge(claims: &Value) -> String {
    sign_with(claims, &FORGER)
}

pub(crate) fn token(email: &str) -> String {
    sign(&claims(email))
}

fn sign_with(claims: &Value, key: &TestKey) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.to_string());
    encode(&header, claims, &key.encoding).expect("token")
}
