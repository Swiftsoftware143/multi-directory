/**
 * Item 7 proof (browser arm) — the Provider API Keys screen in the LIVE admin panel.
 *
 * Proves the FRONT-END MANDATE, not just the API: an operator can open the Integration Center,
 * paste a provider key, watch it save and be reported as set, and delete it — on screen.
 *
 * It also pins the link to item 3 (network email): with mailgun selected as the mail transport,
 * saving a mailgun key on THIS screen is exactly what stops the app refusing password resets.
 * So the run captures the live consequence before / while / after the key exists:
 *   - email service answers "provider_key_missing"  -> forgot-password answers 503 (honest)
 *   - key saved on screen                            -> the key is READ at call time, refusal lifts
 *   - key deleted on screen                          -> back to an honest 503
 * The fake key is removed at the end; residue is asserted zero.
 *
 * Env: ADMIN_URL, SHOT_DIR, NODE_PATH. Exit 0 = all checks passed.
 */
const puppeteer = require('puppeteer-core');
const { execSync } = require('child_process');
const fs = require('fs');
const crypto = require('crypto');

const ADMIN = process.env.ADMIN_URL || 'http://127.0.0.1:8089/admin-panel.html';
const SHOT = process.env.SHOT_DIR || '/tmp';
const ENV_FILE = '/etc/swift/env/multi-directory.env';
const PROVIDER = 'mailgun';
const FAKE = 'key-zzscreenproof-' + Date.now();
const SQLQ = "'" + FAKE.replace(/'/g, "''") + "'";
const EMAIL_SVC = 'http://127.0.0.1:3456';

// Whatever happens, never leave the temporary credential behind.
function sweep() {
  try {
    psql(`DELETE FROM provider_keys WHERE provider='${PROVIDER}' AND label LIKE 'zz screen proof%'`);
  } catch (e) {}
}
process.on('exit', sweep);

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

const mailgunRows = () =>
  parseInt(psql(`SELECT COUNT(*) FROM provider_keys WHERE provider='${PROVIDER}'`) || '0', 10);
const mailgunShape = () =>
  psql(`SELECT is_active::text||'|'||(api_key LIKE 'enc:v1:%')::text||'|'||COALESCE(octet_length(api_key_encrypted),0)::text ` +
       `||'|'||length(api_key)::text||'|'||(api_key = ${SQLQ})::text||'|'||` +
       `(COALESCE(encode(api_key_encrypted,'escape'),'') LIKE '%'||${SQLQ}||'%')::text ` +
       `FROM provider_keys WHERE provider='${PROVIDER}' ORDER BY created_at DESC LIMIT 1`);

async function emailProbeDetail() {
  const r = await fetch(`${EMAIL_SVC}/send`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ to: 'zz-screen-proof@probe.invalid', subject: 'p', html: 'x', text: 'x' }),
  });
  const j = await r.json();
  return j;
}
async function forgotStatus() {
  const r = await fetch('http://127.0.0.1:8089/api/v1/auth/forgot-password', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email: 'zz-screen-proof@probe.invalid' }),
  });
  return r.status;
}

(async () => {
  const before = mailgunRows();
  console.log(`mailgun keys before: ${before}`);
  check('the probe starts with NO mailgun key (so the run cannot shadow a real credential)', before === 0, `rows=${before}`);
  const svcBefore = await emailProbeDetail();
  check('with no mailgun key the mail service says so plainly', /provider_key_missing|skipped/.test(svcBefore.status + svcBefore.detail), `${svcBefore.status} / ${svcBefore.detail}`);
  const fpBefore = await forgotStatus();
  check('a locked-out user is refused HONESTLY today (mail cannot be sent)', fpBefore === 503, `HTTP ${fpBefore}`);

  const chrome = findChrome();
  if (!chrome) { console.log('FAIL no chromium'); process.exit(1); }
  const browser = await puppeteer.launch({
    executablePath: chrome, headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--window-size=1440,1000'],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1440, height: 1000 });
  const consoleErrors = [];
  page.on('console', m => { if (m.type() === 'error') consoleErrors.push(m.text()); });
  page.on('dialog', async d => { await d.accept(); });
  await page.evaluateOnNewDocument(t => {
    localStorage.setItem('md_admin_token', t);
    localStorage.setItem('md_admin_user', JSON.stringify({ role: 'super_admin', email: 'zz-screen-proof@invalid' }));
  }, mintToken());

  await page.goto(ADMIN, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await sleep(1500);
  // The card's own loader, the panel's own nav.
  await page.evaluate(() => { if (typeof loadProviderKeys === 'function') loadProviderKeys(); });
  await page.waitForFunction(
    () => !!document.querySelector('[data-pk-row="mailgun"]'), { timeout: 30000 });

  const cardOk = await page.evaluate(() => {
    const c = document.getElementById('provider-keys-section');
    if (!c) return { present: false };
    const r = c.getBoundingClientRect();
    const row = c.querySelector('[data-pk-row="mailgun"]');
    return {
      present: true,
      visible: c.style.display !== 'none' && r.height > 0,
      title: (c.querySelector('.feature-title, h2, h3') || {}).textContent || '',
      badge: row ? (row.querySelector('.pk-badge') || {}).textContent : null,
      hasInput: !!document.getElementById('pk-key-mailgun'),
      hasSave: !!row && !!row.querySelector('[data-pk-action="save"]'),
      hasTest: !!row && !!row.querySelector('[data-pk-action="test"]'),
      providerRows: c.querySelectorAll('[data-pk-row]').length,
    };
  });
  check('the admin panel serves the Provider API Keys card on screen', cardOk.present && cardOk.visible, cardOk.title);
  check('the card lists the provider catalogue', cardOk.providerRows > 20, `rows=${cardOk.providerRows}`);
  check('mailgun is one of the offered providers, marked Not set', /Not set/.test(cardOk.badge || ''), cardOk.badge);
  check('it has a paste field and an Add key control', cardOk.hasInput && cardOk.hasSave);
  await page.screenshot({ path: `${SHOT}/screen-1-before.png` });

  // --- save a (fake) mailgun key through the screen ---
  await page.evaluate(v => {
    document.getElementById('pk-label-mailgun').value = 'zz screen proof (temporary)';
    document.getElementById('pk-key-mailgun').value = v;
    const nav = document.querySelector('a[href="#provider-keys-section"]');
    if (nav) nav.click();
  }, FAKE);
  await page.evaluate(() => {
    const row = document.querySelector('[data-pk-row="mailgun"]');
    row.querySelector('[data-pk-action="save"]').click();
  });
  await page.waitForFunction(
    () => { const s = document.getElementById('pk-status'); return s && /saved/i.test(s.textContent); },
    { timeout: 20000 }).catch(() => {});
  const savedMsg = await page.evaluate(() => (document.getElementById('pk-status') || {}).textContent || '');
  check('the screen reports the key saved', /saved/i.test(savedMsg), savedMsg.trim().slice(0, 120));

  await page.waitForFunction(
    () => { const row = document.querySelector('[data-pk-row="mailgun"]'); return row && /1 key/.test(row.textContent); },
    { timeout: 20000 }).catch(() => {});
  const afterBadge = await page.evaluate(() => {
    const row = document.querySelector('[data-pk-row="mailgun"]');
    return { badge: row ? (row.querySelector('.pk-badge') || {}).textContent : null, preview: row ? row.textContent.match(/[a-z0-9•]{4,}•+/i) : null };
  });
  check('the row now shows the key as SET, with a masked preview (never the raw value)', /1 key/.test(afterBadge.badge || ''), `${afterBadge.badge} ${afterBadge.preview ? JSON.stringify(afterBadge.preview[0]) : ''}`);
  const hasTest = await page.evaluate(() => {
    const row = document.querySelector('[data-pk-row="mailgun"]');
    return !!row && !!row.querySelector('[data-pk-action="test"]');
  });
  check('a saved key gets a Test control (the capability has a UI, not just an endpoint)', hasTest);
  const rawLeaked = await page.evaluate(v => document.body.innerHTML.includes(v), FAKE);
  check('the raw key is never rendered back into the page', !rawLeaked);
  await page.screenshot({ path: `${SHOT}/screen-2-saved.png` });

  const shape = mailgunShape();
  const [active, encPrefixed, encBytes, storedLen, isRawInText, isRawInBytea] = shape.split('|');
  const encryptedSomewhere = encPrefixed === 'true' || parseInt(encBytes, 10) > 0;
  check('the key is stored ENCRYPTED (enc:v1: envelope), never as the raw value',
    encryptedSomewhere && isRawInText === 'false' && isRawInBytea === 'false',
    `active=${active} encEnvelope=${encPrefixed} ciphertextBytes=${encBytes} lenOnFile=${storedLen} rawInText=${isRawInText} rawInBytea=${isRawInBytea}`);
  check('the stored key is active', active === 'true');

  // --- the live consequence: the key saved on this screen is what the mail path reads ---
  const svcAfter = await emailProbeDetail();
  check('the mail service now finds the key at send time (no longer "missing")',
    !/provider_key_missing/.test(JSON.stringify(svcAfter)), `${svcAfter.status} / ${String(svcAfter.detail).slice(0, 90)}`);
  const fpAfter = await forgotStatus();
  check('the honest refusal lifts once the key exists (a reset is now attempted)', fpAfter !== 503, `HTTP ${fpAfter}`);

  // --- the screen's own Test control ---
  await page.evaluate(() => document.querySelector('[data-pk-row="mailgun"] [data-pk-action="test"]').click());
  await sleep(2500);
  const testMsg = await page.evaluate(() => (document.getElementById('pk-status') || {}).textContent || '');
  check('the Test control answers on screen', /mailgun:/i.test(testMsg), testMsg.trim().slice(0, 140));
  await page.screenshot({ path: `${SHOT}/screen-3-tested.png` });

  // --- delete it again, from the screen ---
  await page.evaluate(() => {
    const row = document.querySelector('[data-pk-row="mailgun"]');
    const del = row.querySelector('[data-pk-action="delete"]');
    if (del) del.click();
  });
  await page.waitForFunction(
    () => { const row = document.querySelector('[data-pk-row="mailgun"]'); return row && /Not set/.test(row.textContent); },
    { timeout: 20000 }).catch(() => {});
  await sleep(800);
  const afterDelete = await page.evaluate(() => {
    const row = document.querySelector('[data-pk-row="mailgun"]');
    return { badge: row ? (row.querySelector('.pk-badge') || {}).textContent : null };
  });
  check('the screen shows the key gone again (Not set)', /Not set/.test(afterDelete.badge || ''), afterDelete.badge);
  check('residue: no mailgun key row is left behind', mailgunRows() === 0, `rows=${mailgunRows()}`);
  const svcRestored = await emailProbeDetail();
  check('the mail service is back to reporting the key missing', /provider_key_missing/.test(JSON.stringify(svcRestored)), svcRestored.status);
  const fpRestored = await forgotStatus();
  check('a locked-out user is refused honestly again', fpRestored === 503, `HTTP ${fpRestored}`);
  check('no JavaScript errors during the flow', consoleErrors.length === 0, consoleErrors.slice(0, 2).join(' | '));
  await page.screenshot({ path: `${SHOT}/screen-4-deleted.png` });

  fs.writeFileSync(`${SHOT}/screen-proof.json`, JSON.stringify({
    card: cardOk, saved_msg: savedMsg.trim(), badge_after_save: afterBadge.badge,
    stored: { active, enc_envelope: encPrefixed, ciphertext_bytes: encBytes, len_on_file: storedLen, raw_in_text: isRawInText, raw_in_bytea: isRawInBytea },
    email_service_before: svcBefore, email_service_after_save: svcAfter, email_service_restored: svcRestored,
    forgot_before: fpBefore, forgot_after_save: fpAfter, forgot_restored: fpRestored,
    console_errors: consoleErrors, residue_rows: mailgunRows(),
  }, null, 2));

  await browser.close();
  console.log(fails === 0 ? '\nBROWSER RESULT: all passed' : `\nBROWSER RESULT: ${fails} FAILED`);
  process.exit(fails === 0 ? 0 : 1);
})().catch(e => { console.log('BROWSER THREW: ' + (e && e.message)); process.exit(1); });
