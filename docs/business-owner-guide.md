# Business Owner Guide

Sign into your dashboard through your city's directory page at `directory.swiftsoftware.net`.

## Your Dashboard
Manage your listing, run deals, publish articles, engage with your community. The portal has tabs across the top: Dashboard, Credits, Vouchers, Referrals, Rewards, PIN, Scanner, Pledges, Offers, Legal, **Settings**, and Surveys.

## Connected Services — Unlock Campaigns & Bookings

Your portal can connect to two SwiftSoftware products that add powerful features to your listings. You need separate free accounts for each.

### Where to Find It
**Business Portal → ⚙️ Settings tab → scroll down → 🔗 Connected Services**

### Getting Your Free Accounts

| Service | Sign Up At | What You Get |
|---------|-----------|-------------|
| **IncentiveSwift** | `https://incentiveswift.com` | Loyalty campaigns, SMS funnels, smart surveys, rewards |
| **CoreSwift CRM** | `https://coreswiftcrm.com` | Calendar, bookings, contact management, CRM pipeline |

Use the same email as your directory account so the system auto-detects you. If emails don't match, the service connection won't work and you won't be able to link services. Both sites have a "Get Started" / "Free Trial" button — the signup form is on the app login page.

### Connecting IncentiveSwift

1. **Sign up** at `incentiveswift.com` → click "Get Started" → create your free account
2. Once signed in at `app.incentiveswift.com`, go to **Settings → API Keys** and generate an API key (use the same email as your ZaarHub account)
3. In your directory portal: **Settings tab → 🎯 IncentiveSwift card → Connect**
4. Paste your API key and click ✅ **Connect**
5. The system verifies your key — you're connected

### Connecting CoreSwift CRM

1. **Sign up** at `coreswiftcrm.com` → click "Free Trial" → create your account (use the same email as your ZaarHub account)
2. In your directory portal: **Settings tab → 📋 CoreSwift CRM card → Connect**
3. No API key needed — auto-detects your account
4. You're connected

### What You Unlock After Connecting

**IncentiveSwift — Campaign Linking**
- In your listing editor, pick any active campaign from a dropdown (fetched live from IncentiveSwift)
- The campaign is linked to your business and saved to your listing's meta fields
- Ready for your directory template to render as a CTA on your listing — all on-site

**CoreSwift CRM — Booking Integration**
- Toggle "Book Now" on your business listing
- Set booking type, duration, and buffer time
- Visitors book through a dedicated booking page hosted ON your directory site at `/book/:city/:business`
- The booking page pulls available slots from CoreSwift behind the scenes — visitor never leaves your directory

### How It Renders for Your Customers (All On-Site)

The directory keeps customers on your branded directory — no external redirects, no traffic loss:

| Feature | What Your Customer Sees | Where |
|---------|----------------------|-------|
| **Booking** | "Book Now" button → opens a booking page showing available slots, on your directory domain | Your directory site (`/book/:city/:business`) |
| **Campaign** | Campaign CTA on your listing — engagement is tracked and synced to IncentiveSwift via API | Your directory site (on-listing) |

### Important: These Are Separate Apps (For Management Only)

The campaign builder, calendar manager, and booking dashboard are managed in IncentiveSwift and CoreSwift CRM. The directory is the public-facing bridge — it renders these features on your listing without sending traffic elsewhere.

| Task | Where |
|------|-------|
| Create a loyalty campaign | IncentiveSwift (sign in at `app.incentiveswift.com`) |
| Build SMS funnels | IncentiveSwift |
| Manage your calendar & bookings | CoreSwift CRM (sign in at `app.coreswiftcrm.com`) |
| View/respond to bookings | CoreSwift CRM |
| Link campaigns to your listing | MultiDirectory (business portal) |
| Toggle Book Now on your listing | MultiDirectory (business portal) |
| Customers see/use these features | Your directory site (on-site, no redirects) |

### Disconnecting
Go to Settings tab → click **Disconnect** on any connected service. You can reconnect anytime.

---

## Bookmarks (Saves)
Visitors can bookmark your business by clicking the heart icon on your listing. Each bookmark is a saved lead — visitors who bookmark and return later are more likely to claim a deal or contact you. Check your bookmark count on your listing page.

## Running Deals
Create deals with templates — percentage off, fixed price, BOGO, free gift. Set colors, CTA text, countdown timer, gallery images. **Feature a deal on your listing** with a custom button text ("Deal of the Week 🔥", "Today's Special").

### Tier Limits
| Plan | Active Deals | Rotations |
|---|---|---|
| Free/Listed | 0 | 0 |
| Featured | 3 | 0 |
| Premium | 10 | 5 |

Set rotation schedules on Premium to auto-rotate deals daily, weekly, or monthly.

## Deal Pages
Each deal gets a public page at `zaarhub.com/deals/{id}` with live countdown timer, your branding, and a redeem button. Customers claim deals and get a code to use in-store.

## Community Posts
Share updates with your local community. Posts appear on your city's community page.

## B2B Suppliers
Browse the Suppliers tab in your city to find distributors, farms, and wholesalers. Search by type or product category.

### B2B Messaging — Contact Other Businesses
Once you find a supplier or partner you want to work with, you can message them directly — business-to-business, right within the directory.

**How it works:**
- Browse the **Suppliers** tab, find a business, click their listing
- A "Message" or "Contact" option lets you send a message as YOUR business (not as an individual)
- Your business name and email are used as the sender — the recipient sees your business profile
- The recipient gets a notification: "New message from [Your Business Name]"
- All B2B messages appear in their business dashboard inbox

**Your B2B inbox:**
- View all B2B messages from other businesses in your dashboard
- Unread badges, reply flow, and conversation history
- Mark messages read individually

## Community Polls
Participate in local polls created by your directory admin. Polls appear on your city page — vote on community topics and see live results.

## Managing Reviews
Reviews are social proof — they build trust and drive more visitors to your listing. All reviews go through moderation before appearing publicly.

### Review Notifications
- New reviews appear in your dashboard under the **Reviews** section
- Each review shows the reviewer name, star rating, title, and review text
- Reviews start in **pending** status — you'll see them flagged for action

### Approve or Reject
- **Approve** a review to make it visible on your public listing immediately
- **Reject** a review that is spam, abusive, or inappropriate — it stays hidden
- Approved reviews update your business's average rating and review count automatically

### Review Stats Dashboard
Your dashboard shows key metrics at a glance:
- **Average rating** — overall star rating across all approved reviews
- **Review count** — total number of approved reviews
- **Rating breakdown** — bar chart showing how many 5-star, 4-star, 3-star, 2-star, and 1-star reviews you have
- **Trend** — check if your rating is going up or down over time

### Responding to Reviews
- Reply to reviews to thank happy customers or address concerns
- Responses appear publicly under the review on your listing — it shows you're engaged
- A thoughtful response to a negative review can turn a detractor into a loyal customer

### Where Reviews Appear
- On your public business listing page in the Reviews section
- In the city-wide search results (rating stars next to your business name)
- On the ZaarHub homepage for featured businesses

## Business Messaging
Visitors can message you directly from your business listing. All messages land in your dashboard inbox.

### Message Inbox
- Access from your dashboard: **Messages** tab shows all conversations
- **Unread count badge** — a red badge shows how many new messages are waiting
- Click any conversation to view the full thread and sender details

### Reply Flow
- Open a message thread and type your reply
- Visitor sees your response in their conversation history
- Messages are text-based contact — build a relationship before they even call

### Message Notifications
- Badge on your dashboard indicates unread count
- Check back regularly — quick responses lead to more bookings and deals

## Service Catalog
Showcase exactly what you offer with a services catalog on your listing. Visitors can browse your services, see prices, and book or inquire.

### Adding Services
From your dashboard, navigate to your listing editor:
- Click **Add Service** to create a new service entry
- Fill in:
  - **Name** — what the service is (e.g., "Deep Cleaning" or "Oil Change")
  - **Description** — details about what's included
  - **Price** — optional fixed price or starting-at price
  - **Category** — group similar services (e.g., "Cleaning", "Repairs", "Consulting")
  - **Image** — optional photo of the service or result
- Edit or remove services anytime from the same editor

### Public Display
- Services appear in a dedicated **Services** tab on your business listing page
- Visitors can browse, compare, and contact you about specific services
- Each service shows its name, description, price, and category

## Community Events
Create events tied to your business — grand openings, workshops, seasonal specials, or community gatherings.

### Creating an Event
- From your dashboard, go to **Events** → **Create Event**
- Set:
  - **Title** — catchy event name (e.g., "Summer BBQ at Joe's")
  - **Description** — what to expect, what's included
  - **Event date & end time** — when it starts and ends
  - **Location & address** — where it's happening
  - **Category** — community, food, music, sports, business, workshop, or other
  - **Max attendees** — optional capacity limit
  - **Image** — event flyer or photo
- The event is automatically linked to your business

### Visibility
- Your event appears on the city's **Upcoming Events** section on the homepage
- Full event listing on the **/events** calendar page with all city events
- Shareable event detail page at `/events/:id` with an RSVP button

### Managing Your Events
- Edit event details anytime if plans change
- Cancel an event — it'll show as cancelled and notify RSVP'd attendees
- Track RSVPs: see who's attending and how many spots are filled

## Analytics
Track listing views, deal claims, and engagement.

## CTA Buttons — Set Your Call-to-Action

Your business listing can show a button that tells visitors exactly what to do next. This is your "call to action" or CTA — the most important button on your listing.

### What Each CTA Type Does

You choose one CTA type for each listing. Here's what each option does when a visitor clicks it:

| CTA Type | What the Visitor Sees |
|----------|----------------------|
| **Get a Quote** | Opens a form where the visitor can request a price estimate — you get the request in your inbox |
| **Book Now** | Opens a booking page where visitors can pick a time on your calendar — synced from CoreSwift CRM if connected |
| **Call Now** | Taps to call your business phone number — works on mobile |
| **Visit Website** | Opens your website in a new browser tab |
| **Message Us** | Opens the messaging form — the message lands in your dashboard inbox |
| **Join Rewards** | Links to your loyalty or rewards campaign — synced from IncentiveSwift if connected |
| **Claim Deal** | Takes visitors to your active deal or special offer |
| **View Menu** | Opens your menu page (great for restaurants) |
| **Get Directions** | Opens a map with driving directions to your business |
| **Email Us** | Opens the visitor's email app with your business address filled in |
| **Download App** | Links to your mobile app in the app store |
| **Donate** | Opens a donation page (for nonprofits and charities) |
| **Register** | Opens a registration form (for classes, workshops, or events) |

### How to Set Your CTA

1. Go to your **Business Portal** and open your listing editor
2. Find the **CTA Type** dropdown in your listing settings
3. Pick the CTA that matches how you want customers to reach you
4. Save your listing — the button appears on your public listing immediately

### Gated CTAs (Login Required)

Some CTAs can be set as "gated" — meaning visitors must log in or create an account before using them. This is useful for:
- **Book Now** — saves the booking to the visitor's account
- **Get a Quote** — tracks quote history for the visitor
- **Register** — ensures the visitor's info is captured

If you choose a gated CTA, visitors see a friendly login prompt when they click the button. Once they sign in (or create a free account), they're returned to complete their action.

### Which CTA Should You Choose?

- **Restaurants** → View Menu, Call Now, or Book Now
- **Plumbers/Contractors** → Get a Quote or Call Now
- **Salons/Spas** → Book Now
- **Retail shops** → Visit Website or Call Now
- **Nonprofits** → Donate
- **Gyms/Studios** → Register or Book Now

You can change your CTA anytime. Test different CTAs and see which one brings you the most leads.

## Unified Inbox — Messages in CoreSwift CRM

If you've connected CoreSwift CRM to your business, every message visitors send through the directory also appears in your CoreSwift inbox. You don't have to check two places.

### How It Works

1. A visitor clicks **Message** on your business listing and sends a message
2. The message appears in your directory dashboard inbox (the **Messages** tab)
3. Instantly, the same message is forwarded to your CoreSwift CRM inbox via webhook
4. You can read and reply from either place — CoreSwift CRM or your directory dashboard

### What Syncs

- **New messages** — forwarded from directory to CoreSwift CRM in real time
- **Sender info** — visitor's name, email, and message content all included
- **Reply tracking** — if you reply from CoreSwift CRM, it syncs back to the directory thread

### Why This Matters

- If you already use CoreSwift CRM for contacts and pipeline, you see directory messages alongside everything else
- Build a contact profile automatically — every visitor who messages you creates a contact record
- No double-checking — one inbox for your website, directory, and direct messages

### Setup

No extra setup needed — connect CoreSwift CRM in your **Settings → Connected Services**, and the unified inbox is automatically active. The directory sends messages to CoreSwift via the `/api/messages/webhook` endpoint.

## Sponsors & Ads — Promote Your Business

Get your business seen by more visitors with sponsored placements. Sponsors appear prominently on directory pages — like a billboard on a busy street.

### How Sponsorship Works

1. Go to **Sponsors** in your business portal
2. Choose a sponsorship slot — positions vary by price and visibility:
   - **Homepage banner** — top of the city page, seen by every visitor
   - **Sidebar ad** — appears on search results and category pages
   - **Featured listing** — your business gets a "Featured" badge and appears at the top of search results
   - **Category sponsor** — your ad shows when visitors browse a specific category
3. Upload your ad creative — an image, headline, and destination link
4. Pay for your sponsorship — pricing is set by the directory admin
5. Your ad goes live once approved by the directory moderator

### Viewing Your Ad Stats

- **Impressions** — how many times your ad was shown
- **Clicks** — how many visitors clicked your ad
- **Click-through rate** — percentage of viewers who clicked
- **Cost per click** — what you're paying per visitor who clicks

### Managing Your Sponsorships

- See all your active and past sponsorships in the portal
- Pause or cancel a sponsorship anytime
- Renew expiring sponsorships before they end
- Upload updated creatives for ongoing campaigns

> **Tip:** Sponsorships work best when combined with deals. Run a deal AND sponsor your listing for maximum visibility.

## Landing Pages — Your Own Micro-Site

Create a dedicated landing page for your business — a mini-website hosted on the directory, with its own custom URL.

### What You Get

- A full-page website for your business, separate from your directory listing
- Choose from pre-built **themes** that match different industries (restaurant, salon, contractor, retail)
- Custom URL like `zaarhub.com/your-business-name`
- Add photos, services list, testimonials, contact form, and more

### Creating Your Landing Page

1. Go to **Landing Pages** in your business portal
2. Click **Create Page**
3. Pick a theme from the library — preview how each one looks before choosing
4. Fill in your content:
   - Business name and tagline
   - About section with your story
   - Services or products with photos and prices
   - Customer testimonials (pulled from your approved reviews)
   - Contact information and a message form
   - Links to your social media
5. Click **Publish** — your page is live

### Managing Your Page

- Edit your landing page anytime — changes go live instantly
- Update your theme without losing content
- Track page views and form submissions
- Unpublish temporarily if you're making big changes

### Why Create a Landing Page?

- **More SEO** — a dedicated page with rich content ranks better in search engines than a standard listing
- **More control** — tell your full story, not just the listing basics
- **Better conversions** — visitors who land on a dedicated page are more likely to contact you
- **Shareable** — send your custom URL in emails, on social media, or in ads

## Call Tracking — Know Who's Calling

Get a unique tracking phone number for your business listing. Every call made to this number is logged — you'll see who called, when, and how long they talked.

### How Call Tracking Works

1. In your business portal, go to **Call Tracking**
2. Click **Get a Tracking Number** — you'll receive a unique phone number
3. This number displays on your public business listing instead of your real number
4. When a visitor calls it, the call is automatically forwarded to your real phone — the caller doesn't know the difference
5. Every call is logged with: date, time, duration, caller phone number (if available), and call status

### Viewing Call Logs

- Open **Call Tracking** in your portal to see all your calls
- Sort by date, duration, or status
- See call patterns — what days and times you get the most calls
- Export call logs to CSV for your records

### Call Stats Dashboard

Your call tracking dashboard shows:
- **Total calls** — how many calls you've received through your tracking number
- **Average duration** — how long callers stay on the line
- **Peak days/times** — when you should be ready to answer
- **Missed calls** — calls you didn't pick up, with caller info if available

### Why Use Call Tracking?

- **Measure your directory ROI** — know exactly how many calls your listing generates
- **Never miss a lead** — missed call logs show you who to call back
- **Train your staff** — see if calls are being answered properly
- **Optimize your listing** — if you're not getting calls, try a different CTA or add a deal

> **Note:** Call tracking availability depends on your directory's phone number pool. The admin sets up the pool — you request a number from what's available.

## Email Campaigns — Reach Your Customers

Send targeted emails to visitors who've shown interest in your business — people who bookmarked you, claimed a deal, or attended your event.

### Creating an Email Campaign

1. Go to **Email Campaigns** in your business portal
2. Click **Create Campaign**
3. Choose a template or start from scratch:
   - **Announcement** — new product, service, or location
   - **Special Offer** — limited-time deal or discount
   - **Event Invite** — promote your upcoming event
   - **Newsletter** — regular updates about your business
4. Write your email content — subject line, body text, and a call-to-action button
5. Choose your audience — target visitors by:
   - **Tag** — send to visitors tagged with specific interests (e.g., "foodie", "fitness")
   - **Action** — send to visitors who bookmarked you, claimed a deal, or attended an event
   - **Location** — target visitors in specific cities or zip codes
6. Preview and send — or schedule for later

### Tracking Results

- **Sent** — how many emails went out
- **Opened** — how many people opened your email
- **Clicked** — how many clicked your link or CTA button
- **Unsubscribed** — how many opted out (keeps you compliant)

### Best Practices

- Send no more than 1–2 emails per week — avoid overwhelming your audience
- Always include an unsubscribe link (the system adds this automatically)
- Write short, clear subject lines that tell the reader what's inside
- Include one clear call-to-action per email — don't ask for too many things

## CRM Pipeline — Manage Your Leads

When you connect CoreSwift CRM, every interaction on your directory listing can feed into your CRM pipeline. Turn casual visitors into tracked leads — and leads into customers.

### What the Pipeline Tracks

| Lead Source | What's Created in CRM |
|-------------|----------------------|
| **Visitor messages you** | New contact record with message history |
| **Visitor books an appointment** | Contact + deal in "New Lead" stage |
| **Visitor claims a deal** | Contact + deal in "Interested" stage |
| **Visitor requests a quote** | Contact + deal in "Quote Sent" stage |
| **Visitor RSVPs to your event** | Contact tagged with your event name |

### Pipeline Stages

Your CRM pipeline has standard stages you can customize:

1. **New Lead** — someone showed interest (messaged, bookmarked, or claimed a deal)
2. **Contacted** — you've reached out to them
3. **Quote Sent** — you sent a price or proposal
4. **Negotiating** — working out details
5. **Won** — they became a paying customer 🎉
6. **Lost** — they went elsewhere (but you can still re-engage later)

### Managing Your Pipeline

- Log into CoreSwift CRM at `app.coreswiftcrm.com` to manage your full pipeline
- Drag-and-drop deals between stages as they progress
- Add notes, schedule follow-ups, and set reminders
- Your directory dashboard shows a summary: total leads, deals in progress, and recent activity

### Why Use the Pipeline

- **Never drop a lead** — every visitor interaction is tracked
- **Know your conversion rate** — see how many leads turn into customers
- **Automate follow-ups** — set reminders so you don't forget to call back
- **Report on ROI** — prove your directory listing is paying for itself
