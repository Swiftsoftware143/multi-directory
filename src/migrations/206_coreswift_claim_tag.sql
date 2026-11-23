-- 206_coreswift_claim_tag.sql — admin-configurable CoreSwift tag for claimed-business contacts.
--
-- Card (work-queue line 97): the moment a business CLAIMS its listing it is pushed to CoreSwift
-- as a contact on the "Claimed Businesses" list AND tagged. The list was already configurable
-- (coreswift_list_id_claimed, set in the Integration Center); the TAG was not — push_claimed_business
-- derived it from the directory slug ("{prefix}-biz-zh-interested"), a provider choice living only in
-- code (David's NOTHING-HARDWIRED rule, fatal class 2). This adds a per-directory (and per-network,
-- for inheritance) tag that the claim push reads first, falling back to the slug-derived name only
-- when no tag is configured.
ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_tag_claimed text;
ALTER TABLE networks    ADD COLUMN IF NOT EXISTS coreswift_tag_claimed text;

COMMENT ON COLUMN directories.coreswift_tag_claimed IS
  'Card line 97: CoreSwift tag applied to a claimed-business contact. NULL/empty = fall back to the slug-derived ''{prefix}-biz-zh-interested'' name.';
COMMENT ON COLUMN networks.coreswift_tag_claimed IS
  'Card line 97: network-level default for the claimed-business CoreSwift tag (inherited by directories that do not set their own).';
