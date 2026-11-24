-- 207_ad_bookings.sql — a booking groups N schedule positions (one schedule per ad zone),
-- so a Full Takeover is ONE booking with N positions. Adds the parent table and the
-- `ad_schedules.booking_id` foreign key; the writer (src/ad_fulfillment.rs) fans a booking's
-- positions out to CoreSwift's `ad_fulfillment` object on activation.
--
-- Idempotent: every statement uses IF NOT EXISTS / DO-block guards, so the re-run-on-boot
-- runner (src/db.rs::run_migrations) applies it safely on a live database.

CREATE TABLE IF NOT EXISTS public.ad_bookings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    sponsor_id uuid NOT NULL,
    created_by uuid,
    created_at timestamp with time zone DEFAULT now(),
    status text DEFAULT 'pending'::text NOT NULL,
    external_payment_ref text,
    CONSTRAINT ad_bookings_pkey PRIMARY KEY (id),
    CONSTRAINT ad_bookings_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'active'::text, 'completed'::text, 'cancelled'::text])))
);

-- ad_schedules.booking_id: a schedule that belongs to a booking points at its parent.
-- NULLABLE so a schedule created before this change (none exist — the table is empty) stays valid.
ALTER TABLE public.ad_schedules ADD COLUMN IF NOT EXISTS booking_id uuid;

DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'ad_schedules_booking_id_fkey'
           AND conrelid = 'public.ad_schedules'::regclass
    ) THEN
        ALTER TABLE public.ad_schedules
            ADD CONSTRAINT ad_schedules_booking_id_fkey
            FOREIGN KEY (booking_id) REFERENCES public.ad_bookings(id);
    END IF;
END
$md_bl$;

CREATE INDEX IF NOT EXISTS idx_ad_schedules_booking_id ON public.ad_schedules(booking_id);
