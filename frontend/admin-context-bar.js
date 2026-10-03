/* Multi-Directory — persistent admin CONTEXT BAR (kanban B99).
 *
 * One implementation, every admin surface. Seven admin pages existed and none of them said which
 * network or directory you were managing — the only thing identifying a brand was creation order.
 * This bar sits under each page's sticky header, names the scope from LIVE data
 * ("Managing: ZaarHub network — 10 cities"), and switches between networks (with their cities
 * nested beneath them) and standalone directories. The choice persists in localStorage.
 *
 * Usage (an admin page that already reads localStorage.md_admin_token):
 *   <script src="/admin-context-bar.js"></script>
 *   ... MDContextBar.init({ mount: '#dashboard-page', after: '.header' });   // optional
 *
 * Pages that own a directory-scoped list subscribe to the change event and use dirsForScope():
 *   document.addEventListener('md-context-change', function (e) { render(MDContextBar.dirsForScope(e.detail.scope)); });
 *
 * /networks is platform-operator only (403 for every other caller) so a network's NAME may be
 * unavailable; the group is then synthesised from the directories' own network_id rather than
 * hiding those cities, and the label falls back to "network <id8>".
 */
(function () {
  'use strict';
  if (window.MDContextBar) return;

  var KEY = 'md_admin_ctx';
  var S = { dirs: [], networks: [], scope: null, userChosen: false, loaded: false, opts: {} };

  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
    });
  }

  function token() {
    try { return localStorage.getItem('md_admin_token') || null; } catch (e) { return null; }
  }

  function api(path) {
    var h = { 'Content-Type': 'application/json' };
    var t = token();
    if (t) h['Authorization'] = 'Bearer ' + t;
    return fetch('/api/v1' + path, { headers: h }).then(function (r) {
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.json();
    });
  }

  // ---- data ------------------------------------------------------------------
  function nets() {
    var seen = {}, out = [];
    S.networks.forEach(function (n) {
      if (!n || !n.id || seen[n.id]) return;
      seen[n.id] = true;
      out.push({ id: n.id, name: n.name || n.slug || null });
    });
    S.dirs.forEach(function (d) {
      if (d.network_id && !seen[d.network_id]) {
        seen[d.network_id] = true;
        out.push({ id: d.network_id, name: null });
      }
    });
    return out;
  }
  function net(id) { return nets().filter(function (n) { return n.id === id; })[0] || null; }
  function named(id) { var n = net(id); return !!(n && n.name); }
  function nm(id) { var n = net(id); return (n && n.name) ? n.name : ('network ' + String(id).slice(0, 8)); }
  function dirsOf(netId) { return S.dirs.filter(function (d) { return d.network_id === netId; }); }

  function exists(scope) {
    if (!scope || scope === 'all:') return true;
    if (scope.indexOf('net:') === 0) {
      var nid = scope.slice(4);
      return S.networks.some(function (n) { return n.id === nid; }) ||
             S.dirs.some(function (d) { return d.network_id === nid; });
    }
    if (scope.indexOf('dir:') === 0) {
      var did = scope.slice(4);
      return S.dirs.some(function (d) { return d.id === did; });
    }
    return false;
  }

  // The brand that actually owns directories is the safe default: landing on an empty network
  // would hide everything and look broken. Never leave the scope unresolved.
  function defaultScope() {
    var best = null, bestN = -1;
    nets().forEach(function (n) {
      var c = dirsOf(n.id).length;
      if (c > bestN) { bestN = c; best = n; }
    });
    if (best && bestN > 0) return 'net:' + best.id;
    var standalone = S.dirs.filter(function (d) { return !d.network_id; })[0];
    if (standalone) return 'dir:' + standalone.id;
    return 'all:';
  }

  function resolveScope() {
    if (S.userChosen && exists(S.scope)) return S.scope;
    var stored = null;
    try { stored = localStorage.getItem(KEY); } catch (e) {}
    if (stored && exists(stored)) { S.scope = stored; S.userChosen = true; return S.scope; }
    S.scope = defaultScope();
    return S.scope;
  }

  function dirsForScope(scope) {
    var s = scope || resolveScope();
    if (s.indexOf('net:') === 0) return dirsOf(s.slice(4));
    if (s.indexOf('dir:') === 0) return S.dirs.filter(function (d) { return d.id === s.slice(4); });
    return S.dirs;
  }

  // ---- rendering -------------------------------------------------------------
  function render() {
    var sel = document.getElementById('ctx-switch');
    if (!sel) return;
    if (!S.loaded && !S.dirs.length && !S.networks.length) return;
    resolveScope();

    var html = '<option value="all:">🌐 All networks and directories (platform view)</option>';
    nets().forEach(function (n) {
      var nn = nm(n.id), isNet = named(n.id), cities = dirsOf(n.id);
      html += '<optgroup label="' + esc(nn) + (isNet ? ' — network' : '') +
              (cities.length ? ' (' + cities.length + ' cities)' : ' (no cities yet)') + '">';
      html += '<option value="net:' + esc(n.id) + '">🌐 Manage the whole ' + esc(nn) +
              (isNet ? ' network' : '') + '</option>';
      cities.forEach(function (d) {
        html += '<option value="dir:' + esc(d.id) + '">\u00a0\u00a0↳ ' + esc(d.name || d.slug) + '</option>';
      });
      html += '</optgroup>';
    });
    var standalone = S.dirs.filter(function (d) { return !d.network_id; });
    if (standalone.length) {
      html += '<optgroup label="Standalone directories (no network)">';
      standalone.forEach(function (d) {
        html += '<option value="dir:' + esc(d.id) + '">' + esc(d.name || d.slug) + '</option>';
      });
      html += '</optgroup>';
    }
    sel.innerHTML = html;
    sel.value = S.scope;
    if (sel.value !== S.scope) { sel.value = 'all:'; S.scope = 'all:'; }
    banner();
  }

  function banner() {
    var lab = document.getElementById('ctx-label');
    var hint = document.getElementById('ctx-hint');
    if (!lab) return;
    var s = S.scope || 'all:';
    if (s.indexOf('net:') === 0) {
      var nid = s.slice(4), cities = dirsOf(nid);
      lab.innerHTML = 'Managing: <strong>' + esc(nm(nid)) + (named(nid) ? ' network' : '') +
                      '</strong> — ' + cities.length + ' cit' + (cities.length === 1 ? 'y' : 'ies');
      hint.textContent = 'Network-owned settings (legal pages, branding, provider keys, loyalty, mail) ' +
                         'belong to the whole network; a city owns only its local content.';
    } else if (s.indexOf('dir:') === 0) {
      var did = s.slice(4);
      var d = S.dirs.filter(function (x) { return x.id === did; })[0];
      var dn = d ? (d.name || d.slug) : 'this directory';
      var where = (d && d.network_id)
        ? ('a city in the ' + nm(d.network_id) + (named(d.network_id) ? ' network' : ''))
        : 'a standalone directory';
      lab.innerHTML = 'Managing: <strong>' + esc(dn) + '</strong> — ' + esc(where);
      hint.textContent = (d && d.network_id)
        ? 'This city inherits the network\u2019s legal pages, branding and keys — network-owned controls ' +
          'live with the network (switch to the network above to change them).'
        : 'A standalone directory owns all of its settings locally.';
    } else {
      lab.innerHTML = 'Managing: <strong>all networks and directories</strong>';
      hint.textContent = 'Platform view \u2014 ' + nets().length + ' network(s) \u00b7 ' + S.dirs.length +
                         ' directories. Pick a network or city above to scope this console to one brand.';
    }
  }

  // Carry the chosen city into directory-scoped pickers that already exist on the page.
  function syncSelects() {
    var s = resolveScope(), dirId = null;
    if (s.indexOf('dir:') === 0) dirId = s.slice(4);
    else if (s.indexOf('net:') === 0) {
      var c = dirsOf(s.slice(4));
      if (c.length === 1) dirId = c[0].id;
    }
    if (!dirId) return;
    if (window.__mdFillDirectorySelects) { try { window.__mdFillDirectorySelects(); } catch (e) {} }
    ['pk-effective-directory', 'dm-directory'].forEach(function (id) {
      var el = document.getElementById(id);
      if (!el) return;
      var has = Array.prototype.some.call(el.options || [], function (o) { return o.value === dirId; });
      if (has) el.value = dirId;
    });
  }

  function emit() {
    var s = resolveScope();
    var type = 'all', id = null;
    if (s.indexOf('net:') === 0) { type = 'network'; id = s.slice(4); }
    else if (s.indexOf('dir:') === 0) { type = 'directory'; id = s.slice(4); }
    try {
      document.dispatchEvent(new CustomEvent('md-context-change', { detail: { scope: s, type: type, id: id } }));
    } catch (e) {}
  }

  // ---- DOM -------------------------------------------------------------------
  var CSS = '.ctx-bar{position:sticky;top:0;z-index:90;background:linear-gradient(90deg,rgba(139,211,255,.12),rgba(139,211,255,.03));border-bottom:1px solid rgba(255,255,255,.08);backdrop-filter:blur(12px)}' +
    '.ctx-inner{max-width:1280px;margin:0 auto;padding:8px 24px;display:flex;align-items:center;gap:14px;flex-wrap:wrap;font-size:.82rem;color:#eef4f8}' +
    '.ctx-label{font-weight:600}.ctx-label strong{color:#8bd3ff}' +
    '.ctx-pick{display:flex;align-items:center;gap:6px;margin-left:auto}' +
    '.ctx-pick-cap{font-size:.68rem;text-transform:uppercase;letter-spacing:.06em;color:#6b7d8a}' +
    '.ctx-switch{background:#12121a;color:#eef4f8;border:1px solid rgba(255,255,255,.08);border-radius:6px;padding:5px 8px;font-size:.8rem;max-width:360px}' +
    '.ctx-hint{flex-basis:100%;color:#6b7d8a;font-size:.75rem;line-height:1.35}';

  function injectCss() {
    if (document.getElementById('ctx-bar-css')) return;
    var st = document.createElement('style');
    st.id = 'ctx-bar-css';
    st.textContent = CSS;
    (document.head || document.documentElement).appendChild(st);
  }

  function mount() {
    var existing = document.getElementById('ctx-bar');
    if (existing) return existing;
    var host = S.opts.mount ? document.querySelector(S.opts.mount) : document.body;
    if (!host) return null;
    var el = document.createElement('div');
    el.className = 'ctx-bar';
    el.id = 'ctx-bar';
    el.innerHTML =
      '<div class="ctx-inner">' +
        '<span class="ctx-label" id="ctx-label">Managing: <strong>…</strong></span>' +
        '<label class="ctx-pick"><span class="ctx-pick-cap">Switch to</span>' +
          '<select id="ctx-switch" class="ctx-switch" aria-label="Switch the network or directory you are managing">' +
            '<option value="all:">🌐 All networks and directories (platform view)</option>' +
          '</select></label>' +
        '<span class="ctx-hint" id="ctx-hint"></span>' +
      '</div>';
    var after = null;
    if (S.opts.after) after = document.querySelector(S.opts.after);
    if (!after) {
      // default: sit under the page's own header so a sticky header never covers the bar
      var h = host.querySelector ? host.querySelector('header, .header') : null;
      after = h;
    }
    if (after && after.parentNode) after.parentNode.insertBefore(el, after.nextSibling);
    else if (host === document.body) host.insertBefore(el, host.firstChild);
    else host.insertBefore(el, host.firstChild);

    var prev = el.previousElementSibling;
    if (prev) {
      var cs = window.getComputedStyle(prev);
      if ((cs.position === 'sticky' || cs.position === 'fixed') && prev.offsetHeight) {
        el.style.top = prev.offsetHeight + 'px';
      }
    }
    return el;
  }

  function onPick(sel) {
    S.scope = sel.value;
    S.userChosen = true;
    try { localStorage.setItem(KEY, S.scope); } catch (e) {}
    banner();
    syncSelects();
    emit();
    var anchor = document.getElementById('sec-directories');
    if (anchor && anchor.scrollIntoView) anchor.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  function fetchData() {
    var el = document.getElementById('ctx-bar');
    if (!token()) {
      // No admin session on this page (e.g. the sign-in gate is showing): hide the bar rather than
      // claim "Managing: all networks and directories" to someone who is not managing anything.
      if (el) el.style.display = 'none';
      return Promise.resolve();
    }
    if (el) el.style.display = '';
    return Promise.all([
      api('/networks').catch(function () { return []; }),
      api('/directories?per_page=100').catch(function () { return { data: [] }; })
    ]).then(function (res) {
      var netsArr = Array.isArray(res[0]) ? res[0] : ((res[0] && res[0].data) || []);
      var dirs = (res[1] && res[1].data) || res[1] || [];
      if (!Array.isArray(dirs)) dirs = [];
      if (!Array.isArray(netsArr)) netsArr = [];
      S.networks = netsArr;
      if (dirs.length) S.dirs = dirs;
      S.loaded = true;
      render();
      syncSelects();
      emit();
    }).catch(function () { S.loaded = true; render(); });
  }

  window.MDContextBar = {
    init: function (opts) {
      S.opts = opts || {};
      injectCss();
      mount();
      var sel = document.getElementById('ctx-switch');
      if (sel && !sel.__ctxBound) {
        sel.__ctxBound = true;
        sel.addEventListener('change', function () { onPick(sel); });
      }
      if (!token()) {
        var el = document.getElementById('ctx-bar');
        if (el) el.style.display = 'none';
        return Promise.resolve();
      }
      render();
      banner();
      return fetchData();
    },
    setDirectories: function (list) {
      if (Array.isArray(list) && list.length) { S.dirs = list; S.loaded = true; render(); }
      return S.dirs;
    },
    refresh: fetchData,
    dirsForScope: dirsForScope,
    scope: function () { return resolveScope(); },
    state: S
  };
})();
