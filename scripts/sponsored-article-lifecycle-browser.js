/**
 * ZaarHub — sponsored/SEO article LIFECYCLE, driven from the operator console (queue item 6).
 *
 * The sibling proof (scripts/sponsored-article-smoke.py) starts from a PUBLISHED fixture and shows a
 * visitor can browse to it. That shape can never catch the defect this run found: the two public read
 * routes served an UNPUBLISHED draft, while the console's own copy says "Article generated as a draft.
 * Publish it when ready." and its Archive warning says an archived article "disappears from the public
 * site immediately". So this proof walks the real lifecycle through the console's own controls:
 *
 *   1. sign in as the operator, open the Sponsored & SEO Articles card
 *   2. pick the city, type keyword/city/service, click the card's own "+ Generate" button
 *   3. the new row is a DRAFT, it is listed, and it offers NO public link (a draft is not public)
 *   4. the console's Publish button makes it public, and the row then links to the article page
 *   5. a LOGGED-OUT browser can read the title + body of the published article on screen
 *   6. the console's Archive button removes it from the list
 *   7. the page ran with no script errors and made no failed API call
 *
 * Env:  MD_BASE, TOKEN (platform-operator JWT), PUBLIC, DIR_ID, CITY_NAME, CITY_SLUG, KEYWORD,
 *       SERVICE, AUD, CHROME_PATH
 * Out:  $AUD/lifecycle-proof.json  (+ draft-not-public.png, published-live.png, console-rows.png)
 * Exit: 0 = all browser checks pass, 1 = otherwise.
 */
const puppeteer = require('puppeteer-core');
const fs = require('fs');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TOKEN = process.env.TOKEN;
const PUBLIC = process.env.PUBLIC || 'https://zaarhub.com';
const DIR_ID = process.env.DIR_ID || '';
const CITY_NAME = process.env.CITY_NAME || '';
const CITY_SLUG = process.env.CITY_SLUG || '';
const KEYWORD = process.env.KEYWORD || '';
const SERVICE = process.env.SERVICE || 'plumbing';
const AUD = process.env.AUD || '/tmp';
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const checks = [];
function check(name, ok, detail = '') {
  checks.push({ name, ok: !!ok, detail: String(detail == null ? '' : detail) });
}

const IGNORE_CONSOLE = [/navigator\.vibrate/];

function readRow(page, key) {
  return page.evaluate((k) => {
    const box = document.getElementById('art-list');
    if (!box) return null;
    const tr = [...box.querySelectorAll('tbody tr')].find((t) => t.textContent.includes(k));
    if (!tr) return null;
    const tds = [...tr.querySelectorAll('td')];
    const link = tr.querySelector('td a[href*="/articles/"]');
    const btn = tr.querySelector('button[data-aid]');
    const pub = [...tr.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Publish');
    const arc = [...tr.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Archive');
    return {
      text: tr.textContent.replace(/\s+/g, ' ').trim().slice(0, 220),
      status: (tds[2] ? tds[2].textContent : '').trim(),
      link: link ? link.getAttribute('href') : null,
      id: btn ? btn.getAttribute('data-aid') : null,
      hasPublish: !!pub,
      hasArchive: !!arc,
    };
  }, key);
}

(async () => {
  if (!TOKEN) { console.error('FATAL no TOKEN in env'); process.exit(1); }
  if (!DIR_ID || !KEYWORD || !CITY_SLUG) { console.error('FATAL DIR_ID/KEYWORD/CITY_SLUG required'); process.exit(1); }
  const proof = { checks: [], steps: {} };
  const API_CALLS = [];
  const cerr = [];
  let browser, pub;

  try {
    fs.mkdirSync(AUD, { recursive: true });
    browser = await puppeteer.launch({
      executablePath: CHROME, headless: 'new',
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
    });
    const page = await browser.newPage();
    await page.setViewport({ width: 1440, height: 1200 });
    page.on('console', (m) => {
      if (m.type() !== 'error') return;
      const t = m.text();
      if (IGNORE_CONSOLE.some((re) => re.test(t))) return;
      cerr.push(t.slice(0, 220));
    });
    page.on('pageerror', (e) => cerr.push('pageerror: ' + String(e).slice(0, 220)));
    page.on('response', (r) => {
      const u = r.url();
      if (/\/api\/v1\/(directories\/[^/]+\/business-articles|business-articles)/.test(u)) {
        API_CALLS.push({ method: r.request().method(), status: r.status(), url: u.replace(MD_BASE, '').split('?')[0] });
      }
    });
    page.on('dialog', async (d) => { proof.steps.dialog = d.message(); await d.accept().catch(() => {}); });

    await page.evaluateOnNewDocument((t) => {
      localStorage.setItem('md_admin_token', t);
      localStorage.setItem('token', t);
      localStorage.setItem('md_admin_user', JSON.stringify({ email: 'verify@zzem', role: 'super_admin', name: 'Verify' }));
      localStorage.setItem('user', JSON.stringify({ email: 'verify@zzem', role: 'super_admin', name: 'Verify' }));
    }, TOKEN);

    // ── 1. the operator console ────────────────────────────────────────────────────────────────
    const resp = await page.goto(MD_BASE + '/admin-panel.html', { waitUntil: 'networkidle2', timeout: 60000 });
    check('the operator console loaded', resp && resp.status() === 200, 'status=' + (resp && resp.status()));
    await page.waitForSelector('#articles-section', { timeout: 30000 });
    // the card is reached through its own sidebar entry, so scroll it on screen like an operator does
    await page.evaluate(() => {
      const a = document.querySelector('a[href="#articles-section"]');
      if (a) a.click();
      const s = document.getElementById('articles-section');
      if (s) s.scrollIntoView();
    });
    await sleep(1500);
    check('the Sponsored & SEO Articles card is on screen',
      await page.evaluate(() => {
        const h = document.getElementById('articles-section');
        if (!h) return false;
        const r = h.getBoundingClientRect();
        return /Sponsored & SEO Articles/.test(h.textContent) && r.height > 0;
      }));
    await page.waitForFunction(() => {
      const s = document.getElementById('art-dir');
      return s && s.options.length > 0 && s.options[0].value;
    }, { timeout: 30000 });
    check('the card offers its city picker filled from the live directory list',
      await page.evaluate(() => document.getElementById('art-dir').options.length > 0),
      await page.evaluate(() => document.getElementById('art-dir').options.length + ' cities'));

    // ── 2. pick the city and generate through the card's own control ───────────────────────────
    await page.select('#art-dir', DIR_ID);
    await page.waitForFunction((k) => {
      const box = document.getElementById('art-list');
      if (!box) return false;
      const t = box.textContent || '';
      return /article\(s\)/.test(t) || /No articles for this city yet/.test(t);
    }, { timeout: 30000 }, KEYWORD);
    check('picking the city lists that city\'s articles', true);

    await page.click('#art-keyword');
    await page.type('#art-keyword', KEYWORD);
    await page.click('#art-city');
    await page.type('#art-city', CITY_NAME || CITY_SLUG);
    await page.click('#art-service');
    await page.type('#art-service', SERVICE);
    const clickedGen = await page.evaluate(() => {
      const b = [...document.querySelectorAll('#articles-section button')].find((x) => /Generate/.test(x.textContent));
      if (!b) return false;
      b.click();
      return true;
    });
    check('the card\'s own Generate control was clicked', clickedGen);
    await page.waitForFunction((k) => {
      const box = document.getElementById('art-list');
      return !!box && box.textContent.includes(k);
    }, { timeout: 45000 }, KEYWORD);

    const draftRow = await readRow(page, KEYWORD);
    proof.steps.draftRow = draftRow;
    check('the generated article is listed as a draft', !!draftRow && draftRow.status === 'draft',
      draftRow ? 'status=' + draftRow.status + ' | ' + draftRow.text : 'row not found');
    check('the card says a draft must be published before it is live',
      /draft/.test(await page.evaluate(() => (document.getElementById('art-status') || {}).textContent || '')),
      await page.evaluate(() => (document.getElementById('art-status') || {}).textContent || ''));
    check('a DRAFT row offers no public link (a draft is not public)', !!draftRow && draftRow.link === null,
      'link=' + (draftRow && draftRow.link));
    await page.screenshot({ path: AUD + '/console-rows.png' }).catch(() => {});

    // the row's own identity, read from the console's API (the operator's own view of it)
    const art = await page.evaluate(async (dirId, k) => {
      const t = localStorage.getItem('md_admin_token');
      const r = await fetch('/api/v1/directories/' + dirId + '/business-articles?limit=100', { headers: { Authorization: 'Bearer ' + t } });
      const d = await r.json();
      const rows = (d && d.data) || [];
      const a = rows.find((x) => (x.keyword || '') === k) || {};
      return { id: a.id, slug: a.slug, title: a.title, status: a.status, business_id: a.business_id };
    }, DIR_ID, KEYWORD);
    proof.steps.article = art;
    check('the draft is readable through the console\'s own list endpoint',
      !!art.id && art.status === 'draft' && !!art.slug, JSON.stringify({ slug: art.slug, status: art.status }));
    if (!art.slug) { throw new Error('no slug for the generated article'); }

    await page.screenshot({ path: AUD + '/draft-not-public.png' }).catch(() => {});

    // ── 3. a LOGGED-OUT visitor cannot read the draft ──────────────────────────────────────────
    pub = await puppeteer.launch({
      executablePath: CHROME, headless: 'new',
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
    });
    const pvis = await pub.newPage();
    await pvis.setViewport({ width: 1280, height: 900 });
    const pubErrs = [];
    pvis.on('pageerror', (e) => pubErrs.push(String(e).slice(0, 200)));

    const draftUrl = PUBLIC + '/' + CITY_SLUG + '/articles/' + art.slug;
    const dResp = await pvis.goto(draftUrl, { waitUntil: 'networkidle2', timeout: 60000 });
    const dText = await pvis.evaluate(() => document.body.innerText || '');
    proof.steps.draftPublic = { status: dResp && dResp.status(), url: draftUrl, hasTitle: dText.includes(art.title || 'zzz') };
    check('a logged-out visitor is NOT served the unpublished draft',
      !dText.includes(art.title || 'zzz-none') && !dText.includes(KEYWORD),
      'status=' + (dResp && dResp.status()) + ' url=' + draftUrl);

    // ── 4. the operator publishes it, through the console's own button ────────────────────────
    const pubClicked = await page.evaluate((id) => {
      const tr = [...document.querySelectorAll('#art-list tbody tr')].find((t) => t.querySelector('button[data-aid="' + id + '"]'));
      if (!tr) return false;
      const b = [...tr.querySelectorAll('button')].find((x) => x.textContent.trim() === 'Publish');
      if (!b) return false;
      b.click();
      return true;
    }, art.id);
    check('the card offers a Publish control on the draft row', pubClicked);
    await page.waitForFunction((id) => {
      const tr = [...document.querySelectorAll('#art-list tbody tr')].find((t) => t.querySelector('button[data-aid="' + id + '"]'));
      if (!tr) return false;
      const tds = [...tr.querySelectorAll('td')];
      return tds[2] && tds[2].textContent.trim() === 'published';
    }, { timeout: 30000 }, art.id);
    const pubRow = await readRow(page, KEYWORD);
    proof.steps.publishedRow = pubRow;
    check('the row now shows published', !!pubRow && pubRow.status === 'published', pubRow && pubRow.status);
    check('the published row links to its public article page',
      !!pubRow && !!pubRow.link && pubRow.link.indexOf('/' + CITY_SLUG + '/articles/') === 0,
      'href=' + (pubRow && pubRow.link));

    // ── 5. a logged-out visitor SEES it on screen ──────────────────────────────────────────────
    const liveCode = await pvis.reload({ waitUntil: 'networkidle2', timeout: 60000 }).then((r) => r && r.status());
    const titleVisible = await pvis.evaluate((t) => {
      // NB: the <title> in <head> has a zero-size box, so only look at laid-out body headings.
      const els = [...document.querySelectorAll('h1,h2,h3')].filter((n) => (n.textContent || '').includes(t));
      return els.some((el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; });
    }, art.title || 'zzz-none');
    const bodyText = await pvis.evaluate(() => document.body.innerText || '');
    proof.steps.livePublic = { status: liveCode, url: draftUrl, titleVisible, chars: bodyText.length };
    check('the published article answers on the public host', liveCode === 200, 'status=' + liveCode);
    check('the article TITLE is laid out on screen for a logged-out visitor', titleVisible);
    check('the article BODY copy is on the page', bodyText.length > 400 && bodyText.includes(art.title.slice(0, 24)),
      'chars=' + bodyText.length);
    check('the public article page ran with no script errors', pubErrs.length === 0, JSON.stringify(pubErrs));
    await pvis.screenshot({ path: AUD + '/published-live.png', fullPage: false }).catch(() => {});

    // ── 6. archive it again, through the console's own button ─────────────────────────────────
    const arcClicked = await page.evaluate((id) => {
      const tr = [...document.querySelectorAll('#art-list tbody tr')].find((t) => t.querySelector('button[data-aid="' + id + '"]'));
      if (!tr) return false;
      const b = [...tr.querySelectorAll('button')].find((x) => x.textContent.trim() === 'Archive');
      if (!b) return false;
      b.click();
      return true;
    }, art.id);
    check('the card offers an Archive control on the published row', arcClicked);
    await page.waitForFunction((id) => {
      const tr = [...document.querySelectorAll('#art-list tbody tr')].find((t) => t.querySelector('button[data-aid="' + id + '"]'));
      if (!tr) return false;
      const tds = [...tr.querySelectorAll('td')];
      return tds[2] && tds[2].textContent.trim() === 'archived';
    }, { timeout: 30000 }, art.id);
    const arcRow = await readRow(page, KEYWORD);
    proof.steps.archivedRow = arcRow;
    check('the console now shows the article as archived', !!arcRow && arcRow.status === 'archived',
      arcRow && arcRow.status);
    check('the archived row no longer offers a public link', !!arcRow && arcRow.link === null,
      'link=' + (arcRow && arcRow.link));
    check('the console warned that archiving takes it off the public site',
      /disappears from the public site/.test(proof.steps.dialog || ''), proof.steps.dialog || '');

    const arcCode = await pvis.reload({ waitUntil: 'networkidle2', timeout: 60000 }).then((r) => r && r.status());
    const arcText = await pvis.evaluate(() => document.body.innerText || '');
    check('an archived article is no longer readable by a visitor',
      !arcText.includes(art.title || 'zzz-none') && !arcText.includes(KEYWORD),
      'status=' + arcCode);

    // ── 7. the console made real calls and stayed clean ───────────────────────────────────────
    const gen = API_CALLS.find((c) => c.method === 'POST' && /business-articles\/generate/.test(c.url));
    const put = API_CALLS.find((c) => c.method === 'PUT' && /business-articles\//.test(c.url));
    const del = API_CALLS.find((c) => c.method === 'DELETE' && /business-articles\//.test(c.url));
    check('the browser called the real generate endpoint', !!gen && gen.status === 201, gen ? gen.status + ' ' + gen.url : 'none');
    check('the browser called the real publish endpoint', !!put && put.status === 200, put ? put.status + ' ' + put.url : 'none');
    check('the browser called the real archive endpoint', !!del && del.status === 200, del ? del.status + ' ' + del.url : 'none');
    const failed = API_CALLS.filter((c) => c.status >= 400);
    check('no failed article API call during the flow', failed.length === 0, JSON.stringify(failed).slice(0, 200));
    check('no JavaScript console errors during the flow', cerr.length === 0, JSON.stringify(cerr).slice(0, 220));
  } catch (e) {
    check('the flow completed without throwing', false, String(e && e.message ? e.message : e).slice(0, 200));
  } finally {
    if (pub) await pub.close().catch(() => {});
    if (browser) await browser.close().catch(() => {});
  }

  proof.checks = checks;
  proof.apiCalls = API_CALLS;
  proof.consoleErrors = cerr;
  proof.ok = checks.every((c) => c.ok);
  const out = process.env.AUD || '/tmp';
  fs.mkdirSync(out, { recursive: true });
  fs.writeFileSync(out + '/lifecycle-proof.json', JSON.stringify(proof, null, 1));
  for (const c of checks) console.log((c.ok ? 'PASS  ' : 'FAIL  ') + c.name + (c.detail ? '  [' + c.detail + ']' : ''));
  console.log('BROWSER RESULT: ' + (proof.ok ? 'all passed' : 'FAILURES'));
  process.exit(proof.ok ? 0 : 1);
})();
