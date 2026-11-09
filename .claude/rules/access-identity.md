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
  from a loopback peer or no peer (the #793 rule). Keys: cached by `kid`, 1 h TTL, refetch ≤ 1/min, ONE
  request fetches (`try_lock`), the others use the cache — a slow Cloudflare never stalls requests.
- **The device cookie** (`access/device_cookie.rs`): `presenter_device=b64url(v1|issued|email).b64url(HMAC)`,
  HttpOnly, SameSite=Lax, 1 year, `Secure` when `X-Forwarded-Proto: https`; issued after a verified login
  when the browser has no cookie for that e-mail or one ≥ 30 days old (never on every response). Key file
  `access-device.key` next to the sqlite file (0600); delete + restart = every device forgotten.
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
