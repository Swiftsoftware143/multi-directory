/**
 * ZaarHub complete-record export — real-browser arm (item 4 "Data export").
 *
 * Loads the REAL admin console (the app-served /admin-panel.html) with an operator session,
 * finds the "Complete Export" card on screen, types a real business id, and CLICKS the panel's
 * own "This record — JSON" / "This record — CSV" buttons. It then proves the download reached
 * the browser two independent ways:
 *   1. the browser's own network stack received the export response (status + the
 *      Content-Disposition filename the server set), captured from the page's response stream;
 *   2. the page assembled a downloadable file from those bytes — the very Blob the panel hands
 *      to the anchor's `download` — read back out of the page and parsed.
 * Headless Chromium discards the OS-level blob write, so (2) is the byte-level stand-in for the
 * saved file: same bytes, same page, same click. A build whose button is dead fails both.
 *
 * Env: PUBLIC, BIZ, DIR, TOKEN, AUD, CHROME_PATH, NODE_PATH
 * Output: $AUD/browser-proof.json (+ export-card.png)
 */
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const PUBLIC = process.env.PUBLIC || 'https://zaarhub.com';
const BIZ = process.env.BIZ;
const TOKEN = process.env.TOKEN;
const AUD = process.env.AUD;
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';
const DL = path.join(AUD || '/tmp', 'downloads');
const PANEL = `${PUBLIC}/admin-panel.html`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Wait until the page has assembled a blob whose bytes satisfy `pred(text)`. */
async function waitBlob(page, pred, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const hit = await page.evaluate((src) => {
      const fn = new Function('t', 'return (' + src + ')(t)');
      const b = (window.__blobs || []).find(
        (x) => typeof x.text === 'string' && fn(x.text));
      return b ? { name: b.name, type: b.type, size: b.size, text: b.text } : null;
    }, pred.toString());
    if (hit) return hit;
    await sleep(300);
  }
  return null;
}

(async () => {
  fs.mkdirSync(DL, { recursive: true });
  const proof = {
    panelLoaded: false, panelStatus: 0, cardVisible: false, buttonsVisible: 0,
    jsonClicked: false, jsonStatusMsg: null,
    jsonResponse: null, jsonDisposition: null,
    jsonBlobLanded: false, jsonBlobName: null, jsonBlobBytes: null,
    jsonEntityId: null, jsonReviewCount: null, jsonArticleCount: null,
    csvClicked: false, csvStatusMsg: null,
    csvResponse: null, csvDisposition: null,
    csvBlobLanded: false, csvBlobName: null, csvHasSections: false,
    pageErrors: [],
  };
  const browser = await puppeteer.launch({
    executablePath: CHROME, headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  try {
    const page = await browser.newPage();
    const client = await page.target().createCDPSession();
    await client.send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: DL, eventsEnabled: true });
    await page.setViewport({ width: 1440, height: 1400 });
    page.on('pageerror', (e) => proof.pageErrors.push(String(e.message).slice(0, 200)));
    page.on('console', (m) => { if (m.type() === 'error') proof.pageErrors.push('console: ' + m.text().slice(0, 200)); });
    page.on('response', (r) => {
      const u = r.url();
      if (u.includes('/export/entity/')) {
        const h = r.headers() || {};
        const rec = {
          url: u, status: r.status(),
          disposition: h['content-disposition'] || null,
          ctype: h['content-type'] || null,
        };
        if (u.includes('format=json')) proof.jsonResponse = rec;
        if (u.includes('format=csv')) proof.csvResponse = rec;
      }
    });
    // Record every Blob the page hands to the download anchor, keyed by the bytes it will save.
    await page.evaluateOnNewDocument((t) => {
      try {
        localStorage.setItem('md_admin_token', t);
        localStorage.setItem('md_admin_user', JSON.stringify(
          { email: 'builds@swiftsoftware.com', role: 'super_admin' }));
      } catch (e) {}
      window.__blobs = [];
      const orig = URL.createObjectURL.bind(URL);
      URL.createObjectURL = function (blob) {
        const url = orig(blob);
        try {
          blob.text().then((txt) => window.__blobs.push({
            url, type: blob.type || '', size: blob.size || 0, text: txt,
          }));
        } catch (e) { /* ignore */ }
        return url;
      };
      // Remember the filename the panel asked the browser to save it as.
      const click = HTMLAnchorElement.prototype.click;
      HTMLAnchorElement.prototype.click = function () {
        if (this.download) {
          window.__blobs.push({ name: this.download, isAnchor: true });
        }
        return click.apply(this, arguments);
      };
    }, TOKEN);

    const resp = await page.goto(PANEL, { waitUntil: 'networkidle2', timeout: 60000 });
    proof.panelStatus = resp ? resp.status() : 0;
    proof.panelLoaded = proof.panelStatus === 200;
    await page.waitForSelector('#entity-export-section', { timeout: 30000 });
    proof.cardVisible = await page.evaluate(() => {
      const el = document.getElementById('entity-export-section');
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return el.offsetParent !== null && r.height > 0;
    });
    proof.buttonsVisible = await page.evaluate(() => {
      const el = document.getElementById('entity-export-section');
      return el ? el.querySelectorAll('button').length : 0;
    });
    await page.evaluate(() => {
      document.getElementById('entity-export-section').scrollIntoView({ block: 'start' });
    });
    await sleep(400);
    await page.screenshot({ path: path.join(AUD || '/tmp', 'export-card.png') });

    // Fill the panel's own form: entity type = business (default), id = the probe record.
    await page.select('#ex-kind', 'business');
    await page.click('#ex-id');
    await page.evaluate(() => { document.getElementById('ex-id').value = ''; });
    await page.type('#ex-id', BIZ);

    // ---- click "This record — JSON" exactly as a human would ----
    const jsonBtn = await page.evaluateHandle(() =>
      [...document.querySelectorAll('#entity-export-section button')]
        .find((b) => /JSON/.test(b.textContent) && /This record/i.test(b.textContent)));
    proof.jsonClicked = await page.evaluate((b) => { b.click(); return true; }, jsonBtn);
    const jb = await waitBlob(page, (t) => t.includes('"related"') && t.includes('"counts"'), 20000);
    proof.jsonStatusMsg = await page.evaluate(() =>
      (document.getElementById('ex-msg') || {}).textContent || null);
    if (jb) {
      proof.jsonBlobLanded = true;
      proof.jsonBlobBytes = jb.size;
      proof.jsonDisposition = jb.name || null;
      try {
        const doc = JSON.parse(jb.text);
        proof.jsonEntityId = doc.entity && doc.entity.id;
        proof.jsonReviewCount = doc.counts ? doc.counts.reviews : null;
        proof.jsonArticleCount = doc.counts ? doc.counts.business_articles : null;
      } catch (e) { proof.pageErrors.push('json parse: ' + e.message); }
    }

    // ---- click "This record — CSV" ----
    const csvBtn = await page.evaluateHandle(() =>
      [...document.querySelectorAll('#entity-export-section button')]
        .find((b) => /CSV/.test(b.textContent) && /This record/i.test(b.textContent)));
    proof.csvClicked = await page.evaluate((b) => { b.click(); return true; }, csvBtn);
    const cb = await waitBlob(page, (t) => t.includes('# entity') && t.includes('# table: reviews'), 20000);
    proof.csvStatusMsg = await page.evaluate(() =>
      (document.getElementById('ex-msg') || {}).textContent || null);
    if (cb) {
      proof.csvBlobLanded = true;
      proof.csvHasSections = cb.text.includes('# entity') && cb.text.includes('# table: reviews')
        && cb.text.includes(BIZ);
    }
    // The download names the panel remembers handing to the anchor.
    const names = await page.evaluate(() =>
      (window.__blobs || []).filter((b) => b.isAnchor).map((b) => b.name));
    proof.downloadNames = names;
  } catch (e) {
    proof.pageErrors.push('fatal: ' + String(e.message).slice(0, 300));
  } finally {
    await browser.close();
  }
  if (AUD) fs.writeFileSync(path.join(AUD, 'browser-proof.json'), JSON.stringify(proof, null, 2));
  console.log(JSON.stringify(proof));
  process.exit(proof.jsonBlobLanded && proof.csvBlobLanded ? 0 : 1);
})();
