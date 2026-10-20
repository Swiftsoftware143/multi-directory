-- 173_user_sessions_valid_from.sql — a password change/reset must end the sessions that predate it
-- (kanban t_dd9d99c9, the third residual of the deleted-account class closed here by t_fb37dfc0).
--
-- `POST /api/v1/auth/login` mints a 24 h JWT and the only thing the boundary ever asked of it was
-- signature + `exp` + `iss` + `aud`. So `PUT /api/v1/auth/password` and `POST
-- /api/v1/auth/reset-password` rewrote `users.password_hash` and revoked NOTHING: measured
-- 2026-10-20, a token minted before the change kept answering 200 on every guarded route for the
-- rest of its life (the compromised-session case the user is trying to end by changing the
-- password).
--
-- This column is the instant the account last changed or reset its password. Both writers stamp it
-- with NOW(); the boundary read (`auth::middleware::session_state`) returns it as a unix epoch and
-- refuses a session whose `iat` predates it — the same 401 a deleted or disabled account gets.
--
-- NULL = never rotated, which is every account until it rotates once: additive, so every token
-- already in the wild keeps working and nothing changes until a password is actually written.
--
-- Idempotent and additive: `ADD COLUMN IF NOT EXISTS` (src/db.rs re-applies every file in
-- src/migrations on each boot, so this must be safe to re-run).
--
-- Only `users` gets it: this app has two identity tables (`users` for staff/directory/business,
-- `visitor_accounts` for portal shoppers), but `visitor_accounts` has no password writer at all —
-- its `password_hash` is set once by INSERT at registration and never updated — so there is no
-- change/reset path to revoke. When one is added, it stamps the same column on that table.

ALTER TABLE public.users ADD COLUMN IF NOT EXISTS sessions_valid_from timestamptz;

COMMENT ON COLUMN public.users.sessions_valid_from IS
  'Instant this account last changed or reset its password; NULL = never. A JWT whose iat predates it is refused with 401 (kanban t_dd9d99c9).';
