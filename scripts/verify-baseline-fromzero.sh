#!/bin/bash
# verify-baseline-fromzero.sh — multi-directory's own proof that src/migrations/ IS an install path.
#
# WHY THIS EXISTS (kanban t_979f881e, measured 2026-10-02)
#   An EMPTY database boot of this app applied 34 of 124 files and FAILED 90; `businesses` was never
#   created, and /api/v1/health still answered 200 (the migrator is deliberately non-fatal,
#   src/db.rs). The live schema had been built out of band and the delta chain layered on top, so
#   nothing had ever asked the chain to build from nothing. Evidence:
#   /opt/swift/audits/t_cb86753e/40-fresh-install-finding.txt.
#   The fix is src/migrations/000_baseline_live_schema.sql (generated from live by
#   scripts/gen-baseline-from-live.py); the superseded chain moved to src/migrations-legacy/.
#   This script is the check that keeps it true - and it is deliberately SELF-CONTAINED (it needs
#   only psql + pg_dump + the app's own DATABASE_URL), because a buyer installing this app does not
#   have /opt/swift/fleet.
#
# VERBS
#   zero       build the schema from an EMPTY database with the app's own migration files, in the
#              order the app's runner applies them, then diff every public object class against live.
#              exit 0 PASS | 3 APPLY-FAIL (a fresh install cannot boot) | 4 DRIFT (build != live)
#   live-noop  restore a schema-only copy of live, apply the migration files to it, and prove the
#              catalog is byte-identical before and after (i.e. the deploy is a no-op on live).
#              exit 0 PASS | 3 NOT-A-NO-OP
#   2 / anything else = the check itself could not run. Never silent: a dump query that errors ABORTS
#   rather than reporting a vacuously empty object class.
#
# USAGE
#   scripts/verify-baseline-fromzero.sh [zero|live-noop] [outdir]
#   MD_MIGRATIONS_DIR overrides the directory judged (default <repo>/src/migrations) — use it to judge
#   the baseline alone while a sibling lane has an unpublished file in the live dir.
#   DATABASE_URL is taken from $MD_LIVE_URL, else from $MD_ENV (default
#   /etc/swift/env/multi-directory.env). Never echoed, never parsed.
set -u

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="${MD_MIGRATIONS_DIR:-$APP_DIR/src/migrations}"   # override to judge ONE file (or a candidate set) in isolation
VERB="${1:-zero}"
OUT="${2:-/tmp/md-fromzero-$VERB}"
PSQL="psql -X -q"

if [ -z "${MD_LIVE_URL:-}" ]; then
  ENVF="${MD_ENV:-/etc/swift/env/multi-directory.env}"
  [ -f "$ENVF" ] || { echo "ABORT: no live DATABASE_URL (\$MD_LIVE_URL unset and $ENVF missing)"; echo "RESULT: ABORT"; exit 2; }
  set -a; . "$ENVF"; set +a
fi
[ -n "${DATABASE_URL:-}" ] || { echo "ABORT: $ENVF sets no DATABASE_URL"; echo "RESULT: ABORT"; exit 2; }
BASEURL="${DATABASE_URL%/*}"                 # postgres://user:***@host:port
LIVE="${DATABASE_URL##*/}"; LIVE="${LIVE%%\?*}"
url() { printf '%s/%s' "$BASEURL" "$1"; }
SCRATCH="md_fromzero_${VERB//-/_}"
mkdir -p "$OUT"

trap '$PSQL "$(url postgres)" -c "DROP DATABASE IF EXISTS '"$SCRATCH"'" >/dev/null 2>&1' EXIT

# The order the app's own runner uses: read_dir(./src/migrations) + sort() = byte order.
FILELIST="$OUT/filelist.txt"
ls "$DIR"/*.sql 2>/dev/null | xargs -n1 basename | sort > "$FILELIST"
files=$(grep -c . "$FILELIST" || true)
[ "$files" -gt 0 ] || { echo "ABORT: no .sql files in $DIR"; echo "RESULT: ABORT"; exit 2; }

# The runner's own ledger, created at boot BEFORE any file runs (src/db.rs::run_migrations).
seed_ledger() { # $1 = db
  $PSQL "$(url "$1")" -c "CREATE TABLE IF NOT EXISTS _migrations (
        id SERIAL PRIMARY KEY,
        filename VARCHAR(255) NOT NULL UNIQUE,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW())" >/dev/null 2>&1 || return 1
  $PSQL "$(url "$1")" -c "ALTER TABLE _migrations ADD COLUMN IF NOT EXISTS had_errors BOOLEAN NOT NULL DEFAULT false" >/dev/null 2>&1 || return 1
}

# A dump query that ERRORS ABORTS: an empty dump file would read as missing=0 extra=0 on both sides,
# i.e. a silently VACUOUS object class (the blindness the fleet harness calls out). Never a pass.
q() { # $1 = db, $2 = sql, $3 = dest
  local out rc
  out=$($PSQL "$(url "$1")" -tAc "$2" 2>&1); rc=$?
  if [ $rc -ne 0 ] || printf '%s' "$out" | grep -q '^ERROR'; then
    printf 'ABORT: schema dump query failed (db=%s dest=%s): %s\n' "$1" "$3" "$(printf '%s' "$out" | head -2 | tr '\n' ' ')" >&2
    echo "RESULT: ABORT"; exit 2
  fi
  printf '%s\n' "$out" > "$3"
}

fingerprint() { # $1 = db, $2 = prefix
  local d=$1 p=$2
  q "$d" "select 'T '||tablename from pg_tables where schemaname='public' order by 1" "$p.tables"
  q "$d" "select 'C '||table_name||'.'||column_name||' '||data_type||' nullable='||is_nullable from information_schema.columns where table_schema='public' order by 1" "$p.columns"
  # Constraint identity is compared on (table, name, type, validated) + a PREDICATE CANONICALISATION.
  # The canon strips only `::text` / `::character varying` casts and parentheses and collapses
  # whitespace, and it is applied to BOTH sides. It exists because PostgreSQL NORMALISES an
  # array-cast CHECK predicate at parse time: a constraint created today whose CHECK was written
  # `= ANY ((ARRAY['a'::character varying,...])::text[])` is now stored/rendered as
  # `= ANY (ARRAY[('a'::character varying)::text,...])` (measured on the live server, PG 16.14). That
  # is a version-induced RENDERING difference with identical semantics — any pg_restore of live into
  # this server produces it too — so it must not be reported as drift. A real difference (another
  # column, operator, literal set, or an action such as ON DELETE SET NULL) survives the canon.
  q "$d" "select 'K '||conrelid::regclass||' '||conname||' '||contype::text||' validated='||convalidated||' '||regexp_replace(regexp_replace(regexp_replace(pg_get_constraintdef(oid), '::(text|character varying|varchar)(\[\])?', '', 'g'), '[()]', '', 'g'), '\s+', ' ', 'g') from pg_constraint where connamespace='public'::regnamespace order by 1" "$p.constraints"
  q "$d" "select 'I '||i.relname||' '||(case when x.indisunique then 'UNIQUE ' else '' end)||pg_get_indexdef(x.indexrelid) from pg_index x join pg_class i on i.oid=x.indexrelid join pg_class t on t.oid=x.indrelid join pg_namespace n on n.oid=t.relnamespace where n.nspname='public' order by 1" "$p.indexes"
  q "$d" "select 'V '||viewname from pg_views where schemaname='public' order by 1" "$p.views"
  q "$d" "select 'S '||sequence_name from information_schema.sequences where sequence_schema='public' order by 1" "$p.seqs"
  q "$d" "select 'R '||routine_name||'('||pg_get_function_identity_arguments(p.oid)||')' from information_schema.routines r join pg_proc p on p.proname=r.routine_name and p.pronamespace='public'::regnamespace where routine_schema='public' order by 1" "$p.routines"
  q "$d" "select 'G '||event_object_table||':'||trigger_name||':'||action_timing from information_schema.triggers where trigger_schema='public' order by 1" "$p.triggers"
}

apply_files() { # $1 = db, $2 = logfile -> prints "<pass> <fail>" as its only stdout line
  local db=$1 log=$2 pass=0 fail=0 f out rc err
  : > "$log"
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    out=$($PSQL "$(url "$db")" -v ON_ERROR_STOP=1 -1 -f "$DIR/$f" 2>&1); rc=$?
    if [ $rc -eq 0 ]; then
      pass=$((pass+1))
      $PSQL "$(url "$db")" -c "INSERT INTO _migrations (filename, had_errors) VALUES ('$f', false) ON CONFLICT (filename) DO UPDATE SET had_errors=false" >/dev/null 2>&1
    else
      fail=$((fail+1))
      err=$(printf '%s' "$out" | grep -E '^psql:.*ERROR|^ERROR' | head -1)
      printf 'FAIL %s :: %s\n' "$f" "$err" >&2     # visibility on stderr, never on the stdout contract
      printf 'FAIL %s :: %s\n' "$f" "$err" >> "$log"
    fi
  done < "$FILELIST"
  echo "$pass $fail"
}

diff_classes() { # $1 $2 = prefixes -> sets DRIFT
  local a=$1 b=$2 kind m e nm ne
  DRIFT=0
  : > "$OUT/diff.txt"
  for kind in tables columns constraints indexes views seqs routines triggers; do
    m=$(comm -13 "$a.$kind" "$b.$kind"); e=$(comm -23 "$a.$kind" "$b.$kind")
    nm=$(printf '%s' "$m" | grep -c . || true); ne=$(printf '%s' "$e" | grep -c . || true)
    echo "CLASS $kind missing=$nm extra=$ne"
    { [ "$nm" -gt 0 ] && printf '%s\n' "$m" | head -20 | sed 's/^/  MISSING /'; [ "$ne" -gt 0 ] && printf '%s\n' "$e" | head -20 | sed 's/^/  EXTRA   /'; } >> "$OUT/diff.txt"
    DRIFT=$((DRIFT+nm+ne))
  done
}

case "$VERB" in
zero)
  $PSQL "$(url postgres)" -c "DROP DATABASE IF EXISTS $SCRATCH" >/dev/null 2>&1
  $PSQL "$(url postgres)" -c "CREATE DATABASE $SCRATCH" >/dev/null 2>&1 || { echo "ABORT: cannot create $SCRATCH"; echo "RESULT: ABORT"; exit 2; }
  seed_ledger "$SCRATCH" || { echo "ABORT: cannot seed _migrations"; echo "RESULT: ABORT"; exit 2; }
  read -r pass fail <<<"$(apply_files "$SCRATCH" "$OUT/fromzero.log")"
  echo "APPLIED files=$pass/$files failed=$fail"
  [ "$fail" -eq 0 ] || { echo "RESULT: APPLY-FAIL — $fail file(s) did not apply; a fresh install/restore CANNOT boot"; exit 3; }
  fingerprint "$SCRATCH" "$OUT/zero"
  fingerprint "$LIVE" "$OUT/live"
  diff_classes "$OUT/zero" "$OUT/live"
  if [ "$DRIFT" -gt 0 ]; then
    echo "RESULT: DRIFT — every file applied but $DRIFT object(s) differ from live (see $OUT/diff.txt)"; exit 4
  fi
  t=$(grep -c '^T ' "$OUT/live.tables")
  echo "RESULT: PASS — src/migrations/ reproduces the live schema from an EMPTY database ($pass files, $t tables, 0 missing/extra in every class)"
  exit 0 ;;
live-noop)
  $PSQL "$(url postgres)" -c "DROP DATABASE IF EXISTS $SCRATCH" >/dev/null 2>&1
  $PSQL "$(url postgres)" -c "CREATE DATABASE $SCRATCH" >/dev/null 2>&1 || { echo "ABORT: cannot create $SCRATCH"; echo "RESULT: ABORT"; exit 2; }
  pg_dump "$(url "$LIVE")" --schema-only --no-owner --no-privileges | $PSQL "$(url "$SCRATCH")" >/dev/null 2>"$OUT/restore.err" \
    || { echo "ABORT: cannot restore live into $SCRATCH (see $OUT/restore.err)"; echo "RESULT: ABORT"; exit 2; }
  fingerprint "$SCRATCH" "$OUT/before"
  read -r pass fail <<<"$(apply_files "$SCRATCH" "$OUT/live-noop.log")"
  echo "APPLIED files=$pass/$files failed=$fail"
  [ "$fail" -eq 0 ] || { echo "RESULT: NOT-A-NO-OP — $fail file(s) errored on a database that already has the schema"; exit 3; }
  fingerprint "$SCRATCH" "$OUT/after"
  diff_classes "$OUT/before" "$OUT/after"
  if [ "$DRIFT" -gt 0 ]; then
    echo "RESULT: NOT-A-NO-OP — applying the migration set CHANGED $DRIFT object(s) on a copy of live (see $OUT/diff.txt)"; exit 3
  fi
  echo "RESULT: PASS — the migration set is a no-op on a copy of live (catalog byte-identical before/after, $pass files)"
  exit 0 ;;
*) echo "usage: ${0##*/} [zero|live-noop] [outdir]"; exit 2 ;;
esac
