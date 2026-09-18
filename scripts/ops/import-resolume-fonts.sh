#!/usr/bin/env bash
set -euo pipefail
#
# import-resolume-fonts.sh <base-url> [--dry-run]  (#778)
#
# Pull the church's TTF/OTF fonts off the Resolume machine and upload them to a
# Presenter instance as WEB fonts (served by Presenter, so every output browser
# — OBS source, remote machine — renders identical typography).
#
#   ./import-resolume-fonts.sh http://10.77.9.205            # import to SNV prod
#   ./import-resolume-fonts.sh http://10.77.8.134:8080 --dry-run
#
# Filtering (never upload a stock Microsoft font — licensing + noise):
#   * .ttc / .fon are skipped (a browser cannot @font-face a collection or a
#     bitmap .fon; ttf-parser cannot read them either).
#   * a font whose name-table manufacturer/copyright contains "Microsoft", OR
#     whose filename is in the known stock Windows list, is skipped. The
#     name-table read is done LOCALLY with python3 stdlib (read-font-metadata.py)
#     — no pip installs, and no stock font is ever uploaded.
#
# The SSH password is read only from ~/.secrets/resolume-ssh (never on argv / in
# the repo). Font files never enter git; the only committed font is the tiny OFL
# test fixture. A temp dir holds the pulled bytes and is removed on exit.

usage() {
    echo "usage: $0 <base-url> [--dry-run]" >&2
    exit 2
}

BASE_URL=""
DRY_RUN=0
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=1 ;;
        http://* | https://*) BASE_URL="${arg%/}" ;;
        -h | --help) usage ;;
        *) echo "unexpected argument: $arg" >&2; usage ;;
    esac
done
[ -n "$BASE_URL" ] || usage

# Host is overridable (RESOLUME_HOST) so the reachability guard below can be
# exercised against a bogus host in a test; defaults to the church's machine.
RESOLUME_HOST="${RESOLUME_HOST:-resolume.lan}"
REMOTE="newlevel@${RESOLUME_HOST}"
SECRET="${RESOLUME_SSH_SECRET:-$HOME/.secrets/resolume-ssh}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
META_READER="$SCRIPT_DIR/read-font-metadata.py"

[ -f "$SECRET" ] || { echo "ERROR: SSH secret not found: $SECRET" >&2; exit 1; }
[ -f "$META_READER" ] || { echo "ERROR: metadata reader not found: $META_READER" >&2; exit 1; }
command -v sshpass >/dev/null || { echo "ERROR: sshpass not installed" >&2; exit 1; }
command -v python3 >/dev/null || { echo "ERROR: python3 not installed" >&2; exit 1; }
command -v curl >/dev/null || { echo "ERROR: curl not installed" >&2; exit 1; }

TMP="$(mktemp -d)"
# Windows fonts arrive read-only (tar preserves the attribute), so make the tree
# writable before removing it; never let cleanup failure change the exit code.
cleanup() { chmod -R u+w "$TMP" 2>/dev/null || true; rm -rf "$TMP" 2>/dev/null || true; }
trap cleanup EXIT

SSH=(sshpass -f "$SECRET" ssh -o ConnectTimeout=20 -o StrictHostKeyChecking=accept-new "$REMOTE")

echo "==> Resolume font import  (target: $BASE_URL$([ "$DRY_RUN" = 1 ] && echo '  [DRY RUN]'))"

# --- Reachability guard (#778) -----------------------------------------------
# A silent SSH timeout used to yield an empty pull that "succeeded" with 0
# uploads (exit 0). Probe the host FIRST and fail loudly if it is unreachable.
# `|| true` here is deliberate: it stops `set -e` from killing us before we can
# print the friendly error; stderr is muted only because we emit a clearer line.
echo "==> Checking SSH reachability of $REMOTE ..."
if [ "$("${SSH[@]}" "echo OK" 2>/dev/null || true)" != "OK" ]; then
    echo "ERROR: cannot reach $REMOTE over SSH" >&2
    exit 1
fi

# --- Remote font source dirs (system + per-user when present) ----------------
REMOTE_DIRS=("C:/Windows/Fonts")
if "${SSH[@]}" 'if exist "%LOCALAPPDATA%\Microsoft\Windows\Fonts\" (echo YES)' 2>/dev/null | grep -q YES; then
    REMOTE_DIRS+=("%LOCALAPPDATA%/Microsoft/Windows/Fonts")
fi

# --- Count skip-by-extension (.ttc/.fon) straight from the remote listing ----
# The Fonts special folder rejects wildcards, so enumerate plainly + filter here.
skip_ttc=0
skip_fon=0
for d in "${REMOTE_DIRS[@]}"; do
    listing="$("${SSH[@]}" "dir /b \"$d\"" 2>/dev/null | tr -d '\r' || true)"
    skip_ttc=$((skip_ttc + $(printf '%s\n' "$listing" | grep -icE '\.ttc$' || true)))
    skip_fon=$((skip_fon + $(printf '%s\n' "$listing" | grep -icE '\.fon$' || true)))
done

# --- Pull all non-.fon files in ONE tar stream per dir (efficient) -----------
# stderr of ssh/tar is kept VISIBLE (no 2>/dev/null) so a transport/tar failure
# is diagnosable. `|| true` is kept ONLY because Windows tar exits non-zero on
# benign warnings (read-only font attributes, CRLF) yet still streams the files;
# the real gate is the zero-candidates check further down, which turns an empty
# pull (unreachable/empty dir) into a hard error rather than a false success.
echo "==> Pulling fonts from ${REMOTE_DIRS[*]} ..."
for d in "${REMOTE_DIRS[@]}"; do
    "${SSH[@]}" "tar -cf - --exclude=*.fon -C \"$d\" ." | tar -xf - -C "$TMP" || true
done

# --- Known stock Windows 11 filenames (belt-and-suspenders on top of the -----
#     name-table "Microsoft" check, for stock fonts that omit it). Lowercase,
#     space-delimited for the substring lookup below.
STOCK_FILES=" arial.ttf arialbd.ttf ariali.ttf arialbi.ttf ariblk.ttf
 times.ttf timesbd.ttf timesi.ttf timesbi.ttf cour.ttf courbd.ttf couri.ttf courbi.ttf
 tahoma.ttf tahomabd.ttf verdana.ttf verdanab.ttf verdanai.ttf verdanaz.ttf
 georgia.ttf georgiab.ttf georgiai.ttf georgiaz.ttf trebuc.ttf trebucbd.ttf trebucit.ttf trebucbi.ttf
 comic.ttf comicbd.ttf comici.ttf comicz.ttf impact.ttf webdings.ttf wingding.ttf symbol.ttf marlett.ttf
 calibri.ttf calibrib.ttf calibrii.ttf calibriz.ttf calibril.ttf calibrili.ttf
 cambriab.ttf cambriai.ttf cambriaz.ttf candara.ttf candarab.ttf candarai.ttf candaraz.ttf candaral.ttf candarali.ttf
 consola.ttf consolab.ttf consolai.ttf consolaz.ttf constan.ttf constanb.ttf constani.ttf constanz.ttf
 corbel.ttf corbelb.ttf corbeli.ttf corbelz.ttf corbell.ttf corbelli.ttf
 seguisym.ttf seguiemj.ttf seguihis.ttf segoeui.ttf segoeuib.ttf segoeuii.ttf segoeuiz.ttf segoeuil.ttf seguili.ttf segoeuisl.ttf seguisli.ttf
 segmdl2.ttf segoepr.ttf segoeprb.ttf segoesc.ttf segoescb.ttf sylfaen.ttf micross.ttf
 ebrima.ttf ebrimabd.ttf gadugi.ttf gadugib.ttf javatext.ttf leelawui.ttf leelawad.ttf
 mvboli.ttf nirmala.ttf nirmalab.ttf nirmalas.ttf phagspa.ttf phagspab.ttf holomdl2.ttf "

is_stock_file() {
    local lower="${1,,}"
    case "$STOCK_FILES" in
        *" $lower "*) return 0 ;;
        *) return 1 ;;
    esac
}

# --- Classify the pulled ttf/otf via the local name-table reader -------------
UPLOAD=()
UPLOAD_FAMILIES=()
skip_ms=0
skip_stock=0
skip_bad=0
candidates=0

# read-font-metadata.py emits TSV: <filename>\t<family>\t<is_microsoft>\t<ok>
while IFS=$'\t' read -r fname family is_ms ok; do
    [ -n "$fname" ] || continue
    candidates=$((candidates + 1))
    if [ "$ok" != "1" ]; then
        skip_bad=$((skip_bad + 1))
        continue
    fi
    if is_stock_file "$fname"; then
        skip_stock=$((skip_stock + 1))
        continue
    fi
    if [ "$is_ms" = "1" ]; then
        skip_ms=$((skip_ms + 1))
        continue
    fi
    UPLOAD+=("$fname")
    [ -n "$family" ] && UPLOAD_FAMILIES+=("$family")
done < <(python3 "$META_READER" "$TMP")

# Distinct, sorted family list of what would be uploaded.
FAMILIES=()
if [ "${#UPLOAD_FAMILIES[@]}" -gt 0 ]; then
    mapfile -t FAMILIES < <(printf '%s\n' "${UPLOAD_FAMILIES[@]}" | sed '/^$/d' | sort -u)
fi

echo
echo "==> Summary"
echo "    candidates copied (ttf/otf) : $candidates"
echo "    WOULD UPLOAD                 : ${#UPLOAD[@]}"
echo "    skipped (Microsoft name)     : $skip_ms"
echo "    skipped (stock filename)     : $skip_stock"
echo "    skipped (unparseable)        : $skip_bad"
echo "    skipped (.ttc)               : $skip_ttc"
echo "    skipped (.fon)               : $skip_fon"
echo "    distinct families to upload  : ${#FAMILIES[@]}"
if [ "${#FAMILIES[@]}" -gt 0 ]; then
    echo "    families:"
    printf '      - %s\n' "${FAMILIES[@]}"
fi

# --- Empty-pull guard (#778) -------------------------------------------------
# Zero ttf/otf candidates means the pull produced nothing — an unreachable host
# (past the probe race) or an empty remote listing. Never report a "0 uploaded"
# success in that case; fail so the caller knows the import did not happen.
if [ "$candidates" -eq 0 ]; then
    echo "ERROR: remote pull produced no font files (host unreachable or empty listing)" >&2
    exit 1
fi

if [ "$DRY_RUN" = 1 ]; then
    echo
    echo "==> DRY RUN — nothing uploaded."
    exit 0
fi

# --- Upload each would-upload file (one multipart request per file) ----------
echo
echo "==> Uploading ${#UPLOAD[@]} font(s) to $BASE_URL/stream/fonts ..."
uploaded=0
failed=0
if [ "${#UPLOAD[@]}" -gt 0 ]; then
    for fname in "${UPLOAD[@]}"; do
        code="$(curl -sS -o /dev/null -w '%{http_code}' -F "file=@$TMP/$fname" "$BASE_URL/stream/fonts" || echo 000)"
        if [ "$code" = "200" ]; then
            uploaded=$((uploaded + 1))
        else
            failed=$((failed + 1))
            echo "    FAILED ($code): $fname" >&2
        fi
    done
fi
echo "==> Done: $uploaded uploaded, $failed failed."
[ "$failed" = 0 ]
