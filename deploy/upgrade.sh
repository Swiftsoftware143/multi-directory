#!/usr/bin/env bash
# Multi-Directory — apply an update on the operator's own server (kanban B85).
#
# Pulls the code (if this is a git checkout), rebuilds the release binary, applies any new
# migrations, and restarts the service. Safe to re-run; safe to run when nothing changed.
#
#   sudo bash deploy/upgrade.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

BIN="$APP_DIR/target/release/multidirectory"
DB_URL=""
SERVICE="multidirectory"
PULL=1
BUILD=1
RESTART=1
DRY_RUN=0

usage() {
  cat <<EOF
USAGE: bash deploy/upgrade.sh [options]
  --app-dir DIR     checkout to update (default: ${APP_DIR})
  --binary PATH     release binary (default: <app>/target/release/multidirectory)
  --database-url U  defaults to DATABASE_URL, then <app>/.env
  --service NAME    systemd unit to restart (default: ${SERVICE})
  --no-pull         skip git pull
  --no-build        skip the cargo rebuild
  --no-restart      skip the service restart
  --dry-run         print the plan, change nothing
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --app-dir) APP_DIR="${2:?}"; shift 2 ;;
    --binary) BIN="${2:?}"; shift 2 ;;
    --database-url) DB_URL="${2:?}"; shift 2 ;;
    --service) SERVICE="${2:?}"; shift 2 ;;
    --no-pull) PULL=0; shift ;;
    --no-build) BUILD=0; shift ;;
    --no-restart) RESTART=0; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage; exit 2 ;;
  esac
done

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
run() { if [ "$DRY_RUN" = "1" ]; then echo "    dry-run: $*"; else (cd "$APP_DIR" && eval "$*"); fi; }

if [ -z "$DB_URL" ]; then DB_URL="${DATABASE_URL:-}"; fi
if [ -z "$DB_URL" ] && [ -f "$APP_DIR/.env" ]; then
  DB_URL="$(grep -E '^DATABASE_URL=' "$APP_DIR/.env" | head -1 | cut -d= -f2- || true)"
fi

if [ "$PULL" = "1" ] && [ -d "$APP_DIR/.git" ]; then
  say "pulling the latest code"
  run "git pull --ff-only"
fi

if [ "$BUILD" = "1" ]; then
  say "building the release binary"
  run "cargo build --release --bin multidirectory"
fi

if [ -n "$DB_URL" ]; then
  say "applying migrations"
  run "\"$BIN\" migrate --database-url \"$DB_URL\""
else
  say "no DATABASE_URL found — skipping the migration step"
fi

if [ "$RESTART" = "1" ] && command -v systemctl >/dev/null 2>&1; then
  say "restarting $SERVICE"
  run "systemctl restart $SERVICE || true"
fi

say "done"
