//! Visitor tracking script injection for directory pages.
//! Inserts the fingerprinting + event tracking script before </body>.

/// Returns the inline tracking script to be injected into directory HTML pages.
pub fn tracking_script_html() -> &'static str {
    r##"<script>
(function(){
if(typeof _vt !== "undefined") return;
window._vt = {};
var fp = (function(){
  var c = document.createElement("canvas"), ctx = c.getContext("2d"), txt = "finger";
  c.width = 200; c.height = 50;
  ctx.textBaseline = "top";
  ctx.font = "14px Arial";
  ctx.fillStyle = "#f60";
  ctx.fillRect(125,1,62,20);
  ctx.fillStyle = "#069";
  ctx.fillText(txt,2,15);
  ctx.fillStyle = "rgba(102,204,0,0.7)";
  ctx.fillText(txt,4,17);
  return c.toDataURL();
})();
var raw = fp + screen.width + "x" + screen.height + screen.colorDepth + navigator.language + Intl.DateTimeFormat().resolvedOptions().timeZone + navigator.userAgent;
var f = btoa(raw.replace(/[^a-z0-9]/gi,"")).substring(0,64);
var sess = localStorage.getItem("_vs");
if(sess){
  try{
    sess = JSON.parse(sess);
    if(sess.v === f && sess.s){
      fetch("/api/v1/visitors/page-view",{
        method:"POST",
        headers:{"Content-Type":"application/json"},
        body:JSON.stringify({session_id:sess.s,page_url:location.href,referrer:document.referrer||null})
      });
      if(sess.e && sess.e.length){
        var batch = sess.e.splice(0,10);
        fetch("/api/v1/visitors/event",{
          method:"POST",
          headers:{"Content-Type":"application/json"},
          body:JSON.stringify({session_id:sess.s,events:batch})
        });
        localStorage.setItem("_vs",JSON.stringify(sess));
      }
      return;
    }
  }catch(e){}
}
var p = {
  fingerprint: f,
  directory_id: window.__dir_id || null,
  language: navigator.language,
  screen_resolution: screen.width + "x" + screen.height,
  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
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
  localStorage.setItem("_vs",JSON.stringify({v:f,s:d.session_id,e:[]}));
}).catch(function(){});
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
window.addEventListener("beforeunload",function(){
  var dur = Math.round((Date.now()-pt)/1000);
  try{var s = JSON.parse(localStorage.getItem("_vs")||"{}");
    if(s.s){
      navigator.sendBeacon("/api/v1/visitors/session/"+s.s+"/end",JSON.stringify({
        exit_page:location.href,pages_viewed:1,scroll_depth_pct:sc,duration_secs:dur,is_bounce:sc<25
      }));
    }
  }catch(e){}
},false);
function trackEvt(et,ev){
  try{var s = JSON.parse(localStorage.getItem("_vs")||"{}");
    if(s.s){
      s.e = s.e || [];
      s.e.push({event_type:et,event_value:ev,page_url:location.href,scroll_depth:sc,duration_ms:Date.now()-pt});
      localStorage.setItem("_vs",JSON.stringify(s));
      if(s.e.length >= 10){
        var b = s.e.splice(0,10);
        navigator.sendBeacon("/api/v1/visitors/event",JSON.stringify({session_id:s.s,events:b}));
        localStorage.setItem("_vs",JSON.stringify(s));
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
