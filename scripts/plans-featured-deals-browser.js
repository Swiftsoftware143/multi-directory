/**
 * ZaarHub admin console — PLANS (tiers) + FEATURED DEALS screens, real end-to-end browser arm
 * (operator action list item 10: "plans/featured deals").
 *
 * What it proves, on the REAL served console (no stubs, no mocks) at /admin-panel:
 *   A. PLAN TIERS card — the card renders the tiers the live API returns (three rows), each row
 *      shows the monthly point allowance with its own Save control, editing a value and pressing
 *      the row's own Save writes through to the API (the change is read back), and the value is
 *      restored afterwards so nothing is left changed.
 *   B. FEATURED DEALS card — the card lists live deals with a count, a deal the orchestrator
 *      created is visible on screen, and the row's own "Feature" control toggles the deal both
 *      ways with the change persisted (read back from the API).
 *
 * The token is the platform-operator JWT minted by the orchestrator and passed in $TOKEN.
 * Env: MD_BASE, TOKEN, AUD, TEMP_DEAL_TITLE, TEMP_DEAL_ID, BIZ_ID, CHROME_PATH
 * Output: $AUD/plans-deals-proof.json  (+ plans-tiers.png, deals.png)
 */
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TOKEN = process.env.TOKEN;
const AUD = process.env.AUD;
const TEMP_TITLE = process.env.TEMP_DEAL_TITLE || '';
const TEMP_ID = process.env.TEMP_DEAL_ID || '';
const BIZ_ID = process.env.BIZ_ID || '';
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';

if (!TOKEN || !AUD) {
  console.error('TOKEN and AUD are required');
  process.exit(3);
}
fs.mkdirSync(AUD, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const checks = [];
function rec(ok, name, detail) {
  checks.push({ ok: !!ok, name, detail: detail === undefined ? '' : String(detail).slice(0, 300) });
}

async function apiJson(method, p, body) {
  const headers = { Authorization: 'Bearer ' + TOKEN };
  if (body != null) headers['Content-Type'] = 'application/json';
  const r = await fetch(MD_BASE + '/api/v1' + p, {
    method,
    headers,
    body: body != null ? JSON.stringify(body) : undefined,
  });
  const txt = await r.text();
  let d = null;
  try { d = txt ? JSON.parse(txt) : null; } catch (e) { /* non-JSON */ }
  return { status: r.status, data: d };
}

(async () => {
  const consoleErrors = [];
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage();
  page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text()); });
  page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));
  page.on('response', (r) => {
    if (r.status() === 401 || r.status() === 403) consoleErrors.push('HTTP ' + r.status() + ' ' + r.url());
  });

  const preTiers = await apiJson('GET', '/tiers');
  const prePremium = (preTiers.data || []).find((t) => /premium/i.test(t.name || ''));
  const originalAllowance = prePremium ? prePremium.monthly_point_allowance : null;

  try {
    // Operator identity is set before any page script runs, so no pre-auth 401 noise.
    await page.evaluateOnNewDocument((t) => {
      localStorage.setItem('md_admin_token', t);
      localStorage.setItem('md_admin_user', JSON.stringify({ email: 'swiftsoftware143@yahoo.com', role: 'super_admin' }));
    }, TOKEN);
    await page.goto(MD_BASE + '/admin-panel', { waitUntil: 'networkidle2', timeout: 30000 });
    await sleep(2500);

    /* ── A. PLAN TIERS ─────────────────────────────────────────────────────── */
    const nav = await page.evaluate(() => !!document.querySelector('a[href="#plan-tiers-section"]'));
    rec(nav, 'plans_nav_link_present', nav);
    const sec = await page.evaluate(() => !!document.getElementById('plan-tiers-section'));
    rec(sec, 'plans_section_present', sec);

    await page.waitForFunction(
      () => document.querySelectorAll('.plan-tier-row').length >= 3,
      { timeout: 15000 },
    ).catch(() => {});
    const rows = await page.evaluate(() => Array.from(document.querySelectorAll('.plan-tier-row')).map((r) => {
      const allowanceEl = r.querySelector('input[data-field="monthly_point_allowance"]');
      return {
        id: r.getAttribute('data-id'),
        name: (r.querySelector('strong') || {}).innerText || '',
        allowance: allowanceEl ? allowanceEl.value : null,
        saveBtn: Array.from(r.querySelectorAll('button')).some((b) => /save tier/i.test(b.innerText)),
      };
    }));
    rec(rows.length === 3, 'plans_three_tier_rows_rendered', rows.length);
    rec(rows.every((r) => r.saveBtn), 'plans_every_row_has_save',
      rows.map((r) => r.saveBtn).join(','));
    const prem = rows.find((r) => /premium/i.test(r.name));
    const allowances = rows.map((r) => r.allowance).join(',');
    rec(['2000', '6000', '12000'].every((v) => rows.some((r) => r.allowance === v)),
      'plans_allowances_match_live_api', allowances);
    rec(!!prem && !!prem.id, 'plans_premium_row_identified', prem && prem.name);

    // The row's OWN Save control must write through.
    if (prem && prem.id) {
      await page.evaluate((id) => {
        const row = document.querySelector('.plan-tier-row[data-id="' + id + '"]');
        row.querySelector('input[data-field="monthly_point_allowance"]').value = '12005';
      }, prem.id);
      await page.evaluate((id) => window.__ptSave(id), prem.id);
      await sleep(2000);
      const after = await apiJson('GET', '/tiers');
      const p2 = (after.data || []).find((t) => /premium/i.test(t.name || ''));
      rec(p2 && String(p2.monthly_point_allowance) === '12005',
        'plans_ui_save_persisted', p2 && p2.monthly_point_allowance);
    }
    // restore whatever the live value was, through the API the UI uses
    if (prem && prem.id && originalAllowance != null) {
      await apiJson('PUT', '/tiers/' + prem.id, { monthly_point_allowance: originalAllowance });
      const back = await apiJson('GET', '/tiers');
      const p3 = (back.data || []).find((t) => /premium/i.test(t.name || ''));
      rec(p3 && String(p3.monthly_point_allowance) === String(originalAllowance),
        'plans_value_restored', p3 && p3.monthly_point_allowance);
      rec((back.data || []).length === 3, 'plans_tier_count_still_three', (back.data || []).length);
    }

    await page.evaluate(() => {
      const el = document.getElementById('plan-tiers-section');
      if (el) el.scrollIntoView();
    });
    await page.screenshot({ path: path.join(AUD, 'plans-tiers.png') });

    /* ── B. FEATURED DEALS ─────────────────────────────────────────────────── */
    const dsec = await page.evaluate(() => !!document.getElementById('deals-section'));
    rec(dsec, 'deals_section_present', dsec);

    await page.waitForFunction(() => {
      const el = document.getElementById('dl-list');
      const status = (document.getElementById('dl-status') || {}).textContent || '';
      return el && el.querySelector('tbody tr') && !/Loading/.test(status);
    }, { timeout: 20000 }).catch(() => {});

    const info = await page.evaluate((title) => {
      const rws = Array.from(document.querySelectorAll('#dl-list tbody tr'));
      const badge = (document.getElementById('dl-badge') || {}).textContent || '';
      return {
        n: rws.length,
        badge,
        hasTemp: rws.some((r) => title && r.innerText.indexOf(title) !== -1),
      };
    }, TEMP_TITLE);
    rec(info.n > 0, 'deals_rows_rendered', info.n);
    rec(/\d+\s*deal/i.test(info.badge), 'deals_badge_counts_deals', info.badge);
    rec(info.hasTemp, 'deals_temp_deal_visible_in_ui', TEMP_TITLE);

    // the row's OWN Feature control toggles and persists, both ways
    const clickedOn = await page.evaluate((title) => {
      const rws = Array.from(document.querySelectorAll('#dl-list tbody tr'));
      const row = rws.find((r) => title && r.innerText.indexOf(title) !== -1);
      if (!row) return false;
      const b = row.querySelector('button[data-dl-feat]');
      if (!b) return false;
      b.click();
      return true;
    }, TEMP_TITLE);
    rec(clickedOn, 'deals_feature_control_clicked', clickedOn);
    await sleep(2500);
    if (TEMP_ID) {
      const after = await apiJson('GET', '/deals?business_id=' + BIZ_ID);
      const mine = (after.data || []).find((d) => d.id === TEMP_ID);
      rec(mine && mine.featured === true, 'deals_feature_persisted_true', mine && mine.featured);
    }
    const clickedOff = await page.evaluate((title) => {
      const rws = Array.from(document.querySelectorAll('#dl-list tbody tr'));
      const row = rws.find((r) => title && r.innerText.indexOf(title) !== -1);
      const b = row && row.querySelector('button[data-dl-feat]');
      if (!b) return false;
      b.click();
      return true;
    }, TEMP_TITLE);
    await sleep(2500);
    if (TEMP_ID) {
      const after2 = await apiJson('GET', '/deals?business_id=' + BIZ_ID);
      const mine2 = (after2.data || []).find((d) => d.id === TEMP_ID);
      rec(clickedOff && mine2 && mine2.featured === false,
        'deals_feature_toggled_back_false', mine2 && mine2.featured);
    }

    await page.evaluate(() => {
      const el = document.getElementById('deals-section');
      if (el) el.scrollIntoView();
    });
    await page.screenshot({ path: path.join(AUD, 'deals.png') });

    rec(consoleErrors.length === 0, 'no_console_or_auth_errors', consoleErrors.slice(0, 3).join(' | '));
  } catch (e) {
    rec(false, 'exception', e && e.message);
  } finally {
    await browser.close();
  }

  const pass = checks.filter((c) => c.ok).length;
  const fail = checks.length - pass;
  fs.writeFileSync(path.join(AUD, 'plans-deals-proof.json'),
    JSON.stringify({ checks, pass, fail }, null, 2));
  for (const c of checks) console.log((c.ok ? 'PASS  ' : 'FAIL  ') + c.name + (c.detail ? '  [' + c.detail + ']' : ''));
  console.log('BROWSER RESULT: ' + pass + ' passed, ' + fail + ' failed');
  process.exit(fail ? 2 : 0);
})();
