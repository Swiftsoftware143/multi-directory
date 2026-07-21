(function() {
  'use strict';

  // ── Detect directory slug and portal type from URL ──
  var slug = null;
  var portalType = 'visitor'; // default: visitor (public directory pages)

  var path = window.location.pathname;
  var pathParts = path.split('/').filter(Boolean);

  if (path.startsWith('/portal/business') || path.startsWith('/portal/business/')) {
    // Business portal
    // Fetch from a default directory (we need a slug — try first path after /portal/business/)
    portalType = 'business';
    // Business portal users are associated with a directory via auth
    // For now, we try to get slug from the page or from URL
    // The page itself should have a data attribute or we detect from the second path part
    if (pathParts.length >= 3 && pathParts[2] !== 'dashboard') {
      slug = pathParts[2];
    } else {
      // Fallback: use localStorage cached slug from previous navigation
      slug = localStorage.getItem('md_active_slug');
    }
  } else if (path.startsWith('/distributor') || path.startsWith('/distributor/')) {
    // Supplier portal — network-wide, no city
    portalType = 'supplier';
    // Use the first directory as a reference for fetching survey config
    slug = localStorage.getItem('md_active_slug') || null;
  } else if (pathParts.length > 0) {
    // Public directory page — e.g., /apopka, /palm-coast
    slug = pathParts[0];
    portalType = 'visitor';
  }

  if (!slug) return;

  // ── Check if already responded ──
  var storageKey = 'survey_completed_' + slug + '_' + portalType;
  var skipKey = 'survey_skipped_' + slug + '_' + portalType;

  if (localStorage.getItem(storageKey) || localStorage.getItem(skipKey)) {
    return;
  }

  // ── Fetch survey config ──
  var xhr = new XMLHttpRequest();
  xhr.open('GET', '/api/v1/public/directories/' + encodeURIComponent(slug) + '/survey', true);
  xhr.onload = function() {
    if (xhr.status !== 200) return;
    try {
      var survey = JSON.parse(xhr.responseText);
      if (!survey.enabled || !survey.questions || survey.questions.length === 0) return;

      // Filter questions by portal type using the `tags` array on each question
      var filteredQuestions = survey.questions.filter(function(q) {
        var qTags = q.tags || [];
        return qTags.length === 0 || qTags.indexOf(portalType) !== -1;
      });

      if (filteredQuestions.length === 0) return;

      survey.questions = filteredQuestions;
      showSurvey(survey);
    } catch(e) {
      // silent fail
    }
  };
  xhr.send();

  // ── Update slug reference on visitor portal login ──
  // If the page is a portal, cache the slug from the page's data attributes
  var slugEl = document.querySelector('[data-directory-slug]');
  if (slugEl) {
    localStorage.setItem('md_active_slug', slugEl.getAttribute('data-directory-slug'));
  }

  // ── Survey Modal ──
  function showSurvey(survey) {
    // Wait 2 seconds before showing
    setTimeout(function() {
      renderModal(survey);
    }, 2000);
  }

  function renderModal(survey) {
    // Remove any existing overlay
    var existing = document.getElementById('md-survey-overlay');
    if (existing) existing.remove();

    var overlay = document.createElement('div');
    overlay.id = 'md-survey-overlay';
    overlay.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;background:rgba(0,0,0,0.5);z-index:99999;display:flex;align-items:center;justify-content:center;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;';

    var modal = document.createElement('div');
    modal.style.cssText = 'background:#fff;border-radius:16px;padding:32px;max-width:560px;width:90%;max-height:80vh;overflow-y:auto;box-shadow:0 25px 50px -12px rgba(0,0,0,0.4);position:relative;';

    var closeBtn = document.createElement('button');
    closeBtn.textContent = '×';
    closeBtn.style.cssText = 'position:absolute;top:12px;right:16px;background:none;border:none;font-size:24px;cursor:pointer;color:#94a3b8;line-height:1;';
    closeBtn.onclick = function() { skipSurvey(); };
    modal.appendChild(closeBtn);

    var title = document.createElement('h2');
    title.textContent = survey.title || 'Help us personalize your experience';
    title.style.cssText = 'font-size:1.4rem;font-weight:700;color:#0f172a;margin:0 0 8px 0;';
    modal.appendChild(title);

    if (survey.description) {
      var desc = document.createElement('p');
      desc.textContent = survey.description;
      desc.style.cssText = 'font-size:0.9rem;color:#64748b;margin:0 0 20px 0;';
      modal.appendChild(desc);
    }

    var answers = {};
    var questions = survey.questions;

    var formBody = document.createElement('div');
    formBody.style.cssText = 'margin-bottom:20px;';

    for (var i = 0; i < questions.length; i++) {
      var q = questions[i];

      var qType = q.type || 'text';
      var qLabel = q.label || q.question || (typeof q === 'string' ? q : 'Question ' + (i+1));
      var qId = q.id || ('q_' + i);
      var qOptions = q.options || [];
      var qTags = q.tags || [];

      var qWrap = document.createElement('div');
      qWrap.style.cssText = 'margin-bottom:16px;';

      var label = document.createElement('div');
      label.textContent = qLabel;
      label.style.cssText = 'font-weight:600;font-size:0.9rem;color:#0f172a;margin-bottom:6px;';
      qWrap.appendChild(label);

      if (qType === 'choice') {
        // Radio buttons (single select)
        for (var j = 0; j < qOptions.length; j++) {
          var opt = qOptions[j];
          var optLabel = typeof opt === 'string' ? opt : (opt.label || opt);
          var optVal = typeof opt === 'string' ? opt : (opt.value || opt);

          var radioWrap = document.createElement('label');
          radioWrap.style.cssText = 'display:flex;align-items:center;gap:8px;padding:6px 0;cursor:pointer;font-size:0.9rem;color:#334155;';

          var radio = document.createElement('input');
          radio.type = 'radio';
          radio.name = 'q_' + i;
          radio.value = optVal;
          radio.style.cssText = 'accent-color:#0d9488;';
          radio.onchange = function(val, tags) {
            return function() {
              answers[qId] = { value: val, tags: tags };
            };
          }(optVal, opt.tags || qTags);

          radioWrap.appendChild(radio);
          radioWrap.appendChild(document.createTextNode(optLabel));
          qWrap.appendChild(radioWrap);
        }
      } else if (qType === 'select') {
        // Dropdown menu (single select)
        var select = document.createElement('select');
        select.style.cssText = 'width:100%;padding:10px 14px;border:1px solid #e2e8f0;border-radius:8px;font-size:0.9rem;font-family:inherit;background:#fff;outline:none;box-sizing:border-box;cursor:pointer;';

        var placeholderOpt = document.createElement('option');
        placeholderOpt.value = '';
        placeholderOpt.textContent = 'Select an option...';
        placeholderOpt.disabled = true;
        placeholderOpt.selected = true;
        select.appendChild(placeholderOpt);

        for (var m = 0; m < qOptions.length; m++) {
          var opt3 = qOptions[m];
          var optLabel3 = typeof opt3 === 'string' ? opt3 : (opt3.label || opt3);
          var optVal3 = typeof opt3 === 'string' ? opt3 : (opt3.value || opt3);

          var option = document.createElement('option');
          option.value = optVal3;
          option.textContent = optLabel3;
          select.appendChild(option);
        }

        select.onchange = function(sel, id, tags) {
          return function() {
            answers[id] = { value: sel.value, tags: tags };
          };
        }(select, qId, qTags);

        qWrap.appendChild(select);
      } else if (qType === 'multi') {
        // Checkboxes (multi-select)
        for (var k = 0; k < qOptions.length; k++) {
          var opt2 = qOptions[k];
          var optLabel2 = typeof opt2 === 'string' ? opt2 : (opt2.label || opt2);
          var optVal2 = typeof opt2 === 'string' ? opt2 : (opt2.value || opt2);

          var cbWrap = document.createElement('label');
          cbWrap.style.cssText = 'display:flex;align-items:center;gap:8px;padding:6px 0;cursor:pointer;font-size:0.9rem;color:#334155;';

          var cb = document.createElement('input');
          cb.type = 'checkbox';
          cb.value = optVal2;
          cb.style.cssText = 'accent-color:#0d9488;';

          var cbTags = opt2.tags || qTags;
          (function(input, val, tags, ansObj) {
            input.onchange = function() {
              if (!ansObj[qId]) ansObj[qId] = { value: [], tags: [] };
              if (this.checked) {
                if (ansObj[qId].value.indexOf(val) === -1) ansObj[qId].value.push(val);
                tags.forEach(function(t) { if (ansObj[qId].tags.indexOf(t) === -1) ansObj[qId].tags.push(t); });
              } else {
                var idxVal = ansObj[qId].value.indexOf(val);
                if (idxVal > -1) ansObj[qId].value.splice(idxVal, 1);
                tags.forEach(function(t) {
                  var idxT = ansObj[qId].tags.indexOf(t);
                  if (idxT > -1) ansObj[qId].tags.splice(idxT, 1);
                });
              }
            };
          })(cb, optVal2, cbTags, answers);

          cbWrap.appendChild(cb);
          cbWrap.appendChild(document.createTextNode(optLabel2));
          qWrap.appendChild(cbWrap);
        }
      } else {
        // Text input (default)
        var ta = document.createElement('textarea');
        ta.placeholder = 'Your answer...';
        ta.style.cssText = 'width:100%;padding:10px 14px;border:1px solid #e2e8f0;border-radius:8px;font-size:0.9rem;font-family:inherit;resize:vertical;min-height:60px;outline:none;box-sizing:border-box;';
        ta.oninput = function(id) {
          return function() { answers[id] = { value: this.value, tags: qTags }; };
        }(qId);
        qWrap.appendChild(ta);
      }

      formBody.appendChild(qWrap);
    }

    modal.appendChild(formBody);

    // ── Submit Button ──
    var btnRow = document.createElement('div');
    btnRow.style.cssText = 'display:flex;gap:10px;justify-content:flex-end;';

    var skipBtn = document.createElement('button');
    skipBtn.textContent = 'Skip';
    skipBtn.style.cssText = 'padding:10px 20px;border-radius:8px;font-size:0.9rem;font-weight:600;cursor:pointer;border:1px solid #e2e8f0;background:#fff;color:#64748b;transition:all .2s;';
    skipBtn.onmouseover = function() { this.style.background = '#f1f5f9'; };
    skipBtn.onmouseout = function() { this.style.background = '#fff'; };
    skipBtn.onclick = function() { skipSurvey(); };
    btnRow.appendChild(skipBtn);

    var submitBtn = document.createElement('button');
    submitBtn.textContent = 'Submit';
    submitBtn.style.cssText = 'padding:10px 24px;border-radius:8px;font-size:0.9rem;font-weight:600;cursor:pointer;border:none;background:#0d9488;color:#fff;transition:all .2s;';
    submitBtn.onmouseover = function() { this.style.background = '#0f766e'; };
    submitBtn.onmouseout = function() { this.style.background = '#0d9488'; };
    submitBtn.onclick = function() { submitSurvey(survey, answers, storageKey, overlay); };
    btnRow.appendChild(submitBtn);

    modal.appendChild(btnRow);

    overlay.appendChild(modal);
    document.body.appendChild(overlay);
  }

  function submitSurvey(survey, answers, storageKey, overlay) {
    // Build answers array matching question order
    var answerArray = [];
    var questions = survey.questions || [];
    for (var i = 0; i < questions.length; i++) {
      var q = questions[i];
      var qId = q.id || ('q_' + i);
      var ans = answers[qId] || { value: null, tags: [] };
      answerArray.push({
        question_id: qId,
        question_label: q.label || q.question || '',
        value: ans.value,
        tags: ans.tags || []
      });
    }

    var payload = {
      answers: answerArray,
      visitor_fingerprint: generateFingerprint()
    };

    var xhr = new XMLHttpRequest();
    xhr.open('POST', '/api/v1/public/directories/' + encodeURIComponent(slug) + '/survey/respond', true);
    xhr.setRequestHeader('Content-Type', 'application/json');
    xhr.onload = function() {
      if (xhr.status >= 200 && xhr.status < 300) {
        localStorage.setItem(storageKey, '1');
        if (overlay && overlay.parentNode) overlay.parentNode.removeChild(overlay);
      }
    };
    xhr.onerror = function() {
      // Still close and store on error
      localStorage.setItem(storageKey, '1');
      if (overlay && overlay.parentNode) overlay.parentNode.removeChild(overlay);
    };
    xhr.send(JSON.stringify(payload));
  }

  function skipSurvey() {
    localStorage.setItem(skipKey, '1');
    var overlay = document.getElementById('md-survey-overlay');
    if (overlay) overlay.remove();
  }

  function generateFingerprint() {
    var parts = [];
    if (navigator.userAgent) parts.push(navigator.userAgent);
    if (navigator.language) parts.push(navigator.language);
    if (screen.width) parts.push(screen.width + 'x' + screen.height);
    if (screen.colorDepth) parts.push(screen.colorDepth);
    // Simple hash
    var str = parts.join('|||');
    var hash = 0;
    for (var i = 0; i < str.length; i++) {
      var chr = str.charCodeAt(i);
      hash = ((hash << 5) - hash) + chr;
      hash |= 0;
    }
    return 'b_' + Math.abs(hash).toString(36) + '_' + Date.now().toString(36);
  }
})();
