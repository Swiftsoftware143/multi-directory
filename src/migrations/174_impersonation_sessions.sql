-- 174_impersonation_sessions.sql — a support session that is ENDED must actually end
-- (kanban t_a9c33fea, the last leg of the t_498836c3 operator support-session work).
--
-- `handlers::impersonate` mints a 1-hour JWT that signs in AS a tenant account. Until now the
-- app had no way to end one: the module said so in as many words ("there is deliberately no
-- 'stop impersonation' route — the session is stateless"), and the practical consequence was
-- measured on the console: pressing "Return to operator" only swapped localStorage back. The
-- impersonation bearer stayed valid on every route for the rest of its hour, from any tab or
-- device that still held it, and nothing in the database recorded that the operator had left.
--
-- This table is that record. ONE row per minted support session, keyed by
-- (target_user_id, operator_id, token_iat) — `iat` is second-granularity and is the only claim
-- that differs between two mints for the same pair, so it identifies the session without adding
-- a `jti` to `Claims` (which would be absent from every token already in the wild).
--
--   stopped_at IS NULL      -> the session is live
--   stopped_at IS NOT NULL  -> the session was ended; `auth::middleware::session_verdict` and
--                              `routes::auth_guard` refuse it with 401 exactly like an expired
--                              token, on the private surface AND on the public-but-personalising
--                              handlers that parse the header themselves.
--
-- The row is written by the MINT (status: live) and stamped by `POST
-- /api/v1/admin/stop-impersonation` (status: ended). It is deliberately NOT a foreign key to
-- `users` or `tenants`: an audit row must outlive the accounts it names, and a deleted tenant
-- must not cascade away the record that an operator was inside it.
--
-- `expires_at` mirrors the token's own `exp`; the read path does not consult it (an expired token
-- is already refused by `verify_token`), it is there so the audit answers "how long was this
-- session open" without decoding a token that no longer exists.
--
-- Idempotent and additive: `src/db.rs` re-applies every file in `src/migrations` on each boot.

CREATE TABLE IF NOT EXISTS public.impersonation_sessions (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    operator_id      uuid        NOT NULL,
    target_tenant_id uuid        NOT NULL,
    target_user_id   uuid        NOT NULL,
    token_iat        bigint      NOT NULL,
    issued_at        timestamptz NOT NULL DEFAULT NOW(),
    expires_at       timestamptz NOT NULL,
    stopped_at       timestamptz,
    CONSTRAINT impersonation_sessions_one_mint
        UNIQUE (target_user_id, operator_id, token_iat)
);

-- The live/ended lookup runs on every request that carries an `impersonating` claim.
CREATE INDEX IF NOT EXISTS impersonation_sessions_lookup_idx
    ON public.impersonation_sessions (target_user_id, operator_id, token_iat);

COMMENT ON TABLE public.impersonation_sessions IS
  'One row per operator support-session mint. stopped_at NULL = live; NOT NULL = the session was ended and its token is refused with 401 (kanban t_a9c33fea).';
COMMENT ON COLUMN public.impersonation_sessions.token_iat IS
  'The token''s own `iat` claim: the only claim that differs between two mints for the same (operator, account) pair, so it is the session identity.';
COMMENT ON COLUMN public.impersonation_sessions.stopped_at IS
  'Instant POST /api/v1/admin/stop-impersonation ended this session. Set once (COALESCE) so a repeated stop cannot rewrite the audit trail.';
