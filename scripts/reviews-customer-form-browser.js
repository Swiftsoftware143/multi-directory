/**
 * Item 1 (customer half) — a REAL customer leaves a review on the public business page.
 *
 * The defect this pins: POST /api/v1/reviews existed and the business portal could moderate it,
 * but no screen ever called it. The public business page rendered reviews and offered nothing to
 * write one, so every review on the live site arrived by API/import and the owner's approve queue
 * could never be fed by a real visitor.
 *
 * Drives the real page (frontend/business-detail.html served at /biz/:id):
 *   1. sign the customer in by planting the same localStorage token the site's own sign-in writes;
 *   2. the Reviews card must offer a "Write a review" form with five star controls;
 *   3. pick 5 stars, type the text, click "Post review";
 *   4. the browser must call the real endpoint and the form must say the review awaits approval.
 *
 * Env: BASE, BIZ_ID, TOKEN, SHOT_DIR, CHROME_PATH, NODE_PATH
 * Exit: 0 = all checks passed, 1 = a check failed.
 */
const puppeteer = require('puppeteer-core');
const fs = require('fs');

const BASE = (process.env.BASE || 'http://127.0.0.1:8089').replace(/\/$/, '');
const BIZ = process.env.BIZ_ID;
const TOKEN = process.env.TOKEN;
const SHOT_DIR = process.env.SHOT_DIR || '/tmp';
const TEXT = process.env.REVIEW_TEXT || 'Probe review typed by a customer in the browser.';

(async () => {
  const problems = [];
  const ok = (m) => console.log('PASS  ' + m);
  const bad = (m) => { problems.push(m); console.log('FAIL  ' + m); };

  const browser = await puppeteer.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--window-size=1400,1000'],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1400, height: 1000 });

  const consoleErrors = [];
  page.on('console', (m) => {
    if (m.type() !== 'error') return;
    const t = m.text();
    if (/navigator\.vibrate|Failed to load resource: the server responded with a status of 404 .*favicon/i.test(t)) return;
    consoleErrors.push(t);
  });
  page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));

  const posts = [];
  page.on('response', async (r) => {
    if (r.request().method() !== 'POST') return;
    if (!/\/api\/v1\/reviews\/?$/.test(new URL(r.url()).pathname)) return;
    let body = '';
    try { body = await r.text(); } catch (e) { body = ''; }
    posts.push({ status: r.status(), body });
  });

  try {
    await page.evaluateOnNewDocument((t) => localStorage.setItem('zh_token', t), TOKEN);
    const resp = await page.goto(`${BASE}/biz/${BIZ}`, { waitUntil: 'domcontentloaded', timeout: 45000 });
    if (resp && resp.status() === 200) ok('the business page loaded (200)');
    else bad('the business page did not answer 200: ' + (resp && resp.status()));

    await page.waitForSelector('#reviewForm', { timeout: 20000 });
    const form = await page.$eval('#reviewForm', (el) => ({
      visible: !!(el.offsetWidth || el.offsetHeight),
      text: el.innerText,
      stars: el.querySelectorAll('#reviewStars span').length,
      hasSubmit: !!el.querySelector('#reviewSubmit'),
      hasText: !!el.querySelector('#reviewText'),
    }));
    if (form.visible) ok('the Reviews card shows a "Write a review" form');
    else bad('no visible write-review form on the business page');
    if (/write a review/i.test(form.text)) ok('the form names itself "Write a review"');
    else bad('the form has no title');
    if (form.stars === 5) ok('the form offers five star controls');
    else bad('expected 5 star controls, saw ' + form.stars);

    // Regression guard for a real defect found while building this form: the photo lightbox is
    // `position:fixed` over the whole viewport and, with only `opacity:0`, it stayed clickable —
    // so EVERY control on the business page (Save, Follow, the review form) silently ate clicks.
    // A click must land on the star itself, not on an invisible overlay.
    const clickable = await page.evaluate(() => {
      const s = document.querySelector('#reviewStars span[data-star="5"]');
      s.scrollIntoView({ block: 'center' });
      const r = s.getBoundingClientRect();
      return document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === s;
    });
    if (clickable) ok('nothing invisible covers the page (the photo lightbox no longer swallows clicks)');
    else bad('an invisible overlay is intercepting clicks on the review form');

    await page.click('#reviewStars span[data-star="5"]');
    const hint = await page.$eval('#reviewStarsHint', (el) => el.innerText.trim());
    if (/5 of 5/.test(hint)) ok('picking the 5th star is reflected on screen');
    else bad('star pick not reflected: ' + hint);

    await page.click('#reviewText');
    await page.type('#reviewText', TEXT, { delay: 8 });
    const typed = await page.$eval('#reviewText', (el) => el.value);
    if (typed === TEXT) ok('the customer can type the review text');
    else bad('textarea did not take the text');

    await page.screenshot({ path: `${SHOT_DIR}/customer-review-form.png` });

    await page.click('#reviewSubmit');
    await page.waitForFunction(
      () => /for approval/i.test((document.getElementById('reviewForm') || {}).innerText || ''),
      { timeout: 20000 }
    );
    ok('the screen confirms the review was sent for approval');
    await page.screenshot({ path: `${SHOT_DIR}/customer-review-sent.png` });

    const post = posts.find((p) => p.status === 200 || p.status === 201);
    if (post) ok(`the browser posted the real endpoint (${post.status})`);
    else bad('the browser never posted /api/v1/reviews successfully — saw ' + JSON.stringify(posts.map((p) => p.status)));

    if (consoleErrors.length === 0) ok('no JavaScript console errors during the flow');
    else bad('console errors: ' + consoleErrors.slice(0, 3).join(' | '));
  } catch (e) {
    bad('flow threw: ' + e.message);
    try { await page.screenshot({ path: `${SHOT_DIR}/customer-review-error.png` }); } catch (_) {}
  } finally {
    await browser.close();
  }

  fs.writeFileSync(`${SHOT_DIR}/customer-review-browser.json`, JSON.stringify({ posts, consoleErrors, problems }, null, 2));
  console.log(problems.length ? `BROWSER RESULT: ${problems.length} failed` : 'BROWSER RESULT: all passed');
  process.exit(problems.length ? 1 : 0);
})();
