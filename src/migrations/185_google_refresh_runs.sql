-- W25 — REFRESH FROM GOOGLE: one row per batched run, so the admin panel can say what the last
-- refresh did (added / updated / unchanged / unreachable) instead of showing a spinner.
--
-- The run is per DIRECTORY (a city). `trigger` is 'manual' when an admin pressed the button and
-- 'scheduled' when the cron runner started it, so an operator can tell the two apart afterwards.
-- `details` keeps a bounded sample of the rows the run touched (never the whole batch) — enough
-- to explain a surprising number in the panel without turning this table into a log.

CREATE TABLE IF NOT EXISTS google_refresh_runs (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id     uuid NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    trigger          text NOT NULL DEFAULT 'manual',
    update_arm       boolean NOT NULL DEFAULT true,
    discover_arm     boolean NOT NULL DEFAULT true,
    batch_updates    integer NOT NULL DEFAULT 0,
    batch_adds       integer NOT NULL DEFAULT 0,
    added            integer NOT NULL DEFAULT 0,
    updated          integer NOT NULL DEFAULT 0,
    unchanged        integer NOT NULL DEFAULT 0,
    unreachable      integer NOT NULL DEFAULT 0,
    skipped_claimed  integer NOT NULL DEFAULT 0,
    details          jsonb NOT NULL DEFAULT '{}'::jsonb,
    error            text,
    started_at       timestamptz NOT NULL DEFAULT now(),
    finished_at      timestamptz
);

CREATE INDEX IF NOT EXISTS idx_google_refresh_runs_dir_started
    ON google_refresh_runs (directory_id, started_at DESC);
