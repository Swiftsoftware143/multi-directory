-- Migration 041: Directory Notifications / Ticker Bar
-- Phase 4: Builder's Spotlight & Notifications

CREATE TABLE IF NOT EXISTS directory_notifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    message TEXT NOT NULL,
    link_text TEXT,
    link_url TEXT,
    notification_type TEXT NOT NULL DEFAULT 'info',
    is_active BOOLEAN NOT NULL DEFAULT true,
    starts_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_directory_notifications_active
    ON directory_notifications (directory_id, is_active, starts_at, expires_at);
