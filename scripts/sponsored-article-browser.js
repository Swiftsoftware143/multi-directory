/**
 * ZaarHub sponsored articles — real-browser arm (queue item 6).
 *
 * Browses the city blog page exactly as a visitor would, finds the sponsored article in the
 * list, CLICKS it, and asserts the landed page shows the title and the sponsored body laid out
 * on screen. A URL-only check would pass on the broken build; the click-through is the test.
 *
 * Env: PUBLIC, CITY, SLUG, TITLE, MARKER, AUD, CHROME_PATH
 * Output: $AUD/browser-proof.json  (+ browse.png, article.png)
 */
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const PUBLIC = process.env.PUBLIC || 'https://zaarhub.com';
const CITY = process.env.CITY || 'palm-bay';
const SLUG = process.env.SLUG;
const TITLE = process.env.TITLE;
const MARKER = process.env.MARKER;
const AUD = process.env.AUD;
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';
const ARTICLE_PATH = `/${CITY}/articles/${SLUG}`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const proof = {
    browseLoaded: false, browseStatus: 0, listed: false,
    clickedThrough: false, landedUrl: null,
    titleVisible: false, bodyVisible: false, bodyLaidOut: false, pageErrors: [],
  };
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  try {
    const page = await browser.newPage();
    await page.setViewport({ width: 1440, height: 1200 });
    page.on('pageerror', (e) => proof.pageErrors.push(String(e.message).slice(0, 200)));
    page.on('console', (m) => { if (m.type() === 'error') proof.pageErrors.push('console: ' + m.text().slice(0, 200)); });

    // 1) browse the city
    const resp = await page.goto(`${PUBLIC}/${CITY}/blog`, { waitUntil: 'networkidle2', timeout: 60000 });
    proof.browseStatus = resp ? resp.status() : 0;
    proof.browseLoaded = proof.browseStatus === 200;
    await sleep(2500);

    // 2) the sponsored article must be visible in the list
    const listed = await page.evaluate((slug, title) => {
      const a = [...document.querySelectorAll('a')].find(
        (e) => (e.getAttribute('href') || '').includes(slug));
      if (!a) return false;
      const r = a.getBoundingClientRect();
      return r.width > 0 && r.height > 0 && getComputedStyle(a).visibility !== 'hidden';
    }, SLUG, TITLE);
    proof.listed = listed;
    await page.screenshot({ path: path.join(AUD, 'browse.png') }).catch(() => {});

    // 3) click through like a visitor
    const link = await page.$(`a[href*="${SLUG}"]`);
    if (link) {
      await Promise.all([
        page.waitForNavigation({ waitUntil: 'networkidle2', timeout: 60000 }).catch(() => {}),
        link.click(),
      ]);
      await sleep(2500);
      proof.landedUrl = page.url();
      proof.clickedThrough = page.url().includes(ARTICLE_PATH);
    }

    // 4) title + body on screen, laid out
    if (proof.clickedThrough) {
      const body = await page.evaluate(() => document.body.innerText);
      proof.titleVisible = body.includes(TITLE);
      proof.bodyVisible = body.includes(MARKER);
      proof.bodyLaidOut = await page.evaluate((m) => {
        const el = [...document.querySelectorAll('p,div,article,section')]
          .find((e) => e.textContent.includes(m));
        if (!el) return false;
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== 'hidden';
      }, MARKER);
      await page.screenshot({ path: path.join(AUD, 'article.png'), fullPage: false }).catch(() => {});
    }
  } catch (e) {
    proof.pageErrors.push('HARNESS: ' + String(e.message).slice(0, 200));
  } finally {
    await browser.close();
  }
  fs.mkdirSync(AUD, { recursive: true });
  fs.writeFileSync(path.join(AUD, 'browser-proof.json'), JSON.stringify(proof, null, 2));
  const ok = proof.browseLoaded && proof.listed && proof.clickedThrough &&
             proof.titleVisible && proof.bodyVisible && proof.bodyLaidOut &&
             proof.pageErrors.length === 0;
  console.log(`browser: listed=${proof.listed} clicked=${proof.clickedThrough} title=${proof.titleVisible} body=${proof.bodyVisible} => ${ok ? 'PASS' : 'FAIL'}`);
  process.exit(ok ? 0 : 1);
})().catch((e) => { console.error('HARNESS ERROR: ' + e.message); process.exit(2); });
