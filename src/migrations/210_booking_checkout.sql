-- 210_booking_checkout.sql
--
-- Booking checkout embed (work-queue "BOOKING CHECKOUT", David 2026-11-24): a business owner
-- can attach a checkout form — an iframe URL or an embed snippet from their own payment tool —
-- that renders INLINE on the listing after the booking calendar, with a smooth reveal, instead
-- of a checkout LINK that bounces the visitor off the directory.
--
-- Two columns:
--   booking_checkout_url    — the URL or embed snippet (stored verbatim; the frontend interprets
--                             it safely). NULL = no checkout. An empty string in a write CLEARS it.
--   booking_charge_timing   — WHEN the checkout surfaces: 'after' (pay once the booking is
--                             confirmed — the default) or 'at_booking' (pay up-front to book,
--                             consultant-style). Mirrors David's "consultant charges up-front,
--                             newsletter gets paid afterwards — put it in the proper place".
--
-- Idempotent: guarded by IF NOT EXISTS so a re-run / re-boot applies nothing twice.

ALTER TABLE businesses ADD COLUMN IF NOT EXISTS booking_checkout_url text;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS booking_charge_timing text NOT NULL DEFAULT 'after';

COMMENT ON COLUMN businesses.booking_checkout_url IS
    'Booking checkout embed: an iframe URL or embed snippet the owner pastes so a visitor pays inline after the booking calendar. NULL = none; empty string in a write clears it.';
COMMENT ON COLUMN businesses.booking_charge_timing IS
    'When the booking checkout surfaces: ''after'' (pay once the booking is confirmed) or ''at_booking'' (pay up-front to book). Default ''after''.';
