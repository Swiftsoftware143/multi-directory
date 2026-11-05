/**
 * Plan payment-link screen proof (ZaarHub console + business dashboard).
 *
 * WHY: David's payment switch is NMI/EPD, and neither has a built receiver yet. What CAN work
 * today is the operator pasting the provider's own hosted payment-page link onto a plan tier.
 * That link existed in the database and in the tier API, but the console had NO field to enter
 * it, and a business with no subscription had no reachable way to a plan at all ("contact admin").
 *
 * This proves, in a real browser against the running app:
 *   1. the Plan Tiers edit form has a "Payment page link" control and it is populated from the row;
 *   2. saving through that control persists the value (HTTP 200 + DB row);
 *   3. a business with NO subscription now sees the published plans on its dashboard, with the
 *      operator's link on the tier that has one;
 *   4. clearing the control through the same form restores the row (nothing left behind).
 *
 * Run: MD_TOKEN=$(node /opt/swift/browser-run/md-mint-token.js) node scripts/plan-payment-link-screen-proof.js
 * Exit: 0 = PASS, 1 = FAIL. Evidence in /opt/swift/audits/zz-item16-paylink-screen-<ts>/
 */
const puppeteer = require('puppeteer');
const fs = require('fs');
const { execFileSync } = require('child_process');

const BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TOKEN = process.env.MD_TOKEN || '';
const TIER = process.env.MD_TIER || '733d71d0';
const BIZ = process.env.MD_BIZ || '79fb6665-798f-498a-8526-79aca71d77bd';
const PROBE = process.env.MD_PROBE_URL || (BASE + '/probe-payment-page?plan=featured');
const OUT = '/opt/swift/audits/zz-item16-paylink-screen-' + new Date().toISOString().replace(/[-:T]/g, '').slice(0, 15);
const out = [];
const rec = (ok, name, detail) => out.push((ok ? 'PASS ' : 'FAIL ') + name + ' :: ' + String(detail === undefined ? '' : detail).slice(0, 300));

function psql(q) {
  const pw = execFileSync('bash', ['-c', "grep -E '^DATABASE_URL' /etc/swift/env/multi-directory.env | head -1 | sed -E 's#.*://[^:]+:([^@]+)@.*#\\1#'"], { encoding: 'utf8' }).trim();
  return execFileSync('docker', ['exec', '-e', 'PGPASSWORD=' + pw, 'swift-postgres', 'psql', '-U', 'swift', '-d', 'multi_directory', '-tAc', q], { encoding: 'utf8' }).trim();
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  if (!TOKEN) { console.log('FAIL no MD_TOKEN'); process.exit(1); }
  const browser = await puppeteer.launch({
    executablePath: '/usr/bin/chromium-browser',
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const tierId = psql(`select id from plan_tiers where id::text like '${TIER}%' limit 1`);
  const before = psql(`select coalesce(plan_sales_page_url,'(null)') from plan_tiers where id='${tierId}'`);
  try {
    const page = await browser.newPage();
    const errs = [];
    page.on('pageerror', (e) => errs.push(String(e.message).slice(0, 200)));
    page.on('console', (m) => { if (m.type() === 'error') errs.push('console: ' + m.text().slice(0, 200)); });

    await page.evaluateOnNewDocument((tok) => {
      localStorage.setItem('token', tok);
      localStorage.setItem('visitorToken', tok);
      localStorage.setItem('user', JSON.stringify({ role: 'super_admin', email: 'ops@zaarhub.com' }));
    }, TOKEN);
    await page.goto(BASE + '/portal.html#monetization', { waitUntil: 'networkidle2', timeout: 60000 });
    await page.waitForFunction(() => !!(window.CUR_DIR && window.CUR_DIR.slug), { timeout: 30000 });

    // Open the console's Monetization page through its own render function and wait for tiers.
    await page.evaluate(() => { if (typeof renderMonetization === 'function') renderMonetization(); });
    await page.waitForFunction(() => Array.isArray(window._tierList) && window._tierList.length > 0, { timeout: 30000 });
    rec(true, 'the console listed the plan tiers', 'rows=' + (await page.evaluate(() => window._tierList.length)));

    // Open the edit form for our tier, exactly as the row's pencil button does.
    await page.evaluate((id) => editTier(id), tierId);
    await page.waitForSelector('#tf-salespage', { timeout: 10000 });
    const field = await page.evaluate(() => document.getElementById('tf-salespage').value);
    rec(field === (before === '(null)' ? '' : before), 'the edit form carries the saved payment link', 'field="' + field + '" row=' + before);
    await page.screenshot({ path: OUT + '/tier-form.png' });

    // Save through the form's own Save button.
    const saved = new Promise((res) => {
      const h = (r) => { if (r.url().includes('/monetization/tiers/' + tierId) && r.request().method() === 'PUT') { page.off('response', h); res(r.status()); } };
      page.on('response', h);
    });
    await page.evaluate((u) => { document.getElementById('tf-salespage').value = u; }, PROBE);
    await page.evaluate(() => saveTier());
    const status = await Promise.race([saved, new Promise((r) => setTimeout(() => r(0), 15000))]);
    rec(status === 200, 'saving from the screen is accepted', 'HTTP ' + status);
    const stored = psql(`select coalesce(plan_sales_page_url,'(null)') from plan_tiers where id='${tierId}'`);
    rec(stored === PROBE, 'the saved link is the row value', stored);

    // A plan list fetched anonymously must carry it (that is what the business screen reads).
    const pub = await page.evaluate(async (b) => (await fetch(b + '/api/v1/subscriptions/plans')).json(), BASE);
    const feat = (pub.plans || []).find((p) => p.plan_sales_page_url);
    rec(!!feat && feat.plan_sales_page_url === PROBE, 'the public plan list carries the link', feat ? feat.slug : 'none');

    // The REAL owner portal (/portal): the Upgrade Plan grid and the decision function behind its
    // Subscribe button must send a paid tier to the operator's payment page.
    const owner = await browser.newPage();
    const ownerErrs = [];
    owner.on('pageerror', (e) => ownerErrs.push(String(e.message).slice(0, 200)));
    owner.on('dialog', async (d) => { await d.dismiss(); });
    await owner.evaluateOnNewDocument((tok, biz) => {
      localStorage.setItem('token', tok);
      localStorage.setItem('visitorToken', tok);
      localStorage.setItem('biz_dash_id', biz);
    }, TOKEN, BIZ);
    await owner.goto(BASE + '/portal', { waitUntil: 'networkidle2', timeout: 60000 });
    const hasPlans = await owner.evaluate(async () => {
      if (typeof loadPlans !== 'function') return 'no loadPlans';
      loadPlans();
      for (let i = 0; i < 60 && !(window._cachedPlans || []).length; i++) await new Promise((r) => setTimeout(r, 250));
      return (window._cachedPlans || []).length;
    });
    rec(hasPlans === 3, 'the owner portal lists the published plans', 'plans=' + hasPlans);
    const decision = await owner.evaluate(async (biz) => {
      const plans = window._cachedPlans || [];
      const p = plans.find((x) => x.plan_sales_page_url) || plans.find((x) => parseFloat(x.price_monthly || 0) > 0);
      if (!p) return { err: 'no paid plan' };
      const res = await buyPlan(p, biz, 'monthly');
      return { plan: p.name, ok: res.ok, url: res.checkout_url || '', msg: res.message };
    }, BIZ);
    rec(decision.ok === true && decision.url === PROBE, 'the Subscribe decision sends the buyer to the payment page', JSON.stringify(decision));
    const grid = await owner.evaluate(() => ({ cards: document.querySelectorAll('#plansGrid .plan-card').length, buttons: document.querySelectorAll('#plansGrid button').length }));
    rec(grid.cards === 3 && grid.buttons >= 3, 'the plan grid renders a button per plan', JSON.stringify(grid));
    await owner.screenshot({ path: OUT + '/owner-plans.png', fullPage: true });
    rec(ownerErrs.length === 0, 'the owner portal raised no page errors', ownerErrs.join(' | '));

    // The standalone dashboard page (served at /business-dashboard.html) must not show a dead
    // "contact admin" sentence to a listing with no subscription.
    const dash = await browser.newPage();
    const dashErrs = [];
    dash.on('pageerror', (e) => dashErrs.push(String(e.message).slice(0, 200)));
    await dash.evaluateOnNewDocument((tok, biz) => {
      localStorage.setItem('token', tok);
      localStorage.setItem('visitorToken', tok);
      localStorage.setItem('biz_dash_id', biz);
      localStorage.setItem('biz_dash_email', 'proof@invalid.test');
    }, TOKEN, BIZ);
    await dash.goto(BASE + '/business-dashboard.html', { waitUntil: 'networkidle2', timeout: 60000 });
    await dash.waitForFunction(() => document.querySelectorAll('a[data-plan]').length > 0, { timeout: 30000 }).catch(() => {});
    const view = await dash.evaluate(() => ({
      plans: Array.from(document.querySelectorAll('a[data-plan]')).map((a) => ({ plan: a.getAttribute('data-plan'), href: a.href, text: a.textContent.trim() })),
      card: !!Array.from(document.querySelectorAll('h3')).find((h) => /Subscription & Plan/.test(h.textContent)),
    }));
    const linked = view.plans.find((p) => p.href === PROBE);
    rec(view.card && view.plans.length >= 1, 'a listing with no plan sees the plan card and a way to buy', 'links=' + view.plans.length);
    rec(!!linked, 'that page links the tier to the payment page too', linked ? linked.href : JSON.stringify(view.plans));
    await dash.screenshot({ path: OUT + '/business-plans.png', fullPage: true });
    rec(dashErrs.length === 0, 'the dashboard page raised no page errors', dashErrs.join(' | '));

    // Restore through the same control.
    await page.evaluate((id) => { if (typeof renderMonetization === 'function') renderMonetization(); }, tierId);
    await page.waitForFunction(() => Array.isArray(window._tierList) && window._tierList.length > 0, { timeout: 30000 });
    await page.evaluate((id) => editTier(id), tierId);
    await page.waitForSelector('#tf-salespage', { timeout: 10000 });
    const restored = new Promise((res) => {
      const h = (r) => { if (r.url().includes('/monetization/tiers/' + tierId) && r.request().method() === 'PUT') { page.off('response', h); res(r.status()); } };
      page.on('response', h);
    });
    await page.evaluate(() => { document.getElementById('tf-salespage').value = ''; });
    await page.evaluate(() => saveTier());
    await Promise.race([restored, new Promise((r) => setTimeout(() => r(0), 15000))]);
    const after = psql(`select coalesce(plan_sales_page_url,'(null)') from plan_tiers where id='${tierId}'`);
    const expected = before === PROBE ? '(null)' : before;
    rec(after === expected, 'the row is restored (an empty value really clears the link)', before + ' -> ' + after);
    rec(after === '(null)', 'clearing the field removed the link from the database', after);
    rec(errs.length === 0, 'the console raised no page errors', errs.join(' | '));
  } finally {
    await browser.close();
  }
  const fails = out.filter((l) => l.startsWith('FAIL')).length;
  fs.writeFileSync(OUT + '/proof.txt', out.join('\n') + '\n\nRESULT: ' + (fails ? 'FAIL' : 'PASS') + ' (' + (out.length - fails) + ' passed, ' + fails + ' failed)\n');
  console.log(out.join('\n'));
  console.log('\nRESULT: ' + (fails ? 'FAIL' : 'PASS'));
  console.log('evidence: ' + OUT);
  process.exit(fails ? 1 : 0);
})().catch((e) => { console.error("FAIL " + (e && e.stack || e)); process.exit(1); });
