#!/usr/bin/env bash
# Deploy-configuration guards (#538, #539).
#
# These assert the SHAPE of what we ship to the hosts — the class of bug that CI
# never catches because it lives in the unit file and the deploy workflow, not in
# the Rust:
#
#   #538  The shared `presenter.service` must not hardcode ONE site's stage URL.
#         It did (SNV's), so PP had to override it with a drop-in and every future
#         site that forgets one silently points its TVs at SNV.
#
#   #539  The encoder-ready gate must not burn its full timeout on a host that can
#         never satisfy it (no GPU / no access), and it must recognise the MODERN
#         nvcodec encoders (#541) — not just the legacy element.
#
#   #544  The GStreamer plugin registry must not outlive a permission/driver change.
#         PP cached "va has 0 features" back when the service could not open the GPU;
#         GST_REGISTRY_UPDATE=yes only rescans plugins whose FILE changed, so that
#         verdict was trusted forever and NDI stayed dead even after #540 restored
#         access. The service therefore owns a registry inside its (writable) deploy
#         dir, and every deploy deletes it so the next start rescans against reality.
#
#   #547  The gate must actually PROBE on every poll. `gst-inspect --exists` answers
#         from the cached registry, so the old loop performed ONE real scan and then
#         re-read that same verdict 29 times — it could never see a late-registering
#         encoder (#339's whole scenario), and on a start that inherited a warm
#         registry (reboot, Restart=always) it scanned nothing at all. The gate must
#         therefore delete the registry before each poll, probe FUNCTIONALLY (the same
#         real encode the server does, #541), and bound every external call so a
#         wedged driver cannot hang ExecStartPre and fail the whole unit.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WAIT_SCRIPT="$REPO_ROOT/scripts/deploy/wait-for-h264-encoder.sh"
PROD_UNIT="$REPO_ROOT/scripts/deploy/presenter.service"
DEV_UNIT="$REPO_ROOT/scripts/deploy/presenter-dev.service"

failures=0
check() {
  local name="$1"
  shift
  if "$@"; then
    echo "  ok   — $name"
  else
    echo "  FAIL — $name"
    failures=$((failures + 1))
  fi
}

# ── #538: no site URL baked into the shared unit ────────────────────────────────
not_hardcoding_stage_url() {
  ! grep -qE '^Environment=PRESENTER_ANDROID_STAGE_URL=' "$PROD_UNIT"
}

prod_deploy_writes_stage_url_dropin() {
  grep -q 'stage-url.conf' "$REPO_ROOT/.github/workflows/deploy.yml"
}

pp_release_writes_stage_url_dropin() {
  grep -q 'stage-url.conf' "$REPO_ROOT/.github/workflows/release.yml"
}

# ── #539: the encoder-ready gate ───────────────────────────────────────────────
# The script is driven entirely by env vars so it can be tested without a GPU:
#   PRESENTER_RENDER_NODE     — path to probe instead of /dev/dri/renderD128
#   PRESENTER_ENCODER_WAIT_SECS — timeout
# and it finds encoders via `gst-inspect-1.0` on PATH, which we stub below.

# The gate probes FUNCTIONALLY — it runs the same one-frame encode the server runs
# (`videotestsrc num-buffers=1 ! … ! <encoder> ! fakesink`), not a name lookup (#547).
# This stub stands in for `gst-launch-1.0` and models the two behaviours that matter:
#
#   * it answers from the plugin REGISTRY CACHE — so it records, on every call,
#     whether the registry file was still present (warm) or had been deleted (cold).
#     A gate that does not delete it is re-reading one stale verdict, which is the
#     whole of #547. A real probe rebuilds the registry, so the stub recreates it.
#   * an encoder can register LATE (`available_after` seconds — the #339 boot race),
#     and a probe can HANG (`block_secs` — a wedged driver, #445).
stub_gst_launch() {
  # $1 = bin dir, $2 = encoders that can encode, $3 = seconds until they register,
  # $4 = seconds each probe blocks
  local dir="$1" available="$2" available_after="${3:-0}" block_secs="${4:-0}"
  mkdir -p "$dir"
  cat >"$dir/gst-launch-1.0" <<EOF
#!/usr/bin/env bash
set -euo pipefail
started_at="$dir/started-at"
if [ ! -f "\$started_at" ]; then date +%s >"\$started_at"; fi
echo "\$*" >>"$dir/calls"
if [ -n "\${GST_REGISTRY:-}" ] && [ -e "\$GST_REGISTRY" ]; then
  echo warm >>"$dir/registry-state"
else
  echo cold >>"$dir/registry-state"
fi
if [ -n "\${GST_REGISTRY:-}" ]; then : >"\$GST_REGISTRY"; fi
if [ "$block_secs" -gt 0 ]; then sleep "$block_secs"; fi
if [ \$(( \$(date +%s) - \$(cat "\$started_at") )) -lt $available_after ]; then exit 1; fi
for e in $available; do
  for a in "\$@"; do
    if [ "\$a" = "\$e" ]; then exit 0; fi
  done
done
exit 1
EOF
  chmod +x "$dir/gst-launch-1.0"
}

# Runs the gate against a stubbed toolchain. Sets $rc / $elapsed / $out for the caller.
run_gate() {
  # $1 = bin dir, $2 = render-node path, $3 = wait secs, $4 = per-probe timeout secs,
  # $5 = registry path ("" = unset)
  local bin="$1" node="$2" wait_secs="$3" probe_timeout="$4" registry="$5"
  local start=$SECONDS
  set +e
  out="$(GST_REGISTRY="$registry" PATH="$bin:$PATH" \
    PRESENTER_RENDER_NODE="$node" \
    PRESENTER_ENCODER_WAIT_SECS="$wait_secs" \
    PRESENTER_ENCODER_PROBE_TIMEOUT_SECS="$probe_timeout" \
    bash "$WAIT_SCRIPT" 2>&1)"
  rc=$?
  set -e
  elapsed=$((SECONDS - start))
}

exits_fast_without_a_render_node() {
  local tmp
  tmp="$(mktemp -d)"
  stub_gst_launch "$tmp/bin" ""
  run_gate "$tmp/bin" "$tmp/definitely-not-a-render-node" 30 8 ""
  rm -rf "$tmp"
  # The whole point of #539: a GPU-less host must not sit through the timeout —
  # and it must still let the service start (rc 0), just without NDI.
  [ "$rc" -eq 0 ] && [ "$elapsed" -lt 5 ]
}

exits_fast_when_the_node_is_not_accessible() {
  local tmp
  tmp="$(mktemp -d)"
  stub_gst_launch "$tmp/bin" ""
  # A node that exists but this user cannot open — the #540 shape.
  install -m 000 /dev/null "$tmp/renderD128"
  run_gate "$tmp/bin" "$tmp/renderD128" 2 8 ""
  rm -rf "$tmp"
  # Non-root: the gate must SAY it cannot open the node (that message is the only
  # thing that tells an operator the service user is missing the `render` group,
  # #540) and skip at once. Root can open anything, so it falls through to the
  # (short) bounded wait instead — both must exit 0 and neither may hang.
  if [ "$rc" -ne 0 ] || [ "$elapsed" -ge 5 ]; then return 1; fi
  if [ "$(id -u)" -ne 0 ]; then
    grep -q "not openable" <<<"$out" && grep -q "render" <<<"$out"
  fi
}

probes_functionally_rather_than_by_name() {
  # #547: `gst-inspect --exists <name>` only asks the cached registry whether the
  # NAME is known. The server decides with a real one-frame encode (#541), so a
  # host whose first REGISTERED candidate is broken satisfied a name-only gate
  # while the server then rejected that same encoder. The gate must run the same
  # encode — i.e. it must invoke gst-launch with a videotestsrc→encoder pipeline.
  local tmp
  tmp="$(mktemp -d)"
  stub_gst_launch "$tmp/bin" "vah264enc"
  install -m 666 /dev/null "$tmp/renderD128"
  run_gate "$tmp/bin" "$tmp/renderD128" 5 8 "$tmp/registry.bin"
  local calls="$tmp/calls"
  local ok=1
  { [ "$rc" -eq 0 ] &&
    [ -f "$calls" ] &&
    grep -q 'videotestsrc' "$calls" &&
    grep -q 'vah264enc' "$calls"; } || ok=0
  rm -rf "$tmp"
  [ "$ok" -eq 1 ]
}

forces_a_registry_rescan_on_every_poll() {
  # THE #547 REGRESSION. Every gst tool answers from the cached plugin registry, so
  # unless the gate deletes it first, poll 2..N re-read poll 1's verdict and the
  # loop is decorative. Worse: a start that inherits a warm registry (reboot,
  # Restart=always — the registry is only deleted at DEPLOY time) never scans at all.
  # The stub records warm/cold per call; every call must be COLD.
  local tmp
  tmp="$(mktemp -d)"
  # Encoder registers only after 3s → the gate must poll several times to find it.
  stub_gst_launch "$tmp/bin" "vah264enc" 3
  install -m 666 /dev/null "$tmp/renderD128"
  # Pre-existing (warm) registry — exactly what a reboot inherits.
  echo stale >"$tmp/registry.bin"
  run_gate "$tmp/bin" "$tmp/renderD128" 20 8 "$tmp/registry.bin"
  local states="$tmp/registry-state"
  local ok=1
  { [ "$rc" -eq 0 ] &&
    grep -q 'encodes on this host' <<<"$out" &&
    [ -f "$states" ] &&
    [ "$(grep -c warm "$states" || true)" -eq 0 ] &&
    [ "$(grep -c cold "$states" || true)" -ge 2 ]; } || ok=0
  rm -rf "$tmp"
  [ "$ok" -eq 1 ]
}

finds_a_late_registering_encoder() {
  # #339's actual scenario: udev creates the render node, but VA-API registers its
  # encoders another 30-50s later. That IS what the gate exists for — it must still
  # be probing (not re-reading a cached "no") when the encoder finally appears.
  local tmp
  tmp="$(mktemp -d)"
  stub_gst_launch "$tmp/bin" "vah264enc" 4
  install -m 666 /dev/null "$tmp/renderD128"
  run_gate "$tmp/bin" "$tmp/renderD128" 20 8 "$tmp/registry.bin"
  local ok=1
  { [ "$rc" -eq 0 ] &&
    grep -q 'vah264enc encodes on this host' <<<"$out" &&
    [ "$elapsed" -ge 4 ] && [ "$elapsed" -lt 20 ]; } || ok=0
  rm -rf "$tmp"
  [ "$ok" -eq 1 ]
}

bounds_every_probe_so_a_wedged_driver_cannot_hang_the_unit() {
  # The old gate was `ExecStartPre=-/usr/bin/timeout 30 sh -c …` — hard-bounded. The
  # rewrite moved the deadline INSIDE the loop, where it is only evaluated BETWEEN
  # iterations: one gst call blocking in a wedged driver (#445, a recurring dev2
  # event) hangs ExecStartPre until TimeoutStartSec and then FAILS the unit — taking
  # lyrics, Bible and timers down over a missing encoder. Every probe must be bounded.
  local tmp
  tmp="$(mktemp -d)"
  stub_gst_launch "$tmp/bin" "vah264enc" 0 60 # every probe blocks for 60s
  install -m 666 /dev/null "$tmp/renderD128"
  run_gate "$tmp/bin" "$tmp/renderD128" 3 2 "$tmp/registry.bin"
  rm -rf "$tmp"
  # 4 candidates x 2s probe timeout + the 3s deadline, with generous slack.
  [ "$rc" -eq 0 ] && [ "$elapsed" -lt 25 ]
}

accepts_the_modern_nvcodec_encoder() {
  # #541: the gate used to poll ONLY vah264enc / nvh264enc. On a driver where the
  # legacy element is dead, the modern one is the encoder we will actually use, so
  # waiting for the legacy name would burn the whole timeout on a perfectly good host.
  local tmp
  tmp="$(mktemp -d)"
  stub_gst_launch "$tmp/bin" "nvcudah264enc"
  install -m 666 /dev/null "$tmp/renderD128"
  run_gate "$tmp/bin" "$tmp/renderD128" 20 8 "$tmp/registry.bin"
  rm -rf "$tmp"
  [ "$rc" -eq 0 ] && [ "$elapsed" -lt 5 ]
}

both_units_use_the_gate_script() {
  grep -q 'wait-for-h264-encoder.sh' "$PROD_UNIT" && grep -q 'wait-for-h264-encoder.sh' "$DEV_UNIT"
}

units_hard_bound_the_gate() {
  # Belt to the script's own per-probe braces: the unit caps the WHOLE gate with
  # `timeout`, and pins TimeoutStartSec explicitly instead of inheriting
  # DefaultTimeoutStartSec (90s) — which the gate's own budget must stay under.
  grep -qE '^ExecStartPre=-/usr/bin/timeout [0-9]+ /opt/presenter/wait-for-h264-encoder\.sh' "$PROD_UNIT" &&
    grep -qE '^ExecStartPre=-/usr/bin/timeout [0-9]+ /opt/presenter-dev/wait-for-h264-encoder\.sh' "$DEV_UNIT" &&
    grep -qE '^TimeoutStartSec=' "$PROD_UNIT" &&
    grep -qE '^TimeoutStartSec=' "$DEV_UNIT"
}

# ── #544: the registry must be service-owned, writable and rebuilt on deploy ────
units_own_a_writable_gst_registry() {
  # The path must sit inside the unit's ReadWritePaths (its deploy dir), NOT in
  # $HOME — ProtectHome=read-only makes a $HOME cache unfixable once it goes stale.
  grep -qE '^Environment=GST_REGISTRY=/opt/presenter/' "$PROD_UNIT" &&
    grep -qE '^Environment=GST_REGISTRY=/opt/presenter-dev/' "$DEV_UNIT"
}

every_deploy_rebuilds_the_registry() {
  # Deleting the registry file is what guarantees the next start re-probes the
  # plugins against the CURRENT permissions/driver. Without it, #544 comes back the
  # first time a host's GPU access or driver changes. Match the actual `rm` of the
  # actual path the unit points GST_REGISTRY at — a bare mention of the filename
  # (a leftover comment) used to satisfy this guard.
  grep -qE 'rm -f +/opt/presenter/gstreamer-registry\.bin' "$REPO_ROOT/.github/workflows/deploy.yml" &&
    grep -qE 'rm -f +/opt/presenter/gstreamer-registry\.bin' "$REPO_ROOT/.github/workflows/release.yml" &&
    grep -qE 'rm -f +/opt/presenter-dev/gstreamer-registry\.bin' "$REPO_ROOT/.github/workflows/pipeline.yml"
}

every_deploy_ships_the_gate_script() {
  # The unit calls the gate with a leading `-` (never fatal), so a deploy that stops
  # installing the script disables the gate SILENTLY — no failure, no log, NDI just
  # races the driver again. Assert every deploy actually puts the script on the host.
  grep -q 'wait-for-h264-encoder.sh' "$REPO_ROOT/.github/workflows/deploy.yml" &&
    grep -q 'wait-for-h264-encoder.sh' "$REPO_ROOT/.github/workflows/release.yml" &&
    grep -q 'wait-for-h264-encoder.sh' "$REPO_ROOT/.github/workflows/pipeline.yml"
}

echo "Deploy-config guards (#538, #539, #544, #547):"
check "#538 shared unit does not hardcode a site stage URL" not_hardcoding_stage_url
check "#538 prod deploy writes its own stage-url drop-in" prod_deploy_writes_stage_url_dropin
check "#538 PP release writes its own stage-url drop-in" pp_release_writes_stage_url_dropin
check "#539 gate exits fast when there is no render node" exits_fast_without_a_render_node
check "#539 gate exits fast when the render node is inaccessible" exits_fast_when_the_node_is_not_accessible
check "#539 gate accepts the modern nvcodec encoder (#541)" accepts_the_modern_nvcodec_encoder
check "#539 both deployed units call the gate script" both_units_use_the_gate_script
check "#544 units own a writable GST_REGISTRY in their deploy dir" units_own_a_writable_gst_registry
check "#544 every deploy rebuilds the plugin registry" every_deploy_rebuilds_the_registry
check "#547 gate probes functionally (real encode), not by name" probes_functionally_rather_than_by_name
check "#547 gate forces a real plugin rescan on EVERY poll" forces_a_registry_rescan_on_every_poll
check "#547 gate finds an encoder that registers late (#339)" finds_a_late_registering_encoder
check "#547 gate bounds every probe (a wedged driver cannot hang the unit)" bounds_every_probe_so_a_wedged_driver_cannot_hang_the_unit
check "#547 units hard-bound the gate with timeout + TimeoutStartSec" units_hard_bound_the_gate
check "#547 every deploy ships the gate script" every_deploy_ships_the_gate_script

if [ "$failures" -ne 0 ]; then
  echo "Deploy-config guards: $failures FAILED"
  exit 1
fi
echo "Deploy-config guards: all passed"
