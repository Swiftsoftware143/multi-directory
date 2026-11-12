#!/usr/bin/env python3
"""Item 15 — the company Headquarters is ONE value (city+state), set once, shown on EVERY city page.

David's rule: a city needs no address of its own; the company has ONE headquarters ("Orlando, Florida")
entered once in Branding and every public city page renders it. It must be DATA, not a hardcoded string.

What this proves:
  1. the value is stored in the DB (zaarhub_site_config.headquarters) and equals "Orlando, Florida";
  2. the public company endpoint serves it WITHOUT auth (the footer fetches it anonymously);
  3. every city's page API exposes it as `directory.headquarters`;
  4. in a REAL browser, every city's page renders it — in the footer (zhFillCompany) AND in the
     city hero (the "🏢 Headquarters:" line) — so a user actually SEES it, not just the API.

Read-only: it writes nothing and changes nothing. Run: python3 scripts/headquarters-on-every-city-smoke.py
Exit 0 = pass, 1 = fail.
"""
import json
import os
import subprocess
import sys
import time
import urllib.request

BASE = "http://127.0.0.1:8089"
APP = "/opt/swift/apps/multi-directory"
UA = "Mozilla/5.0 (compatible; SwiftOps/1.0)"
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
EXPECT = "Orlando, Florida"
fails = 0


def check(name, ok, detail=""):
    global fails
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))
    if not ok:
        fails += 1


def dburl():
    return subprocess.run(
        ["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
        capture_output=True, text=True,
    ).stdout.strip()


def sql(q):
    pw = dburl().split("://", 1)[1].split(":", 1)[1].split("@", 1)[0]
    env = dict(os.environ, PGPASSWORD=pw)
    r = subprocess.run(
        ["psql", "-h", "127.0.0.1", "-U", "swift", "-d", "multi_directory", "-At", "-c", q],
        capture_output=True, text=True, env=env,
    )
    return r.stdout.strip() if r.returncode == 0 else None


def get(path, token=None):
    req = urllib.request.Request(BASE + path)
    req.add_header("User-Agent", UA)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.status, json.loads(r.read().decode())


def find_chrome():
    import shutil
    for cand in ("google-chrome", "chromium", "chromium-browser"):
        p = shutil.which(cand)
        if p:
            return p
    import glob
    for p in glob.glob("/root/.cache/puppeteer/chrome/*/chrome-linux*/chrome"):
        return p
    return ""


def main():
    # 1) the value is DATA in the DB, exactly "Orlando, Florida".
    hq = sql("SELECT COALESCE(NULLIF(trim(headquarters),''), '(blank)') FROM zaarhub_site_config LIMIT 1;")
    check("the DB holds one headquarters value", hq == EXPECT, hq)

    # 2) the public company endpoint serves it WITHOUT auth (the footer calls it anonymously).
    try:
        st, body = get("/api/v1/zaarhub/company")
        check("the public company endpoint answers 200 without auth", st == 200, f"status={st}")
        check("the company endpoint carries the headquarters", body.get("headquarters") == EXPECT,
              str(body.get("headquarters")))
    except Exception as e:  # noqa: BLE001
        check("the public company endpoint answers 200 without auth", False, str(e)[:120])

    # 3) every city page API exposes it as directory.headquarters.
    slugs = (sql("SELECT slug FROM directories WHERE network_id='aaaaaaaa-0000-0000-0000-000000000001' "
                 "ORDER BY is_primary DESC, slug;") or "").split()
    check("all ten cities are enumerated", len(slugs) == 10, str(len(slugs)))
    for slug in slugs:
        try:
            st, body = get(f"/api/v1/zaarhub/cities/{slug}")
            val = (body.get("directory") or {}).get("headquarters")
            check(f"{slug}: page API exposes the headquarters", st == 200 and val == EXPECT,
                  f"status={st} hq={val}")
        except Exception as e:  # noqa: BLE001
            check(f"{slug}: page API exposes the headquarters", False, str(e)[:100])

    # 4) a real browser renders it on screen for every city page (footer + hero).
    chrome = find_chrome()
    check("a Chromium browser is available", bool(chrome), chrome or "none")
    if chrome and len(slugs) == 10:
        js = os.path.join(APP, "scripts", "headquarters-on-every-city-browser.js")
        env = dict(os.environ, NODE_PATH=NODE_PATH, CHROME_PATH=chrome, CITY_SLUGS=",".join(slugs))
        r = subprocess.run(["node", js], capture_output=True, text=True, env=env, cwd=APP)
        for line in (r.stdout or "").splitlines():
            s = line.strip()
            if s.startswith(("[PASS]", "[FAIL]", "BROWSER")):
                print("  " + s)
        n_fail = (r.stdout or "").count("[FAIL]")
        check("the browser rendered the headquarters on every city page (footer + hero)",
              r.returncode == 0 and "BROWSER RESULT: all passed" in (r.stdout or "") and n_fail == 0,
              f"rc={r.returncode} fail={n_fail}")

    print("\nRESULT: " + ("PASS" if fails == 0 else f"FAIL ({fails} failed)"))
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
