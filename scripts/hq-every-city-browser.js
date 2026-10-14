/**
 * ZaarHub Headquarters — real browser arm (queue item 15).
 *
 * Loads the REAL served city page for every city (the SPA deep link /city/<slug>,
 * served by the app itself) and reads the rendered Headquarters line out of the DOM.
 * Nothing is stubbed: the value on screen is whatever the page fetched from the API.
 *
 * Env: MD_BASE, CITIES (JSON array of slugs), HQ (expected text), AUD
 * Output: $AUD/browser-proof.json
 */
const puppeteer = require('puppeteer-core');
const fs = require('fs');
const { execSync } = require('child_process');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const CITIES = JSON.parse(process.env.CITIES || '[]');
const HQ = process.env.HQ || '';
const AUD = process.env.AUD || '/tmp';

function chromePath() {
  if (process.env.CHROME_PATH) return process.env.CHROME_PATH;
  try {
    const d = execSync(
      'ls -d /root/.cache/puppeteer/chrome/*/chrome-linux64/chrome 2>/dev/null | sort | tail -1')
      .toString().trim();
    if (d) return d;
  } catch (e) {}
  return '/usr/bin/chromium-browser';
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const browser = await puppeteer.launch({
    executablePath: chromePath(),
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const out = { base: MD_BASE, expected: HQ, cities: {} };

  for (const slug of CITIES) {
    const page = await browser.newPage();
    await page.setViewport({ width: 1280, height: 900, deviceScaleFactor: 1 });
    await page.setUserAgent('Mozilla/5.0 (compatible; SwiftOps/1.0)');
    const pageErrors = [], consoleErrors = [];
    page.on('pageerror', (e) => pageErrors.push(String(e).slice(0, 200)));
    page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text().slice(0, 200)); });

    let status = 0, found = false, text = '', visible = false;
    try {
      const resp = await page.goto(MD_BASE + '/city/' + encodeURIComponent(slug),
        { waitUntil: 'networkidle2', timeout: 45000 });
      status = resp ? resp.status() : 0;
      // Wait for the city hero to render (the same component the homepage uses).
      await page.waitForFunction(
        () => !!document.querySelector('.zh-hero'),
        { timeout: 30000 }).catch(() => {});
      // The Headquarters line is .zh-hero-hq (only present when the config value is non-empty).
      const el = await page.$('.zh-hero-hq');
      if (el) {
        found = true;
        text = (await page.evaluate((n) => n.textContent.trim(), el)) || '';
        visible = await page.evaluate((n) => {
          const s = getComputedStyle(n);
          const r = n.getBoundingClientRect();
          return s.display !== 'none' && s.visibility !== 'hidden' && r.width > 0 && r.height > 0;
        }, el);
      }
    } catch (e) {
      pageErrors.push('nav: ' + String(e).slice(0, 160));
    }
    if (AUD) await page.screenshot({ path: `${AUD}/city-${slug}.png`, fullPage: false }).catch(() => {});
    out.cities[slug] = { status, found, text, visible, pageErrors, consoleErrors };
    await page.close();
  }

  fs.writeFileSync(AUD + '/browser-proof.json', JSON.stringify(out, null, 1));
  console.log(JSON.stringify(out, null, 1));
  await browser.close();
})().catch((e) => { console.log('PROBE FAILED: ' + String(e).slice(0, 400)); process.exit(1); });
