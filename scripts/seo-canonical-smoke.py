#!/usr/bin/env python3
"""Live proof for kanban t_f97664fe — ONE canonical host for the ZaarHub directory.

Run:  python3 scripts/seo-canonical-smoke.py          (exit 0 = every check passed)

Everything here is measured against the LIVE hosts through Cloudflare, not the app on
loopback, because the whole point of the change is what a crawler sees at the edge:

  1. every public page on all THREE hostnames emits the SAME absolute canonical
     `https://zaarhub.com/<path>`                                        (item 2)
  2. an unmatched path is a real 404 (not the 200 SPA shell) on the primary host and on
     the factory host, and the six Search-Console soft-404s are gone      (item 3)
  3. exactly ONE sitemap is advertised, by zaarhub.com only; it still lists ~4,030 real
     urls and carries no junk path                                       (item 4)
  4. www.zaarhub.com answers ONE 301 to https://zaarhub.com, path and query preserved
                                                                         (item 1)
  5. the FACTORY is unbroken on directory.swiftsoftware.net: admin shell, static admin
     alias, /admin-panel and an authenticated admin read all still work  (item 1)
  6. /visitor and /zaarhub-offer.html are noindex; /claim is still indexable
  7. real public pages still answer 200 on the primary host
"""
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request

PRIMARY = "https://zaarhub.com"
FACTORY = "https://directory.swiftsoftware.net"
WWW = "https://www.zaarhub.com"
HOSTS = [PRIMARY, WWW, FACTORY]
UA = "Mozilla/5.0 (compatible; md-seo-probe/1.0)"

CANON_RE = re.compile(r"<link[^>]*\brel=[\"']canonical[\"'][^>]*>", re.I)
HREF_RE = re.compile(r"\bhref=[\"']([^\"']+)[\"']", re.I)
LOC_RE = re.compile(r"<loc>(.*?)</loc>")
ROBOTS_META_RE = re.compile(r"<meta[^>]*name=[\"']robots[\"'][^>]*content=[\"']([^\"']*)", re.I)

# Pages every hostname must canonicalise to the SAME absolute url.
CANON_PAGES = ["/", "/apopka", "/portal"]
# Search Console soft-404s + a made-up path.
JUNK_PATHS = [
    "/this-path-does-not-exist-xyz123",
    "/meta",
    "/end",
    "/search/",
    "/transfers/",
    "/home",
    "/zaarhub/cities/",
]
# Public pages that must still be served.
REAL_PAGES = [
    "/",
    "/apopka",
    "/apopka/businesses",
    "/portal",
    "/visitor",
    "/claim",
    "/city/apopka",
    "/d/apopka",
    "/zaarhub",
    "/login",
    "/submit-business.html",
    "/guides/index.html",
    "/guides/participant-guide.html",
    "/b2b-marketplace.html",
    "/rfq-marketplace",
]

FAILS = []
CHECKS = [0]


def fetch(url, method="GET", headers=None, follow=True, token=None):
    """Return (status, headers, body_bytes). Never raises on an HTTP error status."""

    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *a, **k):
            return None

    hdrs = {"User-Agent": UA}
    if token:
        hdrs["Authorization"] = "Bearer " + token
    if headers:
        hdrs.update(headers)
    req = urllib.request.Request(url, method=method, headers=hdrs)
    opener = (
        urllib.request.build_opener()
        if follow
        else urllib.request.build_opener(NoRedirect)
    )
    try:
        with opener.open(req, timeout=30) as r:
            return r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read()


def check(ok, label, detail=""):
    CHECKS[0] += 1
    if ok:
        print(f"PASS  {label}" + (f"  [{detail}]" if detail else ""))
    else:
        print(f"FAIL  {label}" + (f"  [{detail}]" if detail else ""))
        FAILS.append(label)
    return ok


def canon_of(body):
    m = CANON_RE.search(body)
    if not m:
        return None
    h = HREF_RE.search(m.group(0))
    return h.group(1) if h else None


def expected_canonical(path):
    return PRIMARY + ("/" if path == "/" else path.rstrip("/"))


def main():
    # ── 1. absolute canonical, identical on every hostname ───────────────────────
    for path in CANON_PAGES:
        want = expected_canonical(path)
        seen = {}
        for host in HOSTS:
            status, _h, body = fetch(host + path)
            text = body.decode("utf-8", "replace")
            seen[host] = canon_of(text)
            check(
                status == 200 and canon_of(text) == want,
                f"canonical {path} on {host.replace('https://', '')}",
                f"HTTP {status} canonical={canon_of(text)}",
            )
        check(
            len(set(seen.values())) == 1,
            f"canonical {path} is IDENTICAL on all three hosts",
            str(sorted(set(seen.values()))),
        )

    # ── 2. a real 404 for unmatched paths ───────────────────────────────────────
    for path in JUNK_PATHS:
        for host in (PRIMARY, FACTORY):
            status, _h, body = fetch(host + path)
            meta = ROBOTS_META_RE.search(body.decode("utf-8", "replace"))
            check(
                status == 404 and meta is not None and "noindex" in meta.group(1).lower(),
                f"404 (noindex) {path} on {host.replace('https://', '')}",
                f"HTTP {status} robots={meta.group(1) if meta else None}",
            )

    # ── 3. one sitemap, advertised by the primary host only ─────────────────────
    status, _h, body = fetch(PRIMARY + "/robots.txt")
    text = body.decode("utf-8", "replace")
    sm = [ln for ln in text.splitlines() if ln.lower().startswith("sitemap:")]
    check(
        status == 200 and sm == [f"Sitemap: {PRIMARY}/sitemap.xml"],
        "zaarhub.com robots.txt advertises exactly one sitemap",
        str(sm),
    )
    check("Disallow: /admin/" in text, "zaarhub.com robots.txt still disallows /admin/")
    status, _h, body = fetch(FACTORY + "/robots.txt")
    text = body.decode("utf-8", "replace")
    sm = [ln for ln in text.splitlines() if ln.lower().startswith("sitemap:")]
    check(
        status == 200 and sm == [],
        "directory.swiftsoftware.net robots.txt advertises NO sitemap",
        str(sm),
    )
    check("Disallow: /admin/" in text, "factory robots.txt still disallows /admin/")

    status, _h, body = fetch(PRIMARY + "/sitemap.xml")
    locs = LOC_RE.findall(body.decode("utf-8", "replace"))
    junk = [u for u in locs if any(j.rstrip("/") == u.replace(PRIMARY, "").rstrip("/") for j in JUNK_PATHS)]
    check(status == 200 and 3900 <= len(locs) <= 4200, "sitemap still lists ~4,030 urls", f"{len(locs)} locs")
    check(
        all(u.startswith(PRIMARY + "/") for u in locs),
        "every sitemap url is on the primary host",
    )
    check(not junk, "sitemap carries no junk path", str(junk[:3]))

    # ── 4. www -> apex, single hop ──────────────────────────────────────────────
    status, hdrs, _b = fetch(WWW + "/apopka?x=1", follow=False)
    loc = hdrs.get("Location") or hdrs.get("location")
    check(
        status == 301 and loc == f"{PRIMARY}/apopka?x=1",
        "www.zaarhub.com -> ONE 301 to https://zaarhub.com (path+query kept)",
        f"HTTP {status} Location={loc}",
    )
    status2, _h, _b = fetch(PRIMARY + "/apopka?x=1", follow=False)
    check(status2 == 200, "the 301 target answers 200 (no redirect loop)", f"HTTP {status2}")

    # ── 5. the FACTORY is unbroken ──────────────────────────────────────────────
    status, hdrs, body = fetch(FACTORY + "/admin/login")
    xrt = hdrs.get("X-Robots-Tag") or hdrs.get("x-robots-tag") or ""
    check(status == 200 and "text/html" in (hdrs.get("Content-Type") or "").lower(),
          "factory /admin/login still serves the admin shell", f"HTTP {status}")
    check("noindex" in xrt.lower(), "factory /admin/login keeps X-Robots-Tag noindex", xrt)
    for p in ("/admin/", "/admin-panel"):
        status, _h, _b = fetch(FACTORY + p)
        check(status == 200, f"factory {p} still 200", f"HTTP {status}")

    # Authenticated admin read on the factory host, using the fleet's own mint script:
    # the claim set a real login issues (sub/tid/role/aud/iss), signed with the app's
    # JWT_SECRET. Nothing is written to the database by this call.
    token = ""
    try:
        token = subprocess.run(
            ["node", "/opt/swift/browser-run/md-mint-token.js"],
            capture_output=True, text=True, timeout=30,
        ).stdout.strip()
    except Exception as e:  # noqa: BLE001 — the check below reports it
        print(f"      (token mint failed: {e})")
    status, _h, body = fetch(FACTORY + "/api/v1/admin/tenants", token=token or None)
    ok_rows = 0
    try:
        rows = json.loads(body.decode("utf-8", "replace"))
        ok_rows = len(rows if isinstance(rows, list) else rows.get("data") or rows.get("tenants") or [])
    except Exception:  # noqa: BLE001
        rows = None
    check(
        status == 200 and ok_rows > 0,
        "authenticated admin read /api/v1/admin/tenants returns data on the factory host",
        f"HTTP {status} rows={ok_rows}",
    )
    # The login route itself must still be wired (wrong password -> 401, never 404/5xx).
    status, _h, _b = fetch(
        FACTORY + "/api/v1/auth/login",
        method="POST",
        headers={"Content-Type": "application/json"},
    )
    check(status in (400, 401, 408, 422), "factory login endpoint rejects a bad body honestly", f"HTTP {status}")

    # ── 6. the factory host is never the indexed copy ───────────────────────────
    for path in ("/", "/apopka", "/portal"):
        status, hdrs, _b = fetch(FACTORY + path)
        xrt = (hdrs.get("X-Robots-Tag") or hdrs.get("x-robots-tag") or "").lower()
        check(
            status == 200 and "noindex" in xrt,
            f"factory host {path} is noindex (David's rule: only the brand host ranks)",
            f"HTTP {status} x-robots-tag={xrt or '-'}",
        )
        status, hdrs, _b = fetch(PRIMARY + path)
        xrt = (hdrs.get("X-Robots-Tag") or hdrs.get("x-robots-tag") or "").lower()
        check(
            "noindex" not in xrt and status == 200,
            f"primary host {path} is NOT noindex (it must rank)",
            f"HTTP {status} x-robots-tag={xrt or '-'}",
        )
    for path in ("/admin-panel", "/api/v1/health"):
        status, hdrs, _b = fetch(FACTORY + path)
        xrt = (hdrs.get("X-Robots-Tag") or hdrs.get("x-robots-tag") or "").lower()
        check(
            "noindex" not in xrt and status == 200,
            f"factory {path} is untouched by the noindex rule",
            f"HTTP {status} x-robots-tag={xrt or '-'}",
        )

    # ── 6b. account surfaces noindex, content indexable ─────────────────────────
    for path in ("/visitor", "/zaarhub-offer.html"):
        status, hdrs, body = fetch(PRIMARY + path)
        meta = ROBOTS_META_RE.search(body.decode("utf-8", "replace"))
        xrt = (hdrs.get("X-Robots-Tag") or hdrs.get("x-robots-tag") or "").lower()
        check(
            "noindex" in xrt or (meta is not None and "noindex" in meta.group(1).lower()),
            f"{path} is noindex",
            f"HTTP {status} x-robots-tag={xrt or '-'} meta={meta.group(1) if meta else '-'}",
        )
    for path in ("/claim", "/guides/participant-guide.html"):
        status, hdrs, body = fetch(PRIMARY + path)
        meta = ROBOTS_META_RE.search(body.decode("utf-8", "replace"))
        xrt = (hdrs.get("X-Robots-Tag") or hdrs.get("x-robots-tag") or "").lower()
        check(
            status == 200 and "noindex" not in xrt and not (meta and "noindex" in meta.group(1).lower()),
            f"{path} is still indexable",
            f"HTTP {status}",
        )

    # ── 7. real public pages still served ───────────────────────────────────────
    for path in REAL_PAGES:
        status, _h, _b = fetch(PRIMARY + path)
        check(status == 200, f"public page {path} still 200", f"HTTP {status}")

    print(f"\n{CHECKS[0] - len(FAILS)}/{CHECKS[0]} checks passed")
    if FAILS:
        print("FAILED:")
        for f in FAILS:
            print("  - " + f)
        return 1
    print("ALL PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
