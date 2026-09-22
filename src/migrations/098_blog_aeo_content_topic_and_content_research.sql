-- 098: columns and tables the handlers have always read/written but which were never migrated.
--
-- Killed live 500s (proven 2026-09-21 against the running container):
--   GET  /api/v1/blog/aeo/report      column "aeo_score" does not exist
--   POST /api/v1/blog/aeo/score/:id   column "answer_block" does not exist
--   GET  /api/v1/research/topics      no column found for name: name   (content_topics drift)
--   GET  /api/v1/research/questions   relation "content_research" does not exist
--
-- Also makes Yelp a first-class provider key. It was read from an env var only, so the
-- route 500'd for every admin who had not hand-edited the server environment.

-- ---- blog_posts: AEO / internal-linking feature (src/handlers/blog_features.rs) ----
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS answer_block TEXT;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS aeo_score INTEGER;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS internal_links JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS schema_json JSONB;

-- AeoScoredPost.aeo_score is a non-optional i32 and the report orders by it, so a NULL
-- (never scored) row broke the whole report with "unexpected null".
UPDATE blog_posts SET aeo_score = 0 WHERE aeo_score IS NULL;

ALTER TABLE blog_posts ALTER COLUMN aeo_score SET DEFAULT 0;

ALTER TABLE blog_posts ALTER COLUMN aeo_score SET NOT NULL;

-- ---- content_topics: keyword research feature (src/handlers/content_research.rs) ----
ALTER TABLE content_topics ADD COLUMN IF NOT EXISTS name TEXT;
ALTER TABLE content_topics ADD COLUMN IF NOT EXISTS description TEXT;
ALTER TABLE content_topics ADD COLUMN IF NOT EXISTS keywords JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE content_topics ADD COLUMN IF NOT EXISTS search_phrase TEXT;
ALTER TABLE content_topics ADD COLUMN IF NOT EXISTS question_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE content_topics ADD COLUMN IF NOT EXISTS last_researched TIMESTAMPTZ;

-- Rows created by the older editorial-calendar path carry a title, not a name.
-- ContentTopic.name is a non-optional String, so a NULL here would fail the decode.
UPDATE content_topics
   SET name = COALESCE(NULLIF(title, ''), NULLIF(target_keyword, ''), 'Untitled topic')
 WHERE name IS NULL;

ALTER TABLE content_topics ALTER COLUMN name SET NOT NULL;

-- content_topics is shared by two UIs: the editorial calendar writes `title`, the
-- keyword-research UI writes `name`. Both columns are NOT NULL, so give each a default
-- and have each writer fill the other (name = title) — otherwise one UI's insert 500s.
ALTER TABLE content_topics ALTER COLUMN title SET DEFAULT '';

ALTER TABLE content_topics ALTER COLUMN name SET DEFAULT '';

-- ---- content_research: the questions table the research feature reads ----
CREATE TABLE IF NOT EXISTS content_research (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    topic_id uuid NOT NULL REFERENCES content_topics(id) ON DELETE CASCADE,
    directory_id uuid REFERENCES directories(id) ON DELETE CASCADE,
    question text NOT NULL,
    source_url text,
    source_domain text,
    entry_kind text,
    is_used boolean NOT NULL DEFAULT false,
    used_as_keyword boolean NOT NULL DEFAULT false,
    drafted_post_id uuid,
    freshness_score double precision,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_content_research_topic_id ON content_research(topic_id);

CREATE INDEX IF NOT EXISTS idx_content_research_directory_id ON content_research(directory_id);

CREATE UNIQUE INDEX IF NOT EXISTS uq_content_research_topic_question ON content_research(topic_id, question);

-- ---- Yelp: configurable from the admin panel instead of an env var ----
INSERT INTO available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help)
VALUES (
  'yelp',
  'Yelp',
  'Business search and details from the Yelp Fusion API, used to find and import businesses into a directory.',
  false,
  '[]'::jsonb,
  '🍽️',
  'Yelp API key',
  'From the Yelp Fusion dashboard (yelp.com/developers → Manage App → API Key). Create the app inside the same Yelp account that owns the listings.'
)
ON CONFLICT (key) DO UPDATE
  SET name = EXCLUDED.name,
      description = EXCLUDED.description,
      requires_base_url = EXCLUDED.requires_base_url,
      icon = EXCLUDED.icon,
      field_label = EXCLUDED.field_label,
      field_help = EXCLUDED.field_help;
