#!/usr/bin/env python3
"""ZaarHub sponsored/SEO articles — the whole lifecycle, on screen (queue item 6).

Two arms, both against the RUNNING app and the LIVE hostname:

  A. HTTP arm — the fix that this run shipped, proven directly:
     a `business_articles` fixture is served at `/<city>/articles/<slug>` and
     `GET /api/v1/articles/<slug>` ONLY while it is `published`. Flipped to `draft` the
     title/body must vanish from the page; flipped to `archived` likewise. (Before the fix a
     draft was served 200 with its full copy — measured, see EVIDENCE in the audit dir.)

  B. Browser arm — the operator's own path through the console: sign in, open the Sponsored &
     SEO Articles card, pick the city, click Generate, see the draft listed with NO public link,
     Publish it, see a logged-out browser read the title + body, then Archive it and see it go.

Net zero: every fixture is removed; the article count is compared before and after.

Run:  python3 scripts/sponsored-article-lifecycle-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import json
import os
import re
import subprocess
import sys
import time

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
PUBLIC = os.environ.get("MD_PUBLIC", "https://zaarhub.com")
CITY = os.environ.get("MD_CITY", "palm-bay")
UA = "Mozilla/5.0 (compatible; SwiftOps/1.0)"
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "sponsored-article-lifecycle-browser.js")
MINT = "/opt/swift/browser-run/md-mint-token.js"
NODE_PATH = "/opt/swift/browser-run/node_modules"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item6-lifecycle-{time.strftime('%Y%m%d-%H%M%S')}")
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
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql[:200]}")
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


MARKERS = []


def sweep():
    # only ever our own probe rows
    psql("DELETE FROM business_articles WHERE slug LIKE 'zz-lifecycle-%'")


def token():
    r = sh("node", MINT)
    if r.returncode != 0:
        return ""
    try:
        return open("/tmp/.mdtok").read().strip()
    except OSError:
        return r.stdout.strip()


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep()
    ts = str(int(time.time()))
    dir_id = one(f"SELECT id FROM directories WHERE slug='{CITY}'")
    city_name = one(f"SELECT city FROM directories WHERE slug='{CITY}'")
    check("the probe city resolves", bool(dir_id and city_name), f"city={CITY} id={dir_id[:8]} name={city_name}")
    if not dir_id:
        return finish(1)
    before = one("SELECT count(*) FROM business_articles")

    # ── A. HTTP arm: only a published article is public ────────────────────────────────────────
    SLUG = f"zz-lifecycle-probe-{ts}"
    TITLE = f"ZZ Lifecycle Probe Roofing {ts}"
    MARK = f"LIFECYCLE-MARKER-{ts}"
    aid = one(
        "INSERT INTO business_articles (directory_id, title, slug, keyword, meta_description, "
        f"content, status) VALUES ('{dir_id}', '{TITLE}', '{SLUG}', 'roofing', "
        f"'A probe article about roofing.', '<p>{MARK} body copy.</p>', 'published') RETURNING id")
    check("a PUBLISHED fixture exists", bool(aid), f"slug={SLUG}")

    code, html = http(f"{PUBLIC}/{CITY}/articles/{SLUG}")
    check("a published article answers 200 on the public host", code == "200", f"status={code}")
    check("its TITLE and BODY are served while published", TITLE in html and MARK in html)
    code, api = http(f"{BASE}/api/v1/articles/{SLUG}")
    check("the public article API serves it while published", code == "200" and MARK in api, f"status={code}")

    psql(f"UPDATE business_articles SET status='draft' WHERE id='{aid}'")
    code, html = http(f"{PUBLIC}/{CITY}/articles/{SLUG}")
    check("a DRAFT is not readable on the city blog url", MARK not in html and TITLE not in html,
          f"status={code} (the SPA shell answers 200, the copy must not be in it)")
    code, api = http(f"{BASE}/api/v1/articles/{SLUG}")
    check("the public article API refuses a DRAFT", code == "404" and MARK not in api, f"status={code}")

    psql(f"UPDATE business_articles SET status='archived' WHERE id='{aid}'")
    code, html = http(f"{PUBLIC}/{CITY}/articles/{SLUG}")
    check("an ARCHIVED article is not readable on the city blog url", MARK not in html and TITLE not in html,
          f"status={code}")
    code, blog = http(f"{PUBLIC}/{CITY}/blog")
    check("an unpublished article is not listed on the city blog either", SLUG not in blog, f"status={code}")
    psql(f"DELETE FROM business_articles WHERE id='{aid}'")

    # ── B. Browser arm: the operator's lifecycle through the console ───────────────────────────
    tok = token()
    check("a platform-operator session could be minted", bool(tok))
    if not tok:
        return finish(1)
    KEYWORD = f"zz lifecycle probe {ts}"
    SERVICE = "plumbing"
    env = dict(os.environ, MD_BASE=BASE, PUBLIC=PUBLIC, TOKEN=tok, DIR_ID=dir_id,
               CITY_NAME=city_name, CITY_SLUG=CITY, KEYWORD=KEYWORD, SERVICE=SERVICE,
               AUD=AUD, NODE_PATH=NODE_PATH, CHROME_PATH=chrome_path())
    check("a real browser is available", os.path.exists(env["CHROME_PATH"]), os.path.basename(env["CHROME_PATH"]))
    r = sh("node", BROWSER, env=env, timeout=420)
    proof_path = os.path.join(AUD, "lifecycle-proof.json")
    proof = json.load(open(proof_path)) if os.path.exists(proof_path) else {}
    if not proof:
        print(r.stdout[-800:])
        print(r.stderr[-800:])
    for c in proof.get("checks", []):
        check("console: " + c.get("name", "?"), c.get("ok"), c.get("detail", ""))
    if not proof:
        check("the console flow produced a verdict", False, "no lifecycle-proof.json")

    art = proof.get("steps", {}).get("article", {}) or {}
    check("the console generated a real article row", bool(art.get("id")) and bool(art.get("slug")),
          str(art.get("slug")))
    if art.get("id"):
        # after the console archived it, the row must be archived — and (arm A's rule) not public
        st = one(f"SELECT status FROM business_articles WHERE id='{art['id']}'")
        check("the article the console archived is archived in the database", st == "archived", f"status={st}")
        code, html = http(f"{PUBLIC}/{CITY}/articles/{art.get('slug')}")
        check("the archived article is not reachable on the public host",
              (art.get("title") or "zzz") not in html, f"status={code}")

    # ── net zero ───────────────────────────────────────────────────────────────────────────────
    sweep()
    if art.get("id"):
        psql(f"DELETE FROM business_articles WHERE id='{art['id']}'")
    after = one("SELECT count(*) FROM business_articles")
    check("probe data fully removed (net zero)", after == before, f"before={before} after={after}")
    return finish(1 if FAILS else 0)


def finish(rc):
    print("\nRESULT: " + ("PASS" if rc == 0 else "FAIL"))
    return rc


if __name__ == "__main__":
    print(f"audit dir: {AUD}")
    try:
        sys.exit(main())
    finally:
        try:
            sweep()
        except Exception:
            pass
