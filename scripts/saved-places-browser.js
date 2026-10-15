/**
 * Durable browser proof — the shopper's Saved Places screen actually shows saved businesses.
 *
 * One realistic journey on the live app (no injected session, no stubbed API):
 *   1. /visitor (the Visitor Portal) — sign in through the page's OWN form (#loginEmail/#loginPassword
 *      + the Sign In button). The portal's "Favorites" section must list the business the shopper
 *      saved, not "No favorites yet".
 *   2. Click the page's own "⭐ Saved" link — it must land on a page that actually shows the saved
 *      business (it used to bounce straight back to the portal; the SSR page cannot read a browser
 *      navigation's missing bearer token).
 *   3. /#/saved — the main app's own Saved Places screen (reached by the app's own "Saved" nav link
 *      on the home screen) must list the same business, not "No saved places yet".
 *
 * Fails on: the empty-state text, a console error, or the business name never appearing.
 *
 * Env: BASE, TOKEN, EMAIL, PW, BIZ_NAME, BIZ_CITY, SHOT_DIR, CHROME_PATH, NODE_PATH
 * Exit: 0 = PASS, 1 = FAIL.  Prints one JSON line of checks.
 */
const puppeteer = require('puppeteer');

const BASE = process.env.BASE || 'http://127.0.0.1:8089';
const EMAIL = process.env.EMAIL;
const PW = process.env.PW;
const BIZ = process.env.BIZ_NAME;
const SHOT = process.env.SHOT_DIR || '/tmp';
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';

const checks = [];
function check(name, ok, detail) {
  checks.push({ name, ok: !!ok, detail: detail === undefined ? '' : String(detail) });
}

const sleeping = (ms) => new Promise((r) => setTimeout(r, ms));

async function bodyText(page) {
  return page.evaluate(() => document.body.innerText || '');
}

(async () => {
  const browser = await puppeteer.launch({
    headless: 'new',
    executablePath: CHROME,
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage();
  const errors = [];
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(m.text());
  });
  page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));

  try {
    // ── 1. the Visitor Portal: sign in through the page's own form ────────────────────────────
    await page.goto(BASE + '/visitor', { waitUntil: 'domcontentloaded', timeout: 30000 });
    await page.waitForSelector('#loginEmail', { timeout: 15000 });
    await page.type('#loginEmail', EMAIL);
    await page.type('#loginPassword', PW);
    await page.evaluate(() => {
      const b = [...document.querySelectorAll('button')].find((x) => /sign in/i.test(x.innerText));
      if (b) b.click();
    });
    let portalShowedBiz = false;
    try {
      await page.waitForFunction(
        (name) => (document.getElementById('favoritesList') || {}).innerText
          && document.getElementById('favoritesList').innerText.includes(name),
        { timeout: 20000 }, BIZ);
      portalShowedBiz = true;
    } catch (e) { /* recorded below */ }
    const portalText = await bodyText(page);
    check('portal: signed in through the page form and reached the dashboard', portalShowedBiz,
      portalShowedBiz ? '' : 'favoritesList never showed ' + BIZ);
    check('portal: Favorites section lists the saved business', portalShowedBiz);
    check('portal: no longer says "No favorites yet"', !/No favorites yet/.test(portalText),
      portalText.match(/No favorites yet/) ? 'default empty text still on screen' : '');
    await page.screenshot({ path: SHOT + '/portal-favorites.png', fullPage: false });

    // ── 2. the portal's own "⭐ Saved" link must reach a page showing the business ─────────────
    await page.evaluate(() => {
      const a = document.querySelector('a[title="My Saved Places"]');
      if (a) a.click();
    });
    await sleeping(1500);
    const savedUrl = page.url();
    let savedShowedBiz = false;
    try {
      await page.waitForFunction(
        (name) => (document.body.innerText || '').includes(name),
        { timeout: 20000 }, BIZ);
      savedShowedBiz = true;
    } catch (e) { /* recorded below */ }
    check('portal "Saved" link lands on the saved-places page', /\/user\/saved/.test(savedUrl), savedUrl);
    check('that page lists the saved business', savedShowedBiz);
    await page.screenshot({ path: SHOT + '/user-saved.png', fullPage: false });

    // ── 3. the main app's own Saved Places screen, reached by its own nav link ────────────────
    await page.goto(BASE + '/', { waitUntil: 'domcontentloaded', timeout: 30000 });
    await sleeping(1200);
    const navLink = await page.evaluate(() => !!document.querySelector('.zh-bottom-nav a[href="#/saved"]'));
    check('the app has a "Saved" nav control', navLink);
    if (navLink) {
      await page.evaluate(() => document.querySelector('.zh-bottom-nav a[href="#/saved"]').click());
    }
    let spaShowedBiz = false;
    try {
      await page.waitForFunction(
        (name) => (document.getElementById('zh-content') || {}).innerText
          && document.getElementById('zh-content').innerText.includes(name),
        { timeout: 20000 }, BIZ);
      spaShowedBiz = true;
    } catch (e) { /* recorded below */ }
    const spaText = await page.evaluate(() => (document.getElementById('zh-content') || {}).innerText || '');
    check('app Saved Places screen lists the saved business', spaShowedBiz,
      spaShowedBiz ? '' : spaText.slice(0, 200));
    check('app Saved Places screen does not say "No saved places yet"',
      !/No saved places yet/.test(spaText));
    await page.screenshot({ path: SHOT + '/spa-saved.png', fullPage: false });
  } catch (e) {
    check('browser flow completed without an exception', false, e.message);
  }

  const realErrors = errors.filter((t) => !/favicon|net::ERR_ABORTED|Failed to load resource: the server responded with a status of 404/i.test(t));
  check('no JavaScript console errors during the flow', realErrors.length === 0, realErrors.slice(0, 3).join(' | '));

  await browser.close();
  const failed = checks.filter((c) => !c.ok);
  console.log(JSON.stringify({ checks, failed: failed.length, errors }, null, 1));
  process.exit(failed.length === 0 ? 0 : 1);
})();
