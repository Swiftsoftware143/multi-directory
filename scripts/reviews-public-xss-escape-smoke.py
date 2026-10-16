#!/usr/bin/env python3
"""Public business page — ESCAPING proof (item 1, second half; REVIEWS, ZaarHub).

The fix in 1b8e7c2 ("the public business page rendered an approved review's text and reviewer
name as raw HTML") is the newest code change on this app and had no durable proof: the
business-side approve smoke only asserted the review became *visible*, never that its
payload was neutralised. A stored XSS on a public route is the one defect class that must
never be assumed fixed.

This probe proves, with real rows + a real browser on the real route (`/biz/:id`):
  1. a customer can leave a review whose text and reviewer name are HTML/JS payloads;
  2. the listing owner approves it (real approve endpoint);
  3. the approved review is publicly visible (GET /api/v1/reviews, logged out);
  4. a real browser loading /biz/<id> shows BOTH payloads as literal text,
     creates NO element from them, and runs NO injected handler.

The only direct DB writes are the disposable probe listing and teardown.
Run:  python3 scripts/reviews-public-xss-escape-smoke.py
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
HARNESS = {"X-Swift-Harness": "review-xss-probe"}
MARK = "review-xss-probe"
HERE = os.path.dirname(os.path.abspath(__file__))
BROWSER = os.path.join(HERE, "reviews-public-xss-escape-browser.js")
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
AUD = os.environ.get("SHOT_DIR", "/opt/swift/audits/zz-item1-xss-escape-20261016")
FAILS = []

PAYLOAD_CONTENT = '<img src=x onerror="window.__xssProbe=1">XSS-PROBE-CONTENT'
PAYLOAD_NAME = '<img src=y onerror="window.__xssName=1">XSS-NAME'


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
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-xss-%@swiftsoftware.dev'")


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


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep_stale_probe_rows()
    ts = str(int(time.time()))
    cust_email = f"zz-xss-cust-{ts}@swiftsoftware.dev"
    owner_email = f"zz-xss-owner-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    slug = f"zz-xss-probe-{ts}"
    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    created = {"visitors": [], "business": None}

    for em, nm in ((cust_email, "ZZ XSS Customer"), (owner_email, "ZZ XSS Owner")):
        st, _ = http("POST", "/api/v1/visitor/register", {"email": em, "password": pw, "name": nm})
        check(f"account created ({nm})", st in (200, 201), f"status={st}")
        vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{em}'")
        if vid:
            created["visitors"].append(vid)

    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ XSS Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{MARK}') RETURNING id")
    created["business"] = bid
    check("disposable listing created", bool(bid), f"id={bid}")

    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ XSS Owner",
                  "owner_phone": "4075550100"}, extra_headers=HARNESS)
    check("owner can claim the listing", st in (200, 201), f"status={st}")

    st, tok = http("POST", "/api/v1/auth/login", {"email": cust_email, "password": pw})
    cust_token = (tok or {}).get("access_token")
    check("customer can sign in", st == 200 and bool(cust_token), f"status={st}")

    st, rev = http("POST", "/api/v1/reviews",
                   {"business_id": bid, "rating": 5, "title": "XSS probe",
                    "content": PAYLOAD_CONTENT, "reviewer_name": PAYLOAD_NAME},
                   token=cust_token)
    rid = (rev or {}).get("id") if isinstance(rev, dict) else None
    check("a review carrying HTML payloads can be created", st in (200, 201) and bool(rid),
          f"status={st}")
    stored = psql(f"SELECT content FROM reviews WHERE id='{rid}'")
    check("the payload is stored verbatim (nothing sanitised at the API)", stored == PAYLOAD_CONTENT,
          f"stored={stored[:60]!r}")

    st, tok = http("POST", "/api/v1/auth/login", {"email": owner_email, "password": pw})
    owner_token = (tok or {}).get("access_token")
    check("owner can sign in", st == 200 and bool(owner_token), f"status={st}")

    st, _ = http("POST", f"/api/v1/business/reviews/{rid}/approve", {}, token=owner_token)
    check("owner approves the review (200)", st == 200, f"status={st}")
    check("the database row is approved", psql(f"SELECT status FROM reviews WHERE id='{rid}'") == "approved")

    st, pub = http("GET", f"/api/v1/reviews?business_id={bid}")
    pub_ids = [r.get("id") for r in ((pub or {}).get("data") or [])] if isinstance(pub, dict) else []
    check("the approved review is publicly visible (logged out)", rid in pub_ids, f"status={st}")

    # ── the browser arm: the real public route ──────────────────────────────────
    env = dict(os.environ, MD_URL=BASE, BIZ_ID=str(bid),
               PAYLOAD_CONTENT=PAYLOAD_CONTENT, PAYLOAD_NAME=PAYLOAD_NAME,
               SHOT_DIR=AUD, CHROME_PATH=chrome_path(), NODE_PATH=NODE_PATH)
    r = sh("node", BROWSER, env=env)
    with open(os.path.join(AUD, "browser.log"), "w") as fh:
        fh.write(r.stdout + r.stderr)
    print(r.stdout.strip())
    if r.stderr.strip():
        print("  (stderr) " + r.stderr.strip()[:400])
    check("the browser flow passed (both payloads shown as text, nothing executed)",
          r.returncode == 0, f"rc={r.returncode}")

    # ── teardown ────────────────────────────────────────────────────────────────
    psql(f"DELETE FROM reviews WHERE business_id='{bid}'")
    psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    for vid in created["visitors"]:
        psql(f"DELETE FROM visitor_accounts WHERE id='{vid}'")
    left = psql(f"SELECT (SELECT count(*) FROM businesses WHERE id='{bid}')"
                f"+(SELECT count(*) FROM visitor_accounts WHERE email IN ('{cust_email}','{owner_email}'))"
                f"+(SELECT count(*) FROM reviews WHERE business_id='{bid}')")
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
