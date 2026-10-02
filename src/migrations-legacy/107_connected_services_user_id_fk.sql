-- 107 connected_services.user_id is a users.id, so enforce it as one (t_f9049f4a).
--
-- Decision: the column IS meant to be a foreign key, not a loose uuid.
--   * src/auth/models.rs:9 documents Claims.sub as "user id", and both
--     connect_service and disconnect_service parse claims.sub straight into
--     user_id (src/handlers/connected_services.rs:98 and :253) with no
--     existence check anywhere — the auth middleware only verifies the token
--     signature, it never checks the subject resolves to a row.
--   * 12 other tables already FK users(id). The user-OWNED ones (account_links,
--     api_keys, password_resets, polls, user_industry_dashboards) use ON DELETE
--     CASCADE and this follows that convention: a connection is the user's own
--     credential, so it goes when the user goes.
--   * Without it the schema allowed a connection to be armed for a user that
--     does not exist, so the next audit could not tell a real connection from a
--     stale one. Two such orphan rows were live in production
--     (coreswift/186ad242-3228-4d26-9013-a728c01cb15b and
--     incentiveswift/4decc64e-f06e-455e-b03d-dc82df7294f4, created 2026-09-22).
--
-- Idempotent and safe to re-run. Orphans are deleted BEFORE the constraint is
-- validated, so VALIDATE can never fail on pre-existing data.

DELETE FROM connected_services cs
 WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.id = cs.user_id);

ALTER TABLE connected_services DROP CONSTRAINT IF EXISTS connected_services_user_id_fkey;

ALTER TABLE connected_services
  ADD CONSTRAINT connected_services_user_id_fkey
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE NOT VALID;

ALTER TABLE connected_services VALIDATE CONSTRAINT connected_services_user_id_fkey;
