/**
 * ZaarHub scanner — real end-to-end browser arm (queue item 2).
 *
 * What it proves, on the REAL served page (no stubs):
 *   1. the page's OWN QR generator (/qrcode.min.js — the library the member wallet uses)
 *      renders the probe card to a PNG;
 *   2. that PNG is fed through Chromium's camera pipeline
 *      (--use-fake-device-for-media-stream + --use-file-for-fake-video-capture), so
 *      getUserMedia() hands the page real frames and videoWidth > 0;
 *   3. the page's OWN decoder (jsQR) reads the card out of the live frame and stores it
 *      as the scanned payload;
 *   4. a PURCHASE with a real amount is submitted and the page shows the awarded points.
 *
 * Env: MD_BASE, PAYLOAD, BIZ_ID, BIZ_NAME, BIZ_CAT, TOKEN, AUD, AMOUNT
 * Output: $AUD/browser-proof.json  (+ qr-probe.png, scanner-decoded.png, scanner-award.png)
 */
const puppeteer = require('puppeteer-core');
const fs = require('fs');
const { execFileSync } = require('child_process');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const PAYLOAD = process.env.PAYLOAD;
const BIZ_ID = process.env.BIZ_ID;
const BIZ_NAME = process.env.BIZ_NAME || 'Probe Business';
const BIZ_CAT = process.env.BIZ_CAT || 'Home Services';
const TOKEN = process.env.TOKEN;
const AUD = process.env.AUD;
const AMOUNT = process.env.AMOUNT || '25.00';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';

async function renderQrCard(path) {
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage();
  await page.goto(MD_BASE + '/', { waitUntil: 'domcontentloaded', timeout: 30000 });
  // the app's own generator, served by the app itself
  await page.addScriptTag({ url: MD_BASE + '/qrcode.min.js' });
  await page.evaluate((payload) => {
    const wrap = document.createElement('div');
    wrap.id = 'qrProbeWrap';
    wrap.style.cssText =
      'position:fixed;left:0;top:0;z-index:2147483647;background:#fff;padding:60px;';
    document.body.appendChild(wrap);
    // eslint-disable-next-line no-undef
    new QRCode(wrap, {
      text: payload,
      width: 520,
      height: 520,
      colorDark: '#000000',
      colorLight: '#ffffff',
      correctLevel: QRCode.CorrectLevel.M,
    });
  }, PAYLOAD);
  await sleep(1200);
  const el = await page.$('#qrProbeWrap');
  await el.screenshot({ path });
  const rendered = await page.evaluate(() => {
    const c = document.querySelector('#qrProbeWrap canvas');
    return { hasCanvas: !!c, w: c ? c.width : 0 };
  });
  await browser.close();
  return rendered;
}

async function scanAndAward(y4m) {
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage',
           '--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream',
           '--use-file-for-fake-video-capture=' + y4m],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 430, height: 900, deviceScaleFactor: 2 });
  await page.setUserAgent('Mozilla/5.0 (compatible; SwiftOps/1.0)');

  const pageErrors = [], consoleErrors = [], scanReqs = [];
  page.on('pageerror', (e) => pageErrors.push(String(e).slice(0, 200)));
  page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text().slice(0, 200)); });
  page.on('response', async (r) => {
    if (r.url().includes('/api/v1/loyalty/scan')) {
      let body = ''; try { body = (await r.text()).slice(0, 500); } catch (e) {}
      scanReqs.push({ status: r.status(), body });
    }
  });

  await page.evaluateOnNewDocument((t, b, n, c) => {
    localStorage.setItem('scannerToken', t);
    localStorage.setItem('scannerBizId', b);
    localStorage.setItem('scannerBizName', n);
    localStorage.setItem('scannerBizCat', c);
  }, TOKEN, BIZ_ID, BIZ_NAME, BIZ_CAT);

  const resp = await page.goto(MD_BASE + '/scanner', { waitUntil: 'networkidle2', timeout: 45000 });
  await page.waitForFunction(() => {
    const s = document.getElementById('scannerScreen');
    return s && getComputedStyle(s).display !== 'none';
  }, { timeout: 25000 });

  // wait for the real decode: handleQRCode() flips the status text
  let decoded = false;
  try {
    await page.waitForFunction(
      () => document.getElementById('scannerStatus').textContent.includes('QR scanned'),
      { timeout: 30000 });
    decoded = true;
  } catch (e) {}

  const decodeState = await page.evaluate(() => ({
    status: document.getElementById('scannerStatus').textContent.trim(),
    storedQR: document.getElementById('txCard').dataset.qrCode || '',
    guideFound: document.getElementById('scannerGuide').classList.contains('found'),
    videoW: (document.getElementById('scannerVideo') || {}).videoWidth || 0,
  }));
  await page.screenshot({ path: AUD + '/scanner-decoded.png' }).catch(() => {});

  // amount arm: purchase
  let award = { clicked: false };
  if (decoded) {
    await page.click('#segPurchase').catch(() => {});
    await sleep(400);
    await page.evaluate((a) => { document.getElementById('txAmount').value = a; }, AMOUNT);
    await page.click('#processBtn').catch(() => {});
    await page.waitForFunction(
      () => document.getElementById('successOverlay').classList.contains('show'),
      { timeout: 20000 }).catch(() => {});
    award = await page.evaluate(() => ({
      clicked: true,
      shown: document.getElementById('successOverlay').classList.contains('show'),
      title: document.getElementById('successTitle').textContent.trim(),
      member: document.getElementById('successMemberName').textContent.trim(),
      points: document.getElementById('successPoints').textContent.trim(),
      label: document.getElementById('successPointsLabel').textContent.trim(),
    }));
    await page.screenshot({ path: AUD + '/scanner-award.png' }).catch(() => {});
  }

  const out = { httpStatus: resp.status(), pageErrors, consoleErrors,
    decoded, payloadMatches: decodeState.storedQR === PAYLOAD, decodeState, award, scanReqs };
  await browser.close();
  return out;
}

(async () => {
  const png = AUD + '/qr-probe.png';
  const gen = await renderQrCard(png);
  if (!gen.hasCanvas || gen.w === 0) throw new Error('QR generator produced no canvas');
  const y4m = AUD + '/qr-probe.y4m';
  execFileSync('ffmpeg', ['-y', '-loop', '1', '-i', png, '-t', '5', '-r', '10',
    '-s', '640x640', '-pix_fmt', 'yuv420p', '-f', 'yuv4mpegpipe', y4m],
    { stdio: 'ignore' });
  const out = await scanAndAward(y4m);
  out.generator = gen;
  fs.writeFileSync(AUD + '/browser-proof.json', JSON.stringify(out, null, 1));
  console.log(JSON.stringify(out, null, 1));
})().catch((e) => { console.log('PROBE FAILED: ' + String(e).slice(0, 400)); process.exit(1); });
