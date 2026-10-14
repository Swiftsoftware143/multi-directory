/**
 * Durable proof (browser arm) — item 1: a business OWNER can approve a review of their own
 * listing from the real Business Portal screen.
 *
 * The defect class this pins: the business-side review moderation surface existing only as an API
 * endpoint (or the portal rendering a queue with no working Approve control), so "an owner can
 * approve a review" is never actually reachable on screen. An API-only check cannot see that.
 *
 * Drives the real page (frontend/business-portal.html, served at /portal):
 *   1. sign in through the portal's OWN login form with the fixture owner's real credentials;
 *   2. open the portal's "Reviews" view (its own showPortalView('reviews'));
 *   3. see the pending review of the owner's own listing rendered, with an Approve control;
 *   4. CLICK that control (its own moderateReview -> POST /business/reviews/:id/approve);
 *   5. watch the row leave the pending state on screen, with a 200 seen in the browser.
 *
 * Driven by scripts/reviews-business-portal-approve-smoke.py, which mints the fixture (real
 * accounts + a real claimed listing + a real customer review) and asserts the DB row afterwards.
 *
 * Env: PORTAL_URL, OWNER_EMAIL, PW, RID, SHOT_DIR, CHROME_PATH, NODE_PATH
 * Exit: 0 = all browser checks passed, 1 = a check failed (or the flow threw).
 */
const path = require('path');
const puppeteer = require('puppeteer-core');

const PORTAL = process.env.PORTAL_URL || 'http://127.0.0.1:8089/portal';
const EMAIL = process.env.OWNER_EMAIL;
const PW = process.env.PW;
const RID = process.env.RID;
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
  const apiCalls = [];
  const badResponses = [];
  page.on('response', (r) => {
    const u = r.url();
    if (u.includes('/api/v1/')) apiCalls.push(r.status() + ' ' + u.replace(/^https?:\/\/[^/]+/, ''));
    if (r.status() >= 400) badResponses.push(r.status() + ' ' + u.replace(/^https?:\/\/[^/]+/, ''));
  });
  page.on('requestfailed', (rq) => badResponses.push('FAILED ' + rq.failure().errorText + ' ' + rq.url().replace(/^https?:\/\/[^/]+/, '')));

  try {
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
    await page.waitForFunction(() => {
      const box = document.getElementById('rvList');
      return !!box && /Approve/.test(box.innerHTML);
    }, { timeout: 25000 });

    const before = await page.evaluate((rid) => {
      const box = document.getElementById('rvList');
      const btn = [...box.querySelectorAll('button')]
        .find((b) => /approve/i.test(b.textContent || '') && (b.getAttribute('onclick') || '').includes(rid));
      return { hasApprove: !!btn, pendingBadge: /pending/i.test(box.innerHTML), html: box.innerHTML.slice(0, 4000) };
    }, RID);
    before.hasApprove ? ok("the pending review of the owner's own listing is rendered with an Approve control")
                      : bad('no Approve control rendered for the pending review');
    await page.screenshot({ path: path.join(SHOT_DIR, 'portal-reviews.png') }).catch(() => {});

    // Click the REAL button (runs the page's own moderateReview -> POST /business/reviews/:id/approve).
    await page.evaluate((rid) => {
      const box = document.getElementById('rvList');
      const btn = [...box.querySelectorAll('button')]
        .find((b) => /approve/i.test(b.textContent || '') && (b.getAttribute('onclick') || '').includes(rid));
      btn.click();
    }, RID);

    await page.waitForFunction((rid) => {
      const box = document.getElementById('rvList');
      return ![...box.querySelectorAll('button')]
        .some((b) => /approve/i.test(b.textContent || '') && (b.getAttribute('onclick') || '').includes(rid));
    }, { timeout: 25000 }, RID);
    ok('clicking Approve in the portal flipped the row out of the pending state on screen');

    const approveCall = apiCalls.find((c) => c.startsWith('200') && /\/business\/reviews\/.*\/approve/.test(c));
    approveCall ? ok('the browser called the real approve endpoint: ' + approveCall)
                : bad('no 200 from /business/reviews/:id/approve observed in the browser');
    await page.screenshot({ path: path.join(SHOT_DIR, 'portal-reviews-after.png') }).catch(() => {});

    if (badResponses.length) console.log('BAD RESPONSES: ' + JSON.stringify(badResponses.slice(0, 10)));
    if (consoleErrors.length) bad('console errors: ' + consoleErrors.slice(0, 3).join(' | '));
    else ok('no JavaScript console errors during the flow');
  } catch (e) {
    bad('browser flow threw: ' + e.message);
    try { await page.screenshot({ path: path.join(SHOT_DIR, 'portal-reviews-error.png') }); } catch (_) {}
  }

  await browser.close();
  console.log('BROWSER RESULT: ' + (problems.length ? problems.length + ' failed' : 'all passed'));
  process.exit(problems.length ? 1 : 0);
})();
