-- 132_directories_is_primary.sql — an explicit "primary directory" per network (card B97).
--
-- David (2026-10-01): "the main directory admin … would be the first city that it created, which is
-- Palm Bay. I need verification on that."
--
-- VERIFIED IN FACT: Palm Bay IS the first-created directory of the ZaarHub network (2026-06-28,
-- the other nine cities all landed in one batch on 2026-07-18). But there was NO is_primary / main /
-- default / home flag on `directories` — "the main admin = the first city" was an IMPLICIT
-- convention based on created_at ORDER. That is fragile (re-creating a city would silently move it)
-- and confusing to a buyer ("why is Palm Bay the admin of my network?").
--
-- This migration makes the choice EXPLICIT and admin-settable, exactly one holder per network,
-- defaulting to the first-created directory so nothing changes today. Code must never depend on
-- created_at ordering again — it reads is_primary.
--
-- A STANDALONE directory (network_id IS NULL) has no primary: there is nothing to delegate to.
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

ALTER TABLE directories ADD COLUMN IF NOT EXISTS is_primary boolean NOT NULL DEFAULT false;

-- At most one primary per network. Scoped to network_id IS NOT NULL so standalone directories
-- (which never carry the flag) can never collide.
CREATE UNIQUE INDEX IF NOT EXISTS directories_one_primary_per_network
    ON directories (network_id)
    WHERE is_primary AND network_id IS NOT NULL;

-- Default each existing network's primary to its FIRST-CREATED directory (ZaarHub -> Palm Bay).
-- The NOT EXISTS guard makes this a no-op once a network already has a primary, so a later
-- admin choice is never overwritten by a re-run.
UPDATE directories d
   SET is_primary = true
 WHERE d.network_id IS NOT NULL
   AND d.id = (SELECT f.id FROM directories f
                WHERE f.network_id = d.network_id
                ORDER BY f.created_at, f.id
                LIMIT 1)
   AND NOT EXISTS (SELECT 1 FROM directories x
                    WHERE x.network_id = d.network_id AND x.is_primary);
