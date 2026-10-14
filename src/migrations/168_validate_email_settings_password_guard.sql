-- 168: arm the stronger half of the directory_email_settings password guard.
--
-- 162 ADDed `directory_email_settings_smtp_password_encrypted` NOT VALID and deliberately carried no
-- VALIDATE (its comment says the pre-existing plaintext row would be backfilled out-of-band, and the
-- master key must never be coupled into this directory). The backfill has since happened - the one
-- live row now reads enc:v1:... - but the constraint was never re-validated, so it still reads
-- convalidated = f and the fleet guard-constraint-validity sweep alerted on every tick. New writes
-- were already rejected, but "every existing row is compliant" had never armed.
--
-- This is the canonical self-heal block of /opt/swift/fleet/templates/guard-constraint-not-valid.sql
-- (fleet convention sections 1-4, decision t_c9b09cc1), shipped WITHOUT the DROP/ADD pair because the
-- constraint already exists and 162 must stay untouched - an applied file is skipped by the _migrations
-- ledger, so a new higher-numbered file is the only legal carrier. The block skips a constraint that is
-- already validated and swallows check_violation as a WARNING, so it can never fail a boot and can
-- never leave a guard absent. A fresh install lands it validated too (162 adds it NOT VALID on an
-- empty table, this file then validates it immediately).
-- No semicolon anywhere in these comments - a naive in-app runner may execute the whole file.

DO $guard$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'directory_email_settings_smtp_password_encrypted'
          AND conrelid = 'directory_email_settings'::regclass
          AND convalidated
    ) THEN
        BEGIN
            ALTER TABLE directory_email_settings VALIDATE CONSTRAINT directory_email_settings_smtp_password_encrypted;
            RAISE NOTICE 'directory_email_settings.directory_email_settings_smtp_password_encrypted validated: every existing row is compliant';
        EXCEPTION
            WHEN check_violation THEN
                RAISE WARNING 'directory_email_settings.directory_email_settings_smtp_password_encrypted still NOT VALID: pre-existing rows violate the guard (backfill them, then re-run this file); new writes are still rejected';
        END;
    END IF;
END
$guard$;
