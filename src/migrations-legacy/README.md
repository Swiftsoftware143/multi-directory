# src/migrations-legacy/ — the superseded delta history (NOT an install path)

These 124 `.sql` files (plus `011_api_keys.json`, an unreferenced data file that sat beside 011) were
in `src/migrations/` and are kept only as the record of what was written. They are **not applied by
any code path** — the runner only scans `src/migrations/` — and must not be moved back or renumbered.

## Why they are here (kanban t_979f881e, measured 2026-10-02)

They are a delta history written against a database that was built out of band (a dump/skeleton), and
they were layered on top of it. All 124 were already recorded in live's `_migrations` ledger with
`had_errors = false`, so the chain had never once been asked to build a schema from nothing. Asked
now, on an EMPTY database, it cannot:

* **90 of the 124 files fail.** `001_initial.sql` creates only `tenants`, `users`, `password_resets`
  and `_migrations`; `002_templates_and_colors.sql` then does `ALTER TABLE directories …` and
  `CREATE TABLE business_meta (… REFERENCES businesses(id))`. Neither `directories` nor `businesses`
  exists at that point — the core baseline is not in the chain at all — so the chain collapses there
  and everything downstream fails. `businesses` is only authored much later
  (`068_schema_reconciliation.sql`), which itself fails because the chain has already collapsed by
  the time it is reached.
* **A fresh install still answers `/api/v1/health` 200**, because the migrator is deliberately
  non-fatal (`src/db.rs`) — the install runs but is hollow: `businesses`, `claimed_businesses`,
  `tenant_users` and `email_templates` are missing, and every handler that queries them 500s or
  silently no-ops.
* Census and boot log: `/opt/swift/audits/t_cb86753e/40-fresh-install-finding.txt`.
* The live database's 4,000 businesses are discovery-seeded LISTINGS; nothing about the live data
  depends on these files being re-run.

## What replaced them

`src/migrations/000_baseline_live_schema.sql` — the live catalog generated from the live database by
`scripts/gen-baseline-from-live.py`, applied by the runner at boot. It contains everything these
files were reaching for. See `src/migrations/README.md`.

## Don't re-run these

They are not idempotent (bare `ALTER TABLE … ADD COLUMN`, unguarded `ADD CONSTRAINT`, `CREATE TABLE`
without `IF NOT EXISTS`), so re-applying them to a database that already has the schema would fail on
almost every file. They are history, not a repair tool.
