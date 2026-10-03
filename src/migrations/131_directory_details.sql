-- 131_directory_details.sql — canonical Directory Details (card B119).
--
-- David's rule (2026-10-02): a merge field has to PULL FROM SOMEWHERE. There must be ONE record
-- per directory (or network) where the site's core identity is entered, and every merge field
-- across legal pages, the website, footers, emails, SEO and the onboarding survey reads from it.
--
-- `directories` already carried name / city / state / url_value / custom_domain / color_scheme,
-- which is most of that record, but the SUPPORT EMAIL (David specifically asked for it), the
-- contact email/phone and the legal/entity name had nowhere to live, so `{support_email}` could
-- never resolve to anything a buyer controls. This adds those four columns.
--
-- Nullable and additive: an existing directory keeps working, and a blank inherits from the
-- network context at render time (see merge_fields::MergeContext::for_directory). No backfill is
-- required or wanted — production data is the live tenants' own identity.
ALTER TABLE directories ADD COLUMN IF NOT EXISTS support_email varchar(255);
ALTER TABLE directories ADD COLUMN IF NOT EXISTS contact_email varchar(255);
ALTER TABLE directories ADD COLUMN IF NOT EXISTS contact_phone varchar(32);
ALTER TABLE directories ADD COLUMN IF NOT EXISTS legal_name    varchar(255);
