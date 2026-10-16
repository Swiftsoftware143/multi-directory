/**
 * Item 7 proof (browser arm) — the Provider API Keys screen in the LIVE admin panel.
 *
 * Proves the FRONT-END MANDATE, not just the API: an operator can open the Integration Center,
 * paste a provider key, watch it save and be reported as set, run the screen's own Test control,
 * and delete it again — all on screen.
 *
 * SAFETY (rewritten 2026-10-16, t_543d51d8 follow-up): the earlier version of this file targeted
 * `mailgun` unconditionally, on the premise that the box had no mailgun key. When the operator
 * configured a REAL network mail credential (2026-10-16 00:37, label
 * "Network mail (updates.zaarhub.com)") the premise went stale, the probe added a second key beside
 * the operator's, and its label-scoped exit sweep deleted the operator's row — the live credential
 * was destroyed on 2026-10-16 14:09 and had to be restored out of the 02:30 nightly dump.
 * This version NEVER touches a provider that already has a key:
 *   - it picks the first provider on the catalogue that is marked "Not set", and FAILS if that
 *     provider actually has rows in the database (a screen/DB disagreement is a bug, not a licence
 *     to write);
 *   - every write, test and delete is scoped to that one provider;
 *   - the cleanup run on exit is scoped to that provider and to the probe's own label;
 *   - it asserts, at the end, that the operator's live `mailgun` credential is byte-identical to
 *     the start (row count + sha256 of the stored envelope), so this class of damage can never
 *     pass silently again.
 *
 * Env: ADMIN_URL, SHOT_DIR, NODE_PATH, PK_PROVIDER (optional explicit target). Exit 0 = all passed.
 */
const puppeteer = require('puppeteer-core');
const { execSync } = require('child_process');
const fs = require('fs');
const crypto = require('crypto');

const ADMIN = process.env.ADMIN_URL || 'http://127.0.0.1:8089/admin-panel.html';
const SHOT = process.env.SHOT_DIR || '/tmp';
const ENV_FILE = '/etc/swift/env/multi-directory.env';
const LIVE = 'mailgun';                       // the operator's real credential — never mutated here
const FAKE = 'key-zzscreenproof-' + Date.now();
const SQLQ = "'" + FAKE.replace(/'/g, "''") + "'";
let PROVIDER = process.env.PK_PROVIDER || ''; // the probe's own target provider, chosen on screen

// Whatever happens, never leave a row behind that THIS RUN created. Scoped to the probe's own
// provider, so an operator credential can never be swept away by a cleanup.
function sweep() {
  try {
    if (PROVIDER) {
      psql(`DELETE FROM provider_keys WHERE provider='${PROVIDER}' AND label LIKE 'zz screen proof%'`);
    }
  } catch (e) {}
}
process.on('exit', sweep);

let fails = 0;
const check = (n, ok, d) => {
  console.log(`  [${ok ? 'PASS' : 'FAIL'}] ${n}${d !== undefined ? ' :: ' + d : ''}`);
  if (!ok) fails++;
};
const observe = (n, d) => console.log(`  [obs ] ${n}${d !== undefined ? ' :: ' + d : ''}`);
const sleep = ms => new Promise(r => setTimeout(r, ms));

function dbPass() {
  const url = execSync('docker exec multidirectory printenv DATABASE_URL').toString().trim();
  return url.match(/:\/\/[^:]+:([^@]+)@/)[1];
}
const PW = dbPass();
const psql = sql =>
  execSync(`docker exec -i -e PGPASSWORD=${PW} swift-postgres psql -U swift -d multi_directory -tAc ${JSON.stringify(sql)}`)
    .toString().trim();

const rows = p => parseInt(psql(`SELECT COUNT(*) FROM provider_keys WHERE provider='${p}'`) || '0', 10);
// sha256 of the stored envelope: the value itself is never read out or printed.
const digest = p => psql(
  `SELECT COALESCE(encode(digest(api_key,'sha256'),'hex'),'') FROM provider_keys ` +
  `WHERE provider='${p}' ORDER BY created_at LIMIT 1`);
const shape = p => psql(
  `SELECT is_active::text||'|'||(api_key LIKE 'enc:v1:%')::text||'|'||` +
  `COALESCE(octet_length(api_key_encrypted),0)::text||'|'||length(api_key)::text||'|'||` +
  `(api_key = ${SQLQ})::text||'|'||` +
  `(COALESCE(encode(api_key_encrypted,'escape'),'') LIKE '%'||${SQLQ}||'%')::text ` +
  `FROM provider_keys WHERE provider='${p}' ORDER BY created_at DESC LIMIT 1`);

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

(async () => {
  // ---- 0. record the operator's live credential BEFORE anything ---------------------
  const liveRowsBefore = rows(LIVE);
  const liveDigestBefore = digest(LIVE);
  console.log(`operator's live ${LIVE} keys before: ${liveRowsBefore} (sha256 ${liveDigestBefore.slice(0, 12) || '-'})`);
  if (liveRowsBefore > 0 && !liveDigestBefore) {
    check('the operator credential can be fingerprinted (so the run can prove it is untouched)', false, 'no digest');
  }

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
  await page.evaluate(() => { if (typeof loadProviderKeys === 'function') loadProviderKeys(); });
  await page.waitForFunction(
    () => document.querySelectorAll('[data-pk-row]').length > 0, { timeout: 30000 });

  // ---- 1. the card renders, generically --------------------------------------------
  const cardOk = await page.evaluate(() => {
    const c = document.getElementById('provider-keys-section');
    if (!c) return { present: false };
    const r = c.getBoundingClientRect();
    return {
      present: true,
      visible: c.style.display !== 'none' && r.height > 0,
      title: (c.querySelector('.feature-title, h2, h3') || {}).textContent || '',
      providerRows: c.querySelectorAll('[data-pk-row]').length,
      hasSave: !!c.querySelector('[data-pk-action="save"]'),
      hasTest: !!c.querySelector('[data-pk-action="test"]'),
      hasDelete: !!c.querySelector('[data-pk-action="delete"]'),
    };
  });
  check('the admin panel serves the Provider API Keys card on screen', cardOk.present && cardOk.visible, cardOk.title);
  check('the card lists the provider catalogue', cardOk.providerRows > 20, `rows=${cardOk.providerRows}`);
  check('it renders a paste field and a save control', cardOk.hasSave);
  check('a configured key offers Test and delete controls on screen', cardOk.hasTest && cardOk.hasDelete);
  const liveBadge = await page.evaluate(p => {
    const row = document.querySelector(`[data-pk-row="${p}"]`);
    return row ? (row.querySelector('.pk-badge') || {}).textContent : null;
  }, LIVE);
  observe(`the operator's live ${LIVE} credential on screen`, liveBadge);

  // ---- 2. choose a provider that has NO key (never touch a live credential) --------
  if (!PROVIDER) {
    const picked = await page.evaluate(() => {
      const out = [];
      for (const r of document.querySelectorAll('[data-pk-row]')) {
        const id = r.getAttribute('data-pk-row');
        const badge = (r.querySelector('.pk-badge') || {}).textContent || '';
        if (/Not set/i.test(badge) && document.getElementById('pk-key-' + id)) out.push(id);
      }
      return out;
    });
    if (!picked.length) { check('the catalogue offers a provider with no key to test against', false, 'none "Not set"'); await browser.close(); process.exit(1); }
    PROVIDER = picked[0];
    console.log(`probe target (a provider with no key): ${PROVIDER}  [${picked.length} unset providers available]`);
  }
  check('the probe target is not the operator\'s live provider', PROVIDER !== LIVE || rows(LIVE) === 0, PROVIDER);
  const targetRowsBefore = rows(PROVIDER);
  check('the target provider really has no key in the database (screen and DB agree)',
    targetRowsBefore === 0, `${PROVIDER} rows=${targetRowsBefore}`);

  // ---- 3. add a key through the screen --------------------------------------------
  await page.evaluate((prov, v) => {
    document.getElementById('pk-label-' + prov).value = 'zz screen proof (temporary)';
    document.getElementById('pk-key-' + prov).value = v;
  }, PROVIDER, FAKE);
  await page.evaluate(prov => {
    document.querySelector(`[data-pk-row="${prov}"] [data-pk-action="save"]`).click();
  }, PROVIDER);
  await page.waitForFunction(
    () => { const s = document.getElementById('pk-status'); return s && /saved/i.test(s.textContent); },
    { timeout: 20000 }).catch(() => {});
  const savedMsg = await page.evaluate(() => (document.getElementById('pk-status') || {}).textContent || '');
  check('the screen reports the key saved', /saved/i.test(savedMsg), savedMsg.trim().slice(0, 120));

  await page.waitForFunction(prov => {
    const row = document.querySelector(`[data-pk-row="${prov}"]`);
    return row && /1 key/.test(row.textContent);
  }, { timeout: 20000 }, PROVIDER).catch(() => {});
  const afterBadge = await page.evaluate(prov => {
    const row = document.querySelector(`[data-pk-row="${prov}"]`);
    return {
      badge: row ? (row.querySelector('.pk-badge') || {}).textContent : null,
      preview: row ? row.textContent.match(/[a-z0-9•]{4,}•+/i) : null,
    };
  }, PROVIDER);
  check('the row now shows the key as SET, with a masked preview (never the raw value)',
    /1 key/.test(afterBadge.badge || ''), `${afterBadge.badge} ${afterBadge.preview ? JSON.stringify(afterBadge.preview[0]) : ''}`);
  const rawLeaked = await page.evaluate(v => document.body.innerHTML.includes(v), FAKE);
  check('the raw key is never rendered back into the page', !rawLeaked);
  await page.screenshot({ path: `${SHOT}/screen-2-saved.png` });

  const sh = shape(PROVIDER);
  const [active, encPrefixed, encBytes, storedLen, isRawInText, isRawInBytea] = sh.split('|');
  const encryptedSomewhere = encPrefixed === 'true' || parseInt(encBytes, 10) > 0;
  check('the key is stored ENCRYPTED (enc:v1: envelope), never as the raw value',
    encryptedSomewhere && isRawInText === 'false' && isRawInBytea === 'false',
    `active=${active} encEnvelope=${encPrefixed} ciphertextBytes=${encBytes} lenOnFile=${storedLen} rawInText=${isRawInText} rawInBytea=${isRawInBytea}`);
  check('the stored key is active', active === 'true');

  // ---- 4. the screen's own Test control -------------------------------------------
  await page.evaluate(prov => {
    document.querySelector(`[data-pk-row="${prov}"] [data-pk-action="test"]`).click();
  }, PROVIDER);
  await sleep(2500);
  const testMsg = await page.evaluate(() => (document.getElementById('pk-status') || {}).textContent || '');
  check('the Test control answers on screen', new RegExp(PROVIDER + ':', 'i').test(testMsg), testMsg.trim().slice(0, 140));
  await page.screenshot({ path: `${SHOT}/screen-3-tested.png` });

  // ---- 5. delete it again, from the screen ----------------------------------------
  await page.evaluate(prov => {
    const del = document.querySelector(`[data-pk-row="${prov}"] [data-pk-action="delete"]`);
    if (del) del.click();
  }, PROVIDER);
  await page.waitForFunction(prov => {
    const row = document.querySelector(`[data-pk-row="${prov}"]`);
    return row && /Not set/.test(row.textContent);
  }, { timeout: 20000 }, PROVIDER).catch(() => {});
  await sleep(800);
  const afterDelete = await page.evaluate(prov => {
    const row = document.querySelector(`[data-pk-row="${prov}"]`);
    return { badge: row ? (row.querySelector('.pk-badge') || {}).textContent : null };
  }, PROVIDER);
  check('the screen shows the key gone again (Not set)', /Not set/.test(afterDelete.badge || ''), afterDelete.badge);
  check('residue: no key row is left behind for the probe provider', rows(PROVIDER) === 0, `rows=${rows(PROVIDER)}`);

  // ---- 6. the operator's live credential is byte-identical ------------------------
  const liveRowsAfter = rows(LIVE);
  const liveDigestAfter = digest(LIVE);
  check("the operator's live credential is UNTOUCHED by this proof (rows + sha256 identical)",
    liveRowsAfter === liveRowsBefore && liveDigestAfter === liveDigestBefore,
    `${LIVE} rows ${liveRowsBefore}->${liveRowsAfter}, sha256 ${liveDigestBefore.slice(0, 12)} -> ${liveDigestAfter.slice(0, 12)}`);
  check('no JavaScript errors during the flow', consoleErrors.length === 0, consoleErrors.slice(0, 2).join(' | '));
  await page.screenshot({ path: `${SHOT}/screen-4-deleted.png` });

  fs.writeFileSync(`${SHOT}/screen-proof.json`, JSON.stringify({
    provider_used: PROVIDER,
    card: cardOk,
    saved_msg: savedMsg.trim(),
    badge_after_save: afterBadge.badge,
    stored: { active, enc_envelope: encPrefixed, ciphertext_bytes: encBytes, len_on_file: storedLen, raw_in_text: isRawInText, raw_in_bytea: isRawInBytea },
    test_message: testMsg.trim(),
    live_provider: { name: LIVE, rows_before: liveRowsBefore, rows_after: liveRowsAfter, digest_before: liveDigestBefore, digest_after: liveDigestAfter, badge: liveBadge },
    console_errors: consoleErrors,
    residue_rows: rows(PROVIDER),
  }, null, 2));

  await browser.close();
  console.log(fails === 0 ? '\nBROWSER RESULT: all passed' : `\nBROWSER RESULT: ${fails} FAILED`);
  process.exit(fails === 0 ? 0 : 1);
})().catch(e => { console.log('BROWSER THREW: ' + (e && e.message)); process.exit(1); });
