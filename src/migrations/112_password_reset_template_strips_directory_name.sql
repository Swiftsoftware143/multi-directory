-- 112_password_reset_template_strips_directory_name.sql
--
-- kanban t_ba93aea4 — the only password-reset row mailed `{{directory_name}}` verbatim.
--
-- WHAT WAS WRONG
--   Row: the ONE global default `password_reset` row in email_templates (name = 'password_reset',
--   directory_id IS NULL), variables = {code,token,directory_name}. Its id is printed by the
--   runtime NOTICE below, so it is not repeated as a literal here.
--   Its SUBJECT ("Password Reset Request — {{directory_name}}"), its html body (2x) and its
--   body_text (1x) all carried `{{directory_name}}` — but the only reader of that row,
--   `send_reset_email` in src/email.rs, substitutes exactly `{{token}}` and `{{code}}` and
--   has NO leftover detector, so the token reached the recipient literally and silently
--   (same silence class as t_c8df11e6).
--
--   REACHABLE, measured live 2026-10-02: `send_reset_email`'s selector
--     (name = 'password_reset' AND directory_id IS NULL ORDER BY created_at DESC LIMIT 1)
--   picks this row — it is the only password_reset row in the catalogue — and it is called
--   from the pre-login flow POST /api/v1/auth/forgot-password (src/auth/handlers.rs).
--   Wire capture (loopback :3456, /opt/swift/audits/t_ba93aea4/before-wire-01-send.txt):
--     subject = 'Password Reset Request — {{directory_name}}'
--     html    = '... requested for your <strong>{{directory_name}}</strong> account. ...'
--     text    = '... - {{directory_name}}\n- Multi-Directory'
--
--   ORIGIN: seeded by src/migrations/023_seed_password_reset_template.sql, which carries the
--   same token (copy-paste drift, not a one-off admin edit). That seed file is corrected in
--   the same commit, so a fresh database never ships the drift either.
--
-- ARM (a), decided by measurement — STRIP THE TOKEN FROM THE STORED COPY; do NOT bind
--   `directory_name`. Arm (b) is not available here: the forgot-password flow runs BEFORE
--   login and has no directory context, and this row is directory-agnostic
--   (directory_id IS NULL) — it is the platform's own shipped copy. The row must therefore
--   not ADVERTISE a merge field the renderer cannot bind, so `variables` is trimmed too: the
--   advertised-vs-bound gap IS the defect.
--
-- SCOPE — the GLOBAL DEFAULT row only (name = 'password_reset' AND directory_id IS NULL).
--   A tenant row (directory_id IS NOT NULL) would be the tenant's own text and is not
--   rewritten here; the companion code change widens `send_reset_email` with a leftover
--   detector that NAMES any unresolved `{{token}}`, which covers tenant rows and any future
--   drift on the wire.
--
-- IDEMPOTENT: on replay the guard (`... LIKE '%{{directory_name}}%' OR variables @> ...`)
--   matches 0 rows, so every UPDATE below is a no-op.
-- REVERSING: re-insert the row text from src/migrations/023_seed_password_reset_template.sql.
--
-- The runner is a whole-file, filename-sorted, record-on-success applier: an unmet assertion
-- must be REPORTED (RAISE NOTICE), never RAISE EXCEPTION — a raising file would retry forever.

-- ── 1. name, in the boot log, exactly which shipped row(s) this rewrites ─────────────────────
DO $$
DECLARE r record; n int := 0;
BEGIN
    FOR r IN
        SELECT id, name FROM email_templates
         WHERE name = 'password_reset' AND directory_id IS NULL
           AND (subject LIKE '%{{directory_name}}%'
                OR body LIKE '%{{directory_name}}%'
                OR coalesce(body_text, '') LIKE '%{{directory_name}}%')
         ORDER BY created_at DESC
    LOOP
        n := n + 1;
        RAISE NOTICE 't_ba93aea4: rewriting password_reset global default template % with unrenderable {{directory_name}}', r.id;
    END LOOP;
    RAISE NOTICE 't_ba93aea4: password_reset global default templates carrying {{directory_name}} (before): % row(s)', n;
END $$;

-- ── 2. the row: subject separator+token, the <strong> wrapper, the signature line, the advert ──
-- Each field is a small pipeline of literal regexp_replace steps (the token is stripped from
-- whatever dash/holder form it sits in, in both the live em-dash and the seed's `--` spelling)
-- and every step is a no-op once it has run, so a replay changes nothing.
UPDATE email_templates
   SET subject = trim(both from regexp_replace(
                       regexp_replace(subject,
                         '[[:space:]]*[—–-]+[[:space:]]*\{\{directory_name\}\}', '', 'g'),
                       '\{\{directory_name\}\}', '', 'g')),
       body = regexp_replace(
                regexp_replace(
                  regexp_replace(
                    regexp_replace(
                      regexp_replace(body,
                        '<strong>[[:space:]]*\{\{directory_name\}\}[[:space:]]*</strong>', '', 'g'),
                      '[[:space:]]*[—–]+[[:space:]]*\{\{directory_name\}\}', '', 'g'),
                    '\{\{directory_name\}\}[[:space:]]*[—–-]+', '', 'g'),
                  '\{\{directory_name\}\}', '', 'g'),
                'your[[:space:]]+account', 'your account', 'g'),
       body_text = regexp_replace(
                     regexp_replace(body_text,
                       '^[[:space:]]*-[[:space:]]*\{\{directory_name\}\}[[:space:]]*$', '', 'gn'),
                     'your[[:space:]]+account', 'your account', 'g'),
       variables = array_remove(variables, 'directory_name'),
       updated_at = now()
 WHERE name = 'password_reset'
   AND directory_id IS NULL
   AND (subject LIKE '%{{directory_name}}%'
        OR body LIKE '%{{directory_name}}%'
        OR coalesce(body_text, '') LIKE '%{{directory_name}}%'
        OR variables @> ARRAY['directory_name']::text[]);

-- ── 3. the invariant, asserted in the boot log (never silent, never a boot failure) ───────────
DO $$
DECLARE n int; names text;
BEGIN
    SELECT count(*), string_agg(DISTINCT name, ', ') INTO n, names
      FROM email_templates
     WHERE name = 'password_reset'
       AND (subject LIKE '%{{directory_name}}%'
            OR body LIKE '%{{directory_name}}%'
            OR coalesce(body_text, '') LIKE '%{{directory_name}}%');
    RAISE NOTICE 't_ba93aea4: password_reset rows still carrying {{directory_name}} (after): % row(s) [%]', n, coalesce(names, 'none');
    SELECT count(*) INTO n FROM email_templates WHERE name = 'password_reset' AND directory_id IS NULL;
    RAISE NOTICE 't_ba93aea4: password_reset global default rows present (catalogue unchanged): % row(s)', n;
END $$;
