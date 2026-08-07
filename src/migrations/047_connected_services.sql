-- Migration 047: Connected Services for API Key Integration
-- Supports IncentiveSwift and CoreSwift CRM connections per business user.

CREATE TABLE IF NOT EXISTS connected_services (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL,
    service TEXT NOT NULL CHECK (service IN ('incentiveswift', 'coreswift')),
    api_key_encrypted TEXT,
    is_active BOOLEAN NOT NULL DEFAULT true,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, service)
);

CREATE INDEX idx_connected_services_user ON connected_services(user_id);
CREATE INDEX idx_connected_services_active ON connected_services(service, is_active);
