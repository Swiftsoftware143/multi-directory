-- Migration 043: ZaarHub legal pages + site config
CREATE TABLE IF NOT EXISTS zaarhub_site_config (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001'::uuid,
    site_name VARCHAR(255) DEFAULT 'ZaarHub',
    site_tagline TEXT DEFAULT 'Discover Your Local Community',
    primary_color VARCHAR(7) DEFAULT '#f27f2f',
    secondary_color VARCHAR(7) DEFAULT '#2b3255',
    logo_url TEXT,
    favicon_url TEXT,
    google_analytics_id VARCHAR(64),
    facebook_app_id VARCHAR(64),
    twitter_handle VARCHAR(32),
    contact_email VARCHAR(255),
    contact_phone VARCHAR(32),
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS zaarhub_legal_pages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001'::uuid,
    slug VARCHAR(128) NOT NULL UNIQUE,  -- 'terms', 'privacy', 'cookies', 'accessibility', 'contact', etc.
    title VARCHAR(255) NOT NULL,        -- 'Terms of Service', 'Privacy Policy', etc.
    content TEXT NOT NULL,              -- HTML content
    is_published BOOLEAN DEFAULT false,
    show_in_footer BOOLEAN DEFAULT false,
    display_order INT DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

-- Seed default legal pages
INSERT INTO zaarhub_legal_pages (slug, title, content, is_published, show_in_footer, display_order) VALUES
('terms', 'Terms of Service',
 '<h2>Terms of Service</h2><p>Welcome to ZaarHub. By using our directory services, you agree to these terms.</p><p><strong>Last Updated:</strong> August 2026</p><h3>1. Acceptance of Terms</h3><p>By accessing or using ZaarHub, you agree to be bound by these Terms of Service.</p><h3>2. Directory Listings</h3><p>Business listings are provided by third parties. ZaarHub does not guarantee the accuracy of listing information.</p><h3>3. User Conduct</h3><p>Users agree not to misuse the directory or submit false information.</p><h3>4. Intellectual Property</h3><p>All content on ZaarHub is protected by copyright and trademark laws.</p><h3>5. Limitation of Liability</h3><p>ZaarHub is provided "as is" without warranties of any kind.</p><h3>6. Contact</h3><p>For questions about these terms, visit our <a href="/legal/contact">Contact page</a>.</p>',
 true, true, 1),

('privacy', 'Privacy Policy',
 '<h2>Privacy Policy</h2><p>Your privacy is important to us. This policy explains how ZaarHub collects and uses information.</p><p><strong>Last Updated:</strong> August 2026</p><h3>1. Information We Collect</h3><p>We collect information you provide when claiming a business or submitting a review, including name, email, and phone number.</p><h3>2. How We Use Information</h3><p>Your information is used to facilitate directory services, respond to inquiries, and improve our platform.</p><h3>3. Cookies</h3><p>We use essential cookies for site functionality. Analytics cookies are used to understand site usage.</p><h3>4. Data Sharing</h3><p>We do not sell your personal information to third parties.</p><h3>5. Your Rights</h3><p>You may request access to or deletion of your personal data at any time.</p><h3>6. Contact</h3><p>For privacy inquiries, visit our <a href="/legal/contact">Contact page</a>.</p>',
 true, true, 2),

('contact', 'Contact Us',
 '<h2>Contact ZaarHub</h2><p>We would love to hear from you. Reach out using any of the methods below.</p><h3>Email</h3><p><a href="mailto:swiftsoftware143@yahoo.com">swiftsoftware143@yahoo.com</a></p><h3>Response Time</h3><p>We aim to respond to all inquiries within 24-48 hours.</p>',
 true, true, 3)
ON CONFLICT (slug) DO NOTHING;

-- Seed site config
INSERT INTO zaarhub_site_config (site_name, site_tagline, contact_email)
VALUES ('ZaarHub', 'Discover Your Local Community', 'swiftsoftware143@yahoo.com')
ON CONFLICT DO NOTHING;
