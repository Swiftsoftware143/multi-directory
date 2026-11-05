/**
 * Item 4 — prove a COMPLETE record downloads from the admin UI itself.
 * Opens the live admin console, picks a real business, clicks the page's OWN
 * "This record — JSON" and "— CSV" buttons and captures the actual downloaded files.
 */
let puppeteer;
try { puppeteer = require('puppeteer-core'); }
catch (e) { puppeteer = require('/opt/swift/browser-run/node_modules/puppeteer-core'); }
const fs = require('fs');
const path = require('path');

const URL = process.env.MD_PANEL || 'http://127.0.0.1:8089/admin-panel.html';
const TOKEN = process.env.MD_TOKEN || fs.readFileSync('/tmp/mdtok', 'utf8').trim();
const BIZ = 'bb5f6bff-0db4-4d91-a13f-d19dfe64fb37';
const AUD = process.env.AUD || '/opt/swift/audits/item4-data-export-20261105';
const DL = AUD + '/downloads';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const out = [];
const rec = (ok, name, detail) => out.push((ok ? 'PASS ' : 'FAIL ') + name.padEnd(46) + ' :: ' + String(detail === undefined ? '' : detail).slice(0, 300));

fs.rmSync(DL, { recursive: true, force: true });
fs.mkdirSync(DL, { recursive: true });

async function waitFile(before, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const now = fs.readdirSync(DL).filter((f) => !before.includes(f) && !f.endsWith('.crdownload'));
    if (now.length) { await sleep(400); return now.map((f) => path.join(DL, f)); }
    await sleep(250);
  }
  return [];
}

(async () => {
  let browser;
  try {
    browser = await puppeteer.launch({
      executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
      headless: 'new', args: ['--no-sandbox', '--disable-dev-shm-usage'],
    });
    const page = await browser.newPage();
    await page.setViewport({ width: 1440, height: 1200 });
    await page.setUserAgent('Mozilla/5.0 (compatible; SwiftOps/1.0)');
    const errors = [];
    page.on('pageerror', (e) => errors.push(String(e).slice(0, 200)));
    page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text().slice(0, 200)); });
    await page.evaluateOnNewDocument((t) => {
      localStorage.setItem('md_admin_token', t);
      localStorage.setItem('md_admin_user', JSON.stringify({ email: 'proof@localhost', name: 'UI Proof' }));
      // Capture the exact file the page hands to the browser (blob + download name),
      // so the proof is the page's OWN output even in headless Chromium.
      window.__dl = [];
      const orig = URL.createObjectURL.bind(URL);
      URL.createObjectURL = function (blob) {
        const rec = { name: '', size: blob.size, type: blob.type, text: '' };
        window.__dl.push(rec);
        blob.text().then((txt) => { rec.text = txt; });
        return orig(blob);
      };
      const click = HTMLAnchorElement.prototype.click;
      HTMLAnchorElement.prototype.click = function () {
        if (this.download && window.__dl.length) {
          const last = window.__dl[window.__dl.length - 1];
          if (last && !last.name) last.name = this.download;
        }
        return click.apply(this, arguments);
      };
    }, TOKEN);

    const cdp = await page.createCDPSession();
    await cdp.send('Page.setDownloadBehavior', { behavior: 'allow', downloadPath: DL });
    const bcdp = await browser.target().createCDPSession();
    await bcdp.send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: DL, eventsEnabled: true });
    bcdp.on('Browser.downloadProgress', (e) => { if (e.state === 'completed') console.log('download completed: ' + e.guid); });
    bcdp.on('Browser.downloadWillBegin', (e) => console.log('download begin: ' + e.suggestedFilename));

    const r = await page.goto(URL, { waitUntil: 'networkidle2', timeout: 60000 });
    rec(r.status() === 200, 'admin console served', 'HTTP ' + r.status());
    await page.waitForSelector('#ex-id', { timeout: 30000 });
    await page.waitForFunction(() => typeof window.__exEntity === 'function', { timeout: 30000 });
    await page.evaluate(() => document.getElementById('entity-export-section').scrollIntoView());
    await sleep(500);
    rec(true, 'Complete Export section is on the page', '#entity-export-section');

    await page.select('#ex-kind', 'business');
    await page.click('#ex-id', { clickCount: 3 }).catch(() => {});
    await page.type('#ex-id', BIZ);
    await sleep(200);

    // JSON button
    await page.evaluate(() => {
      const b = Array.from(document.querySelectorAll('button')).find((x) => x.textContent.includes('This record — JSON'));
      b.click();
    });
    await page.waitForFunction(() => (window.__dl || []).some((d) => d.name.endsWith('.json') && d.text.length > 1000), { timeout: 30000 }).catch(() => {});
    const dl1 = await page.evaluate(() => (window.__dl || []).find((d) => d.name.endsWith('.json')) || null);
    rec(!!dl1, 'JSON button produced the file', dl1 ? dl1.name + ' (' + dl1.size + ' bytes)' : 'no blob');
    if (dl1) {
      const d = JSON.parse(dl1.text);
      rec(d.entity && d.entity.id === BIZ, 'downloaded JSON is this record', d.entity && d.entity.name);
      rec((d.sections || []).length === 62, 'downloaded JSON has all 62 sections', String((d.sections || []).length));
      rec(((d.related || {}).reviews || []).length === 2, 'its 2 reviews are inside', String(((d.related || {}).reviews || []).length));
      rec(((d.related || {}).business_listings || []).length === 1, 'its listing is inside', '1');
      fs.writeFileSync(AUD + '/downloaded-record.json', dl1.text);
    }

    // CSV button
    await page.evaluate(() => {
      const b = Array.from(document.querySelectorAll('button')).find((x) => x.textContent.includes('This record — CSV'));
      b.click();
    });
    await page.waitForFunction(() => (window.__dl || []).some((d) => d.name.endsWith('.csv') && d.text.length > 1000), { timeout: 30000 }).catch(() => {});
    const dl2 = await page.evaluate(() => (window.__dl || []).find((d) => d.name.endsWith('.csv')) || null);
    rec(!!dl2, 'CSV button produced the file', dl2 ? dl2.name + ' (' + dl2.size + ' bytes)' : 'no blob');
    if (dl2) {
      const secs = dl2.text.split('\n').filter((l) => l.startsWith('# ')).length;
      rec(secs >= 61, 'CSV has a section per related table', String(secs));
      rec(dl2.text.includes('Face It Aesthetics'), 'CSV contains the record');
      fs.writeFileSync(AUD + '/downloaded-record.csv', dl2.text);
    }

    const msg = await page.evaluate(() => (document.getElementById('ex-msg') || {}).textContent || '');
    rec(/exported/i.test(msg), 'the page reports the export succeeded', msg);
    rec(errors.length === 0, 'no page errors', errors.join(' | '));
    await page.screenshot({ path: AUD + '/export-section.png' }).catch(() => {});
  } catch (e) {
    rec(false, 'exception', String(e && e.message).slice(0, 300));
  } finally {
    if (browser) await browser.close().catch(() => {});
  }
  console.log(out.join('\n'));
  const failed = out.filter((l) => l.startsWith('FAIL'));
  console.log('RESULT: ' + (failed.length ? 'FAIL (' + failed.length + ')' : 'PASS'));
  fs.writeFileSync(AUD + '/ui-proof.txt', out.join('\n') + '\nRESULT: ' + (failed.length ? 'FAIL' : 'PASS') + '\n');
  process.exit(failed.length ? 1 : 0);
})();
