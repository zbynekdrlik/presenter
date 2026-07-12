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

stub_gst_inspect() {
  # $1 = space-separated list of encoder names that "exist"
  local dir="$1" available="$2"
  mkdir -p "$dir"
  cat >"$dir/gst-inspect-1.0" <<EOF
#!/usr/bin/env bash
# stub: only the --exists form is used by the gate
for e in $available; do [ "\$2" = "\$e" ] && exit 0; done
exit 1
EOF
  chmod +x "$dir/gst-inspect-1.0"
}

exits_fast_without_a_render_node() {
  local tmp
  tmp="$(mktemp -d)"
  stub_gst_inspect "$tmp/bin" ""
  local start=$SECONDS
  PATH="$tmp/bin:$PATH" \
    PRESENTER_RENDER_NODE="$tmp/definitely-not-a-render-node" \
    PRESENTER_ENCODER_WAIT_SECS=30 \
    bash "$WAIT_SCRIPT" >/dev/null 2>&1
  local rc=$?
  local elapsed=$((SECONDS - start))
  rm -rf "$tmp"
  # The whole point of #539: a GPU-less host must not sit through the timeout —
  # and it must still let the service start (rc 0), just without NDI.
  [ "$rc" -eq 0 ] && [ "$elapsed" -lt 5 ]
}

exits_fast_when_the_node_is_not_accessible() {
  local tmp
  tmp="$(mktemp -d)"
  stub_gst_inspect "$tmp/bin" ""
  # A node that exists but this user cannot write — the #540 shape.
  install -m 000 /dev/null "$tmp/renderD128"
  local start=$SECONDS
  PATH="$tmp/bin:$PATH" \
    PRESENTER_RENDER_NODE="$tmp/renderD128" \
    PRESENTER_ENCODER_WAIT_SECS=2 \
    bash "$WAIT_SCRIPT" >/dev/null 2>&1
  local rc=$?
  local elapsed=$((SECONDS - start))
  rm -rf "$tmp"
  # Non-root: the gate sees it cannot open the node and skips at once. Root can
  # open anything, so it falls through to the (short) bounded wait instead — both
  # must exit 0 and neither may hang.
  [ "$rc" -eq 0 ] && [ "$elapsed" -lt 5 ]
}

waits_and_succeeds_when_an_encoder_is_present() {
  local tmp
  tmp="$(mktemp -d)"
  stub_gst_inspect "$tmp/bin" "vah264enc"
  install -m 666 /dev/null "$tmp/renderD128"
  PATH="$tmp/bin:$PATH" \
    PRESENTER_RENDER_NODE="$tmp/renderD128" \
    PRESENTER_ENCODER_WAIT_SECS=5 \
    bash "$WAIT_SCRIPT" >/dev/null 2>&1
  local rc=$?
  rm -rf "$tmp"
  [ "$rc" -eq 0 ]
}

accepts_the_modern_nvcodec_encoder() {
  # #541: the gate used to poll ONLY vah264enc / nvh264enc. On a driver where the
  # legacy element is dead, the modern one is the encoder we will actually use, so
  # waiting for the legacy name would burn the whole timeout on a perfectly good host.
  local tmp
  tmp="$(mktemp -d)"
  stub_gst_inspect "$tmp/bin" "nvcudah264enc"
  install -m 666 /dev/null "$tmp/renderD128"
  local start=$SECONDS
  PATH="$tmp/bin:$PATH" \
    PRESENTER_RENDER_NODE="$tmp/renderD128" \
    PRESENTER_ENCODER_WAIT_SECS=20 \
    bash "$WAIT_SCRIPT" >/dev/null 2>&1
  local rc=$?
  local elapsed=$((SECONDS - start))
  rm -rf "$tmp"
  [ "$rc" -eq 0 ] && [ "$elapsed" -lt 5 ]
}

both_units_use_the_gate_script() {
  grep -q 'wait-for-h264-encoder.sh' "$PROD_UNIT" && grep -q 'wait-for-h264-encoder.sh' "$DEV_UNIT"
}

echo "Deploy-config guards (#538, #539):"
check "#538 shared unit does not hardcode a site stage URL" not_hardcoding_stage_url
check "#538 prod deploy writes its own stage-url drop-in" prod_deploy_writes_stage_url_dropin
check "#538 PP release writes its own stage-url drop-in" pp_release_writes_stage_url_dropin
check "#539 gate exits fast when there is no render node" exits_fast_without_a_render_node
check "#539 gate exits fast when the render node is inaccessible" exits_fast_when_the_node_is_not_accessible
check "#539 gate succeeds once an encoder is registered" waits_and_succeeds_when_an_encoder_is_present
check "#539 gate accepts the modern nvcodec encoder (#541)" accepts_the_modern_nvcodec_encoder
check "#539 both deployed units call the gate script" both_units_use_the_gate_script

if [ "$failures" -ne 0 ]; then
  echo "Deploy-config guards: $failures FAILED"
  exit 1
fi
echo "Deploy-config guards: all passed"
