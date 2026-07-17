-- Create GIN index on businesses.search_vector for full-text search performance
CREATE INDEX IF NOT EXISTS idx_businesses_search_vector ON businesses USING GIN(search_vector);
