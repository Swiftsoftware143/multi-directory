-- 177_webhook_dispatch.sql
--
-- WEBHOOKS THAT ACTUALLY FIRE.
--
-- The admin "API Access & Webhooks" screen lets an operator register a URL and tick the events
-- they want. The dispatcher (`dispatch_webhook_event`) had a full retry / signature / delivery-log
-- implementation, but nothing ever called it for the events an operator picks: the platform's own
-- changes (a listing created by an import, a review approved by an owner, a directory edited) are
-- recorded in `directory_events` by a database trigger and are never dispatched. So a subscriber
-- received nothing — the screen configured a black hole.
--
-- One delivery path now covers every recorded event: the app drains `directory_events` in the
-- background and delivers each row to the webhooks subscribed to that event type.
--
--  1. `webhook_dispatched_at` marks a row as delivered (or deliberately skipped), so a restart or a
--     slow receiver can never double-fire an event.
--  2. HISTORY IS NOT REPLAYED: every row that exists today is stamped as dispatched. Registering a
--     webhook must not dump a year of old events on the subscriber.
--  3. Reviews were not recorded at all. This migration adds a trigger so a review's creation,
--     approval and rejection become events like every other change.
--  4. `auto_record_directory_event()` only recorded events for a HARDCODED list of ZaarHub city
--     slugs, so a buyer's own directory emitted nothing. The filter is gone.

-- ── 1. delivery bookkeeping ────────────────────────────────────────────────────────────────────
ALTER TABLE directory_events ADD COLUMN IF NOT EXISTS webhook_dispatched_at timestamptz;

-- Everything already in the table predates the subscription — never replay it.
UPDATE directory_events
   SET webhook_dispatched_at = NOW()
 WHERE webhook_dispatched_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_directory_events_undispatched
    ON directory_events (created_at)
 WHERE webhook_dispatched_at IS NULL;

-- ── 2. reviews become events ───────────────────────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION record_review_event() RETURNS trigger
LANGUAGE plpgsql AS $$
    DECLARE
        v_event text;
        v_dir   uuid;
    BEGIN
        v_dir := COALESCE(
            NEW.directory_id,
            (SELECT directory_id FROM businesses WHERE id = NEW.business_id)
        );

        IF TG_OP = 'INSERT' THEN
            v_event := 'reviews.created';
        ELSE
            -- Only a real status transition is an event; every other UPDATE (a reply being saved,
            -- a featured flag) must not wake a subscriber.
            IF NEW.status IS DISTINCT FROM OLD.status AND NEW.status = 'approved' THEN
                v_event := 'reviews.approved';
            ELSIF NEW.status IS DISTINCT FROM OLD.status AND NEW.status = 'rejected' THEN
                v_event := 'reviews.rejected';
            ELSE
                RETURN NEW;
            END IF;
        END IF;

        INSERT INTO directory_events (event_type, entity_type, entity_id, directory_id, data)
        VALUES (
            v_event,
            'review',
            NEW.id,
            v_dir,
            jsonb_build_object(
                'business_id', NEW.business_id,
                'rating',      NEW.rating,
                'status',      NEW.status,
                'title',       NEW.title
            )
        );
        RETURN NEW;
    END;
$$;

DROP TRIGGER IF EXISTS trg_record_review_event ON reviews;
CREATE TRIGGER trg_record_review_event
    AFTER INSERT OR UPDATE ON reviews
    FOR EACH ROW EXECUTE FUNCTION record_review_event();

-- ── 3. no hardcoded city list ─────────────────────────────────────────────────────────────────
-- Same function as before, with the `slug IN ('palm-bay', …)` guard removed: a directory that is
-- not one of ZaarHub's ten towns still records its events.
CREATE OR REPLACE FUNCTION auto_record_directory_event() RETURNS trigger
LANGUAGE plpgsql AS $$
    DECLARE
        v_event_type text;
        v_entity_type text;
        v_entity_id uuid;
        v_directory_id uuid;
        v_data jsonb;
    BEGIN
        IF TG_OP = 'INSERT' THEN
            v_event_type := TG_TABLE_NAME || '.created';
            v_entity_id := NEW.id;
        ELSIF TG_OP = 'UPDATE' THEN
            v_event_type := TG_TABLE_NAME || '.updated';
            v_entity_id := NEW.id;
        ELSIF TG_OP = 'DELETE' THEN
            v_event_type := TG_TABLE_NAME || '.deleted';
            v_entity_id := OLD.id;
        END IF;

        IF TG_TABLE_NAME = 'directories' THEN
            v_entity_type := 'directory';
            v_directory_id := v_entity_id;
            v_data := jsonb_build_object('slug', NEW.slug, 'name', NEW.name);
        ELSIF TG_TABLE_NAME = 'businesses' THEN
            v_entity_type := 'business';
            v_directory_id := COALESCE(NEW.directory_id, OLD.directory_id);
            v_data := jsonb_build_object('name', COALESCE(NEW.name, OLD.name));
        ELSIF TG_TABLE_NAME = 'directory_categories' THEN
            v_entity_type := 'category';
            v_directory_id := COALESCE(NEW.directory_id, OLD.directory_id);
            v_data := jsonb_build_object('name', COALESCE(NEW.name, OLD.name));
        ELSE
            RETURN COALESCE(NEW, OLD);
        END IF;

        IF v_directory_id IS NOT NULL THEN
            INSERT INTO directory_events (event_type, entity_type, entity_id, directory_id, data)
            VALUES (v_event_type, v_entity_type, v_entity_id, v_directory_id, v_data);
        END IF;

        RETURN COALESCE(NEW, OLD);
    END;
$$;
