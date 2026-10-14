/**
 * ZaarHub "Data Company" tab — real browser arm (queue item 14).
 *
 * Drives the LIVE admin panel (the page https://zaarhub.com/admin-panel proxies to) in real
 * Chromium and proves the Data Company card is a working screen, not a dead section:
 *   - the section renders, its sidebar link is present and its badge names the toolbelt;
 *   - the business-verification queue renders rows and offers an Approve action;
 *   - the Google Places tool runs THROUGH THE PAGE'S OWN CONTROL (window.__dcPlacesSearch) and
 *     renders result rows, and one place's detail card renders (Name + Address + Place id);
 *   - the bulk-export control (entity + format) is on screen;
 *   - the page ran with no script errors and the panel made no failed API call.
 *
 * Read-only: it only reads. The single write attempt in the API arm (a bogus verification status)
 * lives in the python driver and must be refused.
 *
 * Env:  MD_BASE, TOKEN (super_admin JWT), AUD, CHROME_PATH
 * Out:  $AUD/dc-ui-proof.json  (+ dc-card.png)
 * Exit: 0 = all browser checks pass, 1 = otherwise.
 */
const puppeteer = require('puppeteer-core');
const fs = require('fs');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TOKEN = process.env.TOKEN;
const AUD = process.env.AUD || '/tmp';
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const checks = [];
function check(name, ok, detail = '') {
  checks.push({ name, ok: !!ok, detail: String(detail == null ? '' : detail) });
}

(async () => {
  if (!TOKEN) {
    console.error('FATAL no TOKEN in env');
    process.exit(1);
  }
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage();
  const failing = [];
  const cerr = [];
  page.on('response', (r) => {
    const u = r.url();
    if (u.includes('/api/') && r.status() >= 400) failing.push(r.status() + ' ' + u.replace(MD_BASE, ''));
  });
  page.on('console', (m) => {
    if (m.type() === 'error') cerr.push(m.text().slice(0, 200));
  });
  page.on('pageerror', (e) => cerr.push('pageerror: ' + String(e).slice(0, 200)));

  await page.evaluateOnNewDocument((t) => {
    localStorage.setItem('md_admin_token', t);
    localStorage.setItem('token', t);
    localStorage.setItem('md_admin_user', JSON.stringify({ email: 'verify@zzdc', role: 'super_admin', name: 'Verify' }));
    localStorage.setItem('user', JSON.stringify({ email: 'verify@zzdc', role: 'super_admin', name: 'Verify' }));
  }, TOKEN);

  const resp = await page.goto(MD_BASE + '/admin-panel.html', { waitUntil: 'networkidle2', timeout: 60000 });
  check('the admin panel loaded', resp && resp.status() === 200, 'status=' + (resp && resp.status()));
  await sleep(8000);

  check('the Data Company section renders',
    await page.evaluate(() => !!document.getElementById('data-company-section')));
  check('its sidebar link is present',
    await page.evaluate(() => !!document.querySelector('a[href="#data-company-section"]')));
  const badge = await page.evaluate(() => {
    const b = document.getElementById('dc-badge');
    return b ? b.textContent.trim() : null;
  });
  check('the badge names the toolbelt',
    (badge || '').indexOf('Places') !== -1 && (badge || '').indexOf('Yelp') !== -1, badge);

  const vrows = await page.evaluate(() => {
    const el = document.getElementById('dc-v-list');
    return el ? el.querySelectorAll('tr').length : -1;
  });
  check('the verification queue renders rows', vrows > 0, 'rows=' + vrows);
  check('the queue offers an Approve action',
    await page.evaluate(() => {
      const el = document.getElementById('dc-v-list');
      return !!el && el.innerText.indexOf('Approve') !== -1;
    }));

  // Places search through the page's own control (not a stubbed call).
  await page.evaluate(() => {
    document.getElementById('dc-psearch-q').value = 'coffee miami';
    window.__dcPlacesSearch();
  });
  await sleep(7000);
  const prows = await page.evaluate(() => {
    const el = document.getElementById('dc-psearch-results');
    return el ? el.querySelectorAll('tr').length : -1;
  });
  check('the Places tool returns results on screen', prows > 1, 'rows=' + prows);

  await page.evaluate(() => { window.__dcPlaceDetails(0); });
  await sleep(7000);
  const dtext = await page.evaluate(() => {
    const el = document.getElementById('dc-place-detail');
    return el ? el.innerText.replace(/\s+/g, ' ').trim().slice(0, 300) : null;
  });
  check('one place detailcard renders (name + address + place id)',
    !!(dtext && dtext.indexOf('Name:') !== -1 && dtext.indexOf('Address:') !== -1 && dtext.indexOf('Place id:') !== -1),
    dtext);

  check('the bulk-export control is on screen',
    await page.evaluate(() => !!document.getElementById('dc-x-entity') && !!document.getElementById('dc-x-format')));

  try { fs.mkdirSync(AUD, { recursive: true }); } catch (e) { /* exists */ }
  await page.screenshot({ path: AUD + '/dc-card.png', fullPage: false });
  await browser.close();

  check('no browser script errors', cerr.length === 0, JSON.stringify(cerr));
  check('no failed API calls from the panel', failing.length === 0, JSON.stringify(failing));

  const pass = checks.filter((c) => c.ok).length;
  const fail = checks.length - pass;
  fs.writeFileSync(AUD + '/dc-ui-proof.json',
    JSON.stringify({ pass, fail, checks, consoleErrors: cerr, failingApi: failing }, null, 2));
  console.log(JSON.stringify({ pass, fail, checks }, null, 2));
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => {
  console.error('FATAL ' + e.message);
  process.exit(1);
});
