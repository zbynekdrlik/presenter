# PWA via Cloudflare Tunnel Design (#218, closes #221)

**Status:** Proposed
**Date:** 2026-04-16
**Issues:** #218 (HTTPS for PWA), #221 (tablet PWA install — timer part already verified working)

## Problem

Chrome Android refuses to install a PWA over plain HTTP on a non-localhost origin. The current deploy serves presenter over HTTP on LAN IPs (`10.77.9.205`, `10.77.8.134`, companion-pp via Tailscale). "Add to home screen" creates a browser-tabbed shortcut, not a standalone PWA — the symptom reported on the tablet in #221.

Rather than manage HTTPS certs ourselves, wrap each instance in a **Cloudflare Tunnel** that terminates HTTPS at Cloudflare's edge and forwards plaintext HTTP to `localhost`. This mirrors the working pattern in [`zbynekdrlik/reaperiem/docs/cloudflare-tunnel-setup.md`](https://github.com/zbynekdrlik/reaperiem/blob/main/docs/cloudflare-tunnel-setup.md).

## Goals

- `https://presenter.newlevel.media` (prod), `https://presenter-dev.newlevel.media`, `https://presenter-pp.newlevel.media` all serve the app via Cloudflare-managed HTTPS.
- Tablet can install as standalone PWA from the HTTPS URL.
- Existing LAN HTTP URLs keep working — operators on LAN continue to use `http://presenter.lan` at LAN latency.
- UI shows whether the current client is on LAN or WAN (the reaperiem pattern), plus a small info popover with version + hostname + mode.

## Non-goals

- TLS in the Rust server itself. We do not bind 443 in `presenter-server`.
- HTTPS on port 443 via Let's Encrypt. Tunnel does the cert work.
- HTTP → HTTPS redirects. Both URLs coexist.
- LAN/WAN indicator in the operator UI (scope kept to the tablet PWA target; operator can get it later).
- Auto-detecting the church's public IP. Operator provides it via env var.

## Architecture

```
[tablet on WiFi]  https://presenter-pp.newlevel.media
                                  │
                                  ▼
                   Cloudflare edge (HTTPS termination)
                                  │
                                  ▼  (persistent outbound from host)
                   cloudflared daemon on companion-pp.lan
                                  │
                                  ▼
                   http://localhost:80 (presenter-server, unchanged)

[operator on LAN]  http://presenter.lan           (unchanged, no tunnel)
                   → presenter-server direct
```

### Cloudflare Tunnel per instance

Three tunnels, one per machine, each with its own credentials JSON:

| tunnel name       | hostname                         | host                  | backend             |
|-------------------|----------------------------------|-----------------------|---------------------|
| `presenter-prod`  | `presenter.newlevel.media`       | `presenter.lan` (10.77.9.205) | `http://localhost:80`   |
| `presenter-dev`   | `presenter-dev.newlevel.media`   | `10.77.8.134`         | `http://localhost:8080` |
| `presenter-pp`    | `presenter-pp.newlevel.media`    | `companion-pp.lan`    | `http://localhost:80`   |

Each host runs `cloudflared.service` (systemd) with:

```yaml
# /etc/cloudflared/config.yml
tunnel: <tunnel-id>
credentials-file: /etc/cloudflared/<tunnel-id>.json
no-autoupdate: true

ingress:
  - hostname: presenter-pp.newlevel.media
    service: http://localhost:80
  - service: http_status:404
```

The credentials JSON is per-tunnel and contains the tunnel secret. Stored as GitHub Actions secret per environment, written to `/etc/cloudflared/` by the deploy workflow with `chmod 600`.

### DNS

`cloudflared tunnel route dns <tunnel> <hostname>` creates a CNAME in Cloudflare pointing `presenter-pp.newlevel.media` at the tunnel's `<tunnel-id>.cfargotunnel.com`. We do this once per tunnel during setup.

Because traffic routes through Cloudflare edge, these are proxied (orange-cloud) by design — no `grey-cloud / DNS-only` flag needed. RFC1918 private IPs are not involved; the tunnel is the public face.

### Server-side: `detect_network_mode`

Port the reaperiem pattern almost verbatim (`iem-mixer/crates/iem-server/src/routes.rs:229-261`) into `crates/presenter-server/src/router/network_mode.rs`:

```rust
pub fn detect_network_mode(
    headers: &HeaderMap,
    local_public_ip: &Option<String>,
) -> &'static str {
    let client_ip = headers.get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string());

    match (&client_ip, local_public_ip) {
        (Some(client), Some(local)) if client == local => "local",
        (Some(_), Some(_)) => "remote",
        (None, _) => "local",                                // direct LAN
        (Some(ip), None) if is_private_ip(&ip) => "local",  // private range fallback
        (Some(_), None) => "remote",
    }
}
```

**Rule summary:**
- No `CF-Connecting-IP` header → direct LAN connection → `local`.
- `CF-Connecting-IP` matches `PRESENTER_LOCAL_PUBLIC_IP` → client is on the same egress as the server → `local`.
- Anything else → `remote`.

Fallback (when `PRESENTER_LOCAL_PUBLIC_IP` isn't set) uses `IpAddr::is_private`/`is_loopback` so the check still works in dev environments.

**Exposure:** `GET /api/network-mode` returning `{"mode": "local" | "remote"}`. Tablet UI calls it on mount.

### Config

New env var in `presenter-server`:
- `PRESENTER_LOCAL_PUBLIC_IP`: the church's outbound public IP (e.g., `203.0.113.50`). Optional; falls back to private-range detection.

Documented in `CLAUDE.md` and `docs/configuration.md`.

### Tablet UI changes

Add to the tablet timer bar component (`crates/presenter-ui/src/pages/tablet.rs` `TabletTimerBar`):

1. **LAN/WAN pill** next to the existing clock / elapsed / state spans. One signal, updated once on mount via `GET /api/network-mode`. Two classes: `.network-indicator--local` (accent color) and `.network-indicator--remote` (different color).
2. **Info button** (`ⓘ`) — small button next to the pill that opens a lightweight popover showing:
   - Version (`env!("CARGO_PKG_VERSION")` available via `presenter-core`)
   - Build channel (`dev` / `release`)
   - Hostname (`window.location.hostname`)
   - Network mode (duplicate of pill, useful in the popover for clarity)
   - A button labelled "Reload" that does `location.reload()` (service worker drops cache)
3. **PWA manifest:** no change. `start_url` is already relative (`/ui/tablet`), so installing from `https://presenter-pp.newlevel.media/ui/tablet` captures the correct origin into the installed PWA.

### Deploy workflow changes

New `install-cloudflared` step in each deploy job (dev, prod, PP), idempotent:

```bash
if ! command -v cloudflared &>/dev/null; then
  wget -q https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb -O /tmp/cf.deb
  sudo dpkg -i /tmp/cf.deb
  rm /tmp/cf.deb
fi
```

Then write `/etc/cloudflared/config.yml` + credentials (from `CLOUDFLARED_CREDS_DEV` / `_PROD` / `_PP` secrets), create `/etc/systemd/system/cloudflared.service` if missing, `systemctl enable --now cloudflared`.

The credentials JSON must NOT be committed. Stored in GitHub Environment Secrets, written at deploy time, `chmod 600`. Companion-pp currently uses the Tailscale `/etc/hosts` override — unchanged by this work.

### Tunnel creation (one-time, manual)

Before the first deploy can succeed, we:
1. Create the three tunnels via `cloudflared tunnel create presenter-prod` etc. on any machine with Cloudflare auth cookie.
2. Copy each resulting `<tunnel-id>.json` into the appropriate GitHub Environment Secret.
3. Run `cloudflared tunnel route dns <name> <hostname>` once per tunnel to create the Cloudflare CNAME.

The plan step for this is documented but not automated — this is a bootstrap one-off, tracked in a separate "one-time setup" task.

## Data Flow

```
Tablet mount (/ui/tablet)
  ↓
fetch("/api/network-mode")  → server reads CF-Connecting-IP vs PRESENTER_LOCAL_PUBLIC_IP
  ↓
{ "mode": "local" | "remote" }
  ↓
Leptos signal network_mode
  ↓
LAN/WAN pill + info popover render reactively
```

## Error Handling

- `cloudflared` can't reach Cloudflare → app still works on LAN via HTTP; HTTPS URL returns Cloudflare's "tunnel offline" page. Not a code bug.
- `/api/network-mode` fetch fails on the tablet → pill just doesn't render (`Show when=!mode.is_empty()`), like reaperiem.
- `PRESENTER_LOCAL_PUBLIC_IP` misconfigured → classifier falls back to private-range heuristic. Incorrect classification is cosmetic (wrong pill label), nothing breaks.
- Credentials JSON missing at deploy time → deploy fails loudly; `cloudflared` refuses to start without them. No silent degraded state.

## Testing

- **Unit**: `detect_network_mode` — 6 tests covering the match arms (Some/Some match, Some/Some mismatch, None/_, Some/None private, Some/None public, IPv6 loopback).
- **Unit**: `is_private_ip` — 4 tests (10.x, 192.168.x, 127.0.0.1, public IP).
- **HTTP**: `GET /api/network-mode` returns JSON for both `local` and `remote` simulated header sets.
- **E2E (Playwright)**: load `/ui/tablet`, assert `[data-role="network-indicator"]` is visible with either `LAN` or `WAN` text. Assert clicking `[data-role="info-button"]` reveals a popover with the version number.
- **Manual**: tablet on 4G (different public IP from church) hits `https://presenter-pp.newlevel.media/ui/tablet`, sees `WAN` pill; PWA install option appears and works.
- **Manual**: tablet on church WiFi hits `https://presenter-pp.newlevel.media/ui/tablet`, sees `LAN` pill (because CF-Connecting-IP matches church public IP).

## File Structure

| File | Change |
|------|--------|
| `crates/presenter-server/src/router/network_mode.rs` | **New** — `detect_network_mode`, `is_private_ip`, handler |
| `crates/presenter-server/src/router.rs` | Register `GET /api/network-mode` |
| `crates/presenter-server/src/config.rs` | Read `PRESENTER_LOCAL_PUBLIC_IP` env var |
| `crates/presenter-server/src/state/mod.rs` | Expose `local_public_ip` on AppState |
| `crates/presenter-ui/src/pages/tablet.rs` | Add network_mode signal + pill + info button |
| `crates/presenter-ui/src/components/info_popover.rs` | **New** — reusable popover |
| `crates/presenter-ui/src/api/mod.rs` (or similar) | `fetch_network_mode()` helper |
| `crates/presenter-ui/styles/tablet.css` (and parent) | Pill + popover styles |
| `.github/workflows/deploy.yml`, `pipeline.yml`, `release.yml` | Install cloudflared, write config + credentials |
| `deploy/cloudflared-config.yml.tmpl` | **New** — template rendered by deploy |
| `deploy/cloudflared.service` | **New** — systemd unit |
| `docs/architecture.md`, `CLAUDE.md` | Note tunnel setup + env var |

## Open Questions

None at spec time. One bootstrap step (tunnel creation + secret registration) documented but not automated; tracked as a task item in the plan.

## Future Work (out of scope)

- Operator UI version/mode indicator (nice-to-have; operator is always on LAN).
- Automatic public-IP detection via STUN so the env var isn't needed.
- Multi-origin auth tokens (tunnel + direct LAN don't currently share session cookies — not an issue because session state is DB-backed, but worth noting).
- Single shared tunnel with per-hostname ingress rules (vs 3 separate tunnels). Simpler to scale at the cost of tighter coupling across environments.
