-- 114 claimed_businesses.visitor_account_id must be a real FK to visitor_accounts (t_cb86753e).
--
-- Decision: the column IS meant to be a foreign key to visitor_accounts(id).
--   * It is written from the visitor session at supplier-portal registration
--     (src/handlers/b2b.rs:303 inserts visitor_account_id) and read back to resolve a
--     business owner from an access token (b2b.rs:2310 and :2377, lead_sharing.rs:39).
--   * The schema simply never declared it, so deleting a visitor left the claim row
--     pointing at a row that no longer existed. 5 such dangling refs were live on
--     2026-10-02 and the t_6465fdbb visitor sweep would have created 6 more (it
--     un-pointed them in-transaction instead of leaving new danglers behind).
--   * The fleet's own fk_tables() already flags this shape: a column that is not
--     covered by a real FK survives a parent delete.
--
-- Semantic: ON DELETE SET NULL, exactly as the two tables that already FK this same
-- column in the same direction do -- survey_responses.visitor_account_id
-- (migration 006) and deal_redemptions.visitor_id. The claim belongs to the business
-- and survives its visitor. It is the portal login that goes.
--
-- Idempotent and safe to re-run. Pre-existing danglers are un-pointed BEFORE the
-- constraint is validated, so VALIDATE can never fail on legacy data.

UPDATE claimed_businesses cb
   SET visitor_account_id = NULL,
       updated_at = NOW()
 WHERE cb.visitor_account_id IS NOT NULL
   AND NOT EXISTS (SELECT 1 FROM visitor_accounts va WHERE va.id = cb.visitor_account_id);

ALTER TABLE claimed_businesses DROP CONSTRAINT IF EXISTS claimed_businesses_visitor_account_id_fkey;

ALTER TABLE claimed_businesses
  ADD CONSTRAINT claimed_businesses_visitor_account_id_fkey
  FOREIGN KEY (visitor_account_id) REFERENCES visitor_accounts(id) ON DELETE SET NULL NOT VALID;

ALTER TABLE claimed_businesses VALIDATE CONSTRAINT claimed_businesses_visitor_account_id_fkey;
