/**
 * ZaarHub "Search-engine submission" card — real-browser arm (queue item 12).
 *
 * The sitemap-ping feature (GET/PUT /seo/submission-settings, POST /seo/run-submission) is operated
 * from the admin console card #seo-submit-section. This arm proves the OPERATOR, ON THAT CARD, sees
 * the result of a run — not just that a log row appeared somewhere.
 *
 * Why this is not a formality: three console cards historically shared one `sub-*` id/function
 * namespace (Submission Queue, Newsletter Subscribers, Search-engine submission). `getElementById`
 * returns the FIRST match, so the sitemap card's status text was written into the Submission Queue
 * card and the card itself stayed blank — the feature looked dead on the very screen that drives it.
 * The `duplicate_ids` check below is the durable guard: no card's own element may share its id.
 *
 * Env: MD_BASE, MD_ADMIN_TOKEN, AUD, CHROME_PATH, NODE_PATH
 * Output: $AUD/browser-proof.json
 */
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TOKEN = process.env.MD_ADMIN_TOKEN;
const AUD = process.env.AUD || '/tmp';
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  if (!TOKEN) { console.error('FATAL no MD_ADMIN_TOKEN'); process.exit(2); }
  const proof = {
    card_present: false, run_button_found: false,
    card_status_before: '', card_status_after: '',
    duplicate_ids: [], console_errors: [], failing_api: [],
  };
  const browser = await puppeteer.launch({
    executablePath: CHROME, headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  try {
    const page = await browser.newPage();
    page.on('response', (r) => {
      const u = r.url();
      if (u.includes('/api/') && r.status() >= 400) proof.failing_api.push(r.status() + ' ' + u.replace(BASE, ''));
    });
    page.on('console', (m) => { if (m.type() === 'error') proof.console_errors.push(m.text().slice(0, 200)); });
    await page.evaluateOnNewDocument((t) => {
      localStorage.setItem('md_admin_token', t);
      localStorage.setItem('token', t);
      localStorage.setItem('md_admin_user', JSON.stringify({ email: 'verify@sitemap', role: 'super_admin', name: 'Verify' }));
      localStorage.setItem('user', JSON.stringify({ email: 'verify@sitemap', role: 'super_admin', name: 'Verify' }));
    }, TOKEN);

    await page.goto(BASE + '/admin-panel.html', { waitUntil: 'networkidle2', timeout: 60000 });
    await sleep(9000);

    // 1) the card is really on screen, with its own status line
    proof.card_present = await page.evaluate(() => !!document.querySelector('#seo-submit-section'));
    if (!proof.card_present) { throw new Error('sitemap card #seo-submit-section not on the page'); }

    proof.card_status_before = await page.evaluate(() => {
      const el = document.querySelector('#seo-submit-section .feature-status');
      return el ? (el.textContent || '').trim() : '';
    });

    // 2) the Run-now control is inside THIS card and is wired
    proof.run_button_found = await page.evaluate(() => {
      const card = document.querySelector('#seo-submit-section');
      if (!card) return false;
      return Array.from(card.querySelectorAll('button')).some((b) => /Run now/i.test(b.textContent || ''));
    });

    if (proof.run_button_found) {
      await page.evaluate(() => {
        const card = document.querySelector('#seo-submit-section');
        const btn = Array.from(card.querySelectorAll('button')).find((b) => /Run now/i.test(b.textContent || ''));
        btn.click();
      });
      await sleep(10000);
    }

    // 3) the result lands ON THE CARD — the assertion a URL-only or log-only check cannot make
    proof.card_status_after = await page.evaluate(() => {
      const el = document.querySelector('#seo-submit-section .feature-status');
      return el ? (el.textContent || '').trim() : '';
    });

    // 4) no card element may share an id with anything else on the page
    proof.duplicate_ids = await page.evaluate(() => {
      const count = {};
      document.querySelectorAll('[id]').forEach((e) => { count[e.id] = (count[e.id] || 0) + 1; });
      const bad = [];
      ['seo-submit-section', 'submissions-section', 'subscribers-section'].forEach((sel) => {
        const card = document.querySelector('#' + sel);
        if (!card) { bad.push('MISSING #' + sel); return; }
        card.querySelectorAll('[id]').forEach((e) => {
          if ((count[e.id] || 0) > 1) bad.push(e.id + ' x' + count[e.id] + ' (used in #' + sel + ')');
        });
      });
      return Array.from(new Set(bad));
    });

    const statusOk = /Run\b|target\(s\)|On\b|Off\b|Running/.test(proof.card_status_after);
    proof.status_visible_on_card = statusOk;
    proof.pass = proof.card_present && proof.run_button_found && statusOk
      && proof.duplicate_ids.length === 0
      && proof.console_errors.length === 0 && proof.failing_api.length === 0;
  } catch (e) {
    proof.fatal = String(e.message || e);
    proof.pass = false;
  } finally {
    fs.mkdirSync(AUD, { recursive: true });
    fs.writeFileSync(path.join(AUD, 'browser-proof.json'), JSON.stringify(proof, null, 2));
    console.log(JSON.stringify(proof, null, 2));
    await browser.close();
  }
  process.exit(proof.pass ? 0 : 1);
})();
