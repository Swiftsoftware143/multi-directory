/* ZaarHub visitor tracking beacon.
 *
 * Writes real telemetry through the public tracking API:
 *   POST /api/v1/visitors/track      -> visitor row (fingerprint) + visitor_sessions row
 *   POST /api/v1/visitors/page-view  -> pages_viewed / exit_page on the session
 *   POST /api/v1/visitors/event      -> visitor_events rows (page_view, click, phone_click, website_click)
 *
 * Defensive by design: any failure is swallowed so the host page is never affected.
 * Session is cached in localStorage for 30 minutes so a visit is one session, not one per page.
 */
(function () {
  if (window.__zhBeacon) return;
  window.__zhBeacon = true;

  var API = '/api/v1';
  var KEY = '_zh_beacon';
  var SESSION_MINUTES = 30;
  var MAX_EVENTS_PER_PAGE = 25;
  var eventsSent = 0;

  function read() {
    try {
      var raw = localStorage.getItem(KEY);
      return raw ? JSON.parse(raw) : null;
    } catch (e) {
      return null;
    }
  }

  function write(obj) {
    try {
      localStorage.setItem(KEY, JSON.stringify(obj));
    } catch (e) {}
  }

  // Stable-ish browser fingerprint (no cookies, no external calls).
  function fingerprint() {
    var raw = [
      navigator.userAgent || '',
      screen.width + 'x' + screen.height + 'x' + (screen.colorDepth || 0),
      navigator.language || '',
      (window.Intl && Intl.DateTimeFormat ? Intl.DateTimeFormat().resolvedOptions().timeZone : '') || ''
    ].join('|');
    var h = 5381;
    for (var i = 0; i < raw.length; i++) {
      h = ((h << 5) + h + raw.charCodeAt(i)) & 0xffffffff;
    }
    return 'zh' + Math.abs(h).toString(36) + '-' + raw.length.toString(36);
  }

  function send(path, body) {
    try {
      return fetch(API + path, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
        keepalive: true
      }).catch(function () {});
    } catch (e) {
      return null;
    }
  }

  function directoryId() {
    // Pages may expose the directory; otherwise the server resolves by slug/host.
    if (window.__dir_id) return window.__dir_id;
    var el = document.querySelector('[data-directory-id]');
    return (el && el.getAttribute('data-directory-id')) || null;
  }

  function event(type, value, extra) {
    var st = read();
    if (!st || !st.s) return;
    if (eventsSent >= MAX_EVENTS_PER_PAGE) return;
    eventsSent++;
    var payload = {
      session_id: st.s,
      directory_id: st.dir || null,
      event_type: type,
      event_value: value || null,
      page_url: location.href
    };
    if (extra) {
      for (var k in extra) {
        if (Object.prototype.hasOwnProperty.call(extra, k)) payload[k] = extra[k];
      }
    }
    send('/visitors/event', payload);
  }

  function startSession() {
    var params = new URLSearchParams(location.search);
    var body = {
      fingerprint: fingerprint(),
      directory_id: directoryId(),
      language: navigator.language || null,
      screen_resolution: screen.width + 'x' + screen.height,
      timezone: window.Intl && Intl.DateTimeFormat ? Intl.DateTimeFormat().resolvedOptions().timeZone : null,
      referrer: document.referrer || null,
      page_url: location.href,
      utm_source: params.get('utm_source'),
      utm_medium: params.get('utm_medium'),
      utm_campaign: params.get('utm_campaign'),
      utm_term: params.get('utm_term'),
      utm_content: params.get('utm_content')
    };

    return fetch(API + '/visitors/track', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body)
    })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (d) {
        if (!d || !d.session_id) return;
        write({ s: d.session_id, f: body.fingerprint, t: Date.now(), dir: body.directory_id });
        event('page_view', document.title || location.pathname, { scroll_depth: 0 });
      })
      .catch(function () {});
  }

  function pageView() {
    var st = read();
    var fresh =
      st &&
      st.s &&
      st.f === fingerprint() &&
      Date.now() - (st.t || 0) < SESSION_MINUTES * 60 * 1000;

    if (!fresh) {
      startSession();
      return;
    }
    st.t = Date.now();
    write(st);
    send('/visitors/page-view', {
      session_id: st.s,
      page_url: location.href,
      referrer: document.referrer || null
    });
    event('page_view', document.title || location.pathname, { scroll_depth: 0 });
  }

  function labelFor(el) {
    if (!el || !el.tagName) return 'unknown';
    var explicit = el.getAttribute && (el.getAttribute('data-track') || el.getAttribute('aria-label'));
    var text = (el.textContent || '').replace(/\s+/g, ' ').trim();
    return (explicit || text || el.tagName.toLowerCase()).slice(0, 80);
  }

  function onClick(ev) {
    try {
      var el = ev.target;
      if (el && el.closest) el = el.closest('a,button,[data-track]') || ev.target;
      var href = (el && el.getAttribute && el.getAttribute('href')) || '';
      if (href.indexOf('tel:') === 0) {
        event('phone_click', href);
      } else if (/^https?:\/\//.test(href) && href.indexOf(location.host) === -1) {
        event('website_click', href);
      } else {
        event('click', labelFor(el));
      }
    } catch (e) {}
  }

  function boot() {
    pageView();
    document.addEventListener('click', onClick, true);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
  } else {
    boot();
  }
})();
