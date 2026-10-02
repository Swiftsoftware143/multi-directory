-- Migration 088: demand-analytics settings (T1).
--
-- "Nothing hardwired": the date-window choices, the default window, the time
-- bucket, how many top categories a rollup shows and the export row cap are all
-- data. A row with directory_id IS NULL is the global default; a directory may
-- override it. The admin Demand Analytics card edits this row.
--
-- Categories are NOT listed here on purpose — they are read from
-- directory_categories (the DB) at request time, so a new category shows up in the
-- filter without touching code.

CREATE TABLE IF NOT EXISTS demand_analytics_settings (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id        uuid,
    window_options_days integer[] NOT NULL DEFAULT '{7,30,90,180,365}',
    default_window_days integer NOT NULL DEFAULT 90,
    bucket_size         varchar(24) NOT NULL DEFAULT 'hour',
    top_categories      integer NOT NULL DEFAULT 5,
    export_row_limit    integer NOT NULL DEFAULT 5000,
    rollup_modes        text[] NOT NULL DEFAULT ARRAY['matrix', 'rollups', 'supply'],
    updated_at          timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT demand_analytics_settings_bucket_check
        CHECK (bucket_size IN ('hour', 'day_of_week', 'day', 'week')),
    CONSTRAINT demand_analytics_settings_window_check
        CHECK (default_window_days > 0 AND top_categories > 0 AND export_row_limit > 0)
);

CREATE INDEX IF NOT EXISTS idx_demand_analytics_settings_directory_id
    ON demand_analytics_settings(directory_id);

-- Seed the global default row exactly once.
INSERT INTO demand_analytics_settings (directory_id)
SELECT NULL
WHERE NOT EXISTS (SELECT 1 FROM demand_analytics_settings WHERE directory_id IS NULL);
