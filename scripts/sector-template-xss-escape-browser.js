/**
 * Durable proof (browser arm) — the 9 sector template pages (/templates/<sector>.html?slug=…)
 * must render a directory's business rows as TEXT, not HTML.
 *
 * The defect: every card + map popup + filter label on those pages was built with innerHTML from
 * the raw business row, and a business owner chooses their own name/city/address/meta text — so one
 * renamed listing was a live payload on a public URL. Fixed by escaping once at the data boundary
 * (`allBiz = escDeep(...)`), same class as the /biz/:id review XSS (1b8e7c2).
 *
 * Env: MD_URL, SLUG, PAYLOAD_NAME, PAYLOAD_CITY, SHOT_DIR, CHROME_PATH
 * Exit: 0 = all browser checks passed, 1 = a check failed.
 */
const puppeteer = require('puppeteer-core');

const BASE = process.env.MD_URL || 'http://127.0.0.1:8089';
const SLUG = process.env.SLUG;
const PAYLOAD_NAME = process.env.PAYLOAD_NAME;
const PAYLOAD_CITY = process.env.PAYLOAD_CITY;
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
    const responses = [];
    page.on('response', (r) => responses.push({ url: r.url(), status: r.status() }));

    await page.goto(`${BASE}/templates/farm.html?slug=${encodeURIComponent(SLUG)}`,
      { waitUntil: 'networkidle2', timeout: 60000 });
    await page.waitForSelector('.card', { timeout: 30000 }).catch(() => {});
    await new Promise((r) => setTimeout(r, 1000));

    const state = await page.evaluate(() => {
      const cards = Array.from(document.querySelectorAll('.card'));
      const h3 = cards.map((c) => (c.querySelector('h3') || {}).textContent || '');
      const texts = cards.map((c) => c.textContent || '');
      const bodies = cards.map((c) => c.innerHTML);
      return {
        xssProbe: window.__xssProbe || null,
        xssName: window.__xssName || null,
        injectedImgs: document.querySelectorAll('img[src="x"], img[src="y"], img[src="z"]').length,
        cards: cards.length,
        headings: h3,
        texts: texts,
        rawImgInCards: bodies.some((b) => b.includes('<img')),
      };
    });

    if (state.cards >= 1) ok(`the sector page rendered ${state.cards} card(s) from the API`);
    else bad('no card rendered — the escaping cannot be judged');

    if (state.headings.some((h) => h.includes(PAYLOAD_NAME))) ok('the business NAME is displayed verbatim as text');
    else bad(`the payload name was not shown as text (headings: ${JSON.stringify(state.headings.slice(0, 3))})`);

    if (state.texts.some((t) => t.includes(PAYLOAD_CITY))) ok('the payload CITY is displayed verbatim as text');
    else bad(`the payload city was not shown as text (cards: ${JSON.stringify(state.texts.slice(0, 2))})`);

    if (state.xssProbe === null && state.xssName === null) ok('no injected event handler executed (window.__xssProbe / __xssName still unset)');
    else bad(`an injected handler EXECUTED (probe=${state.xssProbe} name=${state.xssName})`);

    if (state.injectedImgs === 0) ok('no <img> element was created from the payload');
    else bad(`${state.injectedImgs} <img> element(s) were created from the payload`);

    if (!state.rawImgInCards) ok('no raw <img> tag in the rendered card markup');
    else bad('a rendered card contains a RAW <img> tag (interpolated, not escaped)');

    if (pageErrors.length === 0) ok('no uncaught page errors');
    else bad(`${pageErrors.length} page error(s): ${pageErrors.slice(0, 2).join(' | ')}`);

    const api = responses.filter((r) => r.url.includes('/api/v1/directories/'));
    const listingApi = api.filter((r) => r.url.includes('/businesses'));
    if (listingApi.length && listingApi.every((r) => r.status === 200)) ok('the page\'s business-list API answered 200');
    else bad(`the business-list API failed: ${JSON.stringify(listingApi)}`);

    await page.screenshot({ path: `${SHOT_DIR}/sector-farm-card.png`, fullPage: false });

    // control — run last so it cannot contaminate the assertions above
    const controlFired = await page.evaluate((p) => new Promise((res) => {
      const d = document.createElement('div');
      d.style.position = 'fixed'; d.style.left = '-9999px';
      document.body.appendChild(d);
      d.innerHTML = p;
      setTimeout(() => { const hit = window.__xssProbe === 1; d.remove(); res(hit); }, 400);
    }), PAYLOAD_NAME);
    if (controlFired) ok('control: the same payload DOES execute when interpolated raw — the probe detects the defect');
    else bad('control failed: the payload did not execute even when interpolated raw — the probe cannot detect the defect');
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
