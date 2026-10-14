/**
 * Durable proof — the Directory Portal's SEO card (/portal.html, SEO page) really loads and
 * really saves the meta a live city page renders.
 *
 * The defect this pins: the card read/wrote `seo_meta` under page_type='directory', while the
 * city page (src/handlers/subfolder.rs:914) reads page_type='city' — so the fields came up
 * blank and "SEO settings saved" changed nothing on the site.
 *
 * Driven by scripts/directory-portal-seo-smoke.py, which mints the token, checks the DB and the
 * live page, and restores the original values. Two modes:
 *   MODE=read   load the SEO page through the portal's own router, report what the fields show
 *   MODE=write  type PROBE_TITLE and press the card's own Save button, report the toast + calls
 *
 * Env: MD_BASE, TOKEN, MODE, PROBE_TITLE, CHROME_PATH, NODE_PATH
 */
const puppeteer = require('puppeteer');

const BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TOKEN = process.env.TOKEN;
const MODE = process.env.MODE || 'read';
const PROBE = process.env.PROBE_TITLE || 'ZZ PORTAL SEO PROBE';

(async () => {
  const browser = await puppeteer.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const out = { mode: MODE, pageErrors: [], consoleErrors: [], api: [], toast: null };
  try {
    const page = await browser.newPage();
    page.on('pageerror', (e) => out.pageErrors.push(String(e.message).slice(0, 200)));
    page.on('console', (m) => { if (m.type() === 'error') out.consoleErrors.push(m.text().slice(0, 200)); });
    page.on('response', (r) => {
      const u = r.url();
      if (u.indexOf('/api/v1/seo/') !== -1 || u.indexOf('/api/v1/directories/') !== -1 || u.indexOf('/api/v1/portal/') !== -1) {
        out.api.push({ url: u.replace(BASE, ''), method: r.request().method(), status: r.status() });
      }
    });
    await page.evaluateOnNewDocument((tok) => {
      localStorage.setItem('token', tok);
      localStorage.setItem('user', JSON.stringify({ email: 'seo-card-proof@swiftsoftware.dev', role: 'super_admin' }));
    }, TOKEN);
    await page.goto(BASE + '/portal.html', { waitUntil: 'networkidle2', timeout: 60000 });
    await page.waitForSelector('.sidebar-nav', { timeout: 30000 });
    out.dir = await page.evaluate(() => (window.CUR_DIR ? { id: window.CUR_DIR.id, slug: window.CUR_DIR.slug, name: window.CUR_DIR.name } : null));
    await page.evaluate(() => window.navigate('seo'));
    await page.waitForSelector('#seo_title', { timeout: 20000 });
    // the card fires two GETs; give the seo_meta one time to land before reading the fields
    await new Promise((r) => setTimeout(r, 1200));
    out.fields = await page.evaluate(() => ({
      title: (document.getElementById('seo_title') || {}).value,
      desc: (document.getElementById('seo_desc') || {}).value,
      kw: (document.getElementById('seo_kw') || {}).value,
    }));
    if (MODE === 'write') {
      await page.evaluate((v) => { document.getElementById('seo_title').value = v; }, PROBE);
      await page.click('button[onclick*="saveSEO"]');
      await page.waitForFunction(
        () => { const t = document.querySelector('.toast, #toast, .toast-container'); return !!t && t.textContent.trim().length > 0; },
        { timeout: 15000 },
      ).catch(() => {});
      await new Promise((r) => setTimeout(r, 900));
      out.toast = await page.evaluate(() => {
        const els = Array.from(document.querySelectorAll('.toast, #toast, .toast-container, .toast-msg'));
        return els.map((e) => e.textContent.trim()).filter(Boolean).join(' | ');
      });
      out.fieldAfterSave = await page.evaluate(() => (document.getElementById('seo_title') || {}).value);
    }
    await page.screenshot({ path: (process.env.OUT || '/tmp') + '/portal-seo-' + MODE + '.png' });
  } finally {
    await browser.close();
  }
  console.log(JSON.stringify(out, null, 1));
})().catch((e) => { console.error('PROBE ERROR: ' + e.message); process.exit(1); });
