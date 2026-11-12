/** Item 15 browser arm — the company Headquarters is rendered on EVERY city page.
 *
 * Loads each city page (http://127.0.0.1:8089/<slug>) in a real browser and asserts the
 * headquarters text "Orlando, Florida" appears TWICE on screen:
 *   1. the footer line (zhFillCompany -> #zh-hq-value), and
 *   2. the city hero ("🏢 Headquarters:" line, .zh-hero-hq).
 *
 * Env: CHROME_PATH, CITY_SLUGS (comma-separated). Exit 0 = all passed.
 */
const puppeteer = require('puppeteer-core');

const BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const SLUGS = (process.env.CITY_SLUGS || '').split(',').map((s) => s.trim()).filter(Boolean);
const EXPECT = 'Orlando, Florida';

(async () => {
  const problems = [];
  const ok = (m) => console.log('[PASS] ' + m);
  const bad = (m) => { problems.push(m); console.log('[FAIL] ' + m); };

  const browser = await puppeteer.launch({
    executablePath: process.env.CHROME_PATH,
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });

  try {
    for (const slug of SLUGS) {
      const page = await browser.newPage();
      await page.setViewport({ width: 1280, height: 900 });
      const errors = [];
      page.on('pageerror', (e) => errors.push(String(e).slice(0, 120)));

      await page.goto(`${BASE}/${slug}`, { waitUntil: 'networkidle2', timeout: 60000 });
      await page.waitForFunction(
        () => document.querySelector('#zh-hq-line') && document.querySelector('#zh-hq-line').style.display === 'block',
        { timeout: 25000 },
      ).catch(() => {});

      const state = await page.evaluate((expected) => {
        const footer = document.getElementById('zh-hq-value');
        const hero = document.querySelector('.zh-hero-hq');
        return {
          footerText: footer ? footer.textContent.trim() : '',
          heroText: hero ? hero.textContent.trim() : '',
          footerVisible: (() => {
            const l = document.getElementById('zh-hq-line');
            return !!(l && l.style.display === 'block');
          })(),
          expected,
        };
      }, EXPECT);

      state.footerText.includes(EXPECT) && state.footerVisible
        ? ok(`${slug}: footer renders "Orlando, Florida"`)
        : bad(`${slug}: footer missing/not visible (footer=${state.footerText}, visible=${state.footerVisible})`);

      state.heroText.includes(EXPECT)
        ? ok(`${slug}: city hero renders "🏢 Headquarters: Orlando, Florida"`)
        : bad(`${slug}: hero missing (hero=${state.heroText})`);

      if (errors.length) bad(`${slug}: page errors: ` + errors.join(' | '));
      await page.close();
    }
  } catch (e) {
    bad('browser flow threw: ' + e.message);
  }

  await browser.close();
  console.log('BROWSER RESULT: ' + (problems.length ? problems.length + ' failed' : 'all passed'));
  process.exit(problems.length ? 1 : 0);
})();
