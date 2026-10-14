#!/usr/bin/env python3
"""ZaarHub sponsored articles — reachable on screen, end-to-end proof (queue item 6).

Proves, with a real row + real HTTP + a real browser, that a PUBLISHED sponsored/SEO
article (a `business_articles` row) is reachable by a visitor who BROWSEs the city —
not merely by typing its URL:

  1. fixture   a transient published `business_articles` row in the probe city
  2. browse(SSR)    GET /<city>/blog                            -> lists it, links to /<city>/articles/<slug>
  3. browse(feed)   GET /api/v1/zaarhub/cities/<city>/blog-posts -> a row with type="article" and that url
  4. article page   GET /<city>/articles/<slug>                  -> 200, title + body on the page
  5. RSS            GET /api/v1/public/directories/<city>/articles.xml -> advertises it, no dead per-city host
  6. real Chromium  the city blog page shows the article, a CLICK lands on the article and the
                    title + body are actually laid out on screen (not hidden).

Why this shape: the shipped defect (commit d3e3652) was that a published sponsored article
was live at /<city>/articles/<slug> but absent from BOTH browse surfaces, so no visitor could
reach it. A URL-only check passes on the broken build, so the browse + click arms are the test.

Run:  python3 scripts/sponsored-article-smoke.py
Exit: 0 = PASS, 1 = FAIL.  The fixture is disposable and removed on every path (net zero).
"""
import json
import os
import re
import subprocess
import sys
import time

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")          # the app itself
PUBLIC = os.environ.get("MD_PUBLIC", "https://zaarhub.com")         # the live public host
CITY = os.environ.get("MD_CITY", "palm-bay")
UA = "Mozilla/5.0 (compatible; SwiftOps/1.0)"
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "sponsored-article-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item6-sponsored-{time.strftime('%Y%m%d-%H%M%S')}")
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


PW = db_password()
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT)\b")


def psql(sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    return [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]


def one(sql):
    rows = psql(sql)
    return rows[0] if rows else ""


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def chrome_path():
    import glob
    if os.environ.get("CHROME_PATH"):
        return os.environ["CHROME_PATH"]
    cands = sorted(glob.glob("/root/.cache/puppeteer/chrome/*/chrome-linux64/chrome"))
    return cands[-1] if cands else "/usr/bin/chromium-browser"


def http(url, timeout=40):
    r = sh("curl", "-s", "-m", str(timeout), "-A", UA, "-w", "\n__CODE__%{http_code}", url)
    body, _, code = r.stdout.rpartition("__CODE__")
    return code.strip(), body


SLUG = ""


def sweep():
    # a disposable fixture only: our own slug pattern, never a real article.
    psql("DELETE FROM business_articles WHERE slug LIKE 'zz-sponsored-probe-%'")


def main():
    global SLUG
    os.makedirs(AUD, exist_ok=True)
    sweep()
    ts = str(int(time.time()))
    SLUG = f"zz-sponsored-probe-{ts}"
    TITLE = f"ZZ Sponsored Probe Roofing Specials {ts}"
    MARKER = f"SPONSORED-PROBE-MARKER-{ts}"
    BODY = f"<p>{MARKER} — the sponsored body copy a visitor must be able to read.</p>"

    dir_id = one(f"SELECT id FROM directories WHERE slug='{CITY}'")
    check("the probe city resolves", bool(dir_id), f"city={CITY} id={dir_id[:8]}")
    if not dir_id:
        return finish(0, [], "")

    before = one("SELECT count(*) FROM business_articles")

    aid = one(
        "INSERT INTO business_articles (directory_id, title, slug, keyword, meta_description, "
        f"content, status) VALUES ('{dir_id}', '{TITLE}', '{SLUG}', 'roofing', "
        f"'A sponsored article about roofing in this city.', '{BODY}', 'published') RETURNING id")
    check("a published sponsored article fixture exists", bool(aid), f"slug={SLUG}")

    # 2) the SSR browse surface a visitor lands on
    code, html = http(f"{PUBLIC}/{CITY}/blog")
    check("the city blog index answers 200", code == "200", f"status={code}")
    check("browsing the city LISTS the sponsored article", SLUG in html,
          "the city blog index links to it")
    check("the listing points at the article's own page", f"/{CITY}/articles/{SLUG}" in html,
          f"/{CITY}/articles/<slug>")

    # 3) the SPA's own city-blog feed
    code, feed = http(f"{BASE}/api/v1/zaarhub/cities/{CITY}/blog-posts")
    row = {}
    if code == "200":
        try:
            for p in json.loads(feed).get("posts", []):
                if p.get("slug") == SLUG:
                    row = p
        except Exception:
            pass
    check("the city blog FEED answers 200", code == "200", f"status={code}")
    check("the feed carries the article as a sponsored row (type=article)",
          row.get("type") == "article", f"type={row.get('type')}")
    check("the feed row links to the article page",
          f"/{CITY}/articles/{SLUG}" in (row.get("url") or ""), f"url={row.get('url')}")

    # 4) the article page itself
    code, art = http(f"{PUBLIC}/{CITY}/articles/{SLUG}")
    check("the article page answers 200", code == "200", f"status={code}")
    check("the article TITLE is on the page", TITLE in art)
    check("the sponsored BODY is on the page", MARKER in art)

    # 5) RSS (already claimed these before the fix — keep it honest)
    code, xml = http(f"{PUBLIC}/api/v1/public/directories/{CITY}/articles.xml")
    check("the articles RSS advertises the article", code == "200" and SLUG in xml, f"status={code}")
    check("the RSS advertises only the live host", f"{CITY}.directory.swiftsoftware.net" not in xml)

    # 6) real browser: browse -> click -> on screen
    env = dict(os.environ, PUBLIC=PUBLIC, CITY=CITY, SLUG=SLUG, TITLE=TITLE, MARKER=MARKER,
               AUD=AUD, NODE_PATH=NODE_PATH, CHROME_PATH=chrome_path())
    check("a real browser is available", os.path.exists(env["CHROME_PATH"]),
          os.path.basename(env["CHROME_PATH"]))
    r = sh("node", BROWSER, env=env, timeout=240)
    proof_path = os.path.join(AUD, "browser-proof.json")
    proof = json.load(open(proof_path)) if os.path.exists(proof_path) else {}
    if not proof:
        print(r.stdout[-600:]); print(r.stderr[-600:])
    check("the browser reached the city blog page", proof.get("browseLoaded") is True,
          f"status={proof.get('browseStatus')}")
    check("the visitor can SEE the sponsored article while browsing", proof.get("listed") is True)
    check("clicking the article lands on the article page", proof.get("clickedThrough") is True,
          f"url={proof.get('landedUrl')}")
    check("the article TITLE is visible on screen after the click", proof.get("titleVisible") is True)
    check("the sponsored BODY is visible and laid out on screen", proof.get("bodyLaidOut") is True)
    check("the page ran with no script errors", proof.get("pageErrors") == [],
          json.dumps(proof.get("pageErrors"))[:200])

    psql(f"DELETE FROM business_articles WHERE id='{aid}'")
    after = one("SELECT count(*) FROM business_articles")
    check("probe data fully removed (net zero)", after == before, f"before={before} after={after}")

    return finish(1 if FAILS else 0, FAILS, aid)


def finish(rc, fails, aid):
    print("\nRESULT: " + ("PASS" if rc == 0 else "FAIL"))
    return rc


if __name__ == "__main__":
    try:
        sys.exit(main())
    finally:
        if SLUG:
            try:
                sweep()
            except Exception:
                pass
