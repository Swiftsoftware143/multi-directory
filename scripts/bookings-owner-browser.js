// Browser arm: the OWNER's own screen must show and act on a booking, not just the API.
// Signs in through the portal's own login form, opens "Booking Requests", sees the customer's
// request, clicks the page's own Confirm button, and the card re-renders as confirmed.
const puppeteer = require('/opt/swift/browser-run/node_modules/puppeteer');

(async () => {
  const email = process.argv[2], pass = process.argv[3], marker = process.argv[4] || 'Bookings ownership probe';
  const out = { stages: [], ok: false };
  const browser = await puppeteer.launch({
    executablePath: '/root/.cache/puppeteer/chrome/linux-127.0.6533.88/chrome-linux64/chrome',
    headless: 'new',
    args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage']
  });
  try {
    const page = await browser.newPage();
    const errors = [];
    const badResponses = [];
    page.on('console', m => { if (m.type() === 'error') errors.push(m.text().slice(0, 200)); });
    page.on('pageerror', e => errors.push('pageerror: ' + String(e).slice(0, 200)));
    page.on('response', r => {
      if (r.status() >= 400) badResponses.push({ status: r.status(), url: r.url().replace('https://zaarhub.com', '') });
    });

    out.stages.push('goto');
    await page.goto('https://zaarhub.com/portal', { waitUntil: 'networkidle2', timeout: 60000 });
    await page.waitForSelector('#loginEmail', { visible: true, timeout: 20000 });
    await page.type('#loginEmail', email);
    await page.type('#loginPassword', pass);
    await page.evaluate(() => doLogin());
    await page.waitForFunction(() => !!localStorage.getItem('bizToken'), { timeout: 30000 });

    out.stages.push('business-loaded');
    await page.waitForFunction(() => {
      const s = document.getElementById('bkBiz');
      return s && s.options.length >= 1 && s.value;
    }, { timeout: 40000 });
    out.bizScope = await page.evaluate(() => {
      const s = document.getElementById('bkBiz');
      return { options: s.options.length, value: s.value };
    });

    out.stages.push('nav');
    await page.click('#navBookings');

    out.stages.push('cards');
    const deadline = Date.now() + 40000;
    let text = '';
    while (Date.now() < deadline) {
      text = await page.evaluate(() => (document.getElementById('bkList') || {}).innerText || '');
      if (text.indexOf(marker) !== -1) break;
      await new Promise(r => setTimeout(r, 1000));
    }
    out.listText = text.replace(/\s+/g, ' ').slice(0, 400);
    out.cardSeen = text.indexOf(marker) !== -1;
    out.denied = /Not authorized|Could not load booking requests/.test(text);

    const clicked = await page.evaluate((marker) => {
      const cards = Array.from(document.querySelectorAll('#bkList .card'));
      const card = cards.find(c => c.innerText.indexOf(marker) !== -1);
      if (!card) return { ok: false, why: 'card not found', cards: cards.length };
      const btn = Array.from(card.querySelectorAll('button')).find(b => b.innerText.trim() === 'Confirm');
      if (!btn) return { ok: false, why: 'no confirm button', text: card.innerText.slice(0, 200) };
      btn.click();
      return { ok: true };
    }, marker);
    out.clicked = clicked;

    out.stages.push('after-click');
    const deadline2 = Date.now() + 30000;
    let after = '';
    while (Date.now() < deadline2) {
      after = await page.evaluate(() => (document.getElementById('bkList') || {}).innerText || '');
      if (/confirmed/i.test(after)) break;
      await new Promise(r => setTimeout(r, 1000));
    }
    out.afterText = after.replace(/\s+/g, ' ').slice(0, 400);
    out.confirmedOnScreen = /confirmed/i.test(after);
    await page.screenshot({ path: process.env.SHOT || '/tmp/bookings-owner-ui.png', fullPage: false });
    out.consoleErrors = errors;
    out.pageErrors = errors.filter(e => e.startsWith('pageerror:'));
    out.badResponses = badResponses;
    // A console line that merely echoes another panel's 4xx is not this screen's failure:
    // fail only on uncaught script errors or on a 4xx from the bookings calls themselves.
    const bookingBad = badResponses.filter(b => /\/bookings/.test(b.url));
    out.bookingBad = bookingBad;
    out.ok = out.cardSeen && !out.denied && out.clicked.ok && out.confirmedOnScreen &&
             out.pageErrors.length === 0 && bookingBad.length === 0;
  } catch (e) {
    out.error = String(e).slice(0, 400);
  } finally {
    await browser.close();
  }
  console.log(JSON.stringify(out));
  process.exit(out.ok ? 0 : 1);
})();
