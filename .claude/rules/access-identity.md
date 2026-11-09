---
paths:
  - "crates/presenter-server/src/access/**"
  - "crates/presenter-server/src/router/access_identity.rs"
  - "crates/presenter-server/src/router/access_identity/**"
  - "crates/presenter-server/src/router/bible/bookmarks.rs"
  - "crates/presenter-server/src/router/bible/sermons.rs"
  - "tests/e2e/study-bookmarks-identity.spec.ts"
---

# Reader identity: Cloudflare Access token + remembered device (#915)

- **One middleware, one answer per request.** `router/access_identity.rs` resolves the reader through
  `crate::access::AccessIdentity` (an `Arc` on `AppState`) and stores `RequestIdentity` in the request
  extensions; handlers read it with the `AccessEmail` extractor (no extension = nobody). It sits INSIDE
  the request-log layer, so its lines carry the request span. Never read an e-mail header in a handler.
- **Only a verified token names a person** (`access/jwt.rs`): RS256, `kid` from the team's
  `/cdn-cgi/access/certs`, `aud` = the site's app AUD(s), `iss` = `https://<team>`, `exp`/`nbf`. The bare
  `Cf-Access-Authenticated-User-Email` header is ignored (a LAN client could send it). A token counts only
  from a loopback peer or no peer (`integrations::is_tunnel_peer`, the #793 rule, shared with
  `extract_client_ip`). `alg` must be RS256 with a `kid` (HS256 / none / kid-less refused, tested).
- **Keys** (`KeyTiming`, ONE fetch at a time under `fetching`): a cached `kid` answers at once, a stale
  cache (> 1 h) refreshes in a background task; an UNCACHED `kid` (first start, rotation) waits for the
  fetch in flight or starts one, so a first login is never "nobody" while keys load (a bookmark would land
  in the shared list). `last_attempt` is stamped when a fetch ENDS; no two fetches within a minute.
  Refused tokens log WARN on the #484 power-of-two streak (`listen::should_log_event`), DEBUG otherwise.
- **The device cookie** (`access/device_cookie.rs`): `presenter_device=b64url(v1|issued|email).b64url(HMAC)`,
  HttpOnly, SameSite=Lax, 1 year, `Secure` when `X-Forwarded-Proto: https`; issued after a verified login
  when the browser has no cookie for that e-mail or one ≥ 30 days old. **Never on a cacheable response**
  (`may_carry_cookie`: only no `Cache-Control`, `private` or `no-store`; never a 101) — a `public,
  immutable` asset in a shared cache would hand the cookie to another browser; a response without its own
  `Cache-Control` gets `private, no-store`. A refused cookie (rotated key, > 1 year, altered) is REMOVED
  (`Max-Age=0`) so it stops arriving — but only by a key read from the key file (`is_persisted`): a
  stand-in key (unreadable file) never removes nor overwrites a cookie it refuses (not even after
  a login), so fixing the file brings those devices back. Key file `access-device.key` next to the sqlite file (0600); delete +
  restart = every device forgotten. The service is built in `AppState::from_config` from the config's DB
  url (`AccessIdentity::unconfigured()` before that and in test states); the validate probe writes no key.
- **Per-site config = the deploy's `access.conf` drop-in** (`PRESENTER_ACCESS_TEAM_DOMAIN`,
  `PRESENTER_ACCESS_AUD`): deploy-site.yml input `access_aud` (SNV, PRBA), release.yml for PP. AUD tags are
  read with the Cloudflare API (`GET /accounts/<id>/access/apps`, field `aud`) through
  `secret exec --file <vault>/cloudflare-presenter-access --env CF_TOKEN`; each host also has a separate
  `/healthz` app with its OWN AUD — use the app whose `domain` is the bare host.
- **Tests:** no private key is committed (the secret-scan hook refuses one). `access::test_support`
  generates two 2048-bit keys per test process (`rsa` dev-dependency; root `Cargo.toml` builds `rsa` +
  `num-bigint-dig` at opt-level 3 in dev, or keygen takes seconds): `SIGNING` (published in `certs_json()`)
  and `FORGER` (untrusted, same kid). `AppState::set_access_identity(test_support::identity())` before
  `build_router`. The E2E (`study-bookmarks-identity.spec.ts`) generates its pair with
  `node:crypto` `generateKeyPairSync`, serves the public JWK from a node mock (`PRESENTER_ACCESS_CERTS_URL`),
  and carries the cookie to a second browser context.
- Logs (`presenter::access`): DEBUG `source=jwt|device-cookie|none` per request, INFO when a device is
  remembered / keys fetched, WARN for a refused token or a failed key fetch. Never the token or cookie value.
