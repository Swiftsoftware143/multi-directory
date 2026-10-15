/**
 * Browser arm — item 18 / search: the public search screen renders the businesses the API matches.
 *
 * The defect this pins: /api/v1/zaarhub/search answered {"listings": [...]} while the page read
 * `results` / `deals` / `total`, so every search showed "No results found" even though the API
 * answered 200 with matching rows (measured 2026-10-15).
 *
 * Drives the REAL page (no stubs) at <BASE>/#/search/<MARKER>:
 *   1. the page's own router loads the search view for the marker;
 *   2. a business card for the probe business is rendered;
 *   3. the card links to #/city/<directory_slug>/biz/<id>;
 *   4. the page shows "<N> results found" and does NOT show "No results found";
 *   5. no JavaScript console error, and the search API answered 200.
 *
 * Driven by scripts/search-query-smoke.py, which seeds the probe business and asserts the API.
 * Env: MD_BASE, MARKER, EXPECT_SLUG, EXPECT_ID, CHROME_PATH, NODE_PATH, SHOT_DIR
 * Exit: 0 = all browser checks passed, 1 = a check failed (or the flow threw).
 */
const puppeteer = require('puppeteer-core');

const BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const MARKER = process.env.MARKER;
const EXPECT_SLUG = process.env.EXPECT_SLUG || '';
const EXPECT_ID = process.env.EXPECT_ID || '';
const SHOT_DIR = process.env.SHOT_DIR || '/tmp';

(async () => {
  const problems = [];
  const ok = (m) => console.log('PASS  ' + m);
  const bad = (m) => { problems.push(m); console.log('FAIL  ' + m); };

  if (!MARKER) { console.log('FAIL  MARKER is required'); process.exit(1); }

  const browser = await puppeteer.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1360, height: 1000 });

  const consoleErrors = [];
  page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text()); });
  page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));

  const searchCalls = [];
  page.on('response', (r) => {
    const u = r.url();
    if (u.includes('/api/v1/zaarhub/search')) {
      searchCalls.push(r.status() + ' ' + u.replace(/^https?:\/\/[^/]+/, ''));
    }
  });

  try {
    const url = BASE + '/#/search/' + encodeURIComponent(MARKER);
    await page.goto(url, { waitUntil: 'networkidle2', timeout: 60000 });

    // The page's own router has to land on the search view for the marker.
    await page.waitForFunction(
      (m) => {
        const el = document.querySelector('#zh-content .zh-section .zh-section-inner');
        if (!el) return false;
        return el.querySelectorAll('.zh-biz-card').length > 0 || /No results found/.test(el.textContent || '');
      },
      { timeout: 30000 },
      MARKER
    );

    const info = await page.evaluate(() => {
      const el = document.querySelector('#zh-content .zh-section .zh-section-inner');
      const cards = Array.from(el.querySelectorAll('.zh-biz-card'));
      return {
        cards: cards.length,
        text: el.textContent || '',
        firstOnclick: cards.length ? (cards[0].getAttribute('onclick') || '') : '',
        firstText: cards.length ? (cards[0].textContent || '') : '',
      };
    });

    if (info.cards > 0) ok(`the search screen rendered ${info.cards} business card(s) for "${MARKER}"`);
    else bad(`the search screen rendered NO business card for "${MARKER}"`);

    if (!/No results found/.test(info.text)) ok('the screen does not say "No results found"');
    else bad('the screen still says "No results found"');

    if (new RegExp('\\d+\\s+results? found').test(info.text)) ok('the screen shows the "<N> results found" header');
    else bad('the screen is missing the results-count header');

    if (info.firstOnclick.includes('#/city/') && info.firstOnclick.includes('/biz/')) {
      ok('the card links to the business-detail route: ' + info.firstOnclick.replace(/\s+/g, ' ').trim());
    } else {
      bad('the card does not link to #/city/<slug>/biz/<id> (onclick=' + info.firstOnclick + ')');
    }

    if (EXPECT_SLUG && EXPECT_ID) {
      const wanted = '#/city/' + EXPECT_SLUG + '/biz/' + EXPECT_ID;
      if (info.firstOnclick.replace(/['"]/g, '').includes(wanted)) {
        ok('the card points at the probe business itself');
      } else {
        bad(`the first card is not the probe business (wanted ${wanted})`);
      }
    }

    if (info.firstText.includes(MARKER)) ok('the rendered card carries the probe business name');
    else bad('the rendered card does not carry the probe business name');

    if (searchCalls.some((c) => c.startsWith('200 '))) ok('the browser called the search API: ' + searchCalls.join(' | '));
    else bad('the browser never got a 200 from /api/v1/zaarhub/search (' + searchCalls.join(' | ') + ')');

    if (consoleErrors.length === 0) ok('no JavaScript console errors during the flow');
    else bad('console errors: ' + JSON.stringify(consoleErrors.slice(0, 3)));

    try {
      await page.screenshot({ path: require('path').join(SHOT_DIR, 'search-results.png') });
    } catch (_) { /* screenshot is evidence, not a check */ }
  } catch (e) {
    bad('the flow threw: ' + e.message);
  }

  await browser.close();
  console.log('BROWSER RESULT: ' + (problems.length ? 'FAILED' : 'all passed'));
  process.exit(problems.length ? 1 : 0);
})();
