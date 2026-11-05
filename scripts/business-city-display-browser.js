/**
 * Durable proof — business city names are complete on the public screens.
 *
 * The defect: `businesses.city` / `businesses.city_slug` were stored as the FIRST
 * SPACE-SEPARATED TOKEN of the real city ("Palm Bay" → "Palm", "St. Petersburg" → "St.",
 * "Boca Raton" → "Boca"), so every screen that prints `business.city` showed "Palm, FL"
 * and the city filters offered "Palm"/"Boca"/"St." as if they were cities.
 * 2,626 of 3,988 listings were affected (measured 2026-11-05).
 *
 * Checks on the real running app in a real browser:
 *   1. the community home's editor picks print a full two-word city (they print business.city);
 *   2. the city page renders its listings with no truncated city anywhere in the visible text;
 *   3. no console/page errors on either screen.
 *
 * Env: BASE (default http://127.0.0.1:8089), SHOT_DIR, CHROME_PATH
 * Exit: 0 = all passed, 1 = a check failed.
 */
const path = require('path');
const puppeteer = require('puppeteer-core');

const BASE = process.env.BASE || 'http://127.0.0.1:8089';
const SHOT_DIR = process.env.SHOT_DIR || '/tmp';
// Tokens that are the first word of a real city, never a city on their own.
const TRUNC = /(^|\s)(Palm|Boca|Pompano|Winter|Fort|Lighthouse|Deerfield|Hallandale|Altamonte|Satellite|Coconut|Madeira|Pembroke|Delray|Pinellas|St\.?)(, FL|\s+\d{3})/;

(async () => {
  const problems = [];
  const ok = (m) => console.log('PASS  ' + m);
  const bad = (m) => { problems.push(m); console.log('FAIL  ' + m); };

  const browser = await puppeteer.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1360, height: 1100 });
  const consoleErrors = [];
  page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text()); });
  page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));

  try {
    // 1. Community home: the editor's picks print business.city under each card.
    await page.goto(BASE + '/zaarhub', { waitUntil: 'networkidle2', timeout: 60000 });
    await new Promise((r) => setTimeout(r, 3000));
    const homeText = await page.evaluate(() => document.body.innerText.replace(/\s+/g, ' '));
    if (TRUNC.test(homeText)) bad('a truncated city is still printed on the community home: ' + homeText.match(TRUNC)[0]);
    else ok('no truncated city on the community home');
    const picked = homeText.match(/(Boca Raton|Palm Bay|Pompano Beach|Winter Garden|St\. Petersburg|St\. Cloud)/);
    if (picked) ok('the home prints a full multi-word city: ' + picked[1]);
    else bad('no full multi-word city found on the community home');
    await page.screenshot({ path: path.join(SHOT_DIR, 'city-home.png') }).catch(() => {});

    // 2. The city page: every card/address, and the whole visible text, must be complete.
    await page.goto(BASE + '/city/palm-bay', { waitUntil: 'networkidle2', timeout: 60000 });
    await page.waitForFunction(() => /Headquarters:/.test(document.body.innerText), { timeout: 30000 });
    await new Promise((r) => setTimeout(r, 1500));
    const cityText = await page.evaluate(() => document.body.innerText.replace(/\s+/g, ' '));
    const addrs = await page.evaluate(() => [...document.querySelectorAll('.addr')].map((n) => n.innerText.trim()).slice(0, 20));
    if (TRUNC.test(cityText)) bad('a truncated city is printed on the Palm Bay page: ' + cityText.match(TRUNC)[0]);
    else ok('no truncated city anywhere on the Palm Bay page');
    const withFL = addrs.filter((a) => /, FL \d{5}/.test(a));
    if (withFL.length >= 3) ok('listing cards carry complete addresses (' + withFL.length + ' checked)');
    else bad('too few complete listing addresses rendered: ' + withFL.length);
    if (/Palm Bay, FL/.test(cityText)) ok('the page text carries "Palm Bay, FL"');
    else bad('the page text does not carry "Palm Bay, FL"');
    await page.screenshot({ path: path.join(SHOT_DIR, 'city-page.png') }).catch(() => {});

    if (consoleErrors.length) bad('console errors: ' + consoleErrors.slice(0, 3).join(' | '));
    else ok('no JavaScript console errors');
  } catch (e) {
    bad('browser flow threw: ' + e.message);
    try { await page.screenshot({ path: path.join(SHOT_DIR, 'city-error.png') }); } catch (_) {}
  }

  await browser.close();
  console.log('BROWSER RESULT: ' + (problems.length ? problems.length + ' failed' : 'all passed'));
  process.exit(problems.length ? 1 : 0);
})();
