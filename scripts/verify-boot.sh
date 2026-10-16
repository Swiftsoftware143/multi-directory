#!/usr/bin/env bash
# Boot the app for `hermes verify` (detect -> build -> test -> boot -> readiness).
#
# `hermes verify` starts a SECOND copy of this app, and on this box that must never mean:
#   1. binding the live container's port, or
#   2. pointing at the live database.
# So the verifier boots on the recipe's port (8000) against a scratch database, which the app's own
# install path (src/migrations/000_baseline_live_schema.sql) builds from nothing — proven by
# scripts/verify-baseline-fromzero.sh (`RESULT: PASS`).
set -uo pipefail

SCRATCH_DB="${VERIFY_SCRATCH_DB:-md_verify_scratch}"
LIVE_URL="$(docker exec multidirectory printenv DATABASE_URL 2>/dev/null || true)"
if [ -z "$LIVE_URL" ]; then
  echo "verify-boot: could not read the live DATABASE_URL from the multidirectory container" >&2
  exit 1
fi

PW="$(printf '%s' "$LIVE_URL" | sed -E 's#.*://[^:]+:([^@]+)@.*#\1#')"
psql_() { docker exec -i -e PGPASSWORD="$PW" swift-postgres psql -U swift -d "$1" -At -c "$2"; }

# same host/user/password as the live app, a different database
psql_ postgres "SELECT 1 FROM pg_database WHERE datname='${SCRATCH_DB}'" | grep -q 1 \
  || docker exec -e PGPASSWORD="$PW" swift-postgres createdb -U swift "${SCRATCH_DB}"

export APP_HOST=127.0.0.1
export APP_PORT="${VERIFY_APP_PORT:-8000}"
export DATABASE_URL="${LIVE_URL%/*}/${SCRATCH_DB}"
# provider-key writes fail closed without it; the verifier writes no key, but the app warns loudly
export PROVIDER_KEY_ENC_SECRET="${PROVIDER_KEY_ENC_SECRET:-$(head -c 48 /dev/urandom | base64 | tr -d '\n')}"

exec cargo run
