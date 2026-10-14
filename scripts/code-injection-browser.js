/**
 * Durable proof of queue item 11 (code injection head/body/footer), owner-portal screen.
 * Loads the LIVE portal.html in real headless Chromium, stubs the API so the SEO page renders
 * with a directory carrying head/body/footer code, then asserts the three code boxes are on
 * screen, carry the directory's values, and that pressing Save really asks the API to store all
 * three. Read-only against production data — window.fetch is replaced, so nothing is written.
 *
 * Run via:  python3 scripts/code-injection-smoke.py   (which sets NODE_PATH + CHROME_PATH)
 */
const puppeteer = require('puppeteer');
const BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const DIR = { id: 'dir-1', slug: 'd1', name: 'D1',
  head_injection: '<meta name="probe-head" content="H">',
  body_injection: '<div id="probe-body">BODY</div>',
  footer_injection: '<div id="probe-foot">FOOT</div>' };
const out = [];
const rec = (ok, name, detail) => out.push((ok ? 'PASS ' : 'FAIL ') + name + ' :: ' + String(detail === undefined ? '' : detail).slice(0, 200));

(async () => {
  const browser = await puppeteer.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    headless: 'new', args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  try {
    const page = await browser.newPage();
    const pageErrors = [];
    page.on('pageerror', (e) => pageErrors.push(String(e.message).slice(0, 160)));
    await page.goto(`${BASE}/portal.html`, { waitUntil: 'domcontentloaded', timeout: 30000 });
    const res = await page.evaluate(async (dir) => {
      const calls = [];
      window.fetch = function (url, opts) {
        const m = (opts && opts.method) || 'GET';
        let body = '';
        try { body = opts && opts.body ? String(opts.body) : ''; } catch (e) { body = ''; }
        calls.push(m + ' ' + url + (body ? ' ' + body : ''));
        let payload = {};
        if (String(url).indexOf('/seo/directory/') !== -1) payload = { title: 'T', description: 'D', keywords: 'K' };
        else if (String(url).indexOf('/directories/') !== -1 && m === 'GET') payload = dir;
        return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(payload), text: () => Promise.resolve('') });
      };
      window.CUR_DIR = dir;
      window.CP = 'seo';
      renderApp();
      await new Promise((r) => setTimeout(r, 400));
      const H = document.getElementById('seo_head');
      const B = document.getElementById('seo_body');
      const F = document.getElementById('seo_footer');
      const vis = (el) => !!(el && el.offsetParent !== null);
      const before = calls.slice();
      // press the card's Save button (the real onclick)
      const btn = Array.from(document.querySelectorAll('#pageContent button')).find((b) => /Save/.test(b.textContent));
      if (btn) btn.click();
      await new Promise((r) => setTimeout(r, 250));
      return {
        hasH: !!H, hasB: !!B, hasF: !!F,
        visH: vis(H), visB: vis(B), visF: vis(F),
        vH: H ? H.value : '', vB: B ? B.value : '', vF: F ? F.value : '',
        labels: Array.from(document.querySelectorAll('#pageContent label')).map((l) => l.textContent).filter((t) => /HTML/.test(t)),
        calls, before, saveClicked: !!btn,
      };
    }, DIR);

    rec(res.hasH && res.hasB && res.hasF, 'three code boxes present (header/body/footer)', `h=${res.hasH} b=${res.hasB} f=${res.hasF}`);
    rec(res.visH && res.visB && res.visF, 'all three code boxes visible on screen', `h=${res.visH} b=${res.visB} f=${res.visF}`);
    rec(res.vH === DIR.head_injection, 'header box loaded the saved code', res.vH);
    rec(res.vB === DIR.body_injection, 'body box loaded the saved code', res.vB);
    rec(res.vF === DIR.footer_injection, 'footer box loaded the saved code', res.vF);
    rec(res.labels.length >= 3, 'boxes are labelled Custom Header/Body/Footer HTML', res.labels.join(' | '));
    const save = res.calls.find((c) => c.startsWith('PUT /api/v1/directories/d1'));
    rec(!!save, 'Save asks the API to store the code', save || 'no matching call');
    if (save) {
      rec(/head_injection/.test(save) && /body_injection/.test(save) && /footer_injection/.test(save),
        'the save carries all three code fields', save.slice(0, 180));
    }
    rec(pageErrors.length === 0, 'no page errors', pageErrors.join(' | '));
  } catch (e) {
    rec(false, 'exception', String(e && e.message).slice(0, 200));
  } finally {
    await browser.close();
  }
  console.log(out.join('\n'));
  const failed = out.filter((l) => l.startsWith('FAIL'));
  console.log('RESULT: ' + (failed.length ? 'FAIL (' + failed.length + ')' : 'PASS'));
  process.exit(failed.length ? 1 : 0);
})();
