/**
 * ZaarHub password reset — honest-refusal browser arm.
 *
 * Item 3 (network email), the front-end half: with the platform unable to send (no email
 * provider configured), a locked-out user must SEE the honest message on the real login page and
 * must NOT be advanced to the code-entry step that no message will ever fill.
 *
 * Drives the REAL served page in real headless Chromium: opens the Forgot Password overlay,
 * types an address, clicks "Send Reset Code", and reads the live DOM
 * (#forgot-msg, #forgot-step-reset, #forgot-success, #forgot-btn).
 *
 * Env: MD_BASE, EMAIL, AUD, CHROME_PATH
 * Output: $AUD/browser-proof.json (+ forgot-honest.png)
 */
const fs = require('fs');
const puppeteer = require('puppeteer-core');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const EMAIL = process.env.EMAIL;
const AUD = process.env.AUD;
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const proof = {
    pageLoaded: false,
    postStatus: 0,
    msgVisible: false,
    msgText: '',
    resetStepVisible: null,
    successVisible: null,
    btnText: '',
    advancedToCodeStep: null,
    claimedSuccess: null,
    expectedRefusalResource: null,
    pageErrors: [],
  };
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  try {
    const page = await browser.newPage();
    await page.setViewport({ width: 1440, height: 1000 });
    page.on('pageerror', (e) => proof.pageErrors.push('page: ' + String(e.message).slice(0, 200)));
    page.on('console', (m) => {
      if (m.type() !== 'error') return;
      const t = m.text();
      // Chrome logs the app's deliberate 503 on the reset call as a failed resource. That is the
      // platform answering honestly, not a script fault — recorded separately, never a failure.
      if (t.includes('Failed to load resource') && (t.includes('503') || t.includes('forgot-password'))) {
        proof.expectedRefusalResource = t.slice(0, 200);
        return;
      }
      proof.pageErrors.push('console: ' + t.slice(0, 200));
    });
    page.on('response', (r) => {
      if (r.url().includes('/auth/forgot-password')) proof.postStatus = r.status();
    });

    const resp = await page.goto(MD_BASE + '/login.html', { waitUntil: 'networkidle2', timeout: 60000 });
    proof.pageLoaded = !!resp && resp.status() === 200;

    await page.evaluate(() => showForgotPw());
    await page.waitForSelector('#forgot-email', { visible: true, timeout: 15000 });
    await page.type('#forgot-email', EMAIL);
    await page.click('#forgot-btn');

    let st = null;
    for (let i = 0; i < 40; i++) {
      st = await page.evaluate(() => ({
        msgShown: document.getElementById('forgot-msg').classList.contains('show'),
        msgText: document.getElementById('forgot-msg').textContent,
        resetVisible: document.getElementById('forgot-step-reset').style.display !== 'none',
        successVisible: document.getElementById('forgot-success').style.display !== 'none',
        btnText: document.getElementById('forgot-btn').textContent,
      }));
      if (st.msgShown) break;
      await sleep(250);
    }
    if (st) {
      proof.msgVisible = st.msgShown;
      proof.msgText = st.msgText;
      proof.resetStepVisible = st.resetVisible;
      proof.successVisible = st.successVisible;
      proof.btnText = st.btnText;
      proof.advancedToCodeStep = st.resetVisible === true;
      proof.claimedSuccess = st.successVisible === true;
    }
    await page.screenshot({ path: AUD + '/forgot-honest.png' });
  } catch (e) {
    proof.pageErrors.push('script: ' + String(e.message).slice(0, 300));
  } finally {
    await browser.close();
  }
  fs.writeFileSync(AUD + '/browser-proof.json', JSON.stringify(proof, null, 1));
  console.log(JSON.stringify(proof));
})();
