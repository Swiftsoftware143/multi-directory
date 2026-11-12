/**
 * Durable proof (browser arm) — the Business Portal must TELL an owner that their claim is waiting
 * for review instead of asking them to claim a listing they have already claimed.
 *
 * The defect this pins (2026-11-12): a claim whose e-mail domain does not match the listing waits
 * for an operator, so `claimed_businesses` is empty for that owner — and the portal rendered its
 * "No claimed businesses yet — claim your business listing" empty state, with no status anywhere.
 *
 * Drives the real page (frontend/business-portal.html at /portal):
 *   1. signs in through the portal's own login form;
 *   2. reads the "My Businesses" panel the owner lands on;
 *   3. requires the plain-English queue message and the listing name;
 *   4. requires that the "claim your listing" empty state is GONE.
 *
 * Env: PORTAL_URL, OWNER_EMAIL, PW, BUSINESS_ID, SHOT_DIR
 * Exit: 0 = all browser checks passed, 1 = a check failed.
 */
const path = require('path');
const puppeteer = require('puppeteer-core');

const PORTAL = process.env.PORTAL_URL || 'http://127.0.0.1:8089/portal';
const EMAIL = process.env.OWNER_EMAIL;
const PW = process.env.PW;
const SHOT_DIR = process.env.SHOT_DIR || '/tmp';

(async () => {
  const problems = [];
  const ok = (m) => console.log('PASS  ' + m);
  const bad = (m) => { problems.push(m); console.log('FAIL  ' + m); };

  if (!EMAIL || !PW) { console.log('FAIL  OWNER_EMAIL/PW missing'); process.exit(1); }

  const browser = await puppeteer.launch({
    executablePath: require('./lib/chrome.cjs')(),
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1360, height: 1000 });

  const consoleErrors = [];
  page.on('console', (m) => {
    if (m.type() === 'error') {
      const loc = (m.location() && m.location().url) || '';
      consoleErrors.push(m.text() + (loc ? ' @ ' + loc : ''));
    }
  });
  page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));
  const badResponses = [];
  page.on('response', (r) => {
    const u = r.url();
    // A brand-new owner has no payout account, so GET /portal/business/payouts legitimately answers
    // 403 — it is not this page's concern and it predates the queue card.
    if (r.status() >= 400 && u.includes('/api/v1/') && !/portal\/business\/payouts/.test(u)) {
      badResponses.push(r.status() + ' ' + u.replace(/^https?:\/\/[^/]+/, ''));
    }
  });

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

    try {
      await page.waitForFunction((name) => {
        const box = document.getElementById('businessList');
        return !!box && box.textContent.includes(name);
      }, { timeout: 25000 }, 'ZZ Pending Claim Co');
    } catch (e) {
      const dump = await page.evaluate(async () => {
        const box = document.getElementById('businessList');
        let prof = '(fetch failed)';
        try {
          const r = await fetch('/api/v1/portal/business/profile', {
            headers: { Authorization: 'Bearer ' + localStorage.getItem('bizToken') } });
          const j = await r.json();
          prof = JSON.stringify({ status: r.status, claimed: (j.claimed_businesses || []).length,
            pending: (j.pending_claims || []).length,
            keys: Object.keys(j), names: (j.pending_claims || []).map(p => p.business_name) });
        } catch (err) { prof = 'threw: ' + err.message; }
        return { text: box ? box.innerText.slice(0, 300) : '(missing #businessList)', prof };
      });
      console.log('DEBUG businessList text: ' + JSON.stringify(dump.text));
      console.log('DEBUG profile as the browser sees it: ' + dump.prof);
      bad('the "My Businesses" panel never showed the queued claim');
    }

    const view = await page.evaluate(() => {
      const box = document.getElementById('businessList');
      return {
        text: box ? box.innerText : '',
        html: box ? box.innerHTML : '',
        heading: (box && box.querySelector('.section-title')) ? box.querySelector('.section-title').innerText : '',
      };
    });

    /in the queue/i.test(view.text)
      ? ok('the portal tells the owner "Your claim is in the queue"')
      : bad('the portal does not say the claim is waiting for review');
    /waiting for review/i.test(view.text)
      ? ok('the panel is headed "Waiting for review"')
      : bad('no "Waiting for review" heading on screen');
    /within one working day/i.test(view.text)
      ? ok('the owner is told how long the check takes')
      : bad('the screen gives the owner no expectation of when it will be approved');
    /No claimed businesses yet/.test(view.text)
      ? bad('the screen still shows the "claim your business listing" empty state')
      : ok('the misleading "claim your listing" empty state is gone');
    // The probe's own fixture name/e-mail are expected on this card (the screen shows the address
    // the claim was made with); what must never appear is internal provenance.
    /builderhq|probe_harness|pending-claim-portal/i.test(view.text)
      ? bad('internal test identifiers leaked into the visible text')
      : ok('no internal identifiers are visible on screen');

    await page.screenshot({ path: path.join(SHOT_DIR, 'portal-pending-claim.png') }).catch(() => {});

    const realErrors = consoleErrors.filter((e) => !/payouts/.test(e));
    if (badResponses.length) console.log('BAD RESPONSES: ' + JSON.stringify(badResponses.slice(0, 10)));
    if (realErrors.length) bad('console errors: ' + realErrors.slice(0, 3).join(' | '));
    else ok('no console errors on the portal page');
  } catch (e) {
    bad('flow threw: ' + e.message.split('\n')[0]);
  } finally {
    await browser.close().catch(() => {});
  }

  if (problems.length) console.log('RESULT: FAIL\n  - ' + problems.join('\n  - '));
  else console.log('RESULT: PASS');
  process.exit(problems.length ? 1 : 0);
})();
