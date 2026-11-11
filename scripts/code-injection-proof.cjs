const fs = require('fs');
const { execSync } = require('child_process');
const puppeteer = require('/opt/swift/browser-run/node_modules/puppeteer-core');

// Proves item 11: the owner portal's SEO screen saves head/body/footer code and the LIVE city
// page actually carries it. Runs against a THROWAWAY city (created and deleted by this script),
// so it never writes to — or wipes the code of — a real city.
//
// History (2026-11-11): this harness used to edit whatever city the portal happened to select
// (CUR_DIR = GET /directories?per_page=100 -> data[0]) while verifying the slug its OWN list
// request returned. The two orderings disagreed, so it saved into one city and read another —
// a FAIL that looked like a product defect. It also cleared the real city's code before
// starting. Both are fixed here: one probe city, read back from the page, deleted afterwards.

const BASE = 'http://127.0.0.1:8089';
const API = BASE + '/api/v1';
const OUT = '/opt/swift/audits/builds-item11-code-injection';
fs.mkdirSync(OUT, { recursive: true });

const steps = [];
const say = (ok, label, detail) => { steps.push({ ok, label, detail: detail || '' }); console.log((ok ? 'PASS ' : 'FAIL ') + label + (detail ? ' | ' + detail : '')); };

function chromePath() {
  try { const m = require('/opt/swift/browser-run/chromium-path.js'); if (typeof m === 'function') return m(); if (m && m.chromiumPath) return m.chromiumPath(); } catch (e) {}
  const c = ['/usr/bin/chromium', '/usr/bin/google-chrome'];
  return c.find(p => { try { return fs.statSync(p).size > 0; } catch (e) { return false; } });
}

const TOKEN = execSync('/opt/swift/bin/md-mint-token.py swiftsoftware143@yahoo.com 1800', { encoding: 'utf8' }).trim();
if (!TOKEN) { console.log('FAIL no token'); process.exit(2); }

const HARNESS = 'builds-codeinj-probe';

async function call(method, path, body) {
  const r = await fetch(API + path, {
    method,
    headers: Object.assign(
      { 'Content-Type': 'application/json', 'X-Swift-Harness': HARNESS },
      TOKEN ? { Authorization: 'Bearer ' + TOKEN } : {}
    ),
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await r.text();
  let json = null; try { json = JSON.parse(text); } catch (e) {}
  return { status: r.status, json, text };
}

const HEAD = '<meta name="zz-ui-head" content="UIHEADMARKER"/>';
const BODY = '<div id="zz-ui-body">UIBODYMARKER</div>';
const FOOT = '<script>window.__zz_ui_footer=1;</script>';

let slug = null, probeId = null, browser = null;

async function noteNow(p) {
  return await p.$eval('#seo_code_note', el => el.textContent.trim()).catch(() => '');
}

async function waitNote(p, re, ms) {
  const t0 = Date.now();
  while (Date.now() - t0 < (ms || 15000)) {
    const n = await noteNow(p);
    if (re.test(n)) return n;
    await new Promise(r => setTimeout(r, 250));
  }
  return await noteNow(p);
}

// click the Save button that belongs to the SEO card (the page has other Save buttons)
async function clickSeoSave(p) {
  return await p.evaluate(() => {
    const el = document.getElementById('seo_head');
    const card = el && el.closest ? el.closest('.card') : null;
    const btn = card && Array.from(card.querySelectorAll('button')).find(x => /Save/i.test(x.textContent));
    if (!btn) return false;
    btn.click(); return true;
  });
}

async function openSeo(p) {
  await p.evaluate(() => {
    const a = Array.from(document.querySelectorAll('.sidebar-nav a')).find(x => /SEO/i.test(x.textContent));
    if (a) a.click();
  });
  await p.waitForSelector('#seo_head', { timeout: 20000 });
  await p.waitForSelector('#seo_body', { timeout: 5000 });
  await p.waitForSelector('#seo_footer', { timeout: 5000 });
}

(async () => {
  try {
    // ── 0. throwaway city, inside an existing network (so no CoreSwift workspace is minted) ──
    const nets = await call('GET', '/networks');
    const netList = Array.isArray(nets.json) ? nets.json : (nets.json && nets.json.data) || [];
    const netId = netList[0] && netList[0].id;
    say(!!netId, 'an existing network is available to hang the probe city off', String(netId || ''));
    slug = 'zz-codeinj-' + Date.now();
    const created = await call('POST', '/directories', {
      name: 'ZZ Code Injection Probe',
      slug,
      city: 'Probeville',
      state: 'FL',
      status: 'active',
      network_mode: 'connect',
      parent_network_id: netId,
      url_type: 'subfolder',
      url_value: slug,
    });
    probeId = created.json && created.json.id;
    say(created.status === 201 && !!probeId, 'a throwaway city was created for the test', 'http=' + created.status + ' slug=' + slug);

    // ── 1. the owner portal, pointed at that throwaway city ────────────────────────────
    browser = await puppeteer.launch({
      executablePath: chromePath(), headless: 'new',
      args: ['--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu'],
    });
    const page = await browser.newPage();
    await page.setViewport({ width: 1440, height: 1000 });
    await page.setCacheEnabled(false);
    const errors = [], badApi = [];
    page.on('console', m => { if (m.type() === 'error') errors.push(m.text().slice(0, 200)); });
    page.on('pageerror', e => errors.push('pageerror: ' + String(e).slice(0, 200)));
    page.on('response', r => {
      const u = r.url();
      if (u.includes('/api/v1/') && r.status() >= 400) badApi.push(r.status() + ' ' + u.replace(BASE, ''));
    });
    await page.evaluateOnNewDocument((tok, user) => {
      localStorage.setItem('token', tok);
      localStorage.setItem('user', JSON.stringify(user));
    }, TOKEN, { email: 'swiftsoftware143@yahoo.com', role: 'super_admin', name: 'Operator' });

    await page.goto(BASE + '/portal.html', { waitUntil: 'networkidle2', timeout: 45000 });
    await page.waitForSelector('.sidebar-nav a', { timeout: 20000 });
    const switched = await page.evaluate((s) => {
      if (typeof window.switchDir !== 'function') return 'no-switchDir';
      window.switchDir(s);
      return window.CUR_DIR && window.CUR_DIR.slug;
    }, slug);
    say(switched === slug, 'the portal is pointed at the throwaway city', String(switched));
    const dirName = await page.$eval('.sidebar-footer .dir-name', el => el.textContent.trim()).catch(() => '');
    say(/ZZ Code Injection Probe/.test(dirName), 'the portal shows which city is being edited', dirName);

    const navLabels = await page.$$eval('.sidebar-nav a', as => as.map(a => a.textContent.trim()));
    say(navLabels.some(l => /SEO/i.test(l)), 'portal sidebar offers an SEO screen', navLabels.join(' / '));

    await openSeo(page);
    say(true, 'the screen shows three code boxes: head, body, footer');

    // it must show what is SAVED (a fresh city: three empty boxes), never a blank form that
    // would silently overwrite saved code
    const note1 = await waitNote(page, /Loaded the code saved/i, 15000);
    say(/Loaded the code saved/i.test(note1), 'the screen loads the code saved for this city first', note1.slice(0, 120));
    const loaded = await page.evaluate(() => ({
      h: document.getElementById('seo_head').value,
      b: document.getElementById('seo_body').value,
      f: document.getElementById('seo_footer').value,
    }));
    say(loaded.h === '' && loaded.b === '' && loaded.f === '', 'the fresh city starts with three empty boxes', JSON.stringify(loaded));

    // ── 2. type and Save, exactly like an operator ────────────────────────────────────
    await page.evaluate((h, b, f) => {
      const set = (id, v) => { const el = document.getElementById(id); el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); };
      set('seo_head', h); set('seo_body', b); set('seo_footer', f);
    }, HEAD, BODY, FOOT);
    const clicked = await clickSeoSave(page);
    say(clicked, 'clicked the Save button on the SEO card');
    await new Promise(r => setTimeout(r, 2500));
    const toastText = await page.$$eval('.toast, #toast', els => els.map(e => e.textContent.trim()).join(' | ')).catch(() => '');
    say(/saved/i.test(toastText), 'the screen confirms the save on screen', toastText.slice(0, 140));

    const after = await call('GET', '/directories/' + slug);
    const a2 = after.json || {};
    say(a2.head_injection === HEAD && a2.body_injection === BODY && a2.footer_injection === FOOT,
      'clicking Save really stored all three on the city', `h=${JSON.stringify(a2.head_injection).slice(0, 40)} b=${(a2.body_injection || '').length} f=${(a2.footer_injection || '').length}`);

    await page.screenshot({ path: OUT + '/seo-screen.png', fullPage: false });

    // re-open the screen from scratch: it must show the code now SAVED, not blanks
    await openSeo(page);
    await waitNote(page, /Loaded the code saved/i, 15000);
    const reloaded = await page.evaluate(() => ({
      h: document.getElementById('seo_head').value,
      b: document.getElementById('seo_body').value,
      f: document.getElementById('seo_footer').value,
    }));
    say(reloaded.h === HEAD && reloaded.b === BODY && reloaded.f === FOOT,
      're-opening the screen shows the saved code in the boxes', JSON.stringify(reloaded).slice(0, 120));

    // ── 3. the live city page must now carry the code ─────────────────────────────────
    const city = await browser.newPage();
    await city.setCacheEnabled(false);
    const cityErrors = [];
    city.on('pageerror', e => cityErrors.push(String(e).slice(0, 160)));
    await city.goto(BASE + '/' + slug, { waitUntil: 'networkidle2', timeout: 45000 });
    const live = await city.evaluate(() => {
      const meta = document.querySelector('meta[name="zz-ui-head"]');
      const bodyEl = document.getElementById('zz-ui-body');
      const all = Array.from(document.querySelectorAll('script'));
      return {
        head: !!meta && meta.getAttribute('content') === 'UIHEADMARKER',
        body: !!bodyEl,
        footerRan: window.__zz_ui_footer === 1,
        footerLast: all.some(s => (s.textContent || '').includes('__zz_ui_footer')),
        bodyFirst: document.body.firstElementChild ? document.body.firstElementChild.id : '',
        title: document.title,
      };
    });
    say(live.head, 'the live city page has the head code', 'meta found=' + live.head);
    say(live.body, 'the live city page has the body code at the top of the page', 'first child=' + live.bodyFirst);
    say(live.footerRan && live.footerLast, 'the footer code actually RAN in the browser', 'window flag=' + live.footerRan);
    say(cityErrors.length === 0, 'no page errors on the live city page', cityErrors.join(' | ').slice(0, 160));
    await city.screenshot({ path: OUT + '/live-city.png', fullPage: false });

    // ── 4. clear it again from the same screen (an operator must be able to remove code) ──
    await page.bringToFront();
    await openSeo(page);
    await waitNote(page, /Loaded the code saved/i, 15000);
    await page.evaluate(() => {
      ['seo_head', 'seo_body', 'seo_footer'].forEach(id => { const el = document.getElementById(id); el.value = ''; el.dispatchEvent(new Event('input', { bubbles: true })); });
    });
    await clickSeoSave(page);
    await new Promise(r => setTimeout(r, 2500));
    const cleared = await call('GET', '/directories/' + slug);
    const c2 = cleared.json || {};
    say(!(c2.head_injection || '').trim() && !(c2.body_injection || '').trim() && !(c2.footer_injection || '').trim(),
      'clearing the boxes and saving removes the code', JSON.stringify({ h: c2.head_injection, b: c2.body_injection, f: c2.footer_injection }).slice(0, 120));

    const fresh = await browser.newPage();
    await fresh.setCacheEnabled(false);
    await fresh.goto(BASE + '/' + slug + '?zz=' + Date.now(), { waitUntil: 'networkidle2', timeout: 45000 });
    const gone = await fresh.evaluate(() => ({
      head: !!document.querySelector('meta[name="zz-ui-head"]'),
      body: !!document.getElementById('zz-ui-body'),
      foot: window.__zz_ui_footer === 1,
    }));
    say(!gone.head && !gone.body && !gone.foot, 'the live page is clean again after clearing', JSON.stringify(gone));

    say(badApi.length === 0, 'no failed API calls on the portal screen', badApi.join(' | ').slice(0, 200));
    say(errors.length === 0, 'no JavaScript errors on the portal screen', errors.join(' | ').slice(0, 240));

    // ── 5. data-loss guard: Save while the code has NOT loaded must not wipe it ─────────
    await call('PUT', '/directories/' + slug, { head_injection: HEAD, body_injection: BODY, footer_injection: FOOT });
    const guard = await browser.newPage();
    await guard.setCacheEnabled(false);
    await guard.evaluateOnNewDocument((tok) => {
      localStorage.setItem('token', tok);
      localStorage.setItem('user', JSON.stringify({ email: 'swiftsoftware143@yahoo.com', role: 'super_admin' }));
    }, TOKEN);
    await guard.setRequestInterception(true);
    guard.on('request', r => {
      if (/\/api\/v1\/directories\/[^/?]+$/.test(r.url()) && r.method() === 'GET') return r.abort().catch(() => {});
      return r.continue().catch(() => {});
    });
    await guard.goto(BASE + '/portal.html', { waitUntil: 'networkidle2', timeout: 45000 });
    await guard.waitForSelector('.sidebar-nav a', { timeout: 20000 });
    await guard.evaluate((s) => { if (typeof window.switchDir === 'function') window.switchDir(s); }, slug);
    await openSeo(guard);
    await new Promise(r => setTimeout(r, 1500));
    const guardNote = await noteNow(guard);
    say(/Could not load/i.test(guardNote), 'the screen warns when the saved code could not be loaded', guardNote.slice(0, 140));
    await clickSeoSave(guard);
    await new Promise(r => setTimeout(r, 2000));
    const guardToast = await guard.$$eval('.toast', els => els.map(e => e.textContent.trim()).join(' | ')).catch(() => '');
    const afterGuard = (await call('GET', '/directories/' + slug)).json || {};
    say(/NOT changed/i.test(guardToast), 'pressing Save without loaded code says so on screen', guardToast.slice(0, 140));
    say(afterGuard.head_injection === HEAD && afterGuard.body_injection === BODY && afterGuard.footer_injection === FOOT,
      'code that had not loaded was NOT wiped by Save', `h=${JSON.stringify(afterGuard.head_injection).slice(0, 30)}`);
    await guard.close();
  } catch (e) {
    say(false, 'ERROR ' + (e && e.message ? e.message.slice(0, 220) : String(e)));
  } finally {
    if (browser) await browser.close().catch(() => {});
    // the throwaway city goes away entirely: no code, no rows, no city, no CoreSwift workspace
    if (slug) {
      const del = await call('DELETE', '/directories/' + slug).catch(() => ({ status: 0, text: '' }));
      const after = await call('GET', '/directories/' + slug).catch(() => ({ status: 0 }));
      const pageAfter = await fetch(BASE + '/' + slug).then(r => r.status).catch(() => 0);
      say(del.status === 200 && after.status >= 400 && pageAfter === 404,
        'the throwaway city is deleted and its page is gone',
        `delete=${del.status} get=${after.status} page=${pageAfter}`);
    }
    const fails = steps.filter(s => !s.ok);
    fs.writeFileSync(OUT + '/browser.json', JSON.stringify({ slug, probe_id: probeId, steps }, null, 2));
    console.log('RESULT: ' + (fails.length === 0 ? 'PASS' : 'FAIL') + ` (${steps.length - fails.length}/${steps.length})`);
    process.exit(fails.length === 0 ? 0 : 1);
  }
})();
