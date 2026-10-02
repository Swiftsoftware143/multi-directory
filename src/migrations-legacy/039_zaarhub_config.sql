-- ZaarHub network visibility & homepage config per directory
-- Extends feature_config with zaarhub-specific toggles

ALTER TABLE directories ADD COLUMN IF NOT EXISTS zaarhub_config JSONB DEFAULT '{
  "network_visible": true,
  "show_deals": true,
  "show_events": true,
  "show_reviews": true,
  "show_activity": true,
  "homepage_featured": false,
  "homepage_hero_title": null,
  "homepage_hero_subtitle": null,
  "featured_image_url": null
}'::jsonb;

-- Add featured flag to deals for admin selection
ALTER TABLE deals ADD COLUMN IF NOT EXISTS zaarhub_featured BOOLEAN DEFAULT false;

-- Add featured flag to events for admin selection
ALTER TABLE community_events ADD COLUMN IF NOT EXISTS zaarhub_featured BOOLEAN DEFAULT false;

CREATE INDEX IF NOT EXISTS idx_deals_zaarhub_featured ON deals(zaarhub_featured) WHERE zaarhub_featured = true;
CREATE INDEX IF NOT EXISTS idx_events_zaarhub_featured ON community_events(zaarhub_featured) WHERE zaarhub_featured = true;
