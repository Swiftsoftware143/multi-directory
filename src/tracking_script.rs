//! Visitor tracking script injection for directory pages.
//! Inserts the fingerprinting + event tracking script before </body>.
//!
//! It shares ONE session with the external `/tracking-beacon.js` beacon: the same
//! localStorage key (`_zh_beacon`), the same browser-fingerprint algorithm and the same
//! stored shape `{s,f,t,dir,e}`. Before this, the two trackers used different keys and
//! different fingerprint algorithms, so a visitor who landed on `/` (external beacon) and
//! then opened a city/blog page (this inline script) was counted as TWO visitors with two
//! sessions — polluting the demand dataset and losing the entry-page link.
//!
//! Every event is delivered ONE per request, in the exact shape
//! `POST /api/v1/visitors/event` accepts (`{session_id, event_type, …}`). The old form sent
//! `{session_id, events:[…]}` — a batch the API cannot deserialize (422), so every event the
//! injected tracker queued (scroll depth, and now link clicks) was silently discarded.
//! Clicks carry the business + category from `window.__biz_ctx`, like the external beacon.

/// Returns the inline tracking script to be injected into directory HTML pages.
pub fn tracking_script_html() -> &'static str {
    r##"<script>
(function(){
if(typeof _vt !== "undefined") return;
window._vt = {};
var KEY = "_zh_beacon";
var SESSION_MINUTES = 30;
var MAX_CLICKS = 20, clicksSent = 0;
// Same fingerprint algorithm as /tracking-beacon.js — a shared session is only reused
// when the fingerprint matches exactly, so both trackers must compute it identically.
function fingerprint(){
  var raw = [
    navigator.userAgent || "",
    screen.width + "x" + screen.height + "x" + (screen.colorDepth || 0),
    navigator.language || "",
    (window.Intl && Intl.DateTimeFormat ? Intl.DateTimeFormat().resolvedOptions().timeZone : "") || ""
  ].join("|");
  var h = 5381;
  for(var i = 0; i < raw.length; i++){ h = ((h << 5) + h + raw.charCodeAt(i)) & 0xffffffff; }
  return "zh" + Math.abs(h).toString(36) + "-" + raw.length.toString(36);
}
function readSess(){ try{ var r = localStorage.getItem(KEY); return r ? JSON.parse(r) : null; }catch(e){ return null; } }
function writeSess(o){ try{ localStorage.setItem(KEY, JSON.stringify(o)); }catch(e){} }
function beaconJson(url, obj){
  try{
    if(navigator.sendBeacon){
      navigator.sendBeacon(url, new Blob([JSON.stringify(obj)], {type:"application/json"}));
    }else{
      fetch(url,{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(obj),keepalive:true});
    }
  }catch(e){}
}
// The SPA sets window.__biz_ctx while a business page is on screen, so a click carries the
// business and its category — the demand signal — exactly as the external beacon does.
function bizCtx(){ return (window.__biz_ctx && typeof window.__biz_ctx === "object") ? window.__biz_ctx : {}; }
// ONE event per call, in the shape POST /api/v1/visitors/event actually accepts. A batch
// body is rejected 422, which is why scroll-depth rows never reached visitor_events.
function sendEvent(sid, obj){
  var c = bizCtx();
  var payload = {
    session_id: sid,
    directory_id: window.__dir_id || c.directory_id || null,
    event_type: obj.event_type,
    event_value: obj.event_value || null,
    page_url: obj.page_url || location.href,
    scroll_depth: obj.scroll_depth || 0,
    duration_ms: obj.duration_ms || 0
  };
  if(c.business_id) payload.business_id = c.business_id;
  if(c.category_id) payload.category_id = c.category_id;
  beaconJson("/api/v1/visitors/event", payload);
}
var f = fingerprint();
var sess = readSess();
var fresh = sess && sess.s && sess.f === f && (Date.now() - (sess.t || 0) < SESSION_MINUTES * 60 * 1000);
if(fresh){
  sess.t = Date.now();
  writeSess(sess);
  fetch("/api/v1/visitors/page-view",{
    method:"POST",
    headers:{"Content-Type":"application/json"},
    body:JSON.stringify({session_id:sess.s,page_url:location.href,referrer:document.referrer||null})
  });
}else{
  var p = {
    fingerprint: f,
    directory_id: window.__dir_id || null,
    language: navigator.language,
    screen_resolution: screen.width + "x" + screen.height,
    timezone: (window.Intl && Intl.DateTimeFormat ? Intl.DateTimeFormat().resolvedOptions().timeZone : null),
    referrer: document.referrer || null,
    page_url: location.href
  };
  var u = new URL(location.href);
  p.utm_source = u.searchParams.get("utm_source");
  p.utm_medium = u.searchParams.get("utm_medium");
  p.utm_campaign = u.searchParams.get("utm_campaign");
  p.utm_term = u.searchParams.get("utm_term");
  p.utm_content = u.searchParams.get("utm_content");
  fetch("/api/v1/visitors/track",{
    method:"POST",
    headers:{"Content-Type":"application/json"},
    body:JSON.stringify(p)
  }).then(function(r){return r.json()}).then(function(d){
    if(d && d.session_id){ writeSess({s:d.session_id,f:f,t:Date.now(),dir:p.directory_id}); }
  }).catch(function(){});
}
// Delivered immediately, ONE event per request (the beacon does the same). A queued event
// was lost whenever the visitor left without a second page view; a batch body is rejected 422.
function trackEvt(et, ev){
  try{var s = readSess();
    if(!s || !s.s) return;
    sendEvent(s.s, {event_type:et,event_value:ev,page_url:location.href,scroll_depth:sc,duration_ms:Date.now()-pt});
  }catch(e){}
}
function labelFor(el){
  if(!el || !el.tagName) return "unknown";
  var explicit = el.getAttribute && (el.getAttribute("data-track") || el.getAttribute("aria-label"));
  var text = (el.textContent || "").replace(/\s+/g, " ").trim();
  return (explicit || text || el.tagName.toLowerCase()).slice(0, 80);
}
// Link clicks — the strongest demand signal (which business/service a visitor wanted).
// Same classification as the external beacon: phone, outbound website, or in-page click.
document.addEventListener("click", function(ev){
  try{
    if(clicksSent >= MAX_CLICKS) return;
    var el = ev.target;
    if(el && el.closest) el = el.closest("a,button,[data-track]") || ev.target;
    var href = (el && el.getAttribute && el.getAttribute("href")) || "";
    clicksSent++;
    if(href.indexOf("tel:") === 0){ trackEvt("phone_click", href); }
    else if(/^https?:\/\//.test(href) && href.indexOf(location.host) === -1){ trackEvt("website_click", href); }
    else { trackEvt("click", labelFor(el)); }
  }catch(e){}
}, true);
var sc = 0, st = 0;
window.addEventListener("scroll",function(){
  var h = document.documentElement.scrollHeight - window.innerHeight;
  if(h > 0){
    var pct = Math.round((window.scrollY / h) * 100);
    if(pct > sc){ sc = pct;
      var step = Math.floor(pct / 25) * 25;
      if(step > st){ st = step; trackEvt("scroll_depth",""+step); }
    }
  }
});
var pt = Date.now();
// sendBeacon with a plain string sends Content-Type: text/plain, which the API rejects with 415,
// so session-end and batched events never recorded. A Blob with an explicit JSON type fixes that.
window.addEventListener("beforeunload",function(){
  var dur = Math.round((Date.now()-pt)/1000);
  try{var s = readSess();
    if(s && s.s){
      beaconJson("/api/v1/visitors/session/"+s.s+"/end",{
        exit_page:location.href,pages_viewed:1,scroll_depth_pct:sc,duration_secs:dur,is_bounce:sc<25
      });
    }
  }catch(e){}
},false);
window.__trackEvent = trackEvt;
})();
</script>"##
}

/// Inject the tracking script into HTML before `</body>` tag.
pub fn inject_tracking_script(html: &str) -> String {
    if let Some(pos) = html.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + 2000);
        result.push_str(&html[..pos]);
        result.push_str(tracking_script_html());
        result.push_str(&html[pos..]);
        result
    } else {
        html.to_string()
    }
}
