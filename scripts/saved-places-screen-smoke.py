#!/usr/bin/env python3
"""Durable proof — the shopper's Saved Places screen shows the businesses they saved.

The defect (measured 2026-10-15): a shopper could bookmark businesses, but every screen that shows
bookmarks answered "nothing saved":
  * `GET /api/v1/visitor/profile` returned a HARDCODED `"favorites": []`, so the Visitor Portal's
    "Favorites" section always said "No favorites yet";
  * the main app's Saved Places view (`frontend/index.html`) was defined but no hash route reached
    it, it read the `{favorites:[...]}` envelope as if it were the list, and it called the API
    without the shopper's token — so a shopper WITH bookmarks was told "No saved places yet";
  * the Visitor Portal's "⭐ Saved" link pointed at the server-rendered page, which can only read a
    bearer header a browser navigation cannot send, so it bounced straight back to the portal;
  * index.html referenced an `authUser` global and a `showAuthModal()` that were never defined, so
    "save this business" threw a ReferenceError and the heart never worked at all.

Two arms run against the live app:

  ARM A (data)    — real accounts + a real bookmark through the product routes, then the profile
                    call the screens read must contain that business (and the public listing).
  ARM B (browser) — `scripts/saved-places-browser.js` drives real Chromium: the shopper signs in
                    through the Visitor Portal's OWN form, sees the business in Favorites, clicks the
                    portal's OWN "⭐ Saved" link (must land on a page showing the business), then
                    opens the app's OWN "Saved" nav link and sees the same business on screen.

Run:  python3 scripts/saved-places-screen-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089), AUD, CHROME_PATH, NODE_PATH
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
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "saved-places-browser.js")
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-saved-places-{time.strftime('%Y%m%d-%H%M%S')}")
MARK = "zz-saved-probe"
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


PW_DB = db_password()
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT)\b")


def psql(sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW_DB}", "swift-postgres",
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


def http(method, path, body=None, token=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            raw = resp.read().decode()
            try:
                return resp.status, json.loads(raw)
            except json.JSONDecodeError:
                return resp.status, raw
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw)
        except json.JSONDecodeError:
            return e.code, raw


def chrome_path():
    import glob
    if os.environ.get("CHROME_PATH"):
        return os.environ["CHROME_PATH"]
    cands = sorted(glob.glob("/root/.cache/puppeteer/chrome/*/chrome-linux64/chrome"))
    return cands[-1] if cands else "/usr/bin/chromium-browser"


def sweep_stale():
    """A run that died before teardown must not leave junk behind."""
    psql("DELETE FROM visitor_favorites WHERE visitor_account_id IN "
         f"(SELECT id FROM visitor_accounts WHERE email LIKE '{MARK}-%@swiftsoftware.dev')")
    psql(f"DELETE FROM visitor_accounts WHERE email LIKE '{MARK}-%@swiftsoftware.dev'")


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep_stale()
    ts = str(int(time.time()))
    email = f"{MARK}-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"

    code = sh("curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", BASE + "/api/v1/health").stdout
    check("the app answers /api/v1/health", code == "200", f"status={code}")

    # ── fixture: a shopper account and one business they bookmark ─────────────────────────────
    st, _ = http("POST", "/api/v1/visitor/register",
                 {"email": email, "password": pw, "name": "ZZ Saved Probe"})
    check("shopper account can be created (POST /visitor/register)", st in (200, 201), f"status={st}")
    vid = one(f"SELECT id FROM visitor_accounts WHERE lower(email)='{email}'")
    check("the shopper exists in the database", bool(vid), vid)

    st, tok = http("POST", "/api/v1/auth/login", {"email": email, "password": pw})
    token = (tok or {}).get("access_token") if isinstance(tok, dict) else None
    check("the shopper can sign in", st == 200 and bool(token), f"status={st}")

    bid = one("SELECT id FROM businesses WHERE status='active' AND owner_id IS NOT NULL "
              "ORDER BY random() LIMIT 1") or one("SELECT id FROM businesses WHERE status='active' LIMIT 1")
    bname = one(f"SELECT name FROM businesses WHERE id='{bid}'")
    bcity = one(f"SELECT coalesce(city,'')||'|'||coalesce(state,'') FROM businesses WHERE id='{bid}'")
    check("a real business to bookmark", bool(bid and bname), f"{bid} {bname}")

    st, body = http("POST", f"/api/v1/visitor/favorites/{bid}", {}, token=token)
    saved = (body or {}).get("saved") if isinstance(body, dict) else None
    check("the shopper can bookmark a business (POST /visitor/favorites/:id)",
          st in (200, 201) and saved is True, f"status={st} body={str(body)[:80]}")

    # ── ARM A: the call the screens read must show the bookmark ───────────────────────────────
    st, prof = http("GET", "/api/v1/visitor/profile", token=token)
    favs = (prof or {}).get("favorites") if isinstance(prof, dict) else None
    check("GET /visitor/profile reports the saved business (was a hardcoded empty list)",
          st == 200 and isinstance(favs, list) and any(f.get("business_id") == bid for f in favs),
          f"status={st} favorites={len(favs or [])}")
    if favs:
        f0 = favs[0]
        check("that profile row carries the fields the screens render",
              bool(f0.get("business_name")) and f0.get("business_id") == bid,
              f"business_name={f0.get('business_name')!r}")

    st, listing = http("GET", "/api/v1/visitor/favorites", token=token)
    rows = (listing or {}).get("favorites") if isinstance(listing, dict) else None
    check("GET /visitor/favorites still lists it (the list endpoint keeps its shape)",
          st == 200 and any(f.get("business_id") == bid for f in (rows or [])),
          f"status={st} count={len(rows or [])}")

    db_row = one(f"SELECT count(*) FROM visitor_favorites WHERE visitor_account_id='{vid}' "
                 f"AND business_id='{bid}'")
    check("the bookmark is a real database row", db_row == "1", f"rows={db_row}")

    # ── ARM B: the screens themselves, in a real browser ──────────────────────────────────────
    env = dict(os.environ, BASE=BASE, TOKEN=token or "", EMAIL=email, PW=pw, BIZ_NAME=bname,
               BIZ_CITY=bcity, SHOT_DIR=AUD, CHROME_PATH=chrome_path(), NODE_PATH=NODE_PATH)
    r = sh("node", BROWSER, env=env, timeout=180)
    with open(os.path.join(AUD, "browser.log"), "w") as fh:
        fh.write(r.stdout + r.stderr)
    print(r.stdout.strip())
    if r.stderr.strip():
        print("  (stderr) " + r.stderr.strip()[:600])
    try:
        proof = json.loads(r.stdout[r.stdout.index("{"):r.stdout.rindex("}") + 1])
        with open(os.path.join(AUD, "browser-proof.json"), "w") as fh:
            json.dump(proof, fh, indent=1)
    except Exception:  # noqa: BLE001
        pass
    check("the browser flow passed (portal + app screens show the saved business)", r.returncode == 0,
          f"rc={r.returncode}")

    # ── teardown ──────────────────────────────────────────────────────────────────────────────
    psql(f"DELETE FROM visitor_favorites WHERE visitor_account_id='{vid}'")
    psql(f"DELETE FROM visitor_accounts WHERE id='{vid}'")
    left = one(f"SELECT (SELECT count(*) FROM visitor_accounts WHERE email='{email}')"
               f"+ (SELECT count(*) FROM visitor_favorites WHERE visitor_account_id='{vid}')")
    check("probe data fully removed", left == "0", f"remaining={left}")

    with open(os.path.join(AUD, "summary.json"), "w") as fh:
        json.dump({"when": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "fails": FAILS,
                   "business": bname, "result": "FAIL" if FAILS else "PASS"}, fh, indent=1)

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
