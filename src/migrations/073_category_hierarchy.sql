-- 073_category_hierarchy.sql — restore Category → Subcategory hierarchy
-- Adds the missing structural columns the code expects on directory_categories
-- and establishes the parent/child (category -> subcategory) mapping via parent_id.
-- Additive only; preserves all existing rows and business links.

-- 1. Structural columns the current code reads/writes (directories.rs, visitors.rs)
ALTER TABLE directory_categories ADD COLUMN IF NOT EXISTS directory_id UUID;
ALTER TABLE directory_categories ADD COLUMN IF NOT EXISTS parent_id UUID REFERENCES directory_categories(id) ON DELETE SET NULL;
ALTER TABLE directory_categories ADD COLUMN IF NOT EXISTS sort_order INTEGER DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_directory_categories_parent ON directory_categories(parent_id) WHERE parent_id IS NOT NULL;

-- 2. Reconcile group_name: "Fine Dining & Cuisine" -> "Fine Dining" (canonical parent name)
UPDATE directory_categories SET group_name = 'Fine Dining' WHERE group_name = 'Fine Dining & Cuisine';

-- 3. Create parent CATEGORY rows (idempotent)
INSERT INTO directory_categories (name, slug, group_name, parent_id, sort_order) VALUES
  ('Fine Dining',      'fine-dining',      'Fine Dining',      NULL, 1),
  ('Food & Drink',     'food-drink',       'Food & Drink',     NULL, 2),
  ('Home & Outdoor',   'home-outdoor',     'Home & Outdoor',   NULL, 3),
  ('Health & Wellness','health-wellness',  'Health & Wellness',NULL, 4),
  ('Professional',     'professional',     'Professional',     NULL, 5),
  ('Personal Care',    'personal-care',    'Personal Care',    NULL, 6)
ON CONFLICT (slug) DO NOTHING;

-- 4. Map every subcategory to its parent category (matching on group_name)
UPDATE directory_categories dc SET parent_id = p.id
  FROM directory_categories p
  WHERE p.slug = 'fine-dining' AND dc.group_name = 'Fine Dining'
    AND dc.parent_id IS NULL AND dc.slug <> 'fine-dining';

UPDATE directory_categories dc SET parent_id = p.id
  FROM directory_categories p
  WHERE p.slug = 'food-drink' AND dc.group_name = 'Food & Drink'
    AND dc.parent_id IS NULL AND dc.slug <> 'food-drink';

UPDATE directory_categories dc SET parent_id = p.id
  FROM directory_categories p
  WHERE p.slug = 'home-outdoor' AND dc.group_name = 'Home & Outdoor'
    AND dc.parent_id IS NULL AND dc.slug <> 'home-outdoor';

UPDATE directory_categories dc SET parent_id = p.id
  FROM directory_categories p
  WHERE p.slug = 'health-wellness' AND dc.group_name = 'Health & Wellness'
    AND dc.parent_id IS NULL AND dc.slug <> 'health-wellness';

UPDATE directory_categories dc SET parent_id = p.id
  FROM directory_categories p
  WHERE p.slug = 'professional' AND dc.group_name = 'Professional'
    AND dc.parent_id IS NULL AND dc.slug <> 'professional';

UPDATE directory_categories dc SET parent_id = p.id
  FROM directory_categories p
  WHERE p.slug = 'personal-care' AND dc.group_name = 'Personal Care'
    AND dc.parent_id IS NULL AND dc.slug <> 'personal-care';
