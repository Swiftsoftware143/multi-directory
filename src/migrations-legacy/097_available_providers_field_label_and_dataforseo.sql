-- 097 the extra credential input's label comes from the data, not from hardcoded frontend text.
--
-- available_providers.requires_base_url means "this provider needs a SECOND input besides the API
-- key". That second input is stored in provider_keys.base_url because for most providers it really
-- is a URL (and for the self-hosted CoreSwift hub it resolves the hub address). For DataForSEO it is
-- the HTTP Basic-auth LOGIN - the account email - so a label reading "Base URL" is simply wrong and
-- leaves the admin guessing what to type.
--
-- Two columns make that label data-driven instead of a per-provider branch in the frontend:
--   field_label  label for the second input (NULL = the frontend falls back to "Base URL")
--   field_help   one-line hint rendered with the input (NULL = no hint)
--
-- This migration also inserts the missing `dataforseo` row. The Integrations list is built from this
-- table alone, so with no row there the Keyword/DataForSEO feature of the Blog+QA tool could not be
-- configured from ANY ui - the key endpoint accepted nothing and the handler's own error message
-- pointed at a page that offered no such field.
--
-- The row keeps requires_base_url = true, because the login has to be captured somewhere. It is
-- stored in provider_keys.base_url (the storage column is unchanged) and the secret half of the pair
-- - the API password - goes in api_key, which is encrypted at rest under PROVIDER_KEY_ENC_SECRET.

ALTER TABLE available_providers ADD COLUMN IF NOT EXISTS field_label TEXT;

ALTER TABLE available_providers ADD COLUMN IF NOT EXISTS field_help TEXT;

INSERT INTO available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help)
VALUES (
  'dataforseo',
  'DataForSEO',
  'Keyword ideas and search volume for the Blog/QA keyword tool. Uses HTTP Basic auth, so it needs BOTH your DataForSEO account email (the login) and the API password from the DataForSEO dashboard.',
  true,
  '[]'::jsonb,
  '🔑',
  'DataForSEO login (account email)',
  'Your DataForSEO account email. The API key field above holds the matching API password from dashboard.dataforseo.com → API Access.'
)
ON CONFLICT (key) DO UPDATE
  SET name = EXCLUDED.name,
      description = EXCLUDED.description,
      requires_base_url = EXCLUDED.requires_base_url,
      icon = EXCLUDED.icon,
      field_label = EXCLUDED.field_label,
      field_help = EXCLUDED.field_help;
