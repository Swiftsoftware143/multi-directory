-- Walk Score / Bike Score integration.
--
-- 1) Register the provider so the Integration Center renders it from data (the admin panel
--    builds its provider list from `available_providers`). The credential itself is stored in
--    `provider_keys` through the normal encrypted write path, at NETWORK or DIRECTORY scope.
-- 2) Cache scores per business.
--
-- Cache-key choice: `business_id`, not `(kind, id)`. A city-page card is a `business_listings`
-- row, but every listing carries `business_id`, and the public detail endpoint reads the same
-- physical listing through either table. Keying the cache by the business means ONE row and ONE
-- API call per physical listing instead of double-fetching the mirror rows. A listing with no
-- linked business simply has nothing cached and renders nothing.

INSERT INTO available_providers
    (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only)
VALUES
    ('walkscore', 'Walk Score', 'Walk Score and Bike Score for each listing address (refreshed quarterly)',
     false, '[]', '🚶', NULL, NULL, false)
ON CONFLICT (key) DO NOTHING;

CREATE TABLE IF NOT EXISTS business_walk_scores (
    business_id     uuid PRIMARY KEY REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id    uuid NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    walk_score      integer,
    bike_score      integer,
    -- Written on EVERY attempt (success or failure) so a vendor outage is retried next
    -- quarter, not every hour. Scores survive a failed refresh: only a success overwrites them.
    last_attempt_at timestamptz NOT NULL DEFAULT now(),
    -- Set only when the vendor returned a usable status=1 payload.
    refreshed_at    timestamptz,
    CONSTRAINT business_walk_scores_range CHECK (
        (walk_score IS NULL OR (walk_score BETWEEN 0 AND 100))
        AND (bike_score IS NULL OR (bike_score BETWEEN 0 AND 100))
    )
);

CREATE INDEX IF NOT EXISTS idx_business_walk_scores_due
    ON business_walk_scores (last_attempt_at);
