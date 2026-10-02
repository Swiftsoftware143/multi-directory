# src/migrations/ — the schema install path

This directory IS how this application's database gets its schema. Booting the app against an empty
database builds the whole live catalog from it, and `scripts/verify-baseline-fromzero.sh` proves that
in ~40 seconds (it needs only `psql`/`pg_dump` and the app's own `DATABASE_URL` — deliberately no
fleet tooling, because a buyer installing this app does not have `/opt/swift`).

## How it runs

* **Runner:** `src/db.rs::run_migrations`, called from `src/main.rs` at boot, before the email worker.
  It `read_dir`s this directory and sorts the `.sql` entries, so the order is **byte order of the
  filename** — that is why every file is zero-padded (`000_`, `001_`, …).
* **Ledger:** `_migrations (id, filename, applied_at, had_errors)`, created at boot. A file recorded
  with `had_errors = false` is skipped. A file that is missing from the ledger, or was recorded with
  `had_errors = true`, is **re-applied on every boot until it is clean**.
* **Statements:** the file is split into statements (dollar-quote/comment aware, `src/db.rs::
  split_statements`) and each is executed separately. A failing statement is logged and the rest of
  the file still runs; the file is then recorded `had_errors = true` and retried next boot.
* **Failure is NON-FATAL:** the migrator logs a warning and the server starts anyway. So a broken
  migration set is invisible to `/api/v1/health` — a hollow install answers 200. That is exactly the
  defect the from-zero check below exists to catch.

## The files

| file | what it is |
|---|---|
| `000_baseline_live_schema.sql` | the live catalog — every table, column, constraint, index, sequence, view, function, trigger and column comment — generated from the live database. A fresh install runs this file first and every later, higher-numbered file on top of it. |

## Rules for a new change

1. **Never edit `000_baseline_live_schema.sql`** and never edit a file that has already been applied.
   Live records it as applied (`had_errors = false`), so a future edit to it would run on FRESH
   installs but never on live — the two would silently diverge. A change is a NEW file.
2. **Naming:** `<NNN>_<description>.sql`, zero-padded, ascending. Files are applied in byte order.
3. **Add-only-when-absent.** Every new file must be safe to apply to a database that already has some
   of what it creates and may be re-run after a partial failure: `CREATE TABLE IF NOT EXISTS`,
   `CREATE INDEX IF NOT EXISTS`, `ALTER TABLE … ADD COLUMN IF NOT EXISTS`, `CREATE OR REPLACE
   FUNCTION/VIEW`, and for constraints the guarded `DO` block used throughout the baseline
   (`IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = … AND conrelid = …::regclass) THEN
   ALTER TABLE … ADD CONSTRAINT …; END IF;`).
4. **Before you push:** `bash scripts/verify-baseline-fromzero.sh zero` must print `RESULT: PASS`, and
   `… verify-baseline-fromzero.sh live-noop` must print `RESULT: PASS`. If the first reports
   APPLY-FAIL the next fresh install or restore cannot boot; if the second fails, the deploy would
   alter the live catalog.

## Provenance of the baseline

`000_baseline_live_schema.sql` was generated from the live database by
`scripts/gen-baseline-from-live.py` (kanban t_979f881e). Two properties are measured, not asserted
(evidence: `/opt/swift/audits/t_979f881e/`):

* **it is a complete install path from nothing** — applied to an EMPTY database it builds exactly the
  live catalog (0 missing / 0 extra in every object class);
* **it is a no-op on the database it came from** — applied to a schema-only copy of live the catalog
  is byte-identical before and after, and live was then re-booted for real with the file in place.

`src/migrations-legacy/` holds the superseded delta history (the 124 files that could never be an
install path), with its own README.
