/**
 * ZaarHub admin console — DIRECTORIES screen, real end-to-end browser arm
 * (kanban t_26639554: "operator-only Directories card — create, edit, delete from the panel").
 *
 * What it proves, on the REAL served console (no stubs, no mocks) at /admin-panel:
 *   A. The Directories card, its nav anchor and its controls exist, and the table renders the
 *      directories the live API holds (count + the live ZaarHub cities).
 *   B. "＋ New directory" opens the editor titled "New directory", offering the placement choice
 *      (inside a network / standalone) with the live networks in the picker.
 *   C. A save with no name is refused before any request, and a DUPLICATE slug is refused ON
 *      SCREEN with the server's own words — neither writes a row.
 *   D. Pressing the card's own "Save directory" with valid values creates a REAL directory row
 *      (read back from the API here, and from the DATABASE by the orchestrator between phases).
 *   E. The row's own "Edit" opens pre-filled for that directory, shows where it lives instead of
 *      offering a placement control that would silently re-URL the city, and its Save writes the
 *      new name + city through (read back from the API, and from the DB by the orchestrator).
 *   F. (PHASE=delete) the row's own "Delete" removes the directory — the browser's confirm dialog
 *      is accepted — and it disappears from both the table and the API.
 *   G. No console or auth errors during the walk, and every 4xx the page saw was one this walk
 *      deliberately provoked.
 *
 * Two phases on purpose: the orchestrator reads the DATABASE between them, so "created" and
 * "edited" are proven against Postgres and "deleted" is proven against Postgres afterwards —
 * an HTTP 200 from the panel is never the evidence.
 *
 * Env: MD_BASE, TOKEN, AUD, PHASE (create|delete_refused|delete), PROBE_NAME, PROBE_SLUG, PROBE_RENAMED,
 *      NETWORK_ID, NETWORK_NAME, EXISTING_SLUG, CHROME_PATH, MD_HARNESS
 * Output: $AUD/directories-proof-<phase>.json (+ directories-<phase>.png)
 */
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const MD_BASE = process.env.MD_BASE || 'http://127.0.0.1:8089';
const TOKEN = process.env.TOKEN;
const AUD = process.env.AUD;
const PHASE = process.env.PHASE || 'create';
const PROBE_NAME = process.env.PROBE_NAME || '';
const PROBE_SLUG = process.env.PROBE_SLUG || '';
const PROBE_RENAMED = process.env.PROBE_RENAMED || '';
const NETWORK_ID = process.env.NETWORK_ID || '';
const NETWORK_NAME = process.env.NETWORK_NAME || '';
const EXISTING_SLUG = process.env.EXISTING_SLUG || '';
const CHROME = process.env.CHROME_PATH || '/usr/bin/chromium-browser';
const HARNESS = process.env.MD_HARNESS || 't_26639554';

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
  const headers = { Authorization: 'Bearer ' + TOKEN, 'X-Swift-Harness': HARNESS };
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

/** Every directory the API holds, as rows (the API wraps its list in {data, page, total…}). */
async function apiDirs() {
  const r = await apiJson('GET', '/directories?per_page=200');
  return Array.isArray(r.data && r.data.data) ? r.data.data : [];
}

(async () => {
  const consoleErrors = [];
  const refused4xx = [];
  const unexpected4xx = [];
  const authErrors = [];
  let browser;
  try {
    browser = await puppeteer.launch({
      executablePath: CHROME,
      headless: 'new',
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
    });
    const page = await browser.newPage();

    // the fleet probe marker rides every SAME-ORIGIN request the page makes (kanban t_8bea040a):
    // the console creates the directory with its own fetch, so the attribution has to survive a
    // human-equivalent click, not just this script's direct calls.
    await page.setRequestInterception(true);
    page.on('request', (req) => {
      if (!req.url().startsWith(MD_BASE)) { req.continue().catch(() => {}); return; }
      req.continue({ headers: Object.assign({}, req.headers(), { 'x-swift-harness': HARNESS }) })
        .catch(() => {});
    });
    page.on('console', (m) => {
      if (m.type() !== 'error') return;
      const t = m.text();
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

    await page.evaluateOnNewDocument((t) => {
      localStorage.setItem('md_admin_token', t);
      localStorage.setItem('md_admin_user', JSON.stringify({ email: 'swiftsoftware143@yahoo.com', role: 'super_admin' }));
    }, TOKEN);
    await page.goto(MD_BASE + '/admin-panel', { waitUntil: 'networkidle2', timeout: 30000 });
    await page.waitForFunction(() => {
      const el = document.getElementById('dr-rows');
      return el && el.querySelector('tbody tr');
    }, { timeout: 25000 }).catch(() => {});
    await sleep(1200);

    const dirsBefore = await apiDirs();

    if (PHASE === 'create') {
      /* ── A. card, nav and the live list ───────────────────────────────────── */
      rec(await page.evaluate(() => !!document.querySelector('a[href="#sec-directories"]')),
        'directories_nav_anchor_present', 'a[href="#sec-directories"]');
      rec(await page.evaluate(() => !!document.getElementById('directories-section')),
        'directories_section_present', 'directories-section');
      rec(await page.evaluate(() => !!(document.getElementById('dr-new') && document.getElementById('dr-save')
        && document.getElementById('dr-refresh'))),
        'new_save_and_refresh_controls_present', 'dr-new/dr-save/dr-refresh');

      const listed = await page.evaluate(() => Array.from(document.querySelectorAll('#dr-rows tbody tr')).map((r) => ({
        text: r.innerText,
        slug: r.getAttribute('data-dr-row'),
        id: r.getAttribute('data-dr-id'),
        where: (r.children[2] || {}).innerText || '',
      })));
      rec(listed.length === dirsBefore.length, 'table_lists_every_directory_the_api_holds',
        'rows=' + listed.length + ' api=' + dirsBefore.length);
      rec(listed.some((r) => r.slug === EXISTING_SLUG), 'live_directory_listed',
        (listed.find((r) => r.slug === EXISTING_SLUG) || {}).slug);
      const withNet = listed.find((r) => r.slug === EXISTING_SLUG);
      rec(!!withNet && withNet.where.indexOf(NETWORK_NAME) !== -1, 'listed_row_shows_where_it_lives',
        withNet && withNet.where);
      const summary = await page.evaluate(() => (document.getElementById('dr-summary') || {}).textContent || '');
      rec(/directories shown/.test(summary), 'summary_counts_the_list', summary);

      /* ── B. the New directory editor and its placement choice ─────────────── */
      await page.evaluate(() => document.getElementById('dr-new').click());
      await sleep(300);
      const opened = await page.evaluate(() => ({
        vis: (document.getElementById('dr-editor') || {}).style.display !== 'none',
        title: (document.getElementById('dr-editor-title') || {}).textContent || '',
        mode: (document.getElementById('dr-mode') || {}).value,
        nets: Array.from(document.querySelectorAll('#dr-network option')).map((o) => o.textContent),
        domainDisabled: !!(document.getElementById('dr-domain') || {}).disabled,
      }));
      rec(opened.vis && /^new directory$/i.test(opened.title.trim()), 'new_directory_opens_editor',
        opened.vis + ' | ' + opened.title);
      rec(opened.mode === 'connect', 'placement_defaults_to_a_city_inside_a_network', opened.mode);
      rec(opened.nets.some((t) => t.indexOf(NETWORK_NAME) !== -1), 'network_picker_lists_the_live_network',
        opened.nets.join(', ').slice(0, 120));
      rec(opened.domainDisabled === true, 'own_domain_box_is_inert_while_inside_a_network',
        'disabled=' + opened.domainDisabled);

      /* ── C. two refusals that must never write a row ──────────────────────── */
      await page.evaluate(() => document.getElementById('dr-save').click());
      await sleep(400);
      const blankMsg = await page.evaluate(() => (document.getElementById('dr-editor-status') || {}).textContent || '');
      rec(/name is required/i.test(blankMsg), 'a_directory_with_no_name_is_refused', blankMsg);

      await page.evaluate((n, s, nid) => {
        document.getElementById('dr-name').value = n;
        document.getElementById('dr-slug').value = s;
        // a network is required before the panel will send a city at all; pick the live one so the
        // request actually reaches the server and the SERVER's duplicate-slug refusal is what shows.
        document.getElementById('dr-network').value = nid;
      }, 'zz dup probe', EXISTING_SLUG, NETWORK_ID);
      await page.evaluate(() => document.getElementById('dr-save').click());
      await sleep(2500);
      const dupMsg = await page.evaluate(() => (document.getElementById('dr-editor-status') || {}).textContent || '');
      rec(/^failed:/i.test(dupMsg) && /exist/i.test(dupMsg), 'duplicate_slug_refused_on_screen', dupMsg);
      const afterDup = await apiDirs();
      rec(afterDup.length === dirsBefore.length, 'refused_saves_created_no_row',
        'api=' + afterDup.length + ' before=' + dirsBefore.length);

      /* ── D. a valid save creates a real directory ─────────────────────────── */
      await page.evaluate((n, s, nid) => {
        document.getElementById('dr-name').value = n;
        document.getElementById('dr-slug').value = s;
        document.getElementById('dr-network').value = nid;
        document.getElementById('dr-city').value = 'Melbourne';
        document.getElementById('dr-state').value = 'FL';
        document.getElementById('dr-support').value = 'zz-probe@zaarhub.com';
        document.getElementById('dr-desc').value = 'zz builds probe directory';
      }, PROBE_NAME, PROBE_SLUG, NETWORK_ID);
      await page.evaluate(() => document.getElementById('dr-save').click());
      await sleep(3500);
      const created = (await apiDirs()).find((d) => d.slug === PROBE_SLUG);
      rec(!!created, 'ui_save_created_a_real_directory_row', created && created.id);
      rec(!!created && created.network_id === NETWORK_ID, 'the_city_joined_the_chosen_network',
        created && created.network_id);
      rec(!!created && created.url_type === 'subfolder', 'the_city_url_is_a_subfolder_of_the_network',
        created && created.url_type);
      rec(!!created && created.status === 'draft', 'the_new_directory_kept_its_draft_status',
        created && created.status);

      const inTable = await page.evaluate((s) => {
        const row = document.querySelector('#dr-rows [data-dr-row="' + s + '"]');
        if (!row) return null;
        return {
          id: row.getAttribute('data-dr-id'),
          where: (row.children[2] || {}).innerText || '',
          edit: !!row.querySelector('[data-dr-edit]'),
          del: !!row.querySelector('[data-dr-del]'),
        };
      }, PROBE_SLUG);
      rec(!!inTable && !!inTable.id, 'created_directory_appears_in_the_table', inTable && inTable.id);
      rec(!!inTable && inTable.where.indexOf(NETWORK_NAME) !== -1,
        'created_row_shows_the_network_it_joined', inTable && inTable.where);
      rec(!!inTable && inTable.edit && inTable.del, 'created_row_offers_edit_and_delete',
        inTable && (inTable.edit + '/' + inTable.del));

      /* ── E. the row's own Edit opens pre-filled and saves through ─────────── */
      await page.evaluate((s) => {
        const b = document.querySelector('#dr-rows [data-dr-edit="' + s + '"]');
        if (b) b.click();
      }, PROBE_SLUG);
      await sleep(600);
      const pre = await page.evaluate(() => ({
        title: (document.getElementById('dr-editor-title') || {}).textContent || '',
        name: (document.getElementById('dr-name') || {}).value || '',
        city: (document.getElementById('dr-city') || {}).value || '',
        slugReadonly: (document.getElementById('dr-slug') || {}).getAttribute('readonly') != null,
        placeNoteShown: (document.getElementById('dr-place-note') || {}).style.display !== 'none',
        placeFieldsShown: (document.getElementById('dr-place-fields') || {}).style.display !== 'none',
        placeText: (document.getElementById('dr-place-text') || {}).textContent || '',
      }));
      rec(/^Edit —/i.test(pre.title) && pre.name === PROBE_NAME && pre.city === 'Melbourne',
        'edit_opens_prefilled_for_the_directory', JSON.stringify(pre).slice(0, 200));
      rec(pre.slugReadonly, 'the_slug_is_not_editable_from_the_edit_form', 'readonly');
      rec(pre.placeNoteShown && !pre.placeFieldsShown && pre.placeText.indexOf(NETWORK_NAME) !== -1,
        'edit_shows_where_it_lives_instead_of_a_moving_control',
        pre.placeText.slice(0, 120));

      await page.evaluate((n) => {
        document.getElementById('dr-name').value = n;
        document.getElementById('dr-city').value = 'Palm Bay';
      }, PROBE_RENAMED);
      await page.evaluate(() => document.getElementById('dr-save').click());
      await sleep(3000);
      const edited = (await apiDirs()).find((d) => d.slug === PROBE_SLUG);
      rec(!!edited && edited.name === PROBE_RENAMED, 'editing_saved_the_new_name', edited && edited.name);
      rec(!!edited && edited.city === 'Palm Bay', 'editing_saved_the_new_city', edited && edited.city);
      const renamedOnScreen = await page.evaluate((s, n) => {
        const row = document.querySelector('#dr-rows [data-dr-row="' + s + '"]');
        return !!row && row.innerText.indexOf(n) !== -1;
      }, PROBE_SLUG, PROBE_RENAMED);
      rec(renamedOnScreen, 'renamed_directory_shows_in_the_table', PROBE_RENAMED);

      await page.evaluate(() => document.getElementById('directories-section').scrollIntoView());
      await page.screenshot({ path: path.join(AUD, 'directories-create.png') });
    } else {
      /* ── F/G. the row's own Delete: refused while a customer record holds it,
         then succeeds once that record is gone (the refusal is a door, not a wall) ────── */
      const expectRefused = PHASE === 'delete_refused';
      await page.waitForFunction((s) => !!document.querySelector('#dr-rows [data-dr-row="' + s + '"]'),
        { timeout: 20000 }, PROBE_SLUG).catch(() => {});
      const present = await page.evaluate((s) => !!document.querySelector('#dr-rows [data-dr-row="' + s + '"]'), PROBE_SLUG);
      rec(present, 'the_probe_directory_is_listed_before_delete', PROBE_SLUG);

      await page.evaluate((s) => {
        const b = document.querySelector('#dr-rows [data-dr-del="' + s + '"]');
        if (b) b.click();
      }, PROBE_SLUG);
      await sleep(4000);
      const delMsg = await page.evaluate(() => (document.getElementById('dr-status') || {}).textContent || '');

      if (expectRefused) {
        rec(/^Refused:/i.test(delMsg) && /CRM contact/i.test(delMsg) && /nothing was deleted/i.test(delMsg),
          'a_directory_holding_a_crm_contact_is_refused_on_screen', delMsg);
        const stillApi = (await apiDirs()).some((d) => d.slug === PROBE_SLUG);
        rec(stillApi, 'the_refused_delete_left_the_directory_in_the_api', 'present');
        const stillRow = await page.evaluate((s) => !!document.querySelector('#dr-rows [data-dr-row="' + s + '"]'), PROBE_SLUG);
        rec(stillRow, 'the_refused_delete_left_the_row_in_the_table', 'present');
        await page.evaluate(() => document.getElementById('directories-section').scrollIntoView());
        await page.screenshot({ path: path.join(AUD, 'directories-refused.png') });
      } else {
        rec(/^Deleted /i.test(delMsg) && delMsg.indexOf(PROBE_SLUG) !== -1,
          'delete_reported_success_on_screen', delMsg);
        rec(/Cleared \d+ ad zone/i.test(delMsg), 'delete_reports_what_it_cleared', delMsg);
        const afterDel = await apiDirs();
        rec(!afterDel.some((d) => d.slug === PROBE_SLUG), 'delete_removed_the_directory_from_the_api',
          'slug absent');
        const goneOnScreen = await page.evaluate((s) => !document.querySelector('#dr-rows [data-dr-row="' + s + '"]'), PROBE_SLUG);
        rec(goneOnScreen, 'deleted_directory_left_the_table', 'absent');
        await page.evaluate(() => document.getElementById('directories-section').scrollIntoView());
        await page.screenshot({ path: path.join(AUD, 'directories-delete.png') });
      }
    }

    // the only 4xx the page may have seen are the two refusals this walk PROVOKED on the
    // directories endpoint: the duplicate-slug create and the delete blocked by a CRM row.
    for (const r of refused4xx) {
      if (!(r.startsWith('409 ') && r.indexOf('/api/v1/directories') !== -1)) unexpected4xx.push(r);
    }
    rec(consoleErrors.length === 0, 'no_page_errors_or_js_exceptions', consoleErrors.slice(0, 3).join(' | '));
    rec(authErrors.length === 0, 'no_auth_errors', authErrors.slice(0, 3).join(' | '));
    rec(unexpected4xx.length === 0, 'every_failed_call_was_an_intended_refusal',
      'intended=' + refused4xx.length + ' unexpected=' + (unexpected4xx.slice(0, 3).join(' | ') || 'none'));
  } catch (e) {
    rec(false, 'exception', e && e.message);
  } finally {
    if (browser) await browser.close();
  }

  const pass = checks.filter((c) => c.ok).length;
  const fail = checks.length - pass;
  fs.writeFileSync(path.join(AUD, 'directories-proof-' + PHASE + '.json'),
    JSON.stringify({ phase: PHASE, checks, pass, fail, probe_slug: PROBE_SLUG }, null, 2));
  for (const c of checks) console.log((c.ok ? 'PASS  ' : 'FAIL  ') + c.name + (c.detail ? '  [' + c.detail + ']' : ''));
  console.log('BROWSER RESULT (' + PHASE + '): ' + pass + ' passed, ' + fail + ' failed');
  process.exit(fail ? 2 : 0);
})();
