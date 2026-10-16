/**
 * Item 8 (admin Webhooks screen) + item 9 (operator Dashboard tiles) — BROWSER proof.
 *
 * Why this exists: both items were previously "proven" only by fetching admin-panel.html and
 * grepping the SOURCE for the control ids. That is not the front-end mandate — it never renders
 * the page, so a screen that throws on load, shows the honest "—" forever, or fails to wire the
 * control to the API would still "pass". This drives the REAL console in Chromium as the operator:
 *
 *   item 9: open the panel, wait for the Dashboard tiles, assert every tile is a REAL number read
 *           from the live DB (never "—", never a fabricated 0), then assert the numbers EQUAL the
 *           direct DB counts.
 *   item 8: on the SAME page open the "API Access & Webhooks" screen, assert the register controls
 *           are really rendered, register a webhook THROUGH the screen, assert it appears in the
 *           on-screen list AND as a DB row, fire a real directory event so the screen's "Deliveries"
 *           button reveals a delivered row ON SCREEN, then delete it THROUGH the screen and prove
 *           residue is zero.
 *
 * Env: ADMIN_URL, SHOT_DIR. Exit 0 = all checks passed.
 */
const puppeteer = require('puppeteer-core');
const { execSync } = require('child_process');
const fs = require('fs');
const http = require('http');
const crypto = require('crypto');
const net = require('net');

const ADMIN = process.env.ADMIN_URL || 'http://127.0.0.1:8089/admin-panel.html';
const SHOT = process.env.SHOT_DIR || '/tmp';
const BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const ENV_FILE = fs.existsSync('/etc/swift/env/multi-directory.env')
  ? '/etc/swift/env/multi-directory.env' : '/opt/swift/apps/multi-directory/.env';
const TS = Date.now();
const TAG = 'zz-wh-screen-' + TS;
const SECRET = 'wh-screen-secret-' + TS;

let fails = 0;
const check = (n, ok, d) => {
  console.log(`  [${ok ? 'PASS' : 'FAIL'}] ${n}${d !== undefined ? ' :: ' + d : ''}`);
  if (!ok) fails++;
};
const sleep = ms => new Promise(r => setTimeout(r, ms));

function dbPass() {
  const url = execSync('docker exec multidirectory printenv DATABASE_URL').toString().trim();
  return url.match(/:\/\/[^:]+:([^@]+)@/)[1];
}
const PW = dbPass();
const psql = sql =>
  execSync(`docker exec -i -e PGPASSWORD=${PW} swift-postgres psql -U swift -d multi_directory -tAc ${JSON.stringify(sql)}`)
    .toString().trim();

function mintToken() {
  const line = fs.readFileSync(ENV_FILE, 'utf8').split('\n').find(l => l.startsWith('JWT_SECRET='));
  const secret = line.split('=').slice(1).join('=').trim().replace(/^"|"$/g, '');
  const b64 = o => Buffer.from(JSON.stringify(o)).toString('base64url');
  const now = Math.floor(Date.now() / 1000);
  const msg = `${b64({ alg: 'HS256', typ: 'JWT' })}.${b64({
    sub: '00000000-0000-0000-0000-000000000002',
    tid: '00000000-0000-0000-0000-000000000001',
    role: 'super_admin', exp: now + 1200, iat: now,
    aud: 'multidirectory-api', iss: 'multidirectory',
  })}`;
  const sig = crypto.createHmac('sha256', secret).update(msg).digest('base64url');
  return `${msg}.${sig}`;
}

function findChrome() {
  const base = '/root/.cache/ms-playwright';
  try {
    for (const d of fs.readdirSync(base)) {
      for (const p of ['chrome-linux64/chrome', 'chrome-linux/chrome', 'chrome-headless-shell-linux64/chrome-headless-shell']) {
        const f = `${base}/${d}/${p}`;
        try { if (fs.statSync(f).size > 0) return f; } catch (e) {}
      }
    }
  } catch (e) {}
  for (const p of ['/usr/bin/chromium-browser', '/usr/bin/chromium', '/usr/bin/google-chrome']) {
    try { if (fs.statSync(p).size > 0) return p; } catch (e) {}
  }
  return null;
}

function freePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.on('error', reject);
    s.listen(0, '127.0.0.1', () => {
      const p = s.address().port;
      s.close(() => resolve(p));
    });
  });
}

const HITS = [];
function startReceiver(port) {
  const srv = http.createServer((req, res) => {
    let body = '';
    req.on('data', c => body += c);
    req.on('end', () => {
      HITS.push({ url: req.url, sig: req.headers['x-webhook-signature'], secretHdr: req.headers['x-webhook-secret'], body });
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end('{"ok":true}');
    });
  });
  return new Promise(r => srv.listen(port, '127.0.0.1', () => r(srv)));
}

async function apiPost(token, path, body) {
  const r = await fetch(BASE + path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + token },
    body: JSON.stringify(body),
  });
  return { status: r.status, text: await r.text() };
}

function sweep() {
  try { psql(`DELETE FROM webhooks WHERE url LIKE 'http://127.0.0.1:%${TAG}%'`); } catch (e) {}
  try { psql(`DELETE FROM webhooks WHERE url LIKE '%${TAG}%'`); } catch (e) {}
}
process.on('exit', sweep);

(async () => {
  // ---- DB truth for item 9 -------------------------------------------------
  const truth = {
    businesses: parseInt(psql('select count(*) from businesses') || '0', 10),
    reviews: parseInt(psql('select count(*) from reviews') || '0', 10),
    directories: parseInt(psql('select count(*) from directories') || '0', 10),
    active: parseInt(psql("select count(*) from directories where status='active'") || '0', 10),
    domains: parseInt(psql('select count(*) from domain_mappings') || '0', 10),
  };
  console.log('DB truth:', JSON.stringify(truth));

  const whBefore = parseInt(psql(`SELECT COUNT(*) FROM webhooks WHERE url LIKE '%${TAG}%'`) || '0', 10);
  check('the probe starts with no webhook of its own', whBefore === 0, `rows=${whBefore}`);

  const port = await freePort();
  const recv = await startReceiver(port);
  const url = `http://127.0.0.1:${port}/hooks/zaarhub-${TAG}`;
  const op = mintToken();

  const chrome = findChrome();
  if (!chrome) { console.log('FAIL no chromium'); process.exit(1); }
  const browser = await puppeteer.launch({
    executablePath: chrome, headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--window-size=1440,1200'],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1440, height: 1200 });
  const pageErrors = [];
  const consoleErrors = [];
  page.on('pageerror', e => pageErrors.push(String(e.message)));
  page.on('console', m => { if (m.type() === 'error') consoleErrors.push(m.text()); });
  page.on('dialog', async d => { await d.accept(); });
  await page.evaluateOnNewDocument(t => {
    localStorage.setItem('md_admin_token', t);
    localStorage.setItem('md_admin_user', JSON.stringify({ role: 'super_admin', email: 'zz-wh-screen@invalid' }));
  }, mintToken());

  await page.goto(ADMIN, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await sleep(1200);
  // Make the console show its Dashboard (the page's own entry point).
  await page.evaluate(() => {
    const d = document.getElementById('dashboard-page');
    if (d && d.classList.contains('hidden') && typeof showDashboard === 'function') showDashboard();
  });

  // ---------- item 9: dashboard tiles render REAL numbers ----------
  await page.waitForFunction(() => {
    const v = document.getElementById('stat-businesses');
    return !!v && /^[0-9][0-9,]*$/.test((v.textContent || '').trim());
  }, { timeout: 30000 }).catch(() => {});

  const tiles = await page.evaluate(() => {
    const t = id => { const e = document.getElementById(id); return e ? (e.textContent || '').trim() : null; };
    return {
      businesses: t('stat-businesses'), reviews: t('stat-reviews'), domains: t('stat-domains'),
      directories: t('stat-total'), active: t('stat-active'),
      visible: (() => { const d = document.getElementById('dashboard-page'); return !!d && !d.classList.contains('hidden'); })(),
    };
  });
  const num = s => s == null ? NaN : parseInt(String(s).replace(/[^0-9]/g, ''), 10);
  check('the Dashboard page is actually shown (not the login screen)', tiles.visible);
  check('the Businesses tile shows a REAL number, not the honest "—"', Number.isFinite(num(tiles.businesses)) && num(tiles.businesses) > 0, `tile=${tiles.businesses}`);
  check('the Reviews tile shows a REAL number', Number.isFinite(num(tiles.reviews)), `tile=${tiles.reviews}`);
  check('the Domain Mappings tile shows a REAL number', Number.isFinite(num(tiles.domains)), `tile=${tiles.domains}`);
  check('the Businesses tile EQUALS the live DB count', num(tiles.businesses) === truth.businesses, `tile=${tiles.businesses} db=${truth.businesses}`);
  check('the Reviews tile EQUALS the live DB count', num(tiles.reviews) === truth.reviews, `tile=${tiles.reviews} db=${truth.reviews}`);
  check('the Domain Mappings tile EQUALS the live DB count', num(tiles.domains) === truth.domains, `tile=${tiles.domains} db=${truth.domains}`);
  check('the Directories tile EQUALS the live DB count', num(tiles.directories) === truth.directories, `tile=${tiles.directories} db=${truth.directories}`);
  check('the Active tile EQUALS the live DB count', num(tiles.active) === truth.active, `tile=${tiles.active} db=${truth.active}`);
  await page.screenshot({ path: `${SHOT}/admin-dashboard.png` });

  // ---------- item 8: the Webhooks screen renders and works ----------
  await page.evaluate(() => {
    const a = document.querySelector('a[href="#developer-api-section"]');
    if (a) a.click();
    if (typeof loadApiAccess === 'function') loadApiAccess();
  });
  await page.waitForSelector('#wh-url', { timeout: 20000 });
  await page.waitForSelector('#wh-events input.wh-ev', { timeout: 20000 }).catch(() => {});

  const screen = await page.evaluate(() => {
    const sec = document.getElementById('developer-api-section');
    const r = sec ? sec.getBoundingClientRect() : null;
    return {
      present: !!sec,
      hasUrl: !!document.getElementById('wh-url'),
      hasSecret: !!document.getElementById('wh-secret'),
      events: document.querySelectorAll('#wh-events input.wh-ev').length,
      list: !!document.getElementById('wh-list'),
      deliveries: !!document.getElementById('wh-deliveries'),
      addBtn: /Add webhook/i.test((document.querySelector('#developer-api-section button[onclick*="__whCreate"]') || {}).textContent || ''),
      height: r ? r.height : 0,
    };
  });
  check('the API Access & Webhooks screen is present and laid out on screen', screen.present && screen.height > 0, `height=${Math.round(screen.height)}`);
  check('it renders a webhook URL field and a secret field', screen.hasUrl && screen.hasSecret);
  check('it renders the event chooser', screen.events >= 8, `events=${screen.events}`);
  check('it renders the webhook list and the deliveries area', screen.list && screen.deliveries);
  check('it renders a real "+ Add webhook" control', screen.addBtn);

  // Register THROUGH the screen.
  await page.evaluate((u, s) => {
    document.getElementById('wh-url').value = u;
    document.getElementById('wh-secret').value = s;
    const cb = document.querySelector('#wh-events input.wh-ev[value="business.created"]');
    if (cb) cb.checked = true;
  }, url, SECRET);
  await page.evaluate(() => document.querySelector('#developer-api-section button[onclick*="__whCreate"]').click());

  await page.waitForFunction(() => /added/i.test((document.getElementById('wh-status') || {}).textContent || ''), { timeout: 20000 }).catch(() => {});
  const addMsg = await page.evaluate(() => (document.getElementById('wh-status') || {}).textContent || '');
  check('the screen reports the webhook added', /added/i.test(addMsg), addMsg.trim().slice(0, 120));

  await page.waitForFunction(u => {
    const box = document.getElementById('wh-list');
    return !!box && box.innerHTML.includes(u);
  }, { timeout: 20000 }, url).catch(() => {});
  const listHas = await page.evaluate(u => (document.getElementById('wh-list') || {}).innerHTML.includes(u), url);
  check('the registered webhook appears in the on-screen list', listHas);

  const rowCount = parseInt(psql(`SELECT COUNT(*) FROM webhooks WHERE url = '${url}'`) || '0', 10);
  check('a matching webhook row exists in the database (the screen wrote through to the API)', rowCount === 1, `rows=${rowCount}`);
  const secretStored = psql(`SELECT (secret IS NOT NULL AND secret <> '')::text FROM webhooks WHERE url = '${url}'`);
  check('the shared secret is recorded with the webhook (write-through)', secretStored === 'true', secretStored);
  const listResp = await fetch(`${BASE}/api/v1/webhooks`, { headers: { Authorization: 'Bearer ' + op } });
  const listBody = await listResp.text();
  check('the API list response never echoes the signing secret', !listBody.includes(SECRET), `status=${listResp.status}`);
  const rawLeaked = await page.evaluate(s => document.body.innerHTML.includes(s), SECRET);
  check('the raw secret is never rendered back into the page', !rawLeaked);
  await page.screenshot({ path: `${SHOT}/admin-webhooks-added.png` });

  // Fire a real event -> the screen's Deliveries button must reveal a delivered row on screen.
  const ev = await apiPost(op, '/api/v1/directory-events', {
    event_type: 'business.created', entity_type: 'business',
    entity_id: '00000000-0000-0000-0000-0000000000aa', data: { probe: TAG },
  });
  let hit = false;
  for (let i = 0; i < 30 && !hit; i++) { await sleep(500); hit = HITS.length > 0; }
  check('firing a real directory event delivers a signed POST to the registered URL', hit, `hits=${HITS.length} status=${ev.status}`);
  if (hit) {
    const h = HITS[0];
    const expect = crypto.createHash('sha256').update(SECRET + h.body).digest('hex');
    check('the delivered signature equals sha256(secret + body)', h.sig === expect, String(h.sig).slice(0, 16) + '…');
  }

  await page.evaluate(() => {
    const btn = document.querySelector('#wh-list button[onclick*="__whDeliveries"]');
    if (btn) btn.click();
  });
  await page.waitForFunction(() => /deliveries/i.test((document.getElementById('wh-deliveries') || {}).innerHTML || ''), { timeout: 20000 }).catch(() => {});
  const delivHtml = await page.evaluate(() => (document.getElementById('wh-deliveries') || {}).innerHTML || '');
  check('the screen\'s Deliveries control renders the delivered row ON SCREEN', /delivered/i.test(delivHtml), delivHtml.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim().slice(0, 120));
  await page.screenshot({ path: `${SHOT}/admin-webhooks-deliveries.png` });

  // Delete THROUGH the screen.
  await page.evaluate(() => {
    const btn = document.querySelector('#wh-list button[onclick*="__whDelete"]');
    if (btn) btn.click();
  });
  await sleep(1500);
  const gone = parseInt(psql(`SELECT COUNT(*) FROM webhooks WHERE url = '${url}'`) || '0', 10);
  check('deleting through the screen removes the webhook row', gone === 0, `rows=${gone}`);
  await page.screenshot({ path: `${SHOT}/admin-webhooks-deleted.png` });

  check('no uncaught page errors during the flow', pageErrors.length === 0, pageErrors.slice(0, 2).join(' | '));
  check('no console errors during the flow', consoleErrors.length === 0, consoleErrors.slice(0, 2).join(' | '));

  fs.writeFileSync(`${SHOT}/dashboard-webhooks-proof.json`, JSON.stringify({
    db_truth: truth, tiles, screen, add_msg: addMsg.trim(), list_has_url: listHas,
    db_row: rowCount, secret_encrypted: secretStored, event_status: ev.status, receiver_hits: HITS.length,
    deliveries_on_screen: /delivered/i.test(delivHtml), row_after_delete: gone,
    page_errors: pageErrors, console_errors: consoleErrors,
  }, null, 2));

  recv.close();
  await browser.close();
  console.log(fails === 0 ? '\nBROWSER RESULT: all passed' : `\nBROWSER RESULT: ${fails} FAILED`);
  process.exit(fails === 0 ? 0 : 1);
})().catch(e => { console.log('BROWSER THREW: ' + (e && e.message)); process.exit(1); });
