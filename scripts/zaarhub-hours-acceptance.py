#!/usr/bin/env python3
"""Re-runnable acceptance probe for opening hours on zaarhub listing pages (kanban t_2b630e2a).

Read-only against the live site and the DB. Prints one line per URL plus a PASS/FAIL summary.
Exit 0 only when every check passes.

  python3 zaarhub-hours-acceptance.py            # 20 pages with hours + 5 without
  python3 zaarhub-hours-acceptance.py --list f   # one URL per line, expect hours
"""
import re
import subprocess
import sys
import urllib.request

DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (builds-hours-acceptance)"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.status, r.read().decode("utf-8", "replace")


def hours_block(html):
    """Return the rendered hours as a compact string, or '' when the page has no block."""
    if 'class="row hours"' not in html:
        return ""
    if "Open 24 hours" in html:
        return "Open 24 hours"
    days = re.findall(
        r'<div class="hours-day">([^<]+)</div><div class="hours-time"[^>]*>([^<]+)</div>', html
    )
    if not days:
        return ""
    return " ".join(f"{d[:3]} {t}" for d, t in days)


def jsonld_hours(html):
    """Return the openingHoursSpecification list from the LocalBusiness block, or None."""
    for blob in re.findall(r'<script type="application/ld\+json">(.*?)</script>', html, re.S):
        if "LocalBusiness" in blob and "openingHoursSpecification" in blob:
            import json
            try:
                data = json.loads(blob)
            except Exception:
                return "unparseable"
            for node in data if isinstance(data, list) else [data]:
                if isinstance(node, dict) and "openingHoursSpecification" in node:
                    return node["openingHoursSpecification"]
    return None


def canonical(html):
    m = re.search(r'<link rel="canonical" href="([^"]+)"', html)
    return m.group(1) if m else ""


def check(url, expect_hours):
    status, html = fetch(url)
    h1 = len(re.findall(r"<h1[ >]", html))
    can = canonical(html)
    hours = hours_block(html)
    ld = jsonld_hours(html)
    problems = []
    if status != 200:
        problems.append(f"status {status}")
    if h1 != 1:
        problems.append(f"h1={h1}")
    if can.rstrip("/") != url.rstrip("/"):
        problems.append(f"canonical={can}")
    if expect_hours:
        if not hours:
            problems.append("no hours block")
        elif hours != "Open 24 hours" and not all(d[:3] in hours for d in DAYS):
            problems.append("block missing days")
        if not ld:
            problems.append("no openingHoursSpecification")
    else:
        if hours:
            problems.append("hours block on an unlinked page")
        if "Hours unknown" in html or "hours unknown" in html.lower():
            problems.append("placeholder text")
        if ld:
            problems.append("openingHoursSpecification without hours")
    print(f"{'OK ' if not problems else 'FAIL'} {url} | {hours or '-'} | ld={'yes' if ld else 'no'}"
          + (f" | {'; '.join(problems)}" if problems else ""))
    return not problems


def db_fingerprint():
    out = subprocess.run(
        ["bash", "/root/.hermes/profiles/builds/cache/scratch/qmd.sh", "-tA", "-F", "|", "-c",
         "select 'businesses', count(*), md5(string_agg(b::text,'|' order by b.id)) from businesses b "
         "union all select 'google_places_cache', count(*), md5(string_agg(c::text,'|' order by c.id)) from google_places_cache c"],
        capture_output=True, text=True, timeout=180)
    return out.stdout.strip()


def main():
    urls = []
    if len(sys.argv) >= 3 and sys.argv[1] == "--list":
        urls = [u.strip() for u in open(sys.argv[2]) if u.strip()]
    else:
        base = "/root/.hermes/profiles/builds/cache/scratch"
        urls = [u.strip() for u in open(f"{base}/sample20.txt") if u.strip()]
    for u in urls:
        pass
    before = db_fingerprint()
    ok = True
    for u in urls:
        ok &= check(u, expect_hours=True)
    nofile = "/root/.hermes/profiles/builds/cache/scratch/nohours20.txt"
    try:
        for u in [x.strip() for x in open(nofile) if x.strip()]:
            ok &= check(u, expect_hours=False)
    except FileNotFoundError:
        pass
    after = db_fingerprint()
    print("db before:", before.replace("\n", " ; "))
    print("db after :", after.replace("\n", " ; "))
    if before != after:
        print("FAIL rows changed while serving pages")
        ok = False
    print("RESULT:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
