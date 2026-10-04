-- B90: business-owner public reply to a review (Thumbtack / Angie's-List parity).
--
-- The business portal's own walkthrough promises "Respond to reviews publicly to show you care
-- about feedback", but the reviews table had no column and there was no route: an owner had no
-- way to answer a review. The reply lives on the review row itself so the public listing can
-- render it directly under the review.
ALTER TABLE reviews ADD COLUMN IF NOT EXISTS owner_response text;
ALTER TABLE reviews ADD COLUMN IF NOT EXISTS owner_responded_at timestamptz;
-- Who answered (audit only). No FK on purpose: the caller may be a user OR a visitor_account.
ALTER TABLE reviews ADD COLUMN IF NOT EXISTS owner_response_by uuid;
