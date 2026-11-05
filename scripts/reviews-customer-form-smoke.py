#!/usr/bin/env python3
"""Item 1, customer half — prove a real visitor can LEAVE a review from the public page.

The business half (owner sees the queue and approves) is proven by
scripts/reviews-business-approve-smoke.py. This one proves the other end of that loop on a real
screen: a signed-in customer opens the public business page, the Reviews card offers a
"Write a review" form, picks a rating, types the text, clicks Post — and the browser really calls
POST /api/v1/reviews. Then the owner approves it and it becomes public.

Setup mints its own disposable listing + two real accounts and removes all of it at the end.

Env: MD_BASE (default http://127.0.0.1:8089), SHOT_DIR
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
HARNESS = {"X-Swift-Harness": "review-customer-form-probe"}
SHOT_DIR = os.environ.get("SHOT_DIR", "/opt/swift/audits/zz-item1-customer-review")
HERE = os.path.dirname(os.path.abspath(__file__))
FAILS = []


def sh(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


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


def http(method, path, body=None, token=None, extra_headers=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    # Cloudflare in front of the public hostnames answers a bare urllib client with 403;
    # the probe must present the browser identity a real customer does.
    req.add_header("User-Agent", "Mozilla/5.0 (compatible; SwiftOps/1.0)")
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


def sweep():
    psql("DELETE FROM reviews WHERE business_id IN "
         "(SELECT id FROM businesses WHERE probe_harness='review-customer-form-probe')")
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         "(SELECT id FROM businesses WHERE probe_harness='review-customer-form-probe')")
    psql("DELETE FROM businesses WHERE probe_harness='review-customer-form-probe'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-custrev-%@swiftsoftware.dev'")


def main():
    sweep()
    os.makedirs(SHOT_DIR, exist_ok=True)
    ts = str(int(time.time()))
    owner_email = f"zz-custrev-owner-{ts}@swiftsoftware.dev"
    customer_email = f"zz-custrev-customer-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    slug = f"zz-custrev-{ts}"
    text = f"Probe review typed in the browser {ts} — friendly staff, quick service."
    visitors = []

    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Customer Review {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'review-customer-form-probe') RETURNING id")
    check("disposable listing created", bool(bid), bid)

    for label, em in (("owner", owner_email), ("customer", customer_email)):
        st, _ = http("POST", "/api/v1/visitor/register",
                     {"email": em, "password": pw, "name": f"ZZ CustRev {label}"})
        vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{em}'")
        if vid:
            visitors.append(vid)
        check(f"the {label} account exists (real signup)", st in (200, 201) and bool(vid), f"status={st}")

    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ Probe Owner",
                  "owner_phone": "4075550100"}, extra_headers=HARNESS)
    check("the owner claims the listing (public claim route)", st in (200, 201), f"status={st}")

    st, tok = http("POST", "/api/v1/auth/login", {"email": customer_email, "password": pw})
    cust_token = (tok or {}).get("access_token") if isinstance(tok, dict) else None
    check("the customer can sign in", st == 200 and bool(cust_token), f"status={st}")

    # ── the browser arm: a real customer uses the real page ──────────────────────
    env = dict(os.environ)
    env.update({"BASE": BASE, "BIZ_ID": bid, "TOKEN": cust_token or "",
                "SHOT_DIR": SHOT_DIR, "REVIEW_TEXT": text,
                "NODE_PATH": "/opt/swift/browser-run/node_modules"})
    r = subprocess.run(["node", os.path.join(HERE, "reviews-customer-form-browser.js")],
                       capture_output=True, text=True, env=env, timeout=180)
    print(r.stdout.strip())
    if r.stderr.strip():
        print("node stderr:", r.stderr.strip()[:800])
    check("the browser arm passed on the live page", r.returncode == 0, f"rc={r.returncode}")

    # ── the row the form produced ────────────────────────────────────────────────
    row = psql(f"SELECT status||'|'||rating||'|'||user_id||'|'||content FROM reviews "
               f"WHERE business_id='{bid}' ORDER BY created_at DESC LIMIT 1")
    parts = row.split("|", 3)
    check("the form created exactly one review row", bool(row), f"row={parts[0] if parts else '-'}")
    check("it is attributed to the signed-in customer", len(parts) > 2 and parts[2] == visitors[-1],
          f"user={parts[2] if len(parts) > 2 else '-'}")
    check("the rating is the 5 stars picked on screen", len(parts) > 1 and parts[1] == "5",
          f"rating={parts[1] if len(parts) > 1 else '-'}")
    check("the text is the text typed into the form", len(parts) > 3 and parts[3] == text,
          "content verified" if len(parts) > 3 and parts[3] == text else "content mismatch")
    check("a customer's review starts PENDING (not auto-published)", parts and parts[0] == "pending",
          f"status={parts[0] if parts else '-'}")
    count = psql(f"SELECT count(*) FROM reviews WHERE business_id='{bid}'")
    check("no duplicate row from the submit click", count == "1", f"rows={count}")

    # ── the owner's queue sees it, then approves it ──────────────────────────────
    st, tok_owner = http("POST", "/api/v1/auth/login", {"email": owner_email, "password": pw})
    owner_token = (tok_owner or {}).get("access_token") if isinstance(tok_owner, dict) else None
    st, pend = http("GET", "/api/v1/business/reviews/pending", token=owner_token)
    ids = [x.get("id") for x in ((pend or {}).get("data") or [])] if isinstance(pend, dict) else []
    rid = psql(f"SELECT id FROM reviews WHERE business_id='{bid}' LIMIT 1")
    check("the owner's pending queue contains the customer's review", rid in ids,
          f"status={st} n={len(ids)}")

    st, appr = http("POST", f"/api/v1/business/reviews/{rid}/approve", {}, token=owner_token)
    appr_status = appr.get("status") if isinstance(appr, dict) else None
    check("the owner approves it from the queue", st == 200 and appr_status == "approved",
          f"status={st}")
    st, pub = http("GET", f"/api/v1/reviews?business_id={bid}")
    pub_ids = [x.get("id") for x in ((pub or {}).get("data") or [])] if isinstance(pub, dict) else []
    check("the approved review is public to a logged-out visitor", rid in pub_ids, f"status={st}")

    # ── teardown ─────────────────────────────────────────────────────────────────
    psql(f"DELETE FROM reviews WHERE business_id='{bid}'")
    psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    for vid in visitors:
        psql(f"DELETE FROM visitor_accounts WHERE id='{vid}'")
    left = psql("SELECT (SELECT count(*) FROM businesses WHERE id='%s')"
                "+ (SELECT count(*) FROM reviews WHERE business_id='%s')"
                "+ (SELECT count(*) FROM claimed_businesses WHERE business_id='%s')"
                "+ (SELECT count(*) FROM visitor_accounts WHERE email IN ('%s','%s'))"
                % (bid, bid, bid, owner_email, customer_email))
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
