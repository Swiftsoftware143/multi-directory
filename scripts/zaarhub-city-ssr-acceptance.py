#!/usr/bin/env python3
"""ZaarHub city-page SSR acceptance gate.

Proves, on the SERVED HTML (no JavaScript), the five things zaarhub-ceo asked for
(bus seq 980) plus the SEO invariants that must NOT change.

SCOPE OF THE H1 CHECK (do not "fix" a passing page):
  Every check below runs on the RAW DOCUMENT with <script>/<style> stripped, i.e.
  it measures what a client that does not execute JS receives. That is deliberate.
  The H1 text "<City>, FL Business Directory" is a NO-JS claim: Google executes JS,
  and after hydration the SPA replaces the shell and the H1 becomes the app's own
  heading ("Discover & Support <City>"). Both are a single H1, so the gate holds
  either way -- but never cite the raw-document H1 as the text Google indexes, and
  never rewrite the SSR shell to force its H1 to survive hydration. Hydration
  replacing the SSR body is the designed behaviour (verified in Chromium).

Usage:
  # against the app directly (bypasses Cloudflare; the public host is the Host header)
  python3 zaarhub-city-ssr-acceptance.py
  python3 zaarhub-city-ssr-acceptance.py --host zaarhub.com --port 8089
  # against the live public site
  python3 zaarhub-city-ssr-acceptance.py --url https://zaarhub.com

Exit 0 = every city passed. Exit 1 = at least one criterion failed (printed per city).
Read-only: one GET per page, no writes anywhere.
"""
import argparse
import re
import sys
import urllib.request

CITIES = [
    "apopka", "boca-raton", "hollywood", "lake-nona", "palm-bay",
    "palm-coast", "pompano-beach", "st-cloud", "st-petersburg", "winter-garden",
]
CITY_NAMES = {
    "apopka": "Apopka", "boca-raton": "Boca Raton", "hollywood": "Hollywood",
    "lake-nona": "Lake Nona", "palm-bay": "Palm Bay", "palm-coast": "Palm Coast",
    "pompano-beach": "Pompano Beach", "st-cloud": "St. Cloud",
    "st-petersburg": "St. Petersburg", "winter-garden": "Winter Garden",
}
UA = "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"
# UNITS: "visible" is DECODED CHARACTERS of visible text (script/style stripped), never words.
# apopka 3,681 chars is ~630 words of the same HTML. Reporting "3,681 words" overstates content
# volume ~5.8x -- quote the char figure and the target as "3,000 characters" or not at all.
MIN_VISIBLE = 3000
MIN_BUSINESSES = 6
MIN_LINKS = 10


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=20) as r:
        return r.status, r.read().decode("utf-8", "replace")


def strip_js_css(html):
    h = re.sub(r"<script\b.*?</script>", " ", html, flags=re.S | re.I)
    h = re.sub(r"<style\b.*?</style>", " ", h, flags=re.S | re.I)
    h = re.sub(r"<!--.*?-->", " ", h, flags=re.S)
    return h


def visible_text(html):
    t = re.sub(r"<[^>]+>", " ", strip_js_css(html))
    t = re.sub(r"&nbsp;?", " ", t)
    t = re.sub(r"\s+", " ", t).strip()
    return t


def check_city(slug, html):
    body = strip_js_css(html)
    vis = visible_text(html)
    name = CITY_NAMES[slug]

    # Business cards: the SSR cards link /{dir}/businesses/{slug}; count distinct hrefs.
    biz_hrefs = sorted(set(re.findall(r'href="([^"]*/businesses/[^"#?]+)"', body)))
    # Category links: /{dir}/{category}
    cat_hrefs = sorted(set(
        re.findall(r'href="(?:https://zaarhub\.com)?/' + re.escape(slug) + r'/([a-z0-9-]+)"', body)
    ))
    cat_hrefs = [c for c in cat_hrefs if c != "businesses"]

    h1s = re.findall(r"<h1\b[^>]*>(.*?)</h1>", body, flags=re.S | re.I)
    h1_text = [re.sub(r"\s+", " ", re.sub(r"<[^>]+>", "", x)).strip() for x in h1s]

    canonical = re.findall(r'<link[^>]+rel="canonical"[^>]+href="([^"]+)"', html, flags=re.I)
    robots = re.findall(r'<meta[^>]+name="robots"[^>]+content="([^"]+)"', html, flags=re.I)

    fails = []
    if len(biz_hrefs) < MIN_BUSINESSES:
        fails.append(f"businesses {len(biz_hrefs)} < {MIN_BUSINESSES}")
    if len(biz_hrefs) + len(cat_hrefs) < MIN_LINKS:
        fails.append(f"listing/category links {len(biz_hrefs) + len(cat_hrefs)} < {MIN_LINKS}")
    if len(vis) < MIN_VISIBLE:
        fails.append(f"visible text {len(vis)} < {MIN_VISIBLE}")
    if len(h1s) != 1:
        fails.append(f"h1 count {len(h1s)} != 1")
    elif name.lower() not in h1_text[0].lower():
        fails.append(f"h1 {h1_text[0]!r} does not name {name}")
    if len(canonical) != 1:
        fails.append(f"canonical count {len(canonical)} != 1")
    elif not re.search(r"zaarhub\.com/" + re.escape(slug) + r"/?$", canonical[0]):
        fails.append(f"canonical not self-referential: {canonical[0]}")
    if robots and "noindex" in robots[0].lower():
        fails.append(f"robots is noindex: {robots[0]}")

    # Composition, not just size: the intro paragraph must be CITY-SPECIFIC.
    # A char count cannot tell real prose from a templated block, so collect the
    # long sentences and let main() assert none is shared between two cities.
    # LIMIT (do not oversell this arm): on these pages the >=100-char sentences are
    # business-card concatenations (name/rating/address), so it catches a shared
    # templated 100+ char block and nothing more. It does NOT prove per-city intro
    # prose exists -- that is proved by the distinct intro_copy text on the page.
    long_sents = sorted({
        s for s in re.split(r"(?<=[.!?])\s+", vis) if len(s) >= 100
    })

    return {
        "slug": slug,
        "visible": len(vis),
        "biz": len(biz_hrefs),
        "cat": len(cat_hrefs),
        "h1": h1_text,
        "long_sents": long_sents,
        "canonical": canonical[0] if canonical else None,
        "robots": robots[0] if robots else None,
        "fails": fails,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="zaarhub.com")
    ap.add_argument("--port", default="8089")
    ap.add_argument("--url", default="")
    ap.add_argument("--slug", default="", help="check one city only")
    a = ap.parse_args()

    slugs = [a.slug] if a.slug else CITIES
    rows, failed = [], 0
    for slug in slugs:
        if a.url:
            url = f"{a.url.rstrip('/')}/{slug}"
        else:
            url = f"http://127.0.0.1:{a.port}/{slug}"
        try:
            req = urllib.request.Request(
                url, headers={"User-Agent": UA, "Host": a.host} if not a.url else {"User-Agent": UA}
            )
            with urllib.request.urlopen(req, timeout=20) as r:
                status, html = r.status, r.read().decode("utf-8", "replace")
        except Exception as e:  # noqa: BLE001
            rows.append((slug, f"FETCH FAILED: {e}", []))
            failed += 1
            continue
        res = check_city(slug, html)
        rows.append((slug, status, res))

    # Cross-city composition: a long sentence that appears on two city pages is a
    # duplicated block, which is exactly what a size-only gate accepts and Google
    # does not. Assert zero shared long sentences between any two cities.
    owner = {}
    for slug, _status, res in rows:
        if isinstance(res, dict):
            for s in res["long_sents"]:
                owner.setdefault(s, []).append(slug)
    for slug, _status, res in rows:
        if not isinstance(res, dict):
            continue
        shared = [(s, v) for s, v in owner.items() if len(v) > 1 and slug in v]
        if shared:
            others = sorted({o for _s, v in shared for o in v if o != slug})
            res["fails"].append(
                f"{len(shared)} long sentence(s) shared with {', '.join(others)}"
            )
    failed = sum(1 for _s, _st, r in rows if isinstance(r, dict) and r["fails"])

    print(f"{'city':15s} {'HTTP':4s} {'visible':>7s} {'biz':>4s} {'cat':>4s} {'h1':>3s}  verdict")
    for slug, status, res in rows:
        if isinstance(res, list):
            print(f"{slug:15s} {str(status):4s}  {res}")
            continue
        verdict = "PASS" if not res["fails"] else "FAIL — " + "; ".join(res["fails"])
        print(
            f"{slug:15s} {status!s:4s} {res['visible']:7d} {res['biz']:4d} {res['cat']:4d} "
            f"{len(res['h1']):3d}  {verdict}"
        )

    print()
    print(f"targets: visible>={MIN_VISIBLE}, businesses>={MIN_BUSINESSES}, "
          f"business+category links>={MIN_LINKS}, exactly 1 H1 naming the city, "
          f"1 self-referential canonical, no noindex, ZERO long sentences shared between cities")
    print(f"RESULT: {'PASS' if failed == 0 else f'FAIL ({failed}/{len(rows)} cities)'}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
