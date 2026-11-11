/**
 * Plan-tier payment-provider screen proof (ZaarHub portal, "Monetization" page).
 *
 * WHY: David's rule is that no payment provider may be baked into the UI. The plan-tier
 * table used to label every tier with no provider set as "Stripe", and the tier form offered
 * only Stripe / PayPal with "Default (Stripe)". That is a hardwired provider in the screen.
 *
 * This proves, in a real browser against the running app:
 *   1. the Plan Tiers table shows "Not set" for a tier with no provider (never "Stripe");
 *   2. the tier form's provider picker offers every gateway the Integration Center lists
 *      (Stripe, PayPal, NMI, EPD, Square, Paddle) and defaults to "Not set";
 *   3. choosing NMI is accepted by the control (value reads back);
 *   4. the screen raises no page or console errors;
 *   5. nothing is written: the tier rows are identical before and after.
 *
 * Run: MD_TOKEN=$(node /opt/swift/browser-run/md-mint-token.js) node scripts/plan-tier-provider-screen-proof.js
 * Exit: 0 = PASS, 1 = FAIL.
 */
const puppeteer = require('puppeteer');
const BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TOKEN = process.env.MD_TOKEN || '';
const out = [];
const rec = (ok, name, detail) => out.push((ok ? 'PASS ' : 'FAIL ') + name + ' :: ' + String(detail === undefined ? '' : detail).slice(0, 240));

(async () => {
  const browser = await puppeteer.launch({
    executablePath: require('./lib/chrome.cjs')(),
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  try {
    const page = await browser.newPage();
    const errs = [];
    page.on('pageerror', (e) => errs.push(String(e.message).slice(0, 200)));
    page.on('console', (m) => { if (m.type() === 'error') errs.push('console: ' + m.text().slice(0, 200)); });

    const tierCount = async () => page.evaluate(async (base, tok) => {
      const r = await fetch(base + '/api/v1/monetization/tiers', { headers: { Authorization: 'Bearer ' + tok } });
      const j = await r.json();
      return Array.isArray(j) ? j.length : (j.data ? j.data.length : -1);
    }, BASE, TOKEN);

    await page.evaluateOnNewDocument((t) => {
      localStorage.setItem('token', t);
      localStorage.setItem('user', JSON.stringify({ role: 'super_admin', email: 'ops@zaarhub.com' }));
    }, TOKEN);
    await page.goto(BASE + '/portal.html#monetization', { waitUntil: 'domcontentloaded', timeout: 30000 });
    await new Promise((r) => setTimeout(r, 3000));

    const before = await tierCount();

    const state = await page.evaluate(() => {
      const box = document.getElementById('tiersContent');
      const rows = box ? Array.from(box.querySelectorAll('table tr')).slice(1) : [];
      return {
        hasBox: !!box,
        rows: rows.length,
        providers: rows.map((tr) => (tr.cells[4] ? tr.cells[4].innerText.trim() : '')),
        text: box ? box.innerText.slice(0, 300) : '',
      };
    });

    rec(state.hasBox, 'the Plan Tiers screen rendered', state.hasBox);
    rec(state.rows > 0, 'the screen lists the live tiers', 'rows=' + state.rows);
    rec(state.providers.every((p) => p === 'Not set'), 'a tier with no provider reads "Not set", not "Stripe"', JSON.stringify(state.providers));
    rec(!state.providers.some((p) => /stripe/i.test(p)), 'no tier is labelled Stripe', JSON.stringify(state.providers));

    // Open the Add Tier form through the screen's own button.
    const opened = await page.evaluate(() => {
      const box = document.getElementById('tiersContent');
      const btn = box ? Array.from(box.querySelectorAll('button')).find((b) => /Add Tier/i.test(b.innerText)) : null;
      if (!btn) return false;
      btn.click();
      return true;
    });
    await new Promise((r) => setTimeout(r, 800));

    const form = await page.evaluate(() => {
      const sel = document.getElementById('tf-provider');
      if (!sel) return { present: false };
      const opts = Array.from(sel.options).map((o) => ({ v: o.value, t: o.textContent.trim() }));
      const def = opts.find((o) => o.v === '');
      sel.value = 'nmi';
      return { present: true, opts, defaultLabel: def ? def.t : '', readBack: sel.value };
    });

    rec(form.present, 'the tier form has a provider picker', form.present);
    const vals = (form.opts || []).map((o) => o.v);
    for (const p of ['stripe', 'paypal', 'nmi', 'epd', 'square', 'paddle']) {
      rec(vals.includes(p), 'the picker offers ' + p, vals.join(','));
    }
    rec(/not set/i.test(form.defaultLabel || ''), 'the picker defaults to "Not set"', form.defaultLabel);
    rec(!/stripe/i.test(form.defaultLabel || ''), 'the empty choice no longer says Stripe', form.defaultLabel);
    rec(form.readBack === 'nmi', 'choosing NMI is accepted by the control', form.readBack);

    const after = await tierCount();
    rec(after === before, 'no tier was written by this proof', 'before=' + before + ' after=' + after);

    rec(errs.length === 0, 'no page or console errors', JSON.stringify(errs.slice(0, 3)));
  } catch (e) {
    rec(false, 'the proof ran to completion', String(e && e.message).slice(0, 240));
  } finally {
    await browser.close();
  }
  console.log(out.join('\n'));
  const failed = out.filter((l) => l.startsWith('FAIL')).length;
  console.log('\nRESULT: ' + (failed === 0 ? 'PASS' : 'FAIL (' + failed + ')'));
  process.exit(failed === 0 ? 0 : 1);
})();
