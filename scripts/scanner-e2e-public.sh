#!/usr/bin/env bash
# scanner-e2e-public.sh — the item-2 scanner proof, driven through the PUBLIC hostname.
#
# Why a separate entry point: scripts/scanner-e2e-smoke.py defaults to http://127.0.0.1:8089, so
# every earlier proof of the scanner exercised the app process directly and never the path a real
# shop employee uses — nginx + Cloudflare + the public origin. This wrapper runs the very same
# headless-Chromium proof (camera -> QR read -> amount -> points -> recorded, all three counter
# modes) against https://zaarhub.com/scanner and verifies the scan rows in the database. The
# checkout/edge is exactly the class of defect the fleet has been bitten by before (a page that
# answers 200 at the API but is broken in a real browser on the real hostname).
#
# Usage:  bash scripts/scanner-e2e-public.sh            # default https://zaarhub.com
#         MD_PUBLIC_BASE=https://directory.swiftsoftware.net bash scripts/scanner-e2e-public.sh
# Exit:   0 = PASS, 1 = FAIL.  Evidence (screenshots + browser-proof.json + out.txt) lands in AUD.
set -u
cd "$(dirname "$0")/.." || exit 2

BASE="${MD_PUBLIC_BASE:-https://zaarhub.com}"
# AUD must be a clean absolute path (no ".."): Chromium's --use-file-for-fake-video-capture
# fails to open a path that contains a parent-directory hop and the fake camera then reports
# "Requested device not found".
AUD="$(readlink -m "$(dirname "$0")/../../../audits")/zz-item2-publichost-$(date +%Y%m%d-%H%M%S)"

if ! curl -s -o /dev/null -m 20 -A 'Mozilla/5.0 (compatible; SwiftOps/1.0)' -w '' "$BASE/scanner"; then
  echo "public scanner page unreachable at $BASE/scanner"
  exit 1
fi

mkdir -p "$AUD"
export AUD
export MD_BASE="$BASE"
export RUSTUP_HOME="${RUSTUP_HOME:-/root/.rustup}"
export CARGO_HOME="${CARGO_HOME:-/root/.cargo}"

echo "scanner public-host proof: MD_BASE=$BASE"
echo "evidence: $AUD"
timeout 600 python3 scripts/scanner-e2e-smoke.py > "$AUD/out.txt" 2>&1
rc=$?
tail -30 "$AUD/out.txt"
grep -q '^RESULT: PASS' "$AUD/out.txt" || rc=1
echo "RESULT: $([ "$rc" -eq 0 ] && echo PASS || echo FAIL)  (evidence $AUD/out.txt)"
exit "$rc"
