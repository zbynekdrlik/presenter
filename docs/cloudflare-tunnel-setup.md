# Cloudflare Tunnel Setup

One-off bootstrap for issue #218. Run once per instance (dev, prod, PP).

The deploy workflows already contain an "Install & configure cloudflared" step that is a no-op until the operator registers the secrets and variables below. Once they exist, the next deploy activates the tunnel automatically.

## Prerequisites

- Cloudflare account with the `newlevel.media` zone.
- Any machine with `cloudflared` installed to run the setup commands below. (The tunnel will run on the target machines — this is just for creation.)

## Create the tunnel

Install cloudflared locally if you haven't:

```bash
wget -q https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb -O /tmp/cf.deb
sudo dpkg -i /tmp/cf.deb
```

Authenticate once (opens a browser):

```bash
cloudflared tunnel login
```

Create three tunnels + CNAMEs, one per instance:

```bash
cloudflared tunnel create presenter-dev
cloudflared tunnel route dns presenter-dev presenter-dev.newlevel.media

cloudflared tunnel create presenter-prod
cloudflared tunnel route dns presenter-prod presenter.newlevel.media

cloudflared tunnel create presenter-pp
cloudflared tunnel route dns presenter-pp presenter-pp.newlevel.media
```

Each `cloudflared tunnel create` prints a Tunnel ID (UUID) and writes `~/.cloudflared/<tunnel-id>.json`. Keep both — you'll paste them into GitHub below.

## Register GitHub secrets and variables

For each environment in the repository (Settings → Environments → Dev / Prod / PP):

**Secrets (encrypted):**

| Name | Value |
|------|-------|
| `CLOUDFLARED_CREDS_DEV` / `_PROD` / `_PP` | Full contents of `~/.cloudflared/<tunnel-id>.json` |

**Variables (plaintext):**

| Name | Value |
|------|-------|
| `CLOUDFLARED_TUNNEL_ID_DEV` / `_PROD` / `_PP` | The Tunnel ID UUID printed by `cloudflared tunnel create` |
| `CLOUDFLARED_HOSTNAME_DEV` / `_PROD` / `_PP` | `presenter-dev.newlevel.media` / `presenter.newlevel.media` / `presenter-pp.newlevel.media` |
| `CLOUDFLARED_BACKEND_PORT_DEV` / `_PROD` / `_PP` | `8080` for dev, `80` for prod and PP |

## Configure the church public IP

The server uses `PRESENTER_LOCAL_PUBLIC_IP` to classify requests arriving via the tunnel as LAN (if the tunnel's `CF-Connecting-IP` matches this value) vs WAN (everything else).

Get the church's outbound IP from any machine on the church LAN:

```bash
curl -s ifconfig.me
```

Set it in each presenter-server service's env file (e.g., `/etc/default/presenter-dev` or `/etc/systemd/system/presenter-dev.service.d/override.conf`):

```
PRESENTER_LOCAL_PUBLIC_IP=203.0.113.50
```

Restart the service. Verify from the same machine:

```bash
curl http://localhost:80/api/network-mode
# → {"mode":"local"}
```

If `PRESENTER_LOCAL_PUBLIC_IP` is unset, the server falls back to the private-range heuristic (10.x / 172.16-31.x / 192.168.x + loopback → local). This is correct for direct LAN access but can't distinguish "on church LAN via tunnel" from "on WAN via tunnel" — both show up with a public `CF-Connecting-IP`.

## Deploy + verify

1. Trigger a deploy (push to dev / merge to main / create a GitHub Release for PP).
2. The "Install & configure cloudflared" step activates the tunnel.
3. From an external network (phone on cellular, not LAN), open `https://presenter-dev.newlevel.media` — page loads, "Add to home screen" installs the PWA as standalone.
4. On the church LAN WiFi, reload the tablet — indicator should flip to `LAN`.

## Troubleshooting

**Tunnel not connecting**

On the deploy target:

```bash
sudo systemctl status cloudflared
sudo journalctl -u cloudflared -n 50
sudo cloudflared tunnel info <tunnel-name>
```

**Wrong LAN/WAN label**

Check `PRESENTER_LOCAL_PUBLIC_IP`:

```bash
systemctl show presenter-dev | grep Environment
```

The value must match what Cloudflare sees. Verify by checking `curl -s ifconfig.me` from the same machine.

**Cert error in browser**

The tunnel terminates HTTPS at Cloudflare's edge. If the browser shows a cert error, the request is not actually reaching Cloudflare — verify DNS. `dig presenter-dev.newlevel.media` should return a Cloudflare IP.

## Notes

- Tunnel credentials live at `/etc/cloudflared/<tunnel-id>.json` on each target, chmod 600, owned by root.
- The tunnel runs as the `cloudflared.service` systemd unit and starts automatically on boot.
- Each tunnel is tied to a single machine (one-to-one). The cross-machine tunnel-sharing setup is more complex and out of scope here.
- Traffic is TLS end-to-end (Cloudflare ↔ cloudflared daemon ↔ localhost). No cert management in our code.
