/**
 * ZaarHub supplier RFQ page (/rfq.html) — real browser arm for the filter bar.
 *
 * Proves, on the REAL served page (no stubs), that the Search / Category / Urgency controls
 * actually narrow the list and that the page puts the values on the wire:
 *   A. the page loads and the filter bar with #fq / #fcat / #furg is visible;
 *   B. typing in Search + "Apply filters" sends ?q=… and leaves ONLY matching cards;
 *   C. Category (upper-cased) + Apply sends ?category=… and leaves only that category;
 *   D. Urgency + Apply sends ?urgency=… and leaves only that urgency;
 *   E. no console errors and no failing API call during the walk.
 *
 * Env: MD_BASE, MARK, MARK_IDS (csv), CAT, CAT_IDS (csv), URGENT_ID, AUD, CHROME_PATH
 * Output: $AUD/browser-proof.json + $AUD/rfq-filtered.png
 */
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const MARK = process.env.MARK || 'ZZRFQFILTER';
const MARK_IDS = (process.env.MARK_IDS || '').split(',').filter(Boolean);
const CAT = process.env.CAT || 'ZZPROBE-CAT-A';
const CAT_IDS = (process.env.CAT_IDS || '').split(',').filter(Boolean);
const URGENT_ID = process.env.URGENT_ID || '';
const AUD = process.env.AUD || '/tmp/rfq-filter-browser';
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';

fs.mkdirSync(AUD, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const checks = [];
const rec = (ok, name, detail) => {
  checks.push({ ok: !!ok, name, detail: String(detail === undefined ? '' : detail).slice(0, 400) });
};

(async () => {
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu'],
  });
  const page = await browser.newPage();
  const consoleErrors = [];
  const failedApi = [];
  const reqUrls = [];
  page.on('console', (m) => {
    if (m.type() === 'error') consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));
  page.on('request', (r) => {
    if (r.url().includes('/api/v1/b2b/rfqs')) reqUrls.push(r.url());
  });
  page.on('response', (r) => {
    if (r.url().includes('/api/') && r.status() >= 400) failedApi.push(r.status() + ' ' + r.url());
  });

  const resp = await page.goto(MD_BASE + '/rfq.html', { waitUntil: 'networkidle2', timeout: 30000 });
  rec(resp && resp.status() === 200, 'the supplier RFQ page loads', 'HTTP ' + (resp && resp.status()));
  await page.waitForSelector('#fq', { timeout: 10000 });

  const controls = await page.evaluate(() => {
    const bar = document.getElementById('filterBar');
    return {
      barVisible: !!bar && getComputedStyle(bar).display !== 'none',
      q: !!document.getElementById('fq'),
      c: !!document.getElementById('fcat'),
      u: !!document.getElementById('furg'),
    };
  });
  rec(
    controls.barVisible && controls.q && controls.c && controls.u,
    'search / category / urgency controls are visible on the page',
    JSON.stringify(controls)
  );

  const cardIds = () => page.$$eval('.rfq-card', (els) => els.map((e) => e.getAttribute('data-id')));
  const metas = () => page.$$eval('.rfq-card .meta', (els) => els.map((e) => e.textContent));
  const titles = () => page.$$eval('.rfq-card h4', (els) => els.map((e) => e.textContent.trim()));

  const apply = async () => {
    const waited = page.waitForResponse(
      (r) => r.url().includes('/api/v1/b2b/rfqs') && r.request().method() === 'GET',
      { timeout: 15000 }
    ).catch(() => null);
    await page.evaluate(() => {
      const b = [...document.querySelectorAll('button')].find((x) => /apply filters/i.test(x.textContent));
      if (b) b.click();
    });
    await waited;
    await sleep(400);
  };
  const setValue = async (sel, val) => {
    await page.evaluate((s, v) => {
      const el = document.querySelector(s);
      el.value = v;
      el.dispatchEvent(new Event('change', { bubbles: true }));
      el.dispatchEvent(new Event('input', { bubbles: true }));
    }, sel, val);
  };

  // ── B. search box ──────────────────────────────────────────────────────────────────────
  await setValue('#fq', MARK);
  await setValue('#fcat', '');
  await setValue('#furg', '');
  await apply();
  let ids = await cardIds();
  let ts = await titles();
  const lastReq = reqUrls[reqUrls.length - 1] || '';
  rec(lastReq.includes('q=' + encodeURIComponent(MARK)), 'the page puts the search term on the wire', lastReq);
  rec(
    MARK_IDS.every((id) => ids.includes(id)) && ts.every((t) => t.toUpperCase().includes(MARK.toUpperCase())),
    'search leaves only matching cards on screen',
    'cards=' + ids.length + ' titles=' + JSON.stringify(ts)
  );

  // ── C. category ───────────────────────────────────────────────────────────────────────
  await setValue('#fq', '');
  await setValue('#fcat', CAT.toUpperCase());
  await setValue('#furg', '');
  await apply();
  ids = await cardIds();
  let ms = await metas();
  const lastReq2 = reqUrls[reqUrls.length - 1] || '';
  rec(lastReq2.toLowerCase().includes('category='), 'the page puts the category on the wire', lastReq2);
  rec(
    CAT_IDS.length > 0 && ids.length === CAT_IDS.length && CAT_IDS.every((id) => ids.includes(id)),
    'category leaves exactly the matching cards on screen',
    'cards=' + JSON.stringify(ids) + ' expected=' + JSON.stringify(CAT_IDS)
  );
  rec(
    ms.every((m) => m.toLowerCase().includes(CAT.toLowerCase())),
    'every visible card shows that category',
    JSON.stringify(ms)
  );

  // ── D. urgency ────────────────────────────────────────────────────────────────────────
  await setValue('#fcat', '');
  await setValue('#furg', 'urgent');
  await apply();
  ids = await cardIds();
  ms = await metas();
  const lastReq3 = reqUrls[reqUrls.length - 1] || '';
  rec(lastReq3.includes('urgency=urgent'), 'the page puts the urgency on the wire', lastReq3);
  rec(
    URGENT_ID !== '' && ids.length === 1 && ids[0] === URGENT_ID,
    'urgency=urgent leaves only the urgent card on screen',
    'cards=' + JSON.stringify(ids) + ' expected=[' + URGENT_ID + ']'
  );
  rec(
    ms.every((m) => m.includes('urgent urgency')),
    'every visible card is labelled urgent',
    JSON.stringify(ms)
  );

  await page.screenshot({ path: path.join(AUD, 'rfq-filtered.png'), fullPage: true });

  rec(consoleErrors.length === 0, 'no JavaScript console errors during the walk', JSON.stringify(consoleErrors.slice(0, 3)));
  rec(failedApi.length === 0, 'no failing API call during the walk', JSON.stringify(failedApi.slice(0, 3)));

  const pass = checks.every((c) => c.ok);
  const proof = { pass, url: MD_BASE + '/rfq.html', checks, reqUrls, consoleErrors, failedApi };
  fs.writeFileSync(path.join(AUD, 'browser-proof.json'), JSON.stringify(proof, null, 1));
  for (const c of checks) console.log((c.ok ? 'PASS  ' : 'FAIL  ') + c.name + ' — ' + c.detail);
  console.log('BROWSER RESULT: ' + (pass ? 'all passed' : checks.filter((c) => !c.ok).length + ' failure(s)'));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => {
  console.error('browser arm crashed: ' + e.message);
  process.exit(2);
});
