/* ZaarHub onboarding survey widget — Multi-Directory's own onboarding (card B65).
 *
 * Serves the PUBLISHED questionnaire for the audience the page belongs to:
 *   /<city>                    → customer
 *   /portal, /portal/business  → business
 *   /visitor                   → customer
 *   /distributor...            → supplier
 *
 * Renders every answer type the admin builder can author:
 *   short_text, long_text, single_choice, multiple_choice, dropdown, number,
 *   yes_no, rating (scale_min..scale_max), date
 * (legacy types text / choice / select / multi are still understood).
 *
 * The completion reward shown on the form and the currency credited on submit come from the
 * questionnaire the admin published — nothing about the reward is hardcoded here.
 */
(function () {
  'use strict';

  // ── Which audience is this page? ──
  function detectAudience() {
    var path = window.location.pathname || '';
    if (path === '/portal' || path.indexOf('/portal/') === 0) return 'business';
    if (path.indexOf('/distributor') === 0) return 'supplier';
    return 'customer';
  }

  function detectSlug(audience) {
    var el = document.querySelector('[data-directory-slug]');
    if (el) {
      var v = el.getAttribute('data-directory-slug');
      if (v) {
        try { localStorage.setItem('md_active_slug', v); } catch (e) {}
        return v;
      }
    }
    var path = window.location.pathname || '';
    var parts = path.split('/').filter(Boolean);
    // A city landing page: /palm-bay — but not a portal, an app page or an asset path.
    var reserved = ['visitor', 'portal', 'distributor', 'zaarhub', 'z', 'user', 'admin',
      'scanner', 'login', 'grow', 'guide', 'claim', 'rfp', 'rfq', 'd', 'public'];
    if (audience === 'customer' && parts.length && reserved.indexOf(parts[0]) === -1 &&
        parts[0].indexOf('.') === -1) {
      return parts[0];
    }
    try { return localStorage.getItem('md_active_slug'); } catch (e) { return null; }
  }

  var audience = detectAudience();
  var slug = detectSlug(audience);
  // Business and supplier signups are network-wide (their account carries no city), so they are
  // served through the network endpoint, which resolves the published questionnaire itself.
  var useNetworkEndpoint = (audience !== 'customer') || !slug;

  // Public landing pages are the visitor audience; never nag the admin/scanner surfaces.
  var path0 = window.location.pathname || '';
  if (/^\/(admin|scanner|portal\/admin)/.test(path0)) return;

  // Before the directory is resolved the key is scoped by audience only ('network'), and it is
  // re-scoped to the resolved directory once the questionnaire is fetched.
  var keyScope = slug || 'network';
  var completeKey = 'survey_completed_' + keyScope + '_' + audience;
  var skipKey = 'survey_skipped_' + keyScope + '_' + audience;
  var activeCompleteKey = completeKey;
  var activeSkipKey = skipKey;
  try {
    if (localStorage.getItem(completeKey) || localStorage.getItem(skipKey)) return;
  } catch (e) { /* storage blocked: still show the survey */ }

  function visitorToken() {
    try { return localStorage.getItem('visitorToken') || ''; } catch (e) { return ''; }
  }

  function fetchSurvey() {
    var networkUrl = '/api/v1/public/onboarding?audience=' + encodeURIComponent(audience);
    var primary = useNetworkEndpoint
      ? networkUrl
      : '/api/v1/public/directories/' + encodeURIComponent(slug) +
        '/survey?audience=' + encodeURIComponent(audience);
    loadFrom(primary, !useNetworkEndpoint, networkUrl);
  }

  function loadFrom(url, allowNetworkFallback, networkUrl) {
    fetch(url, { headers: { Accept: 'application/json' } })
      .then(function (r) {
        if (r.ok) return r.json();
        // The hash-routed pages have no city in the path: fall back to the network resolution.
        return allowNetworkFallback ? loadFrom(networkUrl, false, networkUrl) : null;
      })
      .then(function (survey) {
        if (!survey || !survey.enabled) return;
        var questions = survey.questions || [];
        if (!questions.length) return;
        // The answers are posted to the directory the questionnaire belongs to.
        survey.__slug = survey.directory_slug || slug;
        if (!survey.__slug) return;
        activeCompleteKey = 'survey_completed_' + survey.__slug + '_' + audience;
        activeSkipKey = 'survey_skipped_' + survey.__slug + '_' + audience;
        try {
          if (localStorage.getItem(activeCompleteKey) || localStorage.getItem(activeSkipKey)) return;
        } catch (e) { /* storage blocked: still show the survey */ }
        // Legacy rows carried per-question `tags` to fake per-audience routing; keep honouring
        // them so an older questionnaire still shows on the page it always did.
        var filtered = questions.filter(function (q) {
          var tags = q.tags || [];
          return !tags.length || tags.indexOf(audience) !== -1 ||
            (audience === 'customer' && tags.indexOf('visitor') !== -1);
        });
        if (!filtered.length) return;
        survey.questions = filtered;
        setTimeout(function () { renderModal(survey); }, 2000);
      })
      .catch(function () { /* silent: onboarding must never break the page */ });
  }
  fetchSurvey();

  // ── Rendering ──
  function el(tag, style, text) {
    var n = document.createElement(tag);
    if (style) n.style.cssText = style;
    if (text != null) n.textContent = text;
    return n;
  }

  var CSS = {
    label: 'font-weight:600;font-size:0.9rem;color:#0f172a;margin-bottom:6px;',
    help: 'font-size:0.78rem;color:#64748b;margin:0 0 6px 0;',
    star: 'color:#dc2626;'
  };

  function renderModal(survey) {
    var existing = document.getElementById('md-survey-overlay');
    if (existing) existing.remove();

    var overlay = el('div', 'position:fixed;top:0;left:0;width:100%;height:100%;background:rgba(0,0,0,0.5);z-index:99999;display:flex;align-items:center;justify-content:center;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;');
    overlay.id = 'md-survey-overlay';

    var modal = el('div', 'background:#fff;border-radius:16px;padding:32px;max-width:560px;width:90%;max-height:80vh;overflow-y:auto;box-shadow:0 25px 50px -12px rgba(0,0,0,0.4);position:relative;');

    var closeBtn = el('button', 'position:absolute;top:12px;right:16px;background:none;border:none;font-size:24px;cursor:pointer;color:#94a3b8;line-height:1;', '\u00d7');
    closeBtn.setAttribute('aria-label', 'Skip');
    closeBtn.onclick = function () { skip(overlay); };
    modal.appendChild(closeBtn);

    var title = el('h2', 'font-size:1.4rem;font-weight:700;color:#0f172a;margin:0 0 8px 0;', survey.title || 'Help us personalize your experience');
    modal.appendChild(title);

    if (survey.reward_units > 0) {
      modal.appendChild(el('p', 'display:inline-block;font-size:0.8rem;font-weight:600;color:#0f766e;background:#ccfbf1;border-radius:999px;padding:4px 12px;margin:0 0 12px 0;',
        'Earn ' + survey.reward_units + ' bonus units when you finish'));
    }
    if (survey.description) {
      modal.appendChild(el('p', 'font-size:0.9rem;color:#64748b;margin:0 0 20px 0;', survey.description));
    }

    var answers = {};
    var formBody = el('div', 'margin-bottom:20px;');
    var errorBox = el('p', 'display:none;font-size:0.85rem;color:#b91c1c;margin:0 0 12px 0;');

    (survey.questions || []).forEach(function (q, i) {
      var qid = q.id || ('q_' + i);
      var qtype = (q.type || 'short_text').toLowerCase();
      if (qtype === 'text') qtype = 'short_text';
      if (qtype === 'choice') qtype = 'single_choice';
      if (qtype === 'select') qtype = 'dropdown';
      if (qtype === 'multi') qtype = 'multiple_choice';
      var options = q.options || [];
      var qTags = q.tags || [];

      var wrap = el('div', 'margin-bottom:16px;');
      var label = el('div', CSS.label, (q.label || q.question || 'Question ' + (i + 1)));
      if (q.required) {
        var star = el('span', CSS.star, ' *');
        label.appendChild(star);
      }
      wrap.appendChild(label);
      if (q.help_text) wrap.appendChild(el('p', CSS.help, q.help_text));

      function optionLabel(o) { return (o && typeof o === 'object') ? (o.label || o.value || '') : String(o); }
      function optionValue(o) { return (o && typeof o === 'object') ? (o.value || o.label || '') : String(o); }

      if (qtype === 'single_choice') {
        options.forEach(function (o) {
          var row = el('label', 'display:flex;align-items:center;gap:8px;padding:6px 0;cursor:pointer;font-size:0.9rem;color:#334155;');
          var input = document.createElement('input');
          input.type = 'radio';
          input.name = 'mdq_' + qid;
          input.value = optionValue(o);
          input.style.cssText = 'accent-color:#0d9488;';
          input.onchange = function () { answers[qid] = { value: optionValue(o), tags: qTags }; };
          row.appendChild(input);
          row.appendChild(document.createTextNode(optionLabel(o)));
          wrap.appendChild(row);
        });
      } else if (qtype === 'dropdown') {
        var select = el('select', 'width:100%;padding:10px 14px;border:1px solid #e2e8f0;border-radius:8px;font-size:0.9rem;font-family:inherit;background:#fff;outline:none;box-sizing:border-box;cursor:pointer;');
        var ph = document.createElement('option');
        ph.value = '';
        ph.textContent = 'Select an option...';
        ph.disabled = true;
        ph.selected = true;
        select.appendChild(ph);
        options.forEach(function (o) {
          var opt = document.createElement('option');
          opt.value = optionValue(o);
          opt.textContent = optionLabel(o);
          select.appendChild(opt);
        });
        select.onchange = function () { answers[qid] = { value: select.value, tags: qTags }; };
        wrap.appendChild(select);
      } else if (qtype === 'multiple_choice') {
        options.forEach(function (o) {
          var row = el('label', 'display:flex;align-items:center;gap:8px;padding:6px 0;cursor:pointer;font-size:0.9rem;color:#334155;');
          var cb = document.createElement('input');
          cb.type = 'checkbox';
          cb.value = optionValue(o);
          cb.style.cssText = 'accent-color:#0d9488;';
          cb.onchange = function () {
            if (!answers[qid]) answers[qid] = { value: [], tags: [] };
            if (this.checked) {
              answers[qid].value.push(optionValue(o));
            } else {
              var idx = answers[qid].value.indexOf(optionValue(o));
              if (idx > -1) answers[qid].value.splice(idx, 1);
            }
          };
          row.appendChild(cb);
          row.appendChild(document.createTextNode(optionLabel(o)));
          wrap.appendChild(row);
        });
      } else if (qtype === 'yes_no') {
        (options.length ? options : [{ value: 'yes', label: 'Yes' }, { value: 'no', label: 'No' }]).forEach(function (o) {
          var row = el('label', 'display:flex;align-items:center;gap:8px;padding:6px 0;cursor:pointer;font-size:0.9rem;color:#334155;');
          var input = document.createElement('input');
          input.type = 'radio';
          input.name = 'mdq_' + qid;
          input.value = optionValue(o);
          input.style.cssText = 'accent-color:#0d9488;';
          input.onchange = function () { answers[qid] = { value: optionValue(o), tags: qTags }; };
          row.appendChild(input);
          row.appendChild(document.createTextNode(optionLabel(o)));
          wrap.appendChild(row);
        });
      } else if (qtype === 'rating') {
        var min = Number(q.scale_min || 1);
        var max = Number(q.scale_max || 5);
        var scaleRow = el('div', 'display:flex;gap:6px;flex-wrap:wrap;');
        for (var s = min; s <= max; s++) {
          (function (score) {
            var b = el('button', 'min-width:38px;padding:8px 10px;border:1px solid #e2e8f0;border-radius:8px;background:#fff;color:#334155;font-size:0.9rem;font-weight:600;cursor:pointer;', String(score));
            b.type = 'button';
            b.onclick = function () {
              Array.prototype.forEach.call(scaleRow.children, function (c) {
                c.style.background = '#fff';
                c.style.borderColor = '#e2e8f0';
                c.style.color = '#334155';
              });
              b.style.background = '#0d9488';
              b.style.borderColor = '#0d9488';
              b.style.color = '#fff';
              answers[qid] = { value: score, tags: qTags };
            };
            scaleRow.appendChild(b);
          })(s);
        }
        wrap.appendChild(scaleRow);
      } else if (qtype === 'number') {
        var num = document.createElement('input');
        num.type = 'number';
        num.style.cssText = 'width:100%;padding:10px 14px;border:1px solid #e2e8f0;border-radius:8px;font-size:0.9rem;font-family:inherit;outline:none;box-sizing:border-box;';
        num.oninput = function () {
          answers[qid] = { value: num.value === '' ? null : Number(num.value), tags: qTags };
        };
        wrap.appendChild(num);
      } else if (qtype === 'date') {
        var dt = document.createElement('input');
        dt.type = 'date';
        dt.style.cssText = 'width:100%;padding:10px 14px;border:1px solid #e2e8f0;border-radius:8px;font-size:0.9rem;font-family:inherit;outline:none;box-sizing:border-box;';
        dt.oninput = function () { answers[qid] = { value: dt.value, tags: qTags }; };
        wrap.appendChild(dt);
      } else if (qtype === 'long_text') {
        var ta = document.createElement('textarea');
        ta.placeholder = 'Your answer...';
        ta.style.cssText = 'width:100%;padding:10px 14px;border:1px solid #e2e8f0;border-radius:8px;font-size:0.9rem;font-family:inherit;resize:vertical;min-height:80px;outline:none;box-sizing:border-box;';
        ta.oninput = function () { answers[qid] = { value: ta.value, tags: qTags }; };
        wrap.appendChild(ta);
      } else {
        var ti = document.createElement('input');
        ti.type = 'text';
        ti.placeholder = 'Your answer...';
        ti.style.cssText = 'width:100%;padding:10px 14px;border:1px solid #e2e8f0;border-radius:8px;font-size:0.9rem;font-family:inherit;outline:none;box-sizing:border-box;';
        ti.oninput = function () { answers[qid] = { value: ti.value, tags: qTags }; };
        wrap.appendChild(ti);
      }

      wrap.setAttribute('data-mdq', qid);
      formBody.appendChild(wrap);
    });

    modal.appendChild(errorBox);
    modal.appendChild(formBody);

    var btnRow = el('div', 'display:flex;gap:10px;justify-content:flex-end;');
    var skipBtn = el('button', 'padding:10px 20px;border-radius:8px;font-size:0.9rem;font-weight:600;cursor:pointer;border:1px solid #e2e8f0;background:#fff;color:#64748b;', 'Skip');
    skipBtn.type = 'button';
    skipBtn.onclick = function () { skip(overlay); };
    btnRow.appendChild(skipBtn);

    var submitBtn = el('button', 'padding:10px 24px;border-radius:8px;font-size:0.9rem;font-weight:600;cursor:pointer;border:none;background:#0d9488;color:#fff;', 'Submit');
    submitBtn.type = 'button';
    submitBtn.onclick = function () { submit(survey, answers, overlay, errorBox, submitBtn); };
    btnRow.appendChild(submitBtn);

    modal.appendChild(btnRow);
    overlay.appendChild(modal);
    document.body.appendChild(overlay);
  }

  // ── Submit ──
  function submit(survey, answers, overlay, errorBox, submitBtn) {
    var payloadAnswers = (survey.questions || []).map(function (q, i) {
      var qid = q.id || ('q_' + i);
      var a = answers[qid] || { value: null, tags: [] };
      return {
        question_id: qid,
        question_label: q.label || q.question || '',
        type: q.type || 'short_text',
        value: a.value,
        tags: a.tags || []
      };
    });

    var payload = {
      audience: audience,
      answers: payloadAnswers,
      visitor_fingerprint: fingerprint()
    };

    var headers = { 'Content-Type': 'application/json' };
    var vt = visitorToken();
    if (vt) headers['Authorization'] = 'Bearer ' + vt;

    // Post to the directory the questionnaire belongs to (the network endpoint reports it).
    var postSlug = survey.__slug || slug;

    submitBtn.disabled = true;
    submitBtn.textContent = 'Submitting...';

    fetch('/api/v1/public/directories/' + encodeURIComponent(postSlug) + '/survey/respond', {
      method: 'POST',
      headers: headers,
      body: JSON.stringify(payload)
    }).then(function (res) {
      return res.json().catch(function () { return {}; }).then(function (body) {
        return { ok: res.ok, status: res.status, body: body };
      });
    }).then(function (r) {
      if (r.ok) {
        try { localStorage.setItem(activeCompleteKey, '1'); } catch (e) {}
        if (overlay && overlay.parentNode) overlay.parentNode.removeChild(overlay);
        var reward = r.body && r.body.reward;
        if (reward && reward.units > 0) {
          var toast = el('div', 'position:fixed;bottom:24px;left:50%;transform:translateX(-50%);background:#0f172a;color:#fff;padding:12px 20px;border-radius:10px;font-size:0.9rem;z-index:100000;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;',
            'Thanks! ' + reward.units + ' ' + (reward.currency || 'units') + ' credited to your account.');
          document.body.appendChild(toast);
          setTimeout(function () { if (toast.parentNode) toast.parentNode.removeChild(toast); }, 6000);
        }
        return;
      }
      // A rejection must be visible — silently closing would look like success.
      var msg = (r.body && (r.body.error || r.body.message)) || ('Could not submit (' + r.status + ')');
      errorBox.textContent = msg;
      errorBox.style.display = 'block';
      submitBtn.disabled = false;
      submitBtn.textContent = 'Submit';
    }).catch(function () {
      errorBox.textContent = 'Network error — please try again.';
      errorBox.style.display = 'block';
      submitBtn.disabled = false;
      submitBtn.textContent = 'Submit';
    });
  }

  function skip(overlay) {
    try { localStorage.setItem(activeSkipKey, '1'); } catch (e) {}
    if (overlay && overlay.parentNode) overlay.parentNode.removeChild(overlay);
  }

  function fingerprint() {
    var parts = [];
    if (navigator.userAgent) parts.push(navigator.userAgent);
    if (navigator.language) parts.push(navigator.language);
    if (screen.width) parts.push(screen.width + 'x' + screen.height);
    if (screen.colorDepth) parts.push(screen.colorDepth);
    var str = parts.join('|||');
    var hash = 0;
    for (var i = 0; i < str.length; i++) {
      hash = ((hash << 5) - hash) + str.charCodeAt(i);
      hash |= 0;
    }
    return 'b_' + Math.abs(hash).toString(36) + '_' + Date.now().toString(36);
  }
})();
