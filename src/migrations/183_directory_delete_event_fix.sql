-- 183 — a directory can be DELETED again (kanban W40 / card builds)
--
-- `auto_record_directory_event()` fired on DELETE of `directories` and INSERTed a
-- `directory_events` row whose `directory_id` pointed at the row being removed. The FK is
-- NO ACTION, so Postgres rejected the insert AFTER the parent row was gone and the whole
-- DELETE aborted:
--
--   ERROR: insert or update on table "directory_events" violates foreign key constraint
--          "directory_events_directory_id_fkey"
--   CONTEXT: SQL statement "INSERT INTO directory_events (...)" PL/pgSQL function
--            auto_record_directory_event() line 37
--
-- Consequence: a directory created by mistake (or by a customer who cancels) could never be
-- removed — not from the panel, not with plain SQL — so the app's own DELETE /directories/:slug
-- door was dead and reported "a record type this app does not know how to clear".
--
-- The fix: a directory's own deletion is not a recordable event. `directory_events` is scoped BY
-- directory (its subscribers live on that directory), so an event about a directory that no longer
-- exists has no one to reach and only creates residue. Skip the insert on that one path.
-- Every other table (businesses, directory_categories) keeps its created/updated/deleted events.
--
-- The NEW.<col> reads are also made COALESCE(NEW, OLD) so the function can never dereference an
-- unassigned record on a DELETE path.

CREATE OR REPLACE FUNCTION auto_record_directory_event() RETURNS trigger
LANGUAGE plpgsql AS $$
    DECLARE
        v_event_type text;
        v_entity_type text;
        v_entity_id uuid;
        v_directory_id uuid;
        v_data jsonb;
    BEGIN
        -- A directory that is being deleted cannot own an event about its own deletion: the FK
        -- would point at a row that is already gone. Nothing to deliver to, either.
        IF TG_OP = 'DELETE' AND TG_TABLE_NAME = 'directories' THEN
            RETURN OLD;
        END IF;

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

        IF v_directory_id IS NOT NULL THEN
            INSERT INTO directory_events (event_type, entity_type, entity_id, directory_id, data)
            VALUES (v_event_type, v_entity_type, v_entity_id, v_directory_id, v_data);
        END IF;

        RETURN COALESCE(NEW, OLD);
    END;
$$;
