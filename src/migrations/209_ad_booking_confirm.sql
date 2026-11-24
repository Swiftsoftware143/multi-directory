-- 209_ad_booking_confirm.sql — the ad_fulfillment fan-out fires at CONFIRMATION, not activation.
--
-- The artwork chase keys off `confirmed_at` (the confirmation email goes out at confirmation, so
-- the first artwork reminder must follow THAT moment, not a later schedule activation). This adds
-- the two columns the confirmation write needs:
--
--   * `confirmed_at`  — stamped when the sale is confirmed; the CEO's artwork chase keys off it.
--   * `core_booking_id` — NULLABLE soft reference to CoreSwift's `booking_bookings` (a separate
--     Postgres database, so a real FK is impossible). Stored here and validated over CoreSwift's
--     API at write time; it WRAPS the real booking, never replaces the local parent.
--
-- It also admits `'confirmed'` to both status enums so the booking/schedules can sit confirmed
-- (rows written) before they are separately activated (publishing recorded, no new rows).
--
-- Idempotent: every statement uses IF NOT EXISTS / DROP-IF-EXISTS-then-ADD guards, so the
-- re-run-on-boot runner applies it safely on a live database.

ALTER TABLE public.ad_bookings ADD COLUMN IF NOT EXISTS confirmed_at timestamp with time zone;

ALTER TABLE public.ad_bookings ADD COLUMN IF NOT EXISTS core_booking_id uuid;

ALTER TABLE public.ad_bookings DROP CONSTRAINT IF EXISTS ad_bookings_status_check;
ALTER TABLE public.ad_bookings ADD CONSTRAINT ad_bookings_status_check
    CHECK (status = ANY (ARRAY[
        'pending'::text, 'confirmed'::text, 'active'::text,
        'completed'::text, 'cancelled'::text
    ]));

ALTER TABLE public.ad_schedules DROP CONSTRAINT IF EXISTS ad_schedules_status_check;
ALTER TABLE public.ad_schedules ADD CONSTRAINT ad_schedules_status_check
    CHECK (status = ANY (ARRAY[
        'pending'::text, 'confirmed'::text, 'active'::text,
        'completed'::text, 'cancelled'::text
    ]));
