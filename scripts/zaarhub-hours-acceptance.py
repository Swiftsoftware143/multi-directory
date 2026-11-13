#!/usr/bin/env python3
"""Re-runnable acceptance probe for opening hours on zaarhub listing pages (kanban t_2b630e2a).

Read-only against the live site and the DB. Prints one line per URL plus a PASS/FAIL summary,
and fails if any row changed while the pages were served.

  python3 zaarhub-hours-acceptance.py                 # the fixed 20 + 5 sample below
  python3 zaarhub-hours-acceptance.py --list urls.txt # one URL per line, all expected to show hours

The fixed sample was chosen 2026-11-13 from the DB: two listings per city root across all ten
roots that resolve to a cached schedule, and five whose only cache row has an empty
opening_hours (so they must render no block and no placeholder).
"""
import json
import re
import subprocess
import sys
import urllib.request

DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]

WITH_HOURS = [
    "https://zaarhub.com/apopka/businesses/a-nails-spa-ChIJ35Z1",
    "https://zaarhub.com/apopka/businesses/a-soft-touch-in-family-dental-care-ChIJhS9U",
    "https://zaarhub.com/boca-raton/businesses/001-salon-ChIJ92t9",
    "https://zaarhub.com/boca-raton/businesses/4ever-young-med-spa-wellness-center-boca-raton-ChIJ8e4R",
    "https://zaarhub.com/hollywood/businesses/3-island-g-s-mobile-cuisine-ChIJccQe",
    "https://zaarhub.com/hollywood/businesses/7-tables-on-the-beach-authentic-peruvian-cuisine-ChIJZUUd",
    "https://zaarhub.com/lake-nona/businesses/4-rivers-catering-ChIJg8KY",
    "https://zaarhub.com/lake-nona/businesses/407-cafe-ChIJW-yI",
    "https://zaarhub.com/palm-bay/businesses/1-tom-plumber-melbourne-ChIJ03NI",
    "https://zaarhub.com/palm-bay/businesses/321-boat-rentals-club-ChIJDZ3Z",
    "https://zaarhub.com/palm-coast/businesses/1st-point-realty-of-fl-inc-ChIJUfh1",
    "https://zaarhub.com/palm-coast/businesses/5th-element-palm-coast-ChIJd3i3",
    "https://zaarhub.com/pompano-beach/businesses/100-chiropractic-pompano-beach-ChIJqW-W",
    "https://zaarhub.com/pompano-beach/businesses/360-roofing-services-inc-ChIJ925b",
    "https://zaarhub.com/st-cloud/businesses/3-lil-birds-tattoo-gallery-ChIJxW0V",
    "https://zaarhub.com/st-cloud/businesses/40-thieves-tattoo-and-art-gallery-llc-ChIJwyOv",
    "https://zaarhub.com/st-petersburg/businesses/1-tom-plumber-ChIJh3K8",
    "https://zaarhub.com/st-petersburg/businesses/10-stars-property-management-greater-clearwater-st-petersburg-tampa-bay-ChIJd0sJ",
    "https://zaarhub.com/winter-garden/businesses/4-paws-animal-clinic-ChIJVVVV",
    "https://zaarhub.com/winter-garden/businesses/a-budget-tree-service-ChIJu57o",
]

# Every cache row matching these businesses has an empty opening_hours.
NO_HOURS = [
    "https://zaarhub.com/apopka/businesses/a-h-gantt-cpa-associates-pa-ChIJ4eCr",
    "https://zaarhub.com/apopka/businesses/china-s-sunday-table-ChIJPW3e",
    "https://zaarhub.com/apopka/businesses/gmgm-accounting-services-ChIJFfTT",
    "https://zaarhub.com/apopka/businesses/green-valley-landscape-management-ChIJq6q6",
    "https://zaarhub.com/apopka/businesses/limbachia-pari-dc-ChIJQRit",
]


def fetch(url):
    req = urllib.request.Request(
        url, headers={"User-Agent": "Mozilla/5.0 (builds-hours-acceptance)"}
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.status, r.read().decode("utf-8", "replace")


def hours_block(html):
    """The rendered hours as a compact string, or '' when the page has no block."""
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
    """openingHoursSpecification from the LocalBusiness block, or None."""
    for blob in re.findall(r'<script type="application/ld\+json">(.*?)</script>', html, re.S):
        if "LocalBusiness" in blob and "openingHoursSpecification" in blob:
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
            problems.append("hours block on a listing with no cached hours")
        if "hours unknown" in html.lower():
            problems.append("placeholder text")
        if ld:
            problems.append("openingHoursSpecification without hours")
    print(
        f"{'OK ' if not problems else 'FAIL'} {url} | {hours or '-'} | ld={'yes' if ld else 'no'}"
        + (f" | {'; '.join(problems)}" if problems else "")
    )
    return not problems


def db_fingerprint():
    out = subprocess.run(
        ["bash", "/root/.hermes/profiles/builds/cache/scratch/qmd.sh", "-tA", "-F", "|", "-c",
         "select 'businesses', count(*), md5(string_agg(b::text,'|' order by b.id)) from businesses b "
         "union all select 'google_places_cache', count(*), md5(string_agg(c::text,'|' order by c.id)) "
         "from google_places_cache c"],
        capture_output=True, text=True, timeout=180)
    return out.stdout.strip()


def main():
    with_hours, no_hours = WITH_HOURS, NO_HOURS
    if len(sys.argv) >= 3 and sys.argv[1] == "--list":
        with_hours = [u.strip() for u in open(sys.argv[2]) if u.strip()]
        no_hours = []
    before = db_fingerprint()
    ok = True
    for u in with_hours:
        ok &= check(u, expect_hours=True)
    for u in no_hours:
        ok &= check(u, expect_hours=False)
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
