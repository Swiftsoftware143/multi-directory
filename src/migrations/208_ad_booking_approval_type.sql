-- 208_ad_booking_approval_type.sql — approve/reject a WHOLE ad booking through the same
-- approval_queue the other monetization items use. `create_booking` enqueues item_type
-- 'ad_booking', but the CHECK constraint only admitted sponsor / ad_creative / ad_schedule /
-- featured_listing / subscription, so the enqueue 500'd. Widen the enum to admit it.
--
-- Idempotent: DROP IF EXISTS then ADD (the boot runner skips already-applied files, but a
-- fresh install layers this over the baseline, and a re-run of the file is still safe).

ALTER TABLE public.approval_queue DROP CONSTRAINT IF EXISTS approval_queue_item_type_check;

ALTER TABLE public.approval_queue ADD CONSTRAINT approval_queue_item_type_check
    CHECK (item_type = ANY (ARRAY[
        'sponsor'::text, 'ad_creative'::text, 'ad_schedule'::text,
        'featured_listing'::text, 'subscription'::text, 'ad_booking'::text
    ]));
