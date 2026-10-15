#!/usr/bin/env python3
"""Durable proof — item 1 (business-side reviews, second half): a business OWNER replies to one of
its reviews and rejects another FROM THE SCREEN, and the reply reaches the public listing.

The browser arm (scripts/reviews-business-reply-browser.js) drives the real Business Portal
(/portal -> frontend/business-portal.html): it signs in through the page's own login form, opens the
portal's Reviews view, types a reply into the review's own reply box, clicks the page's own
Post-reply control (POST /reviews/:id/respond), approves that review, then clicks Reject on a second
review (POST /business/reviews/:id/reject). Finally it loads the PUBLIC listing (/biz/:id ->
frontend/business-detail.html) as a logged-out visitor and checks the owner's reply is rendered there.

This driver mints that fixture through the real product routes, asserts the database afterwards,
and tears the fixture down (nothing left behind).

Why a browser arm and not just an API check: the item is "a business can handle its reviews"; an
endpoint returning 200 proves nothing about whether the owner can reach the Reply / Reject controls
on their own portal, or whether the reply is visible to a customer.

Run:  python3 scripts/reviews-business-portal-reply-smoke.py
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
BROWSER = os.path.join(APP, "scripts", "reviews-business-reply-browser.js")
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-review-portal-reply-{time.strftime('%Y%m%d-%H%M%S')}")
HARNESS = {"X-Swift-Harness": "review-portal-reply-probe"}
MARK = "review-portal-reply-probe"
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
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-review-reply-%@swiftsoftware.dev'")


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep_stale_probe_rows()
    ts = str(int(time.time()))
    owner_email = f"zz-review-reply-owner-{ts}@swiftsoftware.dev"
    cust1_email = f"zz-review-reply-cust1-{ts}@swiftsoftware.dev"
    cust2_email = f"zz-review-reply-cust2-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    review_a = f"Portal reply proof review A {ts}"
    review_b = f"Portal reply proof review B {ts}"
    reply_mark = f"Thanks for the kind words — proof reply {ts}"

    code = sh("curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", BASE + "/api/v1/health").stdout
    check("the app answers /api/v1/health", code == "200", f"status={code}")

    # ── fixture: owner account, a claimed listing, two pending customer reviews ──
    st, _ = http("POST", "/api/v1/visitor/register",
                 {"email": owner_email, "password": pw, "name": "ZZ Reply Owner"})
    check("owner account can be created (POST /visitor/register)", st in (200, 201), f"status={st}")
    st, _ = http("POST", "/api/v1/visitor/register",
                 {"email": cust1_email, "password": pw, "name": "ZZ Reply Customer One"})
    check("first customer account can be created", st in (200, 201), f"status={st}")
    st, _ = http("POST", "/api/v1/visitor/register",
                 {"email": cust2_email, "password": pw, "name": "ZZ Reply Customer Two"})
    check("second customer account can be created", st in (200, 201), f"status={st}")

    owner_vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{owner_email}'")
    cust_vids = [psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{e}'") for e in (cust1_email, cust2_email)]
    check("all accounts exist in the database", bool(owner_vid) and all(cust_vids))

    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    slug = f"zz-review-reply-probe-{ts}"
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Reply Review Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{MARK}') RETURNING id")
    check("disposable listing created", bool(bid))

    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ Reply Owner",
                  "owner_phone": "4075550100"}, extra_headers=HARNESS)
    check("the owner can claim the listing (POST /businesses/:id/claim)", st in (200, 201), f"status={st}")

    cust_tokens = []
    for e in (cust1_email, cust2_email):
        st, tok = http("POST", "/api/v1/auth/login", {"email": e, "password": pw})
        cust_tokens.append((tok or {}).get("access_token") if isinstance(tok, dict) else None)
    check("both customers can sign in", all(cust_tokens))

    rids = []
    # One review per customer per business is enforced (409 on a repeat), so each review comes
    # from its own account.
    for marker, ctoken in ((review_a, cust_tokens[0]), (review_b, cust_tokens[1])):
        st, rev = http("POST", "/api/v1/reviews",
                       {"business_id": bid, "rating": 5, "title": "Great service", "content": marker},
                       token=ctoken)
        rid = (rev or {}).get("id") if isinstance(rev, dict) else None
        if not rid:
            check(f"a customer review can be created ({marker[-1]})", False, f"status={st}")
        rids.append(rid)
    check("two customer reviews created (both pending)", all(rids) and len(rids) == 2)
    rid_a, rid_b = rids

    # ── the browser arm ───────────────────────────────────────────────────────
    env = dict(os.environ, PORTAL_URL=BASE + "/portal", OWNER_EMAIL=owner_email, PW=pw,
               RID_A=str(rid_a), RID_B=str(rid_b), PUBLIC_URL=BASE + "/biz/" + str(bid),
               REVIEW_MARK=review_a, REPLY_MARK=reply_mark, SHOT_DIR=AUD,
               CHROME_PATH=chrome_path(), NODE_PATH=NODE_PATH)
    r = sh("node", BROWSER, env=env)
    with open(os.path.join(AUD, "browser.log"), "w") as fh:
        fh.write(r.stdout + r.stderr)
    print(r.stdout.strip())
    if r.stderr.strip():
        print("  (stderr) " + r.stderr.strip()[:400])
    check("the browser flow passed (owner replied + rejected from the portal; reply public)", r.returncode == 0,
          f"rc={r.returncode}")

    # ── the backend really recorded it ────────────────────────────────────────
    row = psql(f"SELECT status||'|'||coalesce(owner_response,'')||'|'||"
               f"(owner_responded_at IS NOT NULL)::text FROM reviews WHERE id='{rid_a}'")
    check("review A is approved with the owner's reply stored", row == f"approved|{reply_mark}|true", f"row={row}")

    row_b = psql(f"SELECT status FROM reviews WHERE id='{rid_b}'")
    check("review B is rejected in the database after the screen click", row_b == "rejected", f"row={row_b}")

    st, pub = http("GET", f"/api/v1/reviews?status=approved&business_id={bid}&per_page=50")
    rows = (pub or {}).get("data") or [] if isinstance(pub, dict) else []
    ids = [x.get("id") for x in rows]
    check("the approved review is on the public listing", str(rid_a) in ids, f"status={st} ids={len(ids)}")
    check("the rejected review is NOT on the public listing", str(rid_b) not in ids, f"ids={len(ids)}")
    carried = next((x.get("owner_response") for x in rows if x.get("id") == str(rid_a)), None)
    check("the public listing carries the owner's reply", carried == reply_mark, f"reply={str(carried)[:40]}")

    # ── teardown ───────────────────────────────────────────────────────────────
    psql(f"DELETE FROM reviews WHERE business_id='{bid}'")
    psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    for vid in ([owner_vid] + cust_vids):
        if vid:
            psql(f"DELETE FROM visitor_accounts WHERE id='{vid}'")
    left = psql(f"SELECT (SELECT count(*) FROM businesses WHERE id='{bid}')"
                f"+ (SELECT count(*) FROM visitor_accounts WHERE email IN ('{owner_email}','{cust1_email}','{cust2_email}'))"
                f"+ (SELECT count(*) FROM claimed_businesses WHERE business_id='{bid}')"
                f"+ (SELECT count(*) FROM reviews WHERE id IN ('{rid_a}','{rid_b}'))")
    check("probe data fully removed", left == "0", f"remaining={left}")

    print()
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)}): " + "; ".join(FAILS))
        return 1
    print("RESULT: PASS")
    print(f"evidence: {AUD}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # noqa: BLE001
        print(f"RESULT: FAIL (exception: {e})")
        sys.exit(1)
