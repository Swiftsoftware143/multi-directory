/**
 * Durable proof (browser arm) — item 1, the signed-in shopper's own pages: /user/saved must
 * render other people's text (a business name, a review body, a perk) as TEXT, never as HTML.
 *
 * The defect class this pins: `frontend/user-saved.html` built its Saved / Following /
 * Recommended / My Reviews / Perks cards with `innerHTML` and interpolated `b.name`,
 * `b.category_name`, `r.content` and `p.title` raw. A business owner controls their business
 * name, so one renamed listing became a live payload on every shopper's saved list — the same
 * class as the /biz/:id review XSS (1b8e7c2), on a different surface.
 *
 * Env: MD_URL, TOKEN, PAYLOAD_NAME, PAYLOAD_CONTENT, SHOT_DIR, CHROME_PATH
 * Exit: 0 = all browser checks passed, 1 = a check failed.
 */
const puppeteer = require('puppeteer-core');

const BASE = process.env.MD_URL || 'http://127.0.0.1:8089';
const TOKEN = process.env.TOKEN;
const PAYLOAD_NAME = process.env.PAYLOAD_NAME;
const PAYLOAD_CONTENT = process.env.PAYLOAD_CONTENT;
const SHOT_DIR = process.env.SHOT_DIR || '/tmp';

(async () => {
  const problems = [];
  const ok = (m) => console.log('PASS  ' + m);
  const bad = (m) => { problems.push(m); console.log('FAIL  ' + m); };

  const browser = await puppeteer.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });

  try {
    const page = await browser.newPage();
    const pageErrors = [];
    page.on('pageerror', (e) => pageErrors.push(String(e)));
    // the page reads the shopper's session from localStorage before it fetches anything
    await page.evaluateOnNewDocument((t) => {
      try { localStorage.setItem('zh_token', t); localStorage.setItem('visitorToken', t); } catch (e) {}
    }, TOKEN);

    await page.goto(`${BASE}/user/saved`, { waitUntil: 'networkidle2', timeout: 60000 });
    await page.waitForSelector('.rev-content', { timeout: 30000 }).catch(() => {});
    await new Promise((r) => setTimeout(r, 1200));

    const state = await page.evaluate((payloadName) => {
      const rev = document.querySelector('.rev-content');
      const nameCard = Array.from(document.querySelectorAll('.saved-card'))
        .find((c) => c.textContent.includes(payloadName));
      return {
        xssProbe: window.__xssProbe || null,
        xssName: window.__xssName || null,
        injectedImgs: document.querySelectorAll('img[src="x"], img[src="y"], img[src="z"]').length,
        reviewCards: document.querySelectorAll('.review-card').length,
        savedCards: document.querySelectorAll('.saved-card').length,
        revText: rev ? rev.textContent : '',
        revHTML: rev ? rev.innerHTML : '',
        nameCardFound: !!nameCard,
        nameCardHTML: nameCard ? nameCard.innerHTML : '',
        title: document.title,
      };
    }, PAYLOAD_NAME);

    if (state.reviewCards >= 1) ok(`the shopper's own review rendered (${state.reviewCards} review card(s), ${state.savedCards} saved card(s))`);
    else bad('no review card rendered — the escaping cannot be judged');

    if (state.revText.includes(PAYLOAD_CONTENT)) ok('the review body is displayed verbatim as text');
    else bad(`the review body was not shown verbatim (saw: ${JSON.stringify(state.revText.slice(0, 120))})`);

    if (state.nameCardFound) ok('the saved-list card shows the payload business NAME as text');
    else bad('the saved-list card did not render the payload business — the name arm cannot be judged');

    if (state.xssProbe === null && state.xssName === null) ok('no injected event handler executed (window.__xssProbe / __xssName still unset)');
    else bad(`an injected handler EXECUTED (probe=${state.xssProbe} name=${state.xssName})`);

    if (state.injectedImgs === 0) ok('no <img> element was created from any payload');
    else bad(`${state.injectedImgs} <img> element(s) were created from a payload`);

    if (!state.revHTML.includes('<img') && !state.nameCardHTML.includes('<img')) ok('no raw <img> tag in the rendered markup of either card');
    else bad('a rendered card contains a RAW <img> tag (interpolated, not escaped)');

    if (pageErrors.length === 0) ok('no uncaught page errors');
    else bad(`${pageErrors.length} page error(s): ${pageErrors.slice(0, 2).join(' | ')}`);

    await page.screenshot({ path: `${SHOT_DIR}/user-saved-cards.png`, fullPage: false });

    // control: prove the probe CAN see the defect — interpolate the same payload raw and check
    // that the handler fires. Run last so it cannot contaminate the assertions above.
    const controlFired = await page.evaluate((p) => new Promise((res) => {
      const d = document.createElement('div');
      d.style.position = 'fixed'; d.style.left = '-9999px';
      document.body.appendChild(d);
      d.innerHTML = p;
      setTimeout(() => { const hit = window.__xssProbe === 1; d.remove(); res(hit); }, 400);
    }), PAYLOAD_CONTENT);
    if (controlFired) ok('control: the same payload DOES execute when interpolated raw — the probe detects the defect');
    else bad('control failed: the payload did not execute even when interpolated raw — the probe cannot detect the defect');
  } catch (e) {
    bad('the browser flow threw: ' + e.message);
  } finally {
    await browser.close();
  }

  console.log('');
  if (problems.length) {
    console.log(`RESULT: FAIL (${problems.length})`);
    process.exit(1);
  }
  console.log('RESULT: PASS');
  process.exit(0);
})();
