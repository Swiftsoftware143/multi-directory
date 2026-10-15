/**
 * Durable proof (browser arm) — item 1, second half: a business OWNER can REPLY to a review of
 * their own listing and REJECT another, from the real Business Portal screen, and the reply is then
 * visible to a logged-out visitor on the public listing page.
 *
 * The defect class this pins: the business-side reviews screen existing only as an API (or the
 * portal rendering Reply/Reject controls that do nothing), so "a business can handle its reviews"
 * is never actually reachable on screen. An API-only check cannot see that.
 *
 * Drives the real pages:
 *   frontend/business-portal.html  (served at /portal) — login form, Reviews view, controls
 *   frontend/business-detail.html  (served at /biz/:id) — the public review + owner-reply display
 *
 * Env: PORTAL_URL, OWNER_EMAIL, PW, RID_A, RID_B, PUBLIC_URL, REVIEW_MARK, REPLY_MARK, SHOT_DIR,
 *      CHROME_PATH, NODE_PATH
 * Exit: 0 = all browser checks passed, 1 = a check failed (or the flow threw).
 */
const path = require('path');
const puppeteer = require('puppeteer-core');

const PORTAL = process.env.PORTAL_URL || 'http://127.0.0.1:8089/portal';
const EMAIL = process.env.OWNER_EMAIL;
const PW = process.env.PW;
const RID_A = process.env.RID_A;
const RID_B = process.env.RID_B;
const PUBLIC_URL = process.env.PUBLIC_URL;
const REVIEW_MARK = process.env.REVIEW_MARK || '';
const REPLY_MARK = process.env.REPLY_MARK || '';
const SHOT_DIR = process.env.SHOT_DIR || '/tmp';

(async () => {
  const problems = [];
  const ok = (m) => console.log('PASS  ' + m);
  const bad = (m) => { problems.push(m); console.log('FAIL  ' + m); };

  const browser = await puppeteer.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1360, height: 1000 });

  const consoleErrors = [];
  page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text()); });
  page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));
  const badResponses = [];
  page.on('response', (r) => {
    if (r.status() >= 400 && r.url().includes('/api/v1/')) {
      badResponses.push(r.status() + ' ' + r.url().replace(/^https?:\/\/[^/]+/, ''));
    }
  });

  try {
    // ── 1. sign in through the portal's OWN login form ─────────────────────────
    await page.goto(PORTAL, { waitUntil: 'networkidle2', timeout: 60000 });
    await page.waitForSelector('#loginEmail', { timeout: 20000 });
    await page.type('#loginEmail', EMAIL);
    await page.type('#loginPassword', PW);
    await page.evaluate(() => {
      const b = [...document.querySelectorAll('button')].find((x) => /sign in/i.test(x.textContent || ''));
      if (b) b.click();
    });
    await page.waitForFunction(() => {
      const d = document.getElementById('dashboard');
      const c = document.getElementById('dashboardContent');
      return !!d && d.style.display === 'block' && !!c && c.style.display !== 'none';
    }, { timeout: 30000 });
    ok('owner signed in through the real portal login form and reached the dashboard');

    await page.evaluate(() => showPortalView('reviews'));
    await page.waitForFunction((rid) => {
      const box = document.getElementById('rvList');
      return !!box && box.innerHTML.includes(rid);
    }, { timeout: 25000 }, RID_A);

    // ── 2. REPLY to review A ──────────────────────────────────────────────────
    const replyControls = await page.evaluate((rid) => {
      const box = document.getElementById('rvList');
      const ta = document.getElementById('rvReply_' + rid);
      const btn = [...box.querySelectorAll('button')]
        .find((b) => (b.getAttribute('onclick') || '').includes('postReviewReply') && (b.getAttribute('onclick') || '').includes(rid));
      return { hasTa: !!ta, hasBtn: !!btn, label: btn ? (btn.textContent || '').trim() : '' };
    }, RID_A);
    replyControls.hasTa ? ok('the review renders a public-reply box on screen')
                        : bad('no reply box rendered for the review');
    replyControls.hasBtn ? ok('the review renders a Post-reply control  [label=' + replyControls.label + ']')
                         : bad('no Post-reply control rendered for the review');

    await page.evaluate((rid, text) => {
      const ta = document.getElementById('rvReply_' + rid);
      if (ta) ta.value = text;
    }, RID_A, REPLY_MARK);

    const replyResp = page.waitForResponse(
      (r) => r.url().includes('/reviews/' + RID_A + '/respond') && r.status() === 200,
      { timeout: 25000 });
    await page.evaluate((rid) => {
      const box = document.getElementById('rvList');
      const btn = [...box.querySelectorAll('button')]
        .find((b) => (b.getAttribute('onclick') || '').includes('postReviewReply') && (b.getAttribute('onclick') || '').includes(rid));
      btn.click();
    }, RID_A);
    await replyResp;
    ok('clicking the portal\'s Post-reply control called POST /reviews/:id/respond -> 200');
    await page.screenshot({ path: path.join(SHOT_DIR, 'portal-reply.png') }).catch(() => {});

    // ── 3. APPROVE review A so the reply reaches the public page ──────────────
    await page.waitForFunction((rid) => {
      const box = document.getElementById('rvList');
      return [...box.querySelectorAll('button')]
        .some((b) => /approve/i.test(b.textContent || '') && (b.getAttribute('onclick') || '').includes(rid));
    }, { timeout: 25000 }, RID_A);
    const approveResp = page.waitForResponse(
      (r) => r.url().includes('/business/reviews/' + RID_A + '/approve') && r.status() === 200,
      { timeout: 25000 });
    await page.evaluate((rid) => {
      const box = document.getElementById('rvList');
      const btn = [...box.querySelectorAll('button')]
        .find((b) => /approve/i.test(b.textContent || '') && (b.getAttribute('onclick') || '').includes(rid));
      btn.click();
    }, RID_A);
    await approveResp;
    ok('the owner approved the replied review from the screen (approve endpoint 200)');

    // ── 4. REJECT review B from the same screen ──────────────────────────────
    await page.waitForFunction((rid) => {
      const box = document.getElementById('rvList');
      return [...box.querySelectorAll('button')]
        .some((b) => /reject/i.test(b.textContent || '') && (b.getAttribute('onclick') || '').includes(rid));
    }, { timeout: 25000 }, RID_B);
    const rejectResp = page.waitForResponse(
      (r) => r.url().includes('/business/reviews/' + RID_B + '/reject') && r.status() === 200,
      { timeout: 25000 });
    await page.evaluate((rid) => {
      const box = document.getElementById('rvList');
      const btn = [...box.querySelectorAll('button')]
        .find((b) => /reject/i.test(b.textContent || '') && (b.getAttribute('onclick') || '').includes(rid));
      btn.click();
    }, RID_B);
    await rejectResp;
    ok('clicking Reject in the portal called POST /business/reviews/:id/reject -> 200');

    await page.waitForFunction((rid) => {
      const box = document.getElementById('rvList');
      return /rejected/i.test(box.innerHTML);
    }, { timeout: 25000 }, RID_B);
    ok('the rejected review now renders as rejected on screen');
    await page.screenshot({ path: path.join(SHOT_DIR, 'portal-reject.png') }).catch(() => {});

    // ── 5. the reply is visible to a LOGGED-OUT visitor on the public listing ─
    const pub = await browser.newPage();
    await pub.setViewport({ width: 1360, height: 1000 });
    const pubErrors = [];
    pub.on('pageerror', (e) => pubErrors.push(String(e).slice(0, 120)));
    await pub.goto(PUBLIC_URL, { waitUntil: 'networkidle2', timeout: 45000 });
    await pub.waitForFunction(() => {
      const c = document.getElementById('reviewsContent');
      return !!c && c.innerHTML.trim().length > 0;
    }, { timeout: 25000 });
    const rendered = await pub.evaluate(() => document.getElementById('reviewsContent').innerText);
    rendered.includes(REPLY_MARK)
      ? ok('the owner\'s reply is rendered on the public listing for a logged-out visitor')
      : bad('the public listing did not render the owner reply');
    /Response from the business/i.test(rendered)
      ? ok('the public listing labels it as a response from the business')
      : bad('the public listing has no "Response from the business" label');
    pubErrors.length ? bad('public page console errors: ' + pubErrors.slice(0, 2).join(' | '))
                     : ok('the public listing rendered with no script errors');
    await pub.screenshot({ path: path.join(SHOT_DIR, 'public-reply.png') }).catch(() => {});
    await pub.close();

    if (badResponses.length) bad('browser saw failing API calls: ' + JSON.stringify(badResponses.slice(0, 6)));
    if (consoleErrors.length) bad('console errors: ' + consoleErrors.slice(0, 3).join(' | '));
    else ok('no JavaScript console errors during the portal flow');
  } catch (e) {
    bad('browser flow threw: ' + e.message);
    try { await page.screenshot({ path: path.join(SHOT_DIR, 'portal-reply-error.png') }); } catch (_) {}
  }

  await browser.close();
  console.log('BROWSER RESULT: ' + (problems.length ? problems.length + ' failed' : 'all passed'));
  process.exit(problems.length ? 1 : 0);
})();
