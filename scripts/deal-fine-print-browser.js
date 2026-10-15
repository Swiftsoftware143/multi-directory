/**
 * Durable proof (browser arm) — a business owner can put FINE PRINT on a deal FROM THE SCREEN,
 * and a shopper then reads it on the public deal page.
 *
 * The defect class this pins: `deals.fine_print` existed in the schema, was returned by the API
 * and rendered by the storefront's own "Fine Print" tab — but no screen could set it and
 * POST/PUT /deals silently DROPPED the field. A capability that exists in the data model and on
 * the public page, with no control anywhere: the owner had no way to fill it (measured 2026-10-15).
 *
 * Drives the real pages:
 *   1. sign in through the Business Portal's OWN login form (frontend/business-portal.html);
 *   2. open the Deals panel's own "+ New Deal" control (visible, not just present in the DOM);
 *   3. type into the form's own Fine Print field;
 *   4. click the form's own "Save Deal" button (its own submitDeal -> POST /api/v1/deals);
 *   5. read the deal id back out of the portal's own deal list;
 *   6. open the public deal page (/#/deal/<id>) as a LOGGED-OUT visitor and click its own
 *      "Fine Print" tab, asserting the exact text the owner typed is rendered there.
 *
 * Env: PORTAL_URL, PUBLIC_URL, OWNER_EMAIL, PW, FINE_PRINT, TITLE, SHOT_DIR, DEAL_OUT,
 *      CHROME_PATH, NODE_PATH
 * Exit: 0 = all browser checks passed, 1 = a check failed (or the flow threw).
 */
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const PORTAL = process.env.PORTAL_URL || 'http://127.0.0.1:8089/portal';
const PUBLIC_BASE = process.env.PUBLIC_URL || 'http://127.0.0.1:8089';
const EMAIL = process.env.OWNER_EMAIL;
const PW = process.env.PW;
const FINE = process.env.FINE_PRINT || '';
const TITLE = process.env.TITLE || 'ZZ fine print probe';
const SHOT_DIR = process.env.SHOT_DIR || '/tmp';
const DEAL_OUT = process.env.DEAL_OUT || '';

const problems = [];
const ok = (m) => console.log('PASS  ' + m);
const bad = (m) => { problems.push(m); console.log('FAIL  ' + m); };

(async () => {
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
  page.on('response', (r) => {
    const u = r.url();
    if (u.includes('/api/v1/')) apiCalls.push(r.status() + ' ' + u.replace(/^https?:\/\/[^/]+/, ''));
  });

  let dealId = null;
  try {
    // ── 1. the portal's own login form ─────────────────────────────────────────
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
      return !!d && d.style.display === 'block';
    }, { timeout: 30000 });
    ok('owner signed in through the real portal login form');

    // ── 2. the Deals panel's own "+ New Deal" control must be ON SCREEN ─────────
    await page.waitForFunction(() => !!document.getElementById('dealsPanel'), { timeout: 20000 });
    const newDealVisible = await page.evaluate(() => {
      const b = [...document.querySelectorAll('button')].find((x) => /new deal/i.test(x.textContent || ''));
      if (!b) return false;
      return b.getClientRects().length > 0;
    });
    newDealVisible
      ? ok('the Deals panel shows its "+ New Deal" control on screen')
      : bad('the "+ New Deal" control is not reachable on screen');
    await page.screenshot({ path: path.join(SHOT_DIR, 'portal-deals.png') }).catch(() => {});

    await page.evaluate(() => {
      const b = [...document.querySelectorAll('button')].find((x) => /new deal/i.test(x.textContent || ''));
      b.click();
    });
    await page.waitForFunction(() => {
      const f = document.getElementById('dealFormCard');
      return !!f && f.style.display !== 'none';
    }, { timeout: 15000 });
    ok('clicking "+ New Deal" opened the deal form');

    // ── 3. the form's own Fine Print control ───────────────────────────────────
    const fineVisible = await page.evaluate(() => {
      const el = document.getElementById('dealFinePrint');
      return !!el && el.getClientRects().length > 0;
    });
    fineVisible
      ? ok('the deal form carries a VISIBLE Fine Print control')
      : bad('the deal form has no visible Fine Print control (the defect this card closes)');

    await page.evaluate((t, f) => {
      document.getElementById('dealTitle').value = t;
      document.getElementById('dealFinePrint').value = f;
      document.getElementById('dealPrice').value = '19.99';
      document.getElementById('dealStartDate').value = new Date().toISOString().slice(0, 10);
    }, TITLE, FINE);
    await page.screenshot({ path: path.join(SHOT_DIR, 'portal-deal-form.png') }).catch(() => {});

    // ── 4. click the real Save button ──────────────────────────────────────────
    await page.evaluate(() => document.getElementById('dealSubmitBtn').click());
    await page.waitForFunction(() => {
      const el = document.getElementById('dealFormResult');
      return !!el && /saved|created|success/i.test(el.innerHTML || '');
    }, { timeout: 25000 }).catch(() => {});
    const saved = apiCalls.find((c) => /^20\d \/api\/v1\/deals(\?|$)/.test(c));
    saved
      ? ok('clicking Save sent the real create-deal request: ' + saved)
      : bad('no successful POST /api/v1/deals came from the screen');

    // ── 5. read the deal id from the portal's OWN deal list call ───────────────
    dealId = await page.evaluate(async (title) => {
      const token = localStorage.getItem('bizToken');
      const r = await fetch('/api/v1/deals?business_id=' + window._selectedBizId, {
        headers: { Authorization: 'Bearer ' + token },
      });
      const j = await r.json();
      const list = Array.isArray(j) ? j : (j.deals || j.data || []);
      const hit = list.find((d) => (d.title || '') === title);
      return hit ? hit.id : null;
    }, TITLE).catch(() => null);
    dealId ? ok('the portal list returns the deal just saved  [id=' + String(dealId).slice(0, 8) + '…]')
           : bad('the saved deal is not in the portal deal list');
    if (consoleErrors.length) bad('portal console errors: ' + consoleErrors.slice(0, 3).join(' | '));
    else ok('no JavaScript console errors in the owner flow');

    // ── 6. the public deal page, logged OUT, renders the owner's fine print ────
    if (dealId) {
      const p2 = await browser.newPage();
      const errs2 = [];
      p2.on('console', (m) => { if (m.type() === 'error') errs2.push(m.text()); });
      p2.on('pageerror', (e) => errs2.push('pageerror: ' + e.message));
      await p2.goto(PUBLIC_BASE + '/#/deal/' + dealId, { waitUntil: 'networkidle2', timeout: 60000 });
      await p2.waitForFunction(() => /fine print/i.test(document.body.innerHTML || ''), { timeout: 25000 })
        .catch(() => {});
      const tabText = await p2.evaluate(() => {
        const btn = [...document.querySelectorAll('button')]
          .find((b) => /fine print/i.test(b.textContent || ''));
        if (btn) btn.click();
        const c = document.getElementById('tab-fineprint');
        return c ? c.textContent : '';
      });
      tabText && tabText.includes(FINE)
        ? ok("the logged-out public deal page's Fine Print tab shows the owner's text")
        : bad('the public deal page does not show the fine print  [tab=' + String(tabText).slice(0, 120) + ']');
      await p2.screenshot({ path: path.join(SHOT_DIR, 'public-deal-fineprint.png') }).catch(() => {});
      errs2.length ? bad('public deal page console errors: ' + errs2.slice(0, 3).join(' | '))
                   : ok('the public deal page ran with no script errors');
    }
  } catch (e) {
    bad('browser flow threw: ' + e.message);
    try { await page.screenshot({ path: path.join(SHOT_DIR, 'portal-deal-error.png') }); } catch (_) {}
  }

  if (dealId && DEAL_OUT) {
    try { fs.writeFileSync(DEAL_OUT, String(dealId)); } catch (_) {}
  }
  await browser.close();
  console.log('BROWSER RESULT: ' + (problems.length ? problems.length + ' failed' : 'all passed'));
  process.exit(problems.length ? 1 : 0);
})();
