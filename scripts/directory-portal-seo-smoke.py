#!/usr/bin/env python3
"""Durable proof — the Directory Portal's SEO card (/portal.html -> SEO) really loads and really
saves the meta that a live city page renders.

The defect this pins (found 2026-10-14): the card read/wrote `seo_meta` under page_type='directory'
while every city page reads page_type='city' (src/handlers/subfolder.rs `seo_override(pool,"city",dir.id)`).
So the three fields came up blank even where a real city title/description was stored, and pressing
Save printed "SEO settings saved" while the live city page did not change: a false-success screen.

Proves, against the running app (real Chromium drives the page's own navigate('seo') / saveSEO()):
  1. the card SHOWS the meta title/description the city page actually renders (read from the DB under
     the same key the page reads);
  2. typing a title and pressing the card's own Save reports success AND the row the city page reads
     now holds it;
  3. the live city page's <title> contains what was typed;
  4. the original values are restored and re-verified, and no probe row is left behind.

Run:  python3 scripts/directory-portal-seo-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089), AUD, CHROME_PATH
Exit: 0 = PASS, 1 = FAIL.
"""
import base64
import hashlib
import hmac
import json
import os
import re
import subprocess
import sys
import time

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "directory-portal-seo-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-portal-seo-{time.strftime('%Y%m%d-%H%M%S')}")
UA = "Mozilla/5.0 (compatible; SwiftOps/1.0)"
PROBE = "ZZ Portal SEO probe " + str(int(time.time()))
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


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


def chrome_path():
    import glob
    if os.environ.get("CHROME_PATH"):
        return os.environ["CHROME_PATH"]
    cands = sorted(glob.glob("/root/.cache/puppeteer/chrome/*/chrome-linux64/chrome"))
    return cands[-1] if cands else "/usr/bin/chromium-browser"


def b64(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def operator_token():
    secret = sh("docker", "exec", "multidirectory", "printenv", "JWT_SECRET").stdout.strip()
    email = sh("docker", "exec", "multidirectory", "printenv", "OPERATOR_EMAIL").stdout.strip()
    row = one("SELECT id||'|'||tenant_id FROM users WHERE role='super_admin' AND is_active "
              "ORDER BY created_at LIMIT 1")
    if not row:
        row = one("SELECT u.id||'|'||u.tenant_id FROM users u JOIN directories d ON d.owner_id = u.id "
                  "WHERE u.is_active LIMIT 1")
    if not row or not secret:
        sys.exit("could not resolve the operator account or JWT_SECRET")
    uid, tid = row.split("|")[0].strip(), row.split("|")[1].strip()
    now = int(time.time())
    hh = b64(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    pp = b64(json.dumps({"sub": uid, "tid": tid, "role": "super_admin", "exp": now + 1800,
                         "iat": now, "aud": "multidirectory-api", "iss": "multidirectory"},
                        separators=(",", ":")).encode())
    sig = b64(hmac.new(secret.encode(), f"{hh}.{pp}".encode(), hashlib.sha256).digest())
    return f"{hh}.{pp}.{sig}"


def run_browser(mode, tok, extra_env=None):
    env = dict(os.environ, MODE=mode, TOKEN=tok, MD_BASE=BASE, PROBE_TITLE=PROBE,
               CHROME_PATH=chrome_path(), NODE_PATH=NODE_PATH, OUT=AUD)
    if extra_env:
        env.update(extra_env)
    r = sh("node", BROWSER, env=env)
    open(os.path.join(AUD, f"browser-{mode}.log"), "w").write(r.stdout + r.stderr)
    try:
        return json.loads(r.stdout)
    except Exception:
        print(r.stdout[-1500:])
        print(r.stderr[-1500:])
        sys.exit(f"browser {mode} run produced no JSON")


def page_title(path):
    r = sh("curl", "-s", "-m", "30", "-A", UA, BASE + path)
    m = re.search(r"<title[^>]*>(.*?)</title>", r.stdout, re.S | re.I)
    return (m.group(1).strip() if m else "")


def main():
    os.makedirs(AUD, exist_ok=True)
    tok = operator_token()
    code = sh("curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", BASE + "/api/v1/health").stdout
    check("the app answers /api/v1/health", code == "200", f"status={code}")

    read = run_browser("read", tok)
    dirinfo = read.get("dir") or {}
    dir_id, slug = dirinfo.get("id"), dirinfo.get("slug")
    check("the portal picked a directory to edit", bool(dir_id and slug), f"{slug} / {dir_id}")
    if not dir_id:
        return finish()

    row = one(f"SELECT title||chr(31)||coalesce(description,'')||chr(31)||coalesce(keywords,'') "
              f"FROM seo_meta WHERE page_type='city' AND page_id='{dir_id}'")
    parts = (row or chr(31) * 2).split(chr(31))
    orig_title, orig_desc, orig_kw = (parts + ["", "", ""])[:3]
    check("the city page has a stored meta title to compare against", bool(orig_title),
          f"db_title={orig_title!r}")

    shown = (read.get("fields") or {})
    check("the SEO card SHOWS the stored title (not a blank field)", shown.get("title") == orig_title,
          f"card={shown.get('title')!r} db={orig_title!r}")
    check("the SEO card SHOWS the stored description", (shown.get("desc") or "") == orig_desc,
          f"card={(shown.get('desc') or '')[:40]!r}")
    check("the card's meta call answered 200 (no 403/404 on the operator's own screen)",
          any(a["status"] == 200 and "/seo/" in a["url"] for a in read.get("api", [])),
          str([a for a in read.get("api", []) if "/seo/" in a["url"]]))
    check("the browser reported no page errors", not read.get("pageErrors"), str(read.get("pageErrors")))

    before_title = page_title("/" + slug)

    write = run_browser("write", tok)
    writes = [a for a in write.get("api", []) if a["method"] in ("PUT", "POST") and "/seo/" in a["url"]]
    check("pressing the card's Save really PUTs the meta", any(a["status"] == 200 for a in writes), str(writes))
    check("the card reported success to the operator", "saved" in (write.get("toast") or "").lower(),
          repr(write.get("toast")))

    stored = one(f"SELECT coalesce(title,'') FROM seo_meta WHERE page_type='city' AND page_id='{dir_id}'")
    check("the row the city page reads now holds what was typed", stored == PROBE, f"db={stored!r}")

    now_title = page_title("/" + slug)
    check("the LIVE city page <title> shows what was typed on the card", PROBE in now_title,
          f"before={before_title!r} after={now_title!r}")

    # restore
    esc = lambda s: s.replace("'", "''")
    psql(f"UPDATE seo_meta SET title='{esc(orig_title)}', description=NULLIF('{esc(orig_desc)}',''), "
         f"keywords=NULLIF('{esc(orig_kw)}',''), updated_at=now() "
         f"WHERE page_type='city' AND page_id='{dir_id}'")
    back = one(f"SELECT coalesce(title,'') FROM seo_meta WHERE page_type='city' AND page_id='{dir_id}'")
    check("the original title is restored in the DB", back == orig_title, f"db={back!r}")
    check("the live city page is back to its original title", page_title("/" + slug) == before_title,
          repr(page_title("/" + slug)))

    # residue: any row the probe created under a different key
    left = psql("SELECT page_type||'|'||page_id FROM seo_meta WHERE title LIKE 'ZZ Portal SEO probe%' "
                "OR description LIKE 'ZZ Portal SEO probe%'")
    check("no probe row is left in seo_meta", not left, str(left))
    for r in left:
        pt, _, pid = r.partition("|")
        psql(f"DELETE FROM seo_meta WHERE page_type='{pt}' AND page_id='{pid}' AND title LIKE 'ZZ Portal SEO probe%'")
    check("no probe text is left on the live page", PROBE not in page_title("/" + slug))

    return finish()


def finish():
    print()
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)}) :: " + "; ".join(FAILS[:6]))
        return 1
    print("RESULT: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
