#!/usr/bin/env python3
"""Sector template pages — ESCAPING proof (live public URLs; ZaarHub).

`/templates/<sector>.html?slug=<directory>` served 200 and rendered a directory's business rows
straight into `innerHTML` (cards, map popups, filter labels). A business owner chooses their own
name, city, address, phone, website and `meta` text, so one renamed listing was a live payload on
a public URL for every visitor — the same defect class as the /biz/:id review XSS (1b8e7c2), on
the widest surface in the app. It had never been proven either way.

This probe proves, with real rows + a real browser:
  1. a listing whose NAME + CITY are HTML payloads is really returned by the page's own API;
  2. a real browser on /templates/farm.html?slug=<probe dir> shows both as literal text;
  3. nothing is created from them and no injected handler runs;
  4. a control interpolates the same payload raw and DOES fire — so a pass is not vacuous.

The only direct DB writes are the disposable probe directory + listing, removed on every path.
Run:  python3 scripts/sector-template-xss-escape-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
MARK = "sector-template-xss-probe"
HERE = os.path.dirname(os.path.abspath(__file__))
BROWSER = os.path.join(HERE, "sector-template-xss-escape-browser.js")
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
AUD = os.environ.get("SHOT_DIR", "/opt/swift/audits/zz-item1-sector-templates-xss-20261016")
FAILS = []

PAYLOAD_NAME = '<img src=x onerror="window.__xssProbe=1">XSS-SECTOR-NAME'
PAYLOAD_CITY = '<img src=y onerror="window.__xssName=1">XSS-CITY'


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
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    return lines[0] if lines else ""


def chrome_path():
    import glob
    if os.environ.get("CHROME_PATH"):
        return os.environ["CHROME_PATH"]
    cands = sorted(glob.glob("/root/.cache/puppeteer/chrome/*/chrome-linux64/chrome"))
    return cands[-1] if cands else "/usr/bin/chromium-browser"


def sweep():
    psql("DELETE FROM businesses WHERE probe_harness='" + MARK + "'")
    psql("DELETE FROM directories WHERE slug LIKE 'zz-xss-probe-%'")


def http(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            raw = resp.read().decode()
            try:
                return resp.status, json.loads(raw)
            except json.JSONDecodeError:
                return resp.status, raw
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:200]


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep()
    ts = str(int(time.time()))
    slug = f"zz-xss-probe-{ts}"

    did = psql("INSERT INTO directories (name, slug, status, is_primary) VALUES "
               f"('ZZ XSS Probe {ts}', '{slug}', 'active', false) RETURNING id")
    check("disposable probe directory created", bool(did), f"slug={slug}")
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, "
        f"probe_harness) VALUES ('{did}', '{PAYLOAD_NAME}', 'zz-xss-biz-{ts}', '{PAYLOAD_CITY}', 'FL', "
        f"'active', 0, 0, '{MARK}') RETURNING id")
    check("disposable listing created with payload NAME + CITY", bool(bid), f"id={bid}")

    st, data = http("GET", f"/api/v1/directories/{slug}/businesses?page=1&per_page=50")
    rows = (data or {}).get("data") if isinstance(data, dict) else data
    names = [r.get("name") for r in (rows or [])]
    cities = [r.get("city") for r in (rows or [])]
    check("the page's own API really returns the payload listing (fixture visible)",
          st == 200 and PAYLOAD_NAME in names, f"status={st} rows={len(rows or [])}")
    check("the API returns the payload city verbatim", PAYLOAD_CITY in cities)

    # the live page really serves the fixed file
    st_page, page_html = http("GET", f"/templates/farm.html?slug={slug}")
    check("the sector page answers 200 (it is a live public URL)",
          st_page == 200 and isinstance(page_html, str) and "escDeep" in page_html,
          f"status={st_page}")

    env = dict(os.environ, MD_URL=BASE, SLUG=slug, PAYLOAD_NAME=PAYLOAD_NAME,
               PAYLOAD_CITY=PAYLOAD_CITY, SHOT_DIR=AUD, CHROME_PATH=chrome_path(),
               NODE_PATH=NODE_PATH)
    r = sh("node", BROWSER, env=env)
    with open(os.path.join(AUD, "browser.log"), "w") as fh:
        fh.write(r.stdout + r.stderr)
    print(r.stdout.strip())
    if r.stderr.strip():
        print("  (stderr) " + r.stderr.strip()[:400])
    check("the browser flow passed (payload shown as text, nothing executed, control fired)",
          r.returncode == 0, f"rc={r.returncode}")

    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    psql(f"DELETE FROM directories WHERE id='{did}'")
    left = psql("SELECT (SELECT count(*) FROM businesses WHERE id='%s')"
                "+(SELECT count(*) FROM directories WHERE id='%s')" % (bid, did))
    check("probe data fully removed", left == "0", f"remaining={left}")

    print()
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)}): " + "; ".join(FAILS))
        return 1
    print("RESULT: PASS")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # noqa: BLE001
        print(f"RESULT: FAIL (exception: {e})")
        sys.exit(1)
