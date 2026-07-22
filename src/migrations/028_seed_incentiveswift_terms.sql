-- Seed IncentiveSwift Loyalty Program Terms as a global legal page
-- This runs after the legal_pages table exists (migration 003).
-- Only inserts if no IncentiveSwift terms already exist.

INSERT INTO legal_pages (title, page_type, content, published, is_global)
SELECT 'IncentiveSwift Loyalty Program Terms', 'terms',
'<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>IncentiveSwift Loyalty Program — Terms &amp; Conditions</title>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body { font-family: -apple-system, BlinkMacSystemFont, ''Segoe UI'', Roboto, Oxygen, sans-serif; background: #f0fdfa; color: #0f172a; line-height: 1.7; }
    .container { max-width: 860px; margin: 0 auto; padding: 48px 24px; }
    h1 { font-size: 1.8rem; color: #0f172a; margin-bottom: 6px; }
    .subtitle { color: #64748b; font-size: 0.95rem; margin-bottom: 32px; }
    h2 { font-size: 1.25rem; color: #0d9488; margin: 36px 0 14px; padding-bottom: 6px; border-bottom: 2px solid #ccfbf1; }
    h3 { font-size: 1.05rem; color: #0f172a; margin: 24px 0 10px; }
    p { margin-bottom: 14px; color: #334155; }
    ul { margin: 10px 0 14px 22px; }
    ul li { margin-bottom: 8px; color: #334155; }
    .terms-card { background: #fff; border-radius: 14px; padding: 44px; box-shadow: 0 1px 4px rgba(0,0,0,0.06); border: 1px solid #e2e8f0; }
    .highlight { background: #f0fdfa; border-left: 4px solid #0d9488; padding: 14px 18px; margin: 16px 0; border-radius: 0 8px 8px 0; }
    .highlight p { margin: 0; }
    .highlight strong { color: #0d9488; }
    .warning { background: #fef2f2; border-left: 4px solid #dc2626; padding: 14px 18px; margin: 16px 0; border-radius: 0 8px 8px 0; }
    .warning p { margin: 0; }
    .warning strong { color: #dc2626; }
    hr { border: none; border-top: 1px solid #e2e8f0; margin: 28px 0; }
    .audience-badge { display: inline-block; font-size: 0.75rem; font-weight: 600; padding: 4px 12px; border-radius: 20px; margin-left: 8px; vertical-align: middle; }
    .badge-participant { background: #dbeafe; color: #1e40af; }
    .badge-merchant { background: #fef3c7; color: #92400e; }
    .badge-supplier { background: #ede9fe; color: #5b21b6; }
    .badge-admin { background: #fce7f3; color: #9d174d; }
    .toc { background: #f8fafc; border: 1px solid #e2e8f0; border-radius: 10px; padding: 20px 24px; margin: 20px 0; }
    .toc h3 { margin: 0 0 10px; font-size: 0.95rem; color: #0d9488; }
    .toc ol { margin: 0; }
    .toc li { margin-bottom: 6px; font-size: 0.9rem; }
    .toc li span { color: #94a3b8; font-size: 0.8rem; }
    @media (max-width: 600px) {
      .container { padding: 24px 16px; }
      .terms-card { padding: 24px 16px; }
      h1 { font-size: 1.4rem; }
    }
  </style>
</head>
<body>
  <div class="container">
    <div class="terms-card">
      <h1>IncentiveSwift Loyalty Program</h1>
      <p style="font-size:1.05rem;color:#0f172a;font-weight:500;margin-bottom:4px;">Terms &amp; Conditions</p>
      <p class="subtitle">Last updated: July 21, 2026</p>

      <p>Welcome to the IncentiveSwift loyalty program (the &ldquo;<strong>Program</strong>&rdquo;), operated by <strong>SwiftSoftware LLC</strong> (&ldquo;<strong>SwiftSoftware</strong>,&rdquo; &ldquo;<strong>we</strong>,&rdquo; &ldquo;<strong>us</strong>,&rdquo; &ldquo;<strong>our</strong>&rdquo;). These Terms &amp; Conditions (&ldquo;<strong>Terms</strong>&rdquo;) govern your participation in the Program, whether you are an end customer earning and redeeming credits, a merchant business offering rewards, or a supplier in the network.</p>
      <p><strong>By participating in the Program, you agree to these Terms.</strong> If you do not agree, do not use the Program or any related services.</p>

      <div class="toc">
        <h3>Contents</h3>
        <ol>
          <li><a href="#section-i">Section I &mdash; Participant Terms</a> <span>for end customers</span></li>
          <li><a href="#section-ii">Section II &mdash; Merchant Terms</a> <span>for businesses</span></li>
          <li><a href="#section-iii">Section III &mdash; Supplier Terms</a> <span>for suppliers</span></li>
          <li><a href="#section-iv">Section IV &mdash; Program Administration</a> <span>ZaarHub / SwiftSoftware</span></li>
        </ol>
      </div>

      <hr>

      <h2 id="section-i">Section I &mdash; Participant Terms <span class="audience-badge badge-participant">For Customers</span></h2>
      <p>This section applies to you if you are an <strong>end customer</strong> who earns and redeems credits in the IncentiveSwift network (a &ldquo;<strong>Participant</strong>&rdquo;).</p>

      <h3>1. Earning Credits</h3>
      <p>Participants earn credits when making qualifying purchases at participating merchants. Credits are calculated as:</p>
      <div class="highlight">
        <p><strong>Credits Earned = Purchase Amount &times; Merchant&rsquo;s Credit Rate</strong></p>
        <p style="font-size:0.9rem;margin-top:4px;color:#64748b;">Example: $20 purchase at a merchant with a 10 credit/$1 rate = 200 credits earned.</p>
      </div>
      <p>Each merchant sets its own credit rate, which is displayed in the merchant&rsquo;s portal. Credits are added to the Participant&rsquo;s account automatically upon successful purchase verification by the merchant.</p>

      <h3>2. Credit Balance &amp; Expiration</h3>
      <p><strong>Credits never expire.</strong> There is no minimum balance requirement and no inactivity timeout that causes forfeiture. However, SwiftSoftware reserves the right to modify this policy with 30 days&rsquo; written notice (see Section IV).</p>
      <p>Credits are <strong>non-transferable</strong>. They are tied to your individual account and may not be sold, traded, gifted, or combined with another Participant&rsquo;s balance.</p>

      <h3>3. Redeeming Credits</h3>
      <p>Participants may redeem credits at any participating merchant that has an active offer. The redemption process:</p>
      <ol>
        <li>Present your unique QR code at checkout.</li>
        <li>The merchant selects a <strong>redeem mode</strong> and chooses an applicable offer.</li>
        <li>The system calculates the discount: the lesser of (a) the offer&rsquo;s percentage of the purchase amount, or (b) the offer&rsquo;s dollar cap, <strong>limited to</strong> the Participant&rsquo;s available credit balance.</li>
        <li>Credits are deducted from the Participant&rsquo;s balance, and the merchant honors the discount.</li>
      </ol>
      <div class="highlight">
        <p><strong>Example:</strong> Offer is &ldquo;25% off up to $6.&rdquo; Purchase is $30. You have 400 credits ($4). Discount = min(min(25% &times; $30, $6), $4) = $4. You pay the remaining $26.</p>
      </div>

      <h3>4. No Cash Value</h3>
      <p>Credits have <strong>no cash value</strong>. They are a promotional reward only and cannot be exchanged for cash, refunds, or any form of monetary compensation. Credits are not legal tender and are not redeemable for cash from SwiftSoftware or any merchant.</p>

      <h3>5. No Double Stacking</h3>
      <div class="warning">
        <p><strong>One transaction = earn OR redeem &mdash; never both.</strong> On any single purchase, the Participant may either earn credits (building their balance) or redeem credits (spending their balance). The merchant selects the mode at the time of scanning. This rule prevents double-dipping and ensures fair use of the Program.</p>
      </div>

      <h3>6. QR Code</h3>
      <p>Each Participant is issued a unique QR code that serves as their account identifier. The QR code:</p>
      <ul>
        <li>Is permanent and does not change.</li>
        <li>Contains only a unique anonymous identifier &mdash; no personal information is encoded.</li>
        <li>Must be presented in person at the point of sale to earn or redeem credits.</li>
        <li>Should not be shared with unauthorized parties. SwiftSoftware is not responsible for unauthorized use of your QR code.</li>
      </ul>

      <h3>7. Fraud &amp; Abuse</h3>
      <p>SwiftSoftware reserves the right to investigate any suspected fraud, abuse, or unusual activity. Prohibited conduct includes but is not limited to:</p>
      <ul>
        <li>Creating multiple accounts to earn additional credits.</li>
        <li>Using another person&rsquo;s QR code without authorization.</li>
        <li>Exploiting system errors or bugs to obtain unauthorized credits.</li>
        <li>Colluding with a merchant to generate fraudulent transactions.</li>
      </ul>
      <p>Violation may result in forfeiture of credits, account suspension, or permanent ban from the Program.</p>

      <hr>

      <h2 id="section-ii">Section II &mdash; Merchant Terms <span class="audience-badge badge-merchant">For Businesses</span></h2>
      <p>This section applies to you if you are a <strong>business or merchant</strong> that enrolls in the Program to offer rewards to customers (a &ldquo;<strong>Merchant</strong>&rdquo;).</p>

      <h3>1. Enrollment &amp; Configuration</h3>
      <p>Upon enrollment, each Merchant receives a business portal account with access to loyalty settings, including credit rate configuration, offer management, and a customer QR scanner. The Merchant is responsible for maintaining the confidentiality of their portal login credentials and their Purchase PIN.</p>

      <h3>2. Credit Rate</h3>
      <p>Each Merchant sets their own <strong>credit rate</strong> &mdash; the number of credits a Participant earns per dollar spent at that business. The Merchant may change this rate at any time from their portal, and changes take effect immediately for all <strong>future</strong> purchases. The Merchant acknowledges that:</p>
      <ul>
        <li>A higher credit rate attracts more customers but costs the Merchant more in deferred value.</li>
        <li>The Merchant is responsible for funding the credits earned by Participants at their establishment.</li>
        <li>Credits issued by the Merchant may be redeemed at <strong>any</strong> participating merchant in the network, not only at the issuing Merchant.</li>
      </ul>

      <h3>3. Creating &amp; Managing Offers</h3>
      <p>Merchants may create <strong>offers</strong> &mdash; discounts that Participants can redeem their credits against. Each offer consists of:</p>
      <ul>
        <li><strong>Name</strong> &mdash; a descriptive title (e.g., &ldquo;25% off up to $6&rdquo;).</li>
        <li><strong>Discount Percentage</strong> &mdash; the percentage off the purchase total.</li>
        <li><strong>Cap ($)</strong> &mdash; the maximum dollar value of the discount.</li>
      </ul>
      <p>Merchants may create multiple offers simultaneously. All offers are subject to the <strong>No Double Stacking</strong> rule (Section I, &sect;5).</p>

      <h3>4. Obligation to Honor Redemptions</h3>
      <p>When a Participant presents a valid QR code and selects a Merchant&rsquo;s active offer:</p>
      <ul>
        <li>The Merchant <strong>must</strong> honor the discount calculated by the system.</li>
        <li>The Merchant may not refuse a valid redemption unless there is a legitimate business reason (e.g., the customer is violating other store policies).</li>
        <li>The discount is applied at the time of purchase. The Merchant may not require additional conditions beyond what is stated in the offer.</li>
      </ul>

      <h3>5. Deactivating Offers</h3>
      <p>Merchants may deactivate any of their offers at any time from their business portal. Deactivated offers are immediately hidden from the scanner and cannot be selected for new redemptions. Transactions already in progress at the time of deactivation may still be completed.</p>

      <h3>6. Purchase PIN</h3>
      <p>Each Merchant is assigned a unique <strong>Purchase PIN</strong> (6-digit code). The Merchant must:</p>
      <ul>
        <li>Keep the PIN confidential and accessible only to authorized staff.</li>
        <li>Have the PIN available at the point of sale for verification.</li>
        <li>Not share the PIN with customers or non-authorized personnel.</li>
      </ul>

      <h3>7. Non-Restriction</h3>
      <p>Merchants may not restrict or discourage Participants from using credits earned at their business at other merchants. The Program is designed as an open network, and participants must be free to earn and redeem wherever they choose.</p>

      <h3>8. Transaction Authentication</h3>
      <p>All transactions must be authenticated through the Merchant&rsquo;s portal by scanning the Participant&rsquo;s QR code or, if scanning is unavailable, by manually entering the Participant&rsquo;s account identifier. Merchants may not process transactions without proper authentication.</p>

      <h3>9. Merchant Compliance</h3>
      <p>Merchants who violate these Terms &mdash; including but not limited to refusing valid redemptions, engaging in fraudulent transactions, or attempting to manipulate the credit system &mdash; may be subject to suspension or permanent removal from the Program.</p>

      <hr>

      <h2 id="section-iii">Section III &mdash; Supplier Terms <span class="audience-badge badge-supplier">For Suppliers</span></h2>
      <p>This section applies to you if you are a <strong>supplier, vendor, or service provider</strong> that participates in the IncentiveSwift network (a &ldquo;<strong>Supplier</strong>&rdquo;). Suppliers operate under substantially the same terms as Merchants, with the following clarifications:</p>

      <h3>1. Relationship</h3>
      <p>Suppliers are treated as merchants for purposes of the Program. All Merchant Terms in Section II apply to Suppliers unless explicitly modified below.</p>

      <h3>2. Supplier-Specific Obligations</h3>
      <ul>
        <li>Suppliers may offer credits to their own customers (e.g., B2B clients, distributors) at a rate they determine.</li>
        <li>Suppliers may create offers that their customers can redeem at the Supplier&rsquo;s own outlet or service point.</li>
        <li>Suppliers are responsible for the same obligations regarding credit funding, offer honor, and transaction authentication as Merchants.</li>
      </ul>

      <h3>3. No Cross-Network Redemption</h3>
      <p>Unless otherwise agreed in writing, credits earned through a Supplier may only be redeemed at that Supplier&rsquo;s own offers or at other explicitly authorized redemption points. Suppliers are not automatically part of the open merchant redemption network unless they opt in.</p>

      <h3>4. Supplier Termination</h3>
      <p>Suppliers may withdraw from the Program with 14 days&rsquo; written notice. Upon withdrawal, the Supplier must settle all outstanding credit obligations with their customers before termination takes effect.</p>

      <hr>

      <h2 id="section-iv">Section IV &mdash; Program Administration <span class="audience-badge badge-admin">ZaarHub / SwiftSoftware</span></h2>
      <p>This section governs the administration of the Program by SwiftSoftware and the <strong>ZaarHub</strong> platform.</p>

      <h3>1. Platform Operator</h3>
      <p>The IncentiveSwift Program is operated by <strong>SwiftSoftware LLC</strong>. The Program is managed on the <strong>ZaarHub</strong> platform, which provides the technical infrastructure, merchant portal, customer API, and administration tools. ZaarHub administers the network on behalf of SwiftSoftware.</p>

      <h3>2. Modifications to Terms</h3>
      <p>SwiftSoftware reserves the right to modify these Terms at any time. Participants and Merchants will be notified of material changes via email and/or in-platform notification at least <strong>30 days</strong> before changes take effect. Continued participation after the effective date constitutes acceptance of the modified Terms.</p>

      <h3>3. Account Suspension &amp; Termination</h3>
      <p>SwiftSoftware and ZaarHub administrators reserve the right to:</p>
      <ul>
        <li>Suspend or terminate any Participant, Merchant, or Supplier account for violation of these Terms.</li>
        <li>Investigate suspicious activity and freeze accounts or credits during investigation.</li>
        <li>Reverse or adjust any transaction found to be fraudulent, erroneous, or abusive.</li>
        <li>Permanently ban accounts involved in serious or repeated violations.</li>
      </ul>

      <h3>4. Limitation of Liability</h3>
      <p>To the maximum extent permitted by applicable law:</p>
      <ul>
        <li>The Program is provided on an &ldquo;as is&rdquo; and &ldquo;as available&rdquo; basis without warranties of any kind, express or implied.</li>
        <li>SwiftSoftware shall not be liable for any indirect, incidental, special, consequential, or punitive damages arising from or related to the use of or inability to use the Program.</li>
        <li>SwiftSoftware&rsquo;s total liability for any claim arising under these Terms shall not exceed the total value of credits in the claimant&rsquo;s account at the time of the claim.</li>
        <li>SwiftSoftware is not responsible for disputes between Participants and Merchants. Such disputes should be resolved directly between the parties.</li>
      </ul>

      <h3>5. Dispute Resolution</h3>
      <p>Disputes arising under these Terms shall first be submitted to SwiftSoftware for informal resolution. If the dispute cannot be resolved informally within 30 days, either party may pursue remedies available under applicable law.</p>

      <h3>6. Governing Law</h3>
      <p>These Terms are governed by the laws of the <strong>State of Delaware</strong>, United States, without regard to its conflict of laws principles.</p>

      <h3>7. Severability</h3>
      <p>If any provision of these Terms is held to be invalid or unenforceable, the remaining provisions shall continue in full force and effect.</p>

      <h3>8. Contact</h3>
      <p>For questions about these Terms or the Program, please contact:</p>
      <ul>
        <li><strong>Email:</strong> support@zaarhub.com</li>
        <li><strong>Platform:</strong> ZaarHub.com</li>
      </ul>

      <hr>
      <p style="font-size:0.85rem;color:#94a3b8;text-align:center;margin-top:24px;">&copy; 2026 SwiftSoftware LLC &mdash; All rights reserved.</p>
    </div>
  </div>
</body>
</html>',
TRUE, TRUE
WHERE NOT EXISTS (
  SELECT 1 FROM legal_pages WHERE title = 'IncentiveSwift Loyalty Program Terms'
);
