/**
 * ZaarHub admin console — NETWORKS screen, real end-to-end browser arm
 * (operator action list item 13: "network creation from admin").
 *
 * What it proves, on the REAL served console (no stubs, no mocks) at /admin-panel:
 *   A. The Networks card + its nav link exist and render the networks the live API holds.
 *   B. "＋ New network" opens the editor titled "New network".
 *   C. A junk root domain is REFUSED ON SCREEN (the server's own message is shown) and no
 *      row is created — a bad value must never reach the database.
 *   D. Pressing the card's own "Save network" with a valid name/slug/root creates a REAL
 *      network row (read back from the API and, in the orchestrator, from the database).
 *   E. The new network appears in the on-screen table and exposes its id for manage actions.
 *   F. The row's own "Edit" opens the editor pre-filled for that network.
 *   G. Saving the edit writes the new name through (read back from the API).
 *   H. The row's own "Delete" removes the row (the browser's confirm dialog is accepted) and
 *      the network disappears from the API and the table.
 *   I. A network that still has cities attached is REFUSED on screen (nothing orphaned) —
 *      run against the live ZaarHub network only when it really has directories.
 *   J. No console or auth errors during the walk.
 *
 * The token is the platform-operator JWT minted by the orchestrator and passed in $TOKEN.
 * Env: MD_BASE, TOKEN, AUD, PROBE_NAME, PROBE_SLUG, PROBE_ROOT, CHROME_PATH
 * Output: $AUD/networks-proof.json  (+ networks-table.png, networks-editor.png)
 */
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TOKEN = process.env.TOKEN;
const AUD = process.env.AUD;
const PROBE_NAME = process.env.PROBE_NAME || '';
const PROBE_SLUG = process.env.PROBE_SLUG || '';
const PROBE_ROOT = process.env.PROBE_ROOT || '';
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';

if (!TOKEN || !AUD || !PROBE_SLUG) {
  console.error('TOKEN, AUD and PROBE_SLUG are required');
  process.exit(3);
}
fs.mkdirSync(AUD, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const checks = [];
function rec(ok, name, detail) {
  checks.push({ ok: !!ok, name, detail: detail === undefined ? '' : String(detail).slice(0, 300) });
}

async function apiJson(method, p, body) {
  const headers = { Authorization: 'Bearer ' + TOKEN };
  if (body != null) headers['Content-Type'] = 'application/json';
  const r = await fetch(MD_BASE + '/api/v1' + p, {
    method,
    headers,
    body: body != null ? JSON.stringify(body) : undefined,
  });
  const txt = await r.text();
  let d = null;
  try { d = txt ? JSON.parse(txt) : null; } catch (e) { /* non-JSON */ }
  return { status: r.status, data: d };
}

let page;

async function openEditor() {
  await page.evaluate(() => document.getElementById('nw-new').click());
  await sleep(300);
}

(async () => {
  const consoleErrors = [];   // page faults — JS exceptions / unexpected console errors
  const refused4xx = [];      // 4xx the page was ASKED to produce by the negative probes
  const unexpected4xx = [];   // 4xx nobody asked for — these are real defects
  const authErrors = [];
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  page = await browser.newPage();
  page.on('console', (m) => {
    if (m.type() !== 'error') return;
    const t = m.text();
    // the browser also logs a console error for every non-2xx fetch; a REFUSED save
    // (our own junk-value probe) is the expected answer, not a page fault.
    if (/^Failed to load resource: the server responded with a status of 4\d\d/.test(t)) return;
    consoleErrors.push(t);
  });
  page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));
  page.on('response', (r) => {
    const st = r.status();
    const url = r.url();
    if (st === 401 || st === 403) authErrors.push(st + ' ' + url);
    else if (st >= 400 && st < 500) refused4xx.push(st + ' ' + url.split('?')[0]);
  });
  // the Delete control asks for confirmation — accept it, or the click does nothing
  page.on('dialog', (d) => { d.accept().catch(() => {}); });

  const before = await apiJson('GET', '/networks');
  const beforeList = Array.isArray(before.data) ? before.data : [];

  try {
    await page.evaluateOnNewDocument((t) => {
      localStorage.setItem('md_admin_token', t);
      localStorage.setItem('md_admin_user', JSON.stringify({ email: 'swiftsoftware143@yahoo.com', role: 'super_admin' }));
    }, TOKEN);
    await page.goto(MD_BASE + '/admin-panel', { waitUntil: 'networkidle2', timeout: 30000 });
    await sleep(2500);

    /* ── A. the card exists and shows the live networks ────────────────────── */
    const nav = await page.evaluate(() => !!document.querySelector('a[href="#networks-section"]'));
    rec(nav, 'networks_nav_link_present', nav);
    const sec = await page.evaluate(() => !!document.getElementById('networks-section'));
    rec(sec, 'networks_section_present', sec);
    const ctl = await page.evaluate(() => !!(document.getElementById('nw-new') && document.getElementById('nw-save')));
    rec(ctl, 'new_and_save_controls_present', ctl);

    await page.waitForFunction(() => {
      const el = document.getElementById('nw-rows');
      return el && el.querySelector('tbody tr');
    }, { timeout: 20000 }).catch(() => {});
    const listed = await page.evaluate(() => Array.from(document.querySelectorAll('#nw-rows tbody tr')).map((r) => r.innerText));
    rec(listed.length >= beforeList.length, 'networks_table_shows_live_rows',
      'rows=' + listed.length + ' api=' + beforeList.length);
    rec(listed.some((t) => /ZaarHub/i.test(t)), 'live_zaarhub_network_listed',
      (listed.find((t) => /ZaarHub/i.test(t)) || '').slice(0, 80));

    /* ── B/C. New network editor + a junk root domain refused on screen ─────── */
    await openEditor();
    const opened = await page.evaluate(() => {
      const e = document.getElementById('nw-editor');
      const t = (document.getElementById('nw-editor-title') || {}).textContent || '';
      return { vis: !!e && e.style.display !== 'none', title: t };
    });
    rec(opened.vis && /new network/i.test(opened.title), 'new_network_opens_editor',
      opened.vis + ' | ' + opened.title);

    await page.evaluate((n, s) => {
      document.getElementById('nw-name').value = n;
      document.getElementById('nw-slug').value = s;
      document.getElementById('nw-root').value = 'not a domain';
    }, PROBE_NAME, PROBE_SLUG);
    await page.evaluate(() => document.getElementById('nw-save').click());
    await sleep(2500);
    const junkMsg = await page.evaluate(() => (document.getElementById('nw-editor-status') || {}).textContent || '');
    rec(/failed/i.test(junkMsg) && /bare domain/i.test(junkMsg), 'junk_root_domain_refused_on_screen', junkMsg);
    const afterJunk = await apiJson('GET', '/networks');
    rec(!(afterJunk.data || []).some((n) => n.slug === PROBE_SLUG),
      'refused_save_created_no_row', 'slug absent from API');

    /* ── D/E. a valid save creates a real row and shows it ─────────────────── */
    await page.evaluate((r) => { document.getElementById('nw-root').value = r; }, PROBE_ROOT);
    await page.evaluate(() => document.getElementById('nw-save').click());
    await sleep(3000);
    const created = await apiJson('GET', '/networks');
    const mine = (created.data || []).find((n) => n.slug === PROBE_SLUG);
    rec(!!mine, 'ui_save_created_a_real_network_row', mine && (mine.id + ' ' + mine.root_domain));
    rec(!!mine && mine.root_domain === PROBE_ROOT, 'created_root_domain_stored',
      mine && mine.root_domain);

    const inTable = await page.evaluate((s) => {
      const rows = Array.from(document.querySelectorAll('#nw-rows tbody tr'));
      const row = rows.find((r) => r.innerText.indexOf(s) !== -1);
      if (!row) return null;
      const b = row.querySelector('[data-nw-edit]');
      return { found: true, id: b ? b.getAttribute('data-nw-edit') : '' };
    }, PROBE_SLUG);
    rec(inTable && inTable.found, 'new_network_appears_in_on_screen_table', inTable && inTable.id);
    rec(inTable && inTable.id && mine && inTable.id === mine.id,
      'created_row_exposes_its_id_for_manage_actions', inTable && inTable.id);

    /* ── F/G. the row's own Edit opens pre-filled and its Save writes through ─ */
    if (inTable && inTable.id) {
      await page.evaluate((id) => {
        const b = document.querySelector('#nw-rows [data-nw-edit="' + id + '"]');
        if (b) b.click();
      }, inTable.id);
      await sleep(500);
      const pre = await page.evaluate(() => ({
        title: (document.getElementById('nw-editor-title') || {}).textContent || '',
        name: document.getElementById('nw-name').value,
        root: document.getElementById('nw-root').value,
      }));
      rec(/^Edit —/.test(pre.title) && pre.name === PROBE_NAME && pre.root === PROBE_ROOT,
        'edit_opens_prefilled_for_the_network', JSON.stringify(pre));

      const newName = PROBE_NAME + ' renamed';
      await page.evaluate((n) => { document.getElementById('nw-name').value = n; }, newName);
      await page.evaluate(() => document.getElementById('nw-save').click());
      await sleep(2500);
      const after = await apiJson('GET', '/networks');
      const now = (after.data || []).find((n) => n.slug === PROBE_SLUG);
      rec(!!now && now.name === newName, 'editing_the_network_saved_the_new_name', now && now.name);
      const renamedOnScreen = await page.evaluate((s) => {
        const rows = Array.from(document.querySelectorAll('#nw-rows tbody tr'));
        return rows.some((r) => r.innerText.indexOf(s) !== -1);
      }, newName);
      rec(renamedOnScreen, 'renamed_network_shows_in_the_table', newName);
    }

    await page.evaluate(() => document.getElementById('networks-section').scrollIntoView());
    await page.screenshot({ path: path.join(AUD, 'networks-table.png') });

    /* ── I. a network with cities attached is refused (nothing orphaned) ────── */
    let liveId = '';
    for (const n of (created.data || [])) {
      const dirs = await apiJson('GET', '/networks/' + n.id + '/directories');
      if (Array.isArray(dirs.data) && dirs.data.length > 0) { liveId = n.id; break; }
    }
    if (liveId) {
      await page.evaluate((id) => {
        const b = document.querySelector('#nw-rows [data-nw-del="' + id + '"]');
        if (b) b.click();
      }, liveId);
      await sleep(2500);
      const refused = await page.evaluate(() => (document.getElementById('nw-status') || {}).textContent || '');
      rec(/refused/i.test(refused) && /attached/i.test(refused),
        'network_with_cities_cannot_be_deleted', refused);
      const still = await apiJson('GET', '/networks');
      rec((still.data || []).some((n) => n.id === liveId),
        'the city-bearing network survived the refused delete', liveId.slice(0, 8));
    } else {
      rec(false, 'no city-bearing network was available for the refusal check', 'none found');
    }

    /* ── H. the row's own Delete removes the probe network ─────────────────── */
    await page.evaluate((id) => {
      const b = document.querySelector('#nw-rows [data-nw-del="' + id + '"]');
      if (b) b.click();
    }, (inTable && inTable.id) || '');
    await sleep(3000);
    const afterDel = await apiJson('GET', '/networks');
    rec(!(afterDel.data || []).some((n) => n.slug === PROBE_SLUG),
      'delete_removed_the_network_from_the_api', 'slug absent');
    const goneOnScreen = await page.evaluate((s) => {
      const rows = Array.from(document.querySelectorAll('#nw-rows tbody tr'));
      return !rows.some((r) => r.innerText.indexOf(s) !== -1);
    }, PROBE_SLUG);
    rec(goneOnScreen, 'deleted_network_left_the_table', 'absent');

    // every 4xx must be a refusal we provoked (the networks endpoint, our own probe values);
    // anything else is an unexplained failed call on the page.
    for (const r of refused4xx) {
      if (!r.startsWith('422 ') || r.indexOf('/api/v1/networks') === -1) unexpected4xx.push(r);
    }
    rec(consoleErrors.length === 0, 'no_page_errors_or_js_exceptions', consoleErrors.slice(0, 3).join(' | '));
    rec(authErrors.length === 0, 'no_auth_errors', authErrors.slice(0, 3).join(' | '));
    rec(unexpected4xx.length === 0, 'every_failed_call_was_an_intended_refusal',
      'intended=' + refused4xx.length + ' unexpected=' + (unexpected4xx.slice(0, 3).join(' | ') || 'none'));
  } catch (e) {
    rec(false, 'exception', e && e.message);
  } finally {
    await browser.close();
  }

  const pass = checks.filter((c) => c.ok).length;
  const fail = checks.length - pass;
  fs.writeFileSync(path.join(AUD, 'networks-proof.json'),
    JSON.stringify({ checks, pass, fail, probe_slug: PROBE_SLUG }, null, 2));
  for (const c of checks) console.log((c.ok ? 'PASS  ' : 'FAIL  ') + c.name + (c.detail ? '  [' + c.detail + ']' : ''));
  console.log('BROWSER RESULT: ' + pass + ' passed, ' + fail + ' failed');
  process.exit(fail ? 2 : 0);
})();
