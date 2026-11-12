-- W25 follow-up — `identified`: a refresh that only STAMPS a Google place id changes nothing a
-- visitor can see, so counting it as "updated" would tell the admin their listings changed when
-- they did not. Keep the visible change and the identity stamp as separate numbers.

ALTER TABLE google_refresh_runs
    ADD COLUMN IF NOT EXISTS identified integer NOT NULL DEFAULT 0;
