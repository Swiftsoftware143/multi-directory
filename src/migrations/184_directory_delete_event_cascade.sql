-- 184 — deleting a directory that OWNS listings must work too (follow-up to 183, kanban W40)
--
-- 183 stopped the trigger from recording its own `directories` DELETE. That fixed an empty
-- directory, but a directory that holds listings still failed:
--
--   DELETE FROM directories  →  businesses cascade  →  trg_auto_record_event (DELETE on
--   businesses) inserts a `businesses.deleted` event whose directory_id is the directory that is
--   being removed in the SAME statement. The FK check cannot see that directory any more, so the
--   insert raises 23503 and the whole delete aborts:
--
--     "A record type this app does not know how to clear still points at 'zz-…' (directory events)"
--
-- The rule this migration encodes: an event is a NOTIFICATION, and a notification must never be
-- able to abort the delete that produced it. On a DELETE path the event is recorded only when its
-- directory is still there; if the directory is going away in the same transaction, the event is
-- skipped (nobody is subscribed to a directory that no longer exists). INSERT/UPDATE paths are
-- untouched — they cannot race a directory delete, and they keep their events unconditionally.

CREATE OR REPLACE FUNCTION auto_record_directory_event() RETURNS trigger
LANGUAGE plpgsql AS $$
    DECLARE
        v_event_type text;
        v_entity_type text;
        v_entity_id uuid;
        v_directory_id uuid;
        v_data jsonb;
    BEGIN
        -- A directory that is being deleted cannot own an event about its own deletion.
        IF TG_OP = 'DELETE' AND TG_TABLE_NAME = 'directories' THEN
            RETURN OLD;
        END IF;

        IF TG_OP = 'INSERT' THEN
            v_event_type := TG_TABLE_NAME || '.created';
            v_entity_id := NEW.id;
        ELSIF TG_OP = 'UPDATE' THEN
            v_event_type := TG_TABLE_NAME || '.updated';
            v_entity_id := NEW.id;
        ELSE
            v_event_type := TG_TABLE_NAME || '.deleted';
            v_entity_id := OLD.id;
        END IF;

        IF TG_TABLE_NAME = 'directories' THEN
            v_entity_type := 'directory';
            v_directory_id := v_entity_id;
            v_data := jsonb_build_object(
                'slug', COALESCE(NEW.slug, OLD.slug),
                'name', COALESCE(NEW.name, OLD.name)
            );
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

        IF v_directory_id IS NULL THEN
            RETURN COALESCE(NEW, OLD);
        END IF;

        IF TG_OP = 'DELETE' THEN
            -- Never let bookkeeping abort the delete. Only reached when the owning directory is
            -- itself being removed by the same statement (a cascade), where nobody could receive
            -- the event anyway.
            BEGIN
                INSERT INTO directory_events (event_type, entity_type, entity_id, directory_id, data)
                VALUES (v_event_type, v_entity_type, v_entity_id, v_directory_id, v_data);
            EXCEPTION WHEN foreign_key_violation THEN
                NULL;
            END;
            RETURN OLD;
        END IF;

        INSERT INTO directory_events (event_type, entity_type, entity_id, directory_id, data)
        VALUES (v_event_type, v_entity_type, v_entity_id, v_directory_id, v_data);

        RETURN NEW;
    END;
$$;
