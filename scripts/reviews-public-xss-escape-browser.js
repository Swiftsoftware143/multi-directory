/**
 * Durable proof (browser arm) — item 1, the ESCAPING half: the public business page
 * (/biz/:id, frontend/business-detail.html) must render an approved review's text, its
 * reviewer name and the business name as TEXT — never as HTML.
 *
 * The defect class this pins (stored XSS, fixed in 1b8e7c2): the page built its review cards
 * with `innerHTML` and interpolated the API's `content` / `reviewer_name` raw, so ANY accepted
 * review payload became live markup on a public page. An API-level check cannot see this —
 * the API returns the payload either way. Only a real browser on the real route can.
 *
 * The probe review's content and reviewer name are event-handler payloads, so a failure is
 * unambiguous: if the markup were interpolated raw, the <img> would be created, its src would
 * 404, onerror would fire and set window.__xssProbe / window.__xssName.
 *
 * Driven by scripts/reviews-public-xss-escape-smoke.py (it mints the fixture and asserts the DB).
 * Env: MD_URL, BIZ_ID, PAYLOAD_CONTENT, PAYLOAD_NAME, SHOT_DIR, CHROME_PATH, NODE_PATH
 * Exit: 0 = all browser checks passed, 1 = a check failed.
 */
const puppeteer = require('puppeteer-core');

const BASE = process.env.MD_URL || 'http://127.0.0.1:8089';
const BID = process.env.BIZ_ID;
const PAYLOAD_CONTENT = process.env.PAYLOAD_CONTENT;
const PAYLOAD_NAME = process.env.PAYLOAD_NAME;
const SHOT_DIR = process.env.SHOT_DIR || '/tmp';

(async () => {
  const problems = [];
  const ok = (m) => console.log('PASS  ' + m);
  const bad = (m) => { problems.push(m); console.log('FAIL  ' + m); };

  const browser = await puppeteer.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });

  try {
    const page = await browser.newPage();
    const pageErrors = [];
    page.on('pageerror', (e) => pageErrors.push(String(e)));

    await page.goto(`${BASE}/biz/${BID}`, { waitUntil: 'networkidle2', timeout: 60000 });
    await page.waitForSelector('.review-content', { timeout: 30000 }).catch(() => {});
    await new Promise((r) => setTimeout(r, 1000));

    const state = await page.evaluate(() => {
      const c = document.querySelector('.review-content');
      const n = document.querySelector('.reviewer-name');
      return {
        xssProbe: window.__xssProbe || null,
        xssName: window.__xssName || null,
        injectedImgs: document.querySelectorAll('img[src="x"], img[src="y"]').length,
        reviewCards: document.querySelectorAll('.review-card').length,
        contentText: c ? c.textContent : '',
        contentHTML: c ? c.innerHTML : '',
        nameText: n ? n.textContent : '',
        title: document.title,
      };
    });

    // 1. the review really rendered at all (guards against a vacuous pass)
    if (state.reviewCards >= 1) ok(`the approved review rendered on the public page (${state.reviewCards} card(s))`);
    else bad('no review card rendered — the escaping cannot be judged');

    // 2. the payload is shown as TEXT
    if (state.contentText.includes(PAYLOAD_CONTENT)) ok('the review text is displayed verbatim as text');
    else bad(`the review text was not shown verbatim (saw: ${JSON.stringify(state.contentText.slice(0, 120))})`);

    if (state.nameText.includes(PAYLOAD_NAME)) ok('the reviewer name is displayed verbatim as text');
    else bad(`the reviewer name was not shown verbatim (saw: ${JSON.stringify(state.nameText.slice(0, 120))})`);

    // 3. nothing executed and no element was created from the payload
    if (state.xssProbe === null && state.xssName === null) ok('no injected event handler executed (window.__xssProbe / __xssName still unset)');
    else bad(`an injected handler EXECUTED (probe=${state.xssProbe} name=${state.xssName})`);

    if (state.injectedImgs === 0) ok('no <img> element was created from the payload');
    else bad(`${state.injectedImgs} <img> element(s) were created from the payload`);

    if (!state.contentHTML.includes('<img')) ok('the review content markup contains no raw <img> tag');
    else bad('the review content markup contains a RAW <img> tag (interpolated, not escaped)');

    if (pageErrors.length === 0) ok('no uncaught page errors');
    else bad(`${pageErrors.length} page error(s): ${pageErrors.slice(0, 2).join(' | ')}`);

    await page.screenshot({ path: `${SHOT_DIR}/biz-xss-review.png`, fullPage: false });
    console.log('  screenshot: ' + `${SHOT_DIR}/biz-xss-review.png`);
  } catch (e) {
    bad('the browser flow threw: ' + e.message);
  } finally {
    await browser.close();
  }

  console.log('');
  if (problems.length) {
    console.log(`RESULT: FAIL (${problems.length})`);
    process.exit(1);
  }
  console.log('RESULT: PASS');
  process.exit(0);
})();
