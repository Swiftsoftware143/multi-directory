#!/usr/bin/env python3
"""Durable proof — item 1: a business OWNER approves a review of their own listing FROM THE SCREEN.

The browser arm (scripts/reviews-business-approve-browser.js) drives the real Business Portal
(/portal -> frontend/business-portal.html): it signs in through the page's own login form, opens the
portal's Reviews view, SEES the pending review of the owner's own listing, clicks the page's own
Approve button, and the row leaves the pending state while the browser shows a 200 from
POST /business/reviews/:id/approve. This driver mints that fixture through the real product routes,
asserts the database afterwards, proves the approved review is then publicly visible, and tears the
fixture down (nothing left behind on either path).

Why a browser arm and not just an API check: an endpoint that returns 200 proves nothing about
whether a real business owner can reach the control on their own portal.

Run:  python3 scripts/reviews-business-portal-approve-smoke.py
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
BROWSER = os.path.join(APP, "scripts", "reviews-business-approve-browser.js")
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-review-portal-approve-{time.strftime('%Y%m%d-%H%M%S')}")
HARNESS = {"X-Swift-Harness": "review-portal-approve-probe"}
MARK = "review-portal-approve-probe"
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
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    return lines[0] if lines else ""


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def http(method, path, body=None, token=None, extra_headers=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    for k, v in (extra_headers or {}).items():
        req.add_header(k, v)
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


def sweep_stale_probe_rows():
    """A run that died before teardown must not leave junk behind."""
    psql("DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-review-portal-%@swiftsoftware.dev'")


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep_stale_probe_rows()
    ts = str(int(time.time()))
    owner_email = f"zz-review-portal-owner-{ts}@swiftsoftware.dev"
    cust_email = f"zz-review-portal-cust-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    body_marker = f"Portal approve proof review {ts}"

    code = sh("curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", BASE + "/api/v1/health").stdout
    check("the app answers /api/v1/health", code == "200", f"status={code}")

    # ── fixture: owner account, a claimed listing, one pending customer review ──
    st, _ = http("POST", "/api/v1/visitor/register",
                 {"email": owner_email, "password": pw, "name": "ZZ Portal Owner"})
    check("owner account can be created (POST /visitor/register)", st in (200, 201), f"status={st}")
    st, _ = http("POST", "/api/v1/visitor/register",
                 {"email": cust_email, "password": pw, "name": "ZZ Portal Customer"})
    check("customer account can be created", st in (200, 201), f"status={st}")

    owner_vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{owner_email}'")
    cust_vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{cust_email}'")
    check("both accounts exist in the database", bool(owner_vid and cust_vid))

    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    slug = f"zz-review-portal-probe-{ts}"
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Portal Review Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{MARK}') RETURNING id")
    check("disposable listing created", bool(bid))

    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ Portal Owner",
                  "owner_phone": "4075550100"}, extra_headers=HARNESS)
    check("the owner can claim the listing (POST /businesses/:id/claim)", st in (200, 201), f"status={st}")

    st, tok_cust = http("POST", "/api/v1/auth/login", {"email": cust_email, "password": pw})
    cust_token = (tok_cust or {}).get("access_token") if isinstance(tok_cust, dict) else None
    check("the customer can sign in", st == 200 and bool(cust_token), f"status={st}")

    st, rev = http("POST", "/api/v1/reviews",
                   {"business_id": bid, "rating": 5, "title": "Great service", "content": body_marker},
                   token=cust_token)
    rid = (rev or {}).get("id") if isinstance(rev, dict) else None
    check("a customer review can be created", st in (200, 201) and bool(rid), f"status={st}")
    check("the new review starts as pending", (rev or {}).get("status") == "pending",
          f"status={(rev or {}).get('status')}")

    # ── the browser arm: the owner approves FROM THE SCREEN ─────────────────────
    env = dict(os.environ, PORTAL_URL=BASE + "/portal", OWNER_EMAIL=owner_email, PW=pw,
               RID=str(rid), SHOT_DIR=AUD, CHROME_PATH=chrome_path(), NODE_PATH=NODE_PATH)
    r = sh("node", BROWSER, env=env)
    with open(os.path.join(AUD, "browser.log"), "w") as fh:
        fh.write(r.stdout + r.stderr)
    print(r.stdout.strip())
    if r.stderr.strip():
        print("  (stderr) " + r.stderr.strip()[:400])
    check("the browser flow passed (owner saw + clicked Approve on the portal)", r.returncode == 0,
          f"rc={r.returncode}")

    # ── the backend really recorded it ─────────────────────────────────────────
    row = psql(f"SELECT status FROM reviews WHERE id='{rid}'")
    check("the database row is approved after the screen click", row == "approved", f"row={row}")
    agg = psql(f"SELECT review_count||'|'||(rating=5) FROM businesses WHERE id='{bid}'")
    check("the business rating/review_count recomputed", agg == "1|true", f"agg={agg}")

    st, pub = http("GET", f"/api/v1/reviews?business_id={bid}")
    pub_ids = [x.get("id") for x in ((pub or {}).get("data") or [])] if isinstance(pub, dict) else []
    check("the approved review is now visible to a logged-out visitor", rid in pub_ids,
          f"status={st} ids={len(pub_ids)}")

    # ── teardown ───────────────────────────────────────────────────────────────
    psql(f"DELETE FROM reviews WHERE business_id='{bid}'")
    psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    for vid in (owner_vid, cust_vid):
        if vid:
            psql(f"DELETE FROM visitor_accounts WHERE id='{vid}'")
    left = psql(f"SELECT (SELECT count(*) FROM businesses WHERE id='{bid}')"
                f"+ (SELECT count(*) FROM visitor_accounts WHERE email IN ('{owner_email}','{cust_email}'))"
                f"+ (SELECT count(*) FROM claimed_businesses WHERE business_id='{bid}')")
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
