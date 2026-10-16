/**
 * ZaarHub — "Network email: ONE setup for all cities" — real browser arm (queue item 3).
 *
 * The card's BACKEND was already proven (scripts/network-email-send-smoke.py): a city that owns no
 * row sends through the ONE network setup. What had no proof was the FRONT-END MANDATE half — that
 * the operator can SEE and use that one setup from the console, in every scope, without a city ever
 * being offered its own copy.
 *
 * Drives the RUNNING admin panel (the page zaarhub.com/admin-panel.html proxies to) in real Chromium:
 *   - platform scope  -> the picker offers the network as ONE entry ("… network — all N cities");
 *   - network scope   -> the picker collapses to EXACTLY ONE entry ("… — whole network (N cities)")
 *                        and the loaded sender is the ONE saved setup (from-email / transport);
 *   - city scope      -> a city that saves NO setup of its own still shows that SAME one entry
 *                        (never a per-city form), and the card says the one setup serves every city;
 *   - the card's own Verify control reaches the real email service (a live call, not a stub);
 *   - the page ran with no script errors and the panel made no failed API call.
 *
 * Read-only: it switches scope and reads; it never clicks Save.
 *
 * Env:  MD_BASE, TOKEN (platform-operator JWT), AUD, CHROME_PATH, NET_ID, CITY_ID,
 *       EXPECT_FROM, EXPECT_TRANSPORT, EXPECT_NETNAME
 * Out:  $AUD/em-screen-proof.json (+ em-card-net.png, em-card-city.png)
 * Exit: 0 = all browser checks pass, 1 = otherwise.
 */
const puppeteer = require('puppeteer-core');
const fs = require('fs');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TOKEN = process.env.TOKEN;
const AUD = process.env.AUD || '/tmp';
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';
const NET_ID = process.env.NET_ID || '';
const CITY_ID = process.env.CITY_ID || '';
const EXPECT_FROM = process.env.EXPECT_FROM || '';
const EXPECT_TRANSPORT = process.env.EXPECT_TRANSPORT || '';
const EXPECT_NETNAME = process.env.EXPECT_NETNAME || '';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const checks = [];
function check(name, ok, detail = '') {
  checks.push({ name, ok: !!ok, detail: String(detail == null ? '' : detail) });
}

async function picker(page) {
  return page.evaluate(() => {
    const sel = document.getElementById('em-directory');
    if (!sel) return null;
    return {
      n: sel.options.length,
      texts: [...sel.options].map((o) => (o.textContent || '').trim()),
      value: sel.value,
    };
  });
}

async function stated(page) {
  return page.evaluate(() => ({
    from: (document.getElementById('em-from-email') || {}).value || '',
    transport: (document.getElementById('em-transport') || {}).value || '',
    badge: (document.getElementById('em-saved-badge') || {}).textContent || '',
    status: (document.getElementById('em-status') || {}).textContent || '',
    result: (document.getElementById('em-result') || {}).textContent || '',
  }));
}

async function settle(page, wantRe) {
  // Wait until the picker has been re-scoped AND the card has finished loading that scope's setup.
  await page.waitForFunction((re) => {
    const sel = document.getElementById('em-directory');
    if (!sel) return false;
    const label = [...sel.options].map((o) => o.textContent || '').join(' | ');
    if (!new RegExp(re).test(label)) return false;
    const st = (document.getElementById('em-status') || {}).textContent || '';
    const from = (document.getElementById('em-from-email') || {}).value || '';
    return st.length > 0 && from.length > 0;
  }, { timeout: 30000 }, wantRe);
}

(async () => {
  if (!TOKEN) { console.error('FATAL no TOKEN in env'); process.exit(1); }
  if (!NET_ID || !CITY_ID) { console.error('FATAL no NET_ID/CITY_ID in env'); process.exit(1); }

  const browser = await puppeteer.launch({
    executablePath: CHROME, headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1440, height: 1200 });
  const failing = [];
  const cerr = [];
  page.on('response', (r) => {
    const u = r.url();
    if (u.includes('/api/') && r.status() >= 400) failing.push(r.status() + ' ' + u.replace(MD_BASE, ''));
  });
  page.on('console', (m) => { if (m.type() === 'error') cerr.push(m.text().slice(0, 200)); });
  page.on('pageerror', (e) => cerr.push('pageerror: ' + String(e).slice(0, 200)));

  await page.evaluateOnNewDocument((t) => {
    localStorage.setItem('md_admin_token', t);
    localStorage.setItem('token', t);
    localStorage.setItem('md_admin_user', JSON.stringify({ email: 'verify@zzem', role: 'super_admin', name: 'Verify' }));
    localStorage.setItem('user', JSON.stringify({ email: 'verify@zzem', role: 'super_admin', name: 'Verify' }));
  }, TOKEN);

  const resp = await page.goto(MD_BASE + '/admin-panel.html', { waitUntil: 'networkidle2', timeout: 60000 });
  check('the admin panel loaded', resp && resp.status() === 200, 'status=' + (resp && resp.status()));
  await page.waitForSelector('#email-settings-section', { timeout: 30000 });
  await page.waitForFunction(() => window.MDContextBar && window.MDContextBar.scope, { timeout: 25000 });
  await sleep(4000);

  check('the Email / SMTP card renders', await page.evaluate(() => !!document.getElementById('email-settings-section')));
  check('its sidebar link is present',
    await page.evaluate(() => !!document.querySelector('a[href="#email-settings-section"]')));

  // ── platform scope: every option is a NETWORK rollup, never a single city ─────────────────────
  await page.evaluate(() => { window.MDContextBar.setScope('all:'); });
  await settle(page, 'all \\d+ cities');
  const p0 = await picker(page);
  check('the platform picker offers the network as ONE entry covering all its cities',
    !!p0 && p0.n >= 1 && p0.texts.every((t) => /all\s+\d+\s+cities/i.test(t)),
    p0 ? JSON.stringify(p0.texts) : 'no picker');
  check('the platform entry names the network',
    !!p0 && !!EXPECT_NETNAME && p0.texts.some((t) => t.indexOf(EXPECT_NETNAME) !== -1),
    'want=' + EXPECT_NETNAME);

  // ── network scope: exactly one entry, "whole network (N cities)", showing the ONE saved setup ─
  const switchedNet = await page.evaluate((nid) => window.MDContextBar.setScope('net:' + nid), NET_ID);
  check('the console can switch to this network', switchedNet === true, 'setScope=' + switchedNet);
  await settle(page, 'whole network');
  const pNet = await picker(page);
  check('in the network scope the picker collapses to EXACTLY ONE entry',
    !!pNet && pNet.n === 1, pNet ? 'options=' + pNet.n + ' ' + JSON.stringify(pNet.texts) : 'no picker');
  check('that one entry says it is the whole network (all cities)',
    !!pNet && pNet.n === 1 && /whole network \(\d+ cities\)/i.test(pNet.texts[0]),
    pNet ? pNet.texts[0] : '');
  const sNet = await stated(page);
  check('the loaded sender is the ONE saved setup (from email)',
    sNet.from === EXPECT_FROM, 'screen=' + sNet.from + ' db=' + EXPECT_FROM);
  check('the loaded transport is the ONE saved setup',
    sNet.transport === EXPECT_TRANSPORT, 'screen=' + sNet.transport + ' db=' + EXPECT_TRANSPORT);
  check('the card states out loud that the ONE setup serves every city',
    /serves the whole network/i.test(sNet.status), sNet.status.slice(0, 200));
  await page.screenshot({ path: AUD + '/em-card-net.png' }).catch(() => {});

  // ── city scope: a city that saves NO setup of its own is STILL offered only the ONE entry ────
  const switchedCity = await page.evaluate((cid) => window.MDContextBar.setScope('dir:' + cid), CITY_ID);
  check('the console can switch to a non-primary city', switchedCity === true, 'setScope=' + switchedCity);
  await settle(page, 'whole network');
  const pCity = await picker(page);
  check('a city that saves no setup of its own is NEVER offered a per-city form (still exactly ONE entry)',
    !!pCity && pCity.n === 1, pCity ? 'options=' + pCity.n + ' ' + JSON.stringify(pCity.texts) : 'no picker');
  check('the city scope shows the SAME network-wide entry (same carrier)',
    !!pCity && !!pNet && pCity.n === 1 && pCity.value === pNet.value,
    (pCity ? pCity.value : '') + ' vs ' + (pNet ? pNet.value : ''));
  const sCity = await stated(page);
  check('the city inherits the SAME sender as the one setup (no city copy)',
    sCity.from === EXPECT_FROM, 'screen=' + sCity.from + ' db=' + EXPECT_FROM);
  check('the card tells the operator the one setup serves every city from the city scope too',
    /serves the whole network/i.test(sCity.status) || /saved on/i.test(sCity.badge + ' ' + sCity.status),
    (sCity.badge + ' :: ' + sCity.status).slice(0, 220));
  await page.screenshot({ path: AUD + '/em-card-city.png' }).catch(() => {});

  // ── the card's own Verify control reaches the real email service ─────────────────────────────
  await page.evaluate(() => { window.__emVerify(); });
  await sleep(9000);
  const sVer = await stated(page);
  check("the card's Verify control got an answer from the real email service",
    /→/.test(sVer.status) || /ok|fail|refus|error/i.test(sVer.status),
    sVer.status.slice(0, 200));
  check('the Verify answer was not a client-side failure',
    !/Verify failed/i.test(sVer.status), sVer.status.slice(0, 200));

  await browser.close();

  check('no browser script errors', cerr.length === 0, JSON.stringify(cerr.slice(0, 4)));
  check('no failed API calls from the panel', failing.length === 0, JSON.stringify(failing.slice(0, 6)));

  const pass = checks.filter((c) => c.ok).length;
  const fail = checks.length - pass;
  try { fs.mkdirSync(AUD, { recursive: true }); } catch (e) { /* exists */ }
  fs.writeFileSync(AUD + '/em-screen-proof.json',
    JSON.stringify({ pass, fail, checks, consoleErrors: cerr, failingApi: failing }, null, 2));
  console.log(JSON.stringify({ pass, fail, checks }, null, 2));
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error('FATAL ' + e.message); process.exit(1); });
