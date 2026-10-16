/**
 * ZaarHub sector landing pages — BRANDING arm (item 18 cleanup / brand-colour wiring).
 *
 * Loads the REAL served sector page (`/templates/<sector>.html?slug=<directory>`), reads the
 * computed `--primary` / `--accent` custom properties off the document element, records the HTTP
 * status of the page's own branding request, and counts the rendered business cards (so a pass
 * cannot come from a page that crashed before rendering).
 *
 * Nothing is stubbed: the colours on screen are whatever the page fetched from the API.
 *
 * Env: MD_BASE, TARGETS (JSON array of {url, slug}) — `url` is the absolute page url,
 *      SLUGS (JSON map slug -> {primary, secondary}), AUD
 * Output: $AUD/branding-browser-proof.json
 */
const puppeteer = require('puppeteer-core');
const fs = require('fs');
const { execSync } = require('child_process');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TARGETS = JSON.parse(process.env.TARGETS || '[]');
const SLUGS = JSON.parse(process.env.SLUGS || '{}');
const AUD = process.env.AUD || '/tmp';
const LABEL = process.env.LABEL || 'base';

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
  const out = { base: MD_BASE, label: LABEL, targets: {} };

  for (let idx = 0; idx < TARGETS.length; idx++) {
    const t = TARGETS[idx];
    const page = await browser.newPage();
    await page.setViewport({ width: 1280, height: 900, deviceScaleFactor: 1 });
    await page.setUserAgent('Mozilla/5.0 (compatible; SwiftOps/1.0)');
    const pageErrors = [], consoleErrors = [], brandingCalls = [], failedResponses = [];
    page.on('pageerror', (e) => pageErrors.push(String(e).slice(0, 200)));
    page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text().slice(0, 200)); });
    page.on('response', (r) => {
      if (r.url().includes('/branding')) brandingCalls.push({ url: r.url(), status: r.status() });
      if (r.status() >= 400) failedResponses.push({ url: r.url().slice(0, 200), status: r.status() });
    });

    const rec = { url: t.url, slug: t.slug, expected: SLUGS[t.slug] || null };
    try {
      const resp = await page.goto(t.url, { waitUntil: 'networkidle2', timeout: 45000 });
      rec.status = resp ? resp.status() : 0;
      // Cards are rendered only after BOTH fetches settle; wait for them.
      await page.waitForFunction(
        () => document.querySelectorAll('#results .card').length > 0,
        { timeout: 30000 }).catch(() => {});
      await sleep(500);
      rec.dom = await page.evaluate(() => {
        const cs = getComputedStyle(document.documentElement);
        return {
          primary: cs.getPropertyValue('--primary').trim(),
          accent: cs.getPropertyValue('--accent').trim(),
          cards: document.querySelectorAll('#results .card').length,
          title: document.title,
        };
      });
      rec.brandingCalls = brandingCalls;
      rec.failedResponses = failedResponses;
      rec.consoleErrors = consoleErrors;
      rec.pageErrors = pageErrors;
      await page.screenshot({ path: `${AUD}/${LABEL}-${t.slug}.png` });
    } catch (e) {
      rec.error = String(e).slice(0, 300);
      rec.consoleErrors = consoleErrors;
      rec.pageErrors = pageErrors;
    }
    out.targets[`${LABEL}:${idx}:${t.slug}`] = rec;
    await page.close();
  }

  fs.writeFileSync(`${AUD}/branding-browser-proof.json`, JSON.stringify(out, null, 1));
  await browser.close();
  console.log(JSON.stringify(out));
})().catch((e) => { console.error(String(e)); process.exit(1); });
