-- Migration 061: Additional RFQ indexes for performance
CREATE INDEX IF NOT EXISTS idx_rfq_bids_status ON rfq_bids(status);
CREATE INDEX IF NOT EXISTS idx_rfq_messages_sender ON rfq_messages(sender_business_id);
