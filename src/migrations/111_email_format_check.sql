-- 111: a login identity must at least LOOK like an address (kanban t_01f183b1).
--
-- Before this, every signup boundary stored the request's email verbatim behind an `is_empty()`
-- guard only (and the business-claim path had no guard at all), so `{"email":"bad"}` minted a real
-- account whose login is not an address — permanently unreachable, because no welcome/credentials
-- mail can ever be delivered to it. Nothing normalised either, so `  A@B.co ` and `A@B.co` became
-- two different rows against users_tenant_id_email_key UNIQUE(tenant_id, email) and
-- visitor_accounts_email_key UNIQUE(email).
--
-- The application now normalises (trim + lowercase) and validates before every write:
-- src/security/email_addr.rs, called from auth::handlers::register, handlers::b2b::b2b_register,
-- handlers::portal::visitor_register and handlers::visitors::claim_business, with the read side
-- (login, forgot-password, visitor_login) matching on lower(email).
--
-- These CHECKs are the STORE-level backstop for every other path (a migration, a psql session, a
-- future handler). Deliberately LOOSER than the Rust rule — no length cap, no dot-part rules — so
-- the database can never refuse a value the application accepted.
--
-- `ADD CONSTRAINT` has no IF NOT EXISTS and this app's boot-time runner retries a file whose
-- statements failed, so each constraint is added behind a pg_constraint probe: on a re-run both
-- already exist and nothing happens (same idempotent shape as 052/053/092).

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'users_email_format_check'
    ) THEN
        ALTER TABLE public.users ADD CONSTRAINT users_email_format_check
            CHECK (email ~ '^[^[:space:]@]+@[^[:space:]@]+\.[^[:space:]@]+$');
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'visitor_accounts_email_format_check'
    ) THEN
        ALTER TABLE public.visitor_accounts ADD CONSTRAINT visitor_accounts_email_format_check
            CHECK (email ~ '^[^[:space:]@]+@[^[:space:]@]+\.[^[:space:]@]+$');
    END IF;
END $$;
