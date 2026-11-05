#!/usr/bin/env bash
# Verification for the business-city repair (2026-11-05).
#   1. the repair is a no-op now  → `drift=0` (no listing carries a half city name)
#   2. the live screens prove it  → real browser, community home + Palm Bay city page
# Exit 0 only when both hold. Safe to run any time; writes nothing to the database.
set -euo pipefail
cd "$(dirname "$0")/.."

drift=$(python3 scripts/business-city-repair.py | sed -n '1s/.*drift=//p')
[[ "$drift" == "0" ]] || { echo "FAIL  $drift listings still carry a half city name"; exit 1; }
echo "PASS  no listing carries a half city name (drift=0)"

NODE_PATH=${NODE_PATH:-/opt/swift/browser-run/node_modules} \
SHOT_DIR=${SHOT_DIR:-/tmp} \
  node scripts/business-city-display-browser.js
