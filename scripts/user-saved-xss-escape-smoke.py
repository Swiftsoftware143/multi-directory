#!/usr/bin/env python3
"""Shopper pages — ESCAPING proof (item 1, the /user/saved surface; ZaarHub).

`frontend/user-saved.html` (served at /user/saved) rendered its Saved / Following / Recommended
/ My Reviews / Perks cards with `innerHTML` and interpolated a business name, its category, a
review body and a perk's title/description RAW. A business owner chooses their own name, so one
renamed listing became a live payload on every shopper's saved list — the same defect class as
the /biz/:id review XSS (1b8e7c2), on a different public-ish surface. Neither had a durable proof.

This probe proves, with real rows + a real browser:
  1. a shopper account can save a listing whose NAME is an HTML payload and leave a review whose
     BODY is an HTML payload (both really stored, both really returned by the page's own APIs);
  2. a real browser signed in as that shopper loads /user/saved and shows BOTH as literal text;
  3. it creates no element from them and runs no injected handler;
  4. a control interpolates the same payload raw and DOES fire — so a pass is not vacuous.

The only direct DB writes are the disposable probe listing and teardown.
Run:  python3 scripts/user-saved-xss-escape-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Nothing is left behind on either path.
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
MARK = "user-saved-xss-probe"
HERE = os.path.dirname(os.path.abspath(__file__))
BROWSER = os.path.join(HERE, "user-saved-xss-escape-browser.js")
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
AUD = os.environ.get("SHOT_DIR", "/opt/swift/audits/zz-item1-usersaved-xss-20261016")
FAILS = []

PAYLOAD_NAME = '<img src=z onerror="window.__xssName=1">XSS-BIZ-NM'
PAYLOAD_CONTENT = '<img src=x onerror="window.__xssProbe=1">XSS-REVIEW-BODY'


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


def sweep_stale_probe_rows():
    psql("DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql("DELETE FROM visitor_favorites WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-us-xss-%@swiftsoftware.dev'")


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


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep_stale_probe_rows()
    ts = str(int(time.time()))
    email = f"zz-us-xss-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    slug = f"zz-us-xss-{ts}"
    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")

    st, _ = http("POST", "/api/v1/visitor/register", {"email": email, "password": pw, "name": "ZZ Saved Probe"})
    check("shopper account created", st in (200, 201), f"status={st}")
    vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{email}'")

    st, tok = http("POST", "/api/v1/auth/login", {"email": email, "password": pw})
    token = (tok or {}).get("access_token")
    check("shopper can sign in", st == 200 and bool(token), f"status={st}")

    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', '{PAYLOAD_NAME}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{MARK}') RETURNING id")
    check("disposable listing created with a payload NAME", bool(bid), f"id={bid}")
    stored_name = psql(f"SELECT name FROM businesses WHERE id='{bid}'")
    check("the payload name is stored verbatim", stored_name == PAYLOAD_NAME)

    st, _ = http("POST", f"/api/v1/visitor/favorites/{bid}", {}, token=token)
    check("the shopper can save it (POST /visitor/favorites/:id)", st in (200, 201), f"status={st}")

    st, rev = http("POST", "/api/v1/reviews",
                   {"business_id": bid, "rating": 5, "title": "saved probe",
                    "content": PAYLOAD_CONTENT, "reviewer_name": "ZZ Saved Probe"},
                   token=token)
    rid = (rev or {}).get("id") if isinstance(rev, dict) else None
    check("the shopper can leave a review with a payload BODY", st in (200, 201) and bool(rid), f"status={st}")

    # the page's own data sources really carry both payloads
    st, favs = http("GET", "/api/v1/visitor/favorites", token=token)
    fav_rows = (favs or {}).get("favorites") if isinstance(favs, dict) else favs
    names = [f.get("business_name") for f in (fav_rows or [])]
    check("the favorites API really returns the payload name (fixture visible to the page)",
          st == 200 and PAYLOAD_NAME in names, f"status={st}")

    st, mine = http("GET", "/api/v1/my-reviews", token=token)
    mine_ids = [r.get("id") for r in (mine or [])] if isinstance(mine, list) else []
    check("my-reviews really returns the payload review (fixture visible to the page)",
          st == 200 and rid in mine_ids, f"status={st}")

    # ── the browser arm ─────────────────────────────────────────────────────────
    env = dict(os.environ, MD_URL=BASE, TOKEN=token,
               PAYLOAD_NAME=PAYLOAD_NAME, PAYLOAD_CONTENT=PAYLOAD_CONTENT,
               SHOT_DIR=AUD, CHROME_PATH=chrome_path(), NODE_PATH=NODE_PATH)
    r = sh("node", BROWSER, env=env)
    with open(os.path.join(AUD, "browser.log"), "w") as fh:
        fh.write(r.stdout + r.stderr)
    print(r.stdout.strip())
    if r.stderr.strip():
        print("  (stderr) " + r.stderr.strip()[:400])
    check("the browser flow passed (both payloads shown as text, nothing executed, control fired)",
          r.returncode == 0, f"rc={r.returncode}")

    # ── teardown ────────────────────────────────────────────────────────────────
    psql(f"DELETE FROM reviews WHERE business_id='{bid}'")
    psql(f"DELETE FROM visitor_favorites WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    if vid:
        psql(f"DELETE FROM visitor_accounts WHERE id='{vid}'")
    left = psql(f"SELECT (SELECT count(*) FROM businesses WHERE id='{bid}')"
                f"+(SELECT count(*) FROM visitor_accounts WHERE lower(email)='{email}')"
                f"+(SELECT count(*) FROM reviews WHERE business_id='{bid}')"
                f"+(SELECT count(*) FROM visitor_favorites WHERE business_id='{bid}')")
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
