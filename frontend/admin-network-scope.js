/* Multi-Directory — NETWORK-OWNED CONTROL BLOCK-OUT (kanban B97).
 *
 * David (2026-10-01): "since each directory is still going to have certain features just make sure
 * when it toggles it blocks it out and if an admin tried to go to each individual directory to use
 * those features that it gives an instruction to go to the main directory admin."
 *
 * The model (verified): the NETWORK is the management surface, not one of its cities. Settings that
 * belong to the network — legal pages, provider keys, the loyalty programme, mail settings and the
 * network's payment/domain configuration — must not be offered as if they were local to a city.
 *
 * When the shared context bar (admin-context-bar.js) has a CITY of a network selected, every
 * network-owned card gets a plain-English block-out notice with a link that actually goes to the
 * network admin (it drives MDContextBar.setScope). Greyed-out alone is not enough: the notice says
 * WHERE to go and WHY. A standalone directory (no network) owns all of its settings locally and is
 * never blocked.
 *
 * Loaded after admin-context-bar.js on every console that renders the cards. Front-end only, served
 * from the bind-mounted frontend/ dir, so it is live on save with no rebuild.
 */
(function () {
  'use strict';
  if (window.MDNetworkScope) return;

  // Cards whose control belongs to the NETWORK. Keyed by the section id used in admin-panel.html.
  var NETWORK_OWNED = [
    { id: 'provider-keys-section', label: 'Provider API keys' },
    { id: 'email-settings-section', label: 'Mail / SMTP settings' },
    { id: 'email-templates-section', label: 'System email templates' },
    { id: 'loyalty-programme-section', label: 'The loyalty programme' },
    { id: 'payment-gateways-section', label: 'Payment gateways' },
    { id: 'domains-section', label: 'Domains, subdomains and subfolders' }
  ];

  var CSS = '.md-net-block{margin:0 0 14px;padding:12px 14px;border:1px solid #b45309;border-left:4px solid #f59e0b;border-radius:8px;background:rgba(245,158,11,.10);line-height:1.5}' +
    '.md-net-block-t{font-weight:700;color:#fbbf24;margin-bottom:4px}' +
    '.md-net-block p{margin:0 0 10px;font-size:.83rem;color:var(--text-secondary,#94a3b8)}' +
    '.md-net-block-btn{cursor:pointer;border:1px solid #f59e0b;background:rgba(245,158,11,.15);color:#fbbf24;border-radius:6px;padding:6px 12px;font-size:.83rem;font-weight:600}' +
    '.md-net-block-btn:hover{background:rgba(245,158,11,.28)}';

  function injectCss() {
    if (document.getElementById('md-net-scope-css')) return;
    var st = document.createElement('style');
    st.id = 'md-net-scope-css';
    st.textContent = CSS;
    (document.head || document.documentElement).appendChild(st);
  }

  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
    });
  }

  // Which network (if any) does the CURRENT scope belong to, and is it a city of that network?
  function blockedNetwork() {
    var cb = window.MDContextBar;
    if (!cb || !cb.scope) return null;
    var s = cb.scope();
    if (s.indexOf('dir:') !== 0) return null;
    var id = s.slice(4);
    var dirs = (cb.state && cb.state.dirs) || [];
    var d = null;
    for (var i = 0; i < dirs.length; i++) { if (dirs[i].id === id) { d = dirs[i]; break; } }
    if (!d || !d.network_id) return null;
    return { id: d.network_id, name: (cb.networkName ? cb.networkName(d.network_id) : ('network ' + String(d.network_id).slice(0, 8))) };
  }

  function lock(card, panel) {
    card.classList.add('md-net-locked');
    card.insertBefore(panel, card.firstChild);
    Array.prototype.forEach.call(card.children, function (ch) {
      if (ch === panel) return;
      ch.setAttribute('inert', '');
      ch.style.opacity = '.35';
    });
  }

  function unlock(card) {
    card.classList.remove('md-net-locked');
    Array.prototype.forEach.call(card.children, function (ch) {
      if (ch.classList && ch.classList.contains('md-net-block')) { if (ch.parentNode) ch.parentNode.removeChild(ch); return; }
      ch.removeAttribute('inert');
      ch.style.opacity = '';
    });
  }

  function buildPanel(net, label) {
    var el = document.createElement('div');
    el.className = 'md-net-block';
    el.setAttribute('role', 'note');
    el.innerHTML =
      '<div class="md-net-block-t">Managed for the whole ' + esc(net.name) + ' network</div>' +
      '<p>' + esc(label) + ' is a network-owned setting \u2014 it lives with the network so every ' +
      'city stays consistent, so it cannot be changed from here. ' +
      'Open the network admin to change it.</p>' +
      '<button type="button" class="md-net-block-btn">Open the ' + esc(net.name) + ' admin \u2192</button>';
    el.querySelector('.md-net-block-btn').addEventListener('click', function () {
      if (window.MDContextBar && window.MDContextBar.setScope) window.MDContextBar.setScope('net:' + net.id);
    });
    return el;
  }

  function apply() {
    var net = blockedNetwork();
    NETWORK_OWNED.forEach(function (item) {
      var card = document.getElementById(item.id);
      if (!card) return;
      var existing = card.querySelector(':scope > .md-net-block');
      if (!net) { if (existing) unlock(card); return; }
      if (existing) return; // already shown for this scope
      lock(card, buildPanel(net, item.label));
    });
  }

  function init() {
    injectCss();
    document.addEventListener('md-context-change', function () { setTimeout(apply, 0); });
    // The context bar fills its directory list asynchronously; re-apply once it lands.
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', function () { setTimeout(apply, 250); });
    } else {
      setTimeout(apply, 250);
    }
    window.addEventListener('load', function () { setTimeout(apply, 400); });
  }

  window.MDNetworkScope = { apply: apply, sections: NETWORK_OWNED };
  init();
})();
