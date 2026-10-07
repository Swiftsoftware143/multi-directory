//! Visitor tracking script injection for directory pages.
//! Inserts the fingerprinting + event tracking script before </body>.
//!
//! It shares ONE session with the external `/tracking-beacon.js` beacon: the same
//! localStorage key (`_zh_beacon`), the same browser-fingerprint algorithm and the same
//! stored shape `{s,f,t,dir,e}`. Before this, the two trackers used different keys and
//! different fingerprint algorithms, so a visitor who landed on `/` (external beacon) and
//! then opened a city/blog page (this inline script) was counted as TWO visitors with two
//! sessions — polluting the demand dataset and losing the entry-page link.

/// Returns the inline tracking script to be injected into directory HTML pages.
pub fn tracking_script_html() -> &'static str {
    r##"<script>
(function(){
if(typeof _vt !== "undefined") return;
window._vt = {};
var KEY = "_zh_beacon";
var SESSION_MINUTES = 30;
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
  if(sess.e && sess.e.length){
    var batch = sess.e.splice(0,10);
    beaconJson("/api/v1/visitors/event",{session_id:sess.s,events:batch});
    writeSess(sess);
  }
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
    writeSess({s:d.session_id,f:f,t:Date.now(),dir:p.directory_id,e:[]});
  }).catch(function(){});
}
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
// so session-end and batched events never recorded. A Blob with an explicit JSON type fixes it.
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
function trackEvt(et,ev){
  try{var s = readSess();
    if(s && s.s){
      s.e = s.e || [];
      s.e.push({event_type:et,event_value:ev,page_url:location.href,scroll_depth:sc,duration_ms:Date.now()-pt});
      s.t = Date.now();
      writeSess(s);
      if(s.e.length >= 10){
        var b = s.e.splice(0,10);
        beaconJson("/api/v1/visitors/event",{session_id:s.s,events:b});
        writeSess(s);
      }
    }
  }catch(e){}
}
window.__trackEvent = trackEvt;
})();
</script>"##
}

/// Inject the tracking script into HTML before </body> tag.
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
