-- Migration 080: Allow network-wide loyalty programs (no single directory)
-- A network program spans all directories in the network, so directory_id is NULL.
ALTER TABLE loyalty_programs ALTER COLUMN directory_id DROP NOT NULL;
