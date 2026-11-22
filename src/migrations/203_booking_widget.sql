-- 203_booking_widget.sql
--
-- Ecosystem widgets, increment 1 of 2 (work-queue "ECOSYSTEM WIDGETS", David's priority
-- "before loyalty, after market-ready"). Per business, a CoreSwift booking calendar so a
-- visitor books INSIDE the directory instead of being hyperlinked off to the business site.
-- The booking backend already exists (`/book/:slug/:business_id` renders CoreSwift slots,
-- `bookings.rs` handles create/available/list); this column is the per-business switch the
-- operator flips in the admin panel. A business with the flag off renders NO widget and no
-- empty box. (The IncentiveSwift campaign widget is a separate, later increment — there is no
-- campaign code yet.)
--
-- Idempotent: guarded by IF NOT EXISTS so a re-run / re-boot applies nothing twice.

ALTER TABLE businesses ADD COLUMN IF NOT EXISTS booking_widget_enabled boolean NOT NULL DEFAULT false;
