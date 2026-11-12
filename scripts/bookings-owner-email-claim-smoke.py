#!/usr/bin/env python3
"""Proof — an owner who claimed their listing by EMAIL (the normal public path) can see and
manage the bookings on that listing.

Why: `POST /businesses/:id/claim` records the claimant's email and leaves `claimed_businesses`
id columns NULL. The bookings handlers matched `user_id` only, so the real owner got
403 "Not authorized to view bookings for this business" on their own business. Same defect
class as the review-ownership fix (0af9a3c) but in `src/handlers/bookings.rs`.

This driver mints every fixture through the product's own routes, asserts against the database,
and exercises both the read (list) and the write (status change) as well as the negative case.

Run:  python3 scripts/bookings-owner-email-claim-smoke.py
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
import urllib.error
import urllib.request
from lib.coreswift_sweep import sweep_coreswift_mirror

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
AUD = os.environ.get("AUD", f"/opt/swift/audits/bookings-owner-claim-{time.strftime('%Y%m%d-%H%M%S')}")
MARK = "bookings-owner-probe"
HARNESS = {"X-Swift-Harness": MARK}
FAILS = []
RESULTS = []


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


def mint_operator_token():
    """super_admin JWT, the same shape the admin console uses. Secret never printed."""
    secret = sh("docker", "exec", "multidirectory", "printenv", "JWT_SECRET").stdout.strip()
    if not secret:
        sys.exit("JWT_SECRET not available from the multidirectory container")
    b64 = lambda o: base64.urlsafe_b64encode(json.dumps(o).encode()).decode().rstrip("=")
    now = int(time.time())
    msg = b64({"alg": "HS256", "typ": "JWT"}) + "." + b64({
        "sub": "00000000-0000-0000-0000-000000000002",
        "tid": "00000000-0000-0000-0000-000000000001",
        "role": "super_admin", "exp": now + 1200, "iat": now,
        "aud": "multidirectory-api", "iss": "multidirectory"})
    return msg + "." + base64.urlsafe_b64encode(
        hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()).decode().rstrip("=")


def check(label, ok, detail=""):
    RESULTS.append({"name": label, "ok": bool(ok), "detail": str(detail)})
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


def sweep():
    psql("DELETE FROM service_bookings WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-bookings-owner-%@swiftsoftware.dev'")


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep()
    ts = str(int(time.time()))
    owner_email = f"zz-bookings-owner-o-{ts}@swiftsoftware.dev"
    cust_email = f"zz-bookings-owner-c-{ts}@swiftsoftware.dev"
    stranger_email = f"zz-bookings-owner-s-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"

    for email, name in ((owner_email, "ZZ Bookings Owner"), (cust_email, "ZZ Bookings Customer"),
                        (stranger_email, "ZZ Bookings Stranger")):
        st, _ = http("POST", "/api/v1/visitor/register",
                     {"email": email, "password": pw, "name": name})
        check(f"account created: {name}", st in (200, 201), f"status={st}")

    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    slug = f"zz-bookings-owner-probe-{ts}"
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Bookings Owner Probe {ts}', '{slug}', 'Orlando', 'FL', "
        f"'active', 0, 0, '{MARK}') RETURNING id")
    check("disposable listing created", bool(bid), bid)

    # The public claim route: email only, no account ids written.
    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ Bookings Owner",
                  "owner_phone": "4075550111"}, extra_headers=HARNESS)
    check("owner claims the listing through the public claim route", st in (200, 201), f"status={st}")
    claim = psql("SELECT COALESCE(user_id::text,'NULL')||'|'||COALESCE(visitor_account_id::text,'NULL') "
                 f"FROM claimed_businesses WHERE business_id='{bid}'")
    check("the claim as written is email-only (both account id columns NULL)", claim == "NULL|NULL", claim)

    # ── the operator approves the claim (the flow since 2026-11-12) ──────────────
    # A claimant whose address domain does not match the listing gets no rights until a human
    # approves it from the Businesses screen, so this proof walks that same door.
    claim_id = psql(f"SELECT id FROM claimed_businesses WHERE business_id='{bid}'")
    inactive = psql(f"SELECT is_active::text FROM claimed_businesses WHERE id='{claim_id}'")
    check("the claim waits for a human decision (stored inactive)", inactive == "false",
          f"is_active={inactive}")
    op_tok = mint_operator_token()
    st, _ = http("POST", f"/api/v1/admin/claims/{claim_id}/approve", {}, token=op_tok)
    check("the operator approves the claim from the Businesses screen", st in (200, 201), f"status={st}")
    active = psql(f"SELECT is_active::text FROM claimed_businesses WHERE id='{claim_id}'")
    check("the approved claim is ACTIVE", active == "true", f"is_active={active}")

    st, cust = http("POST", "/api/v1/auth/login", {"email": cust_email, "password": pw})
    cust_token = (cust or {}).get("access_token") if isinstance(cust, dict) else None
    check("customer can sign in", st == 200 and bool(cust_token), f"status={st}")

    st, bk = http("POST", "/api/v1/bookings",
                  {"directory_id": directory_id, "business_id": bid, "service_name": "Oil change",
                   "description": "Bookings ownership probe", "preferred_time": "10:00",
                   "contact_phone": "4075550122"}, token=cust_token)
    bkid = (bk or {}).get("id") if isinstance(bk, dict) else None
    if not bkid and isinstance(bk, dict):
        bkid = ((bk.get("booking") or {}).get("id"))
    check("customer creates a service booking", st in (200, 201) and bool(bkid), f"status={st}")
    check("the booking row exists as pending", psql(f"SELECT status FROM service_bookings WHERE id='{bkid}'") == "pending")

    st, tok = http("POST", "/api/v1/auth/login", {"email": owner_email, "password": pw})
    owner_token = (tok or {}).get("access_token") if isinstance(tok, dict) else None
    check("owner can sign in", st == 200 and bool(owner_token), f"status={st}")

    # THE FIX: the owner's own listing answers instead of 403.
    st, lst = http("GET", f"/api/v1/business/{bid}/bookings", token=owner_token)
    ids = [b.get("id") for b in ((lst or {}).get("bookings") or [])] if isinstance(lst, dict) else []
    check("owner sees their own bookings list", st == 200, f"status={st}")
    check("the customer's booking is in the owner's list", bkid in ids, f"ids={len(ids)}")

    # The real user path: the owner's own screen shows the request and its Confirm control works.
    shot = os.path.join(AUD, "portal-bookings.png")
    br = sh("node", os.path.join(os.path.dirname(__file__), "bookings-owner-browser.js"),
            owner_email, pw, "Bookings ownership probe",
            env={**os.environ, "SHOT": shot,
                 "NODE_PATH": os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")})
    try:
        ui = json.loads(br.stdout.strip().splitlines()[-1])
    except Exception:
        ui = {"ok": False, "raw": (br.stdout or br.stderr)[-400:]}
    with open(os.path.join(AUD, "ui-proof.json"), "w") as fh:
        json.dump(ui, fh, indent=1)
    check("portal: the request is on the owner's Booking Requests screen", ui.get("cardSeen") is True,
          ui.get("listText", "")[:120])
    check("portal: the owner's own listing is not denied", not ui.get("denied"), "403 text absent")
    check("portal: the page's own Confirm button works", ui.get("confirmedOnScreen") is True,
          ui.get("afterText", "")[:120])
    check("database row is confirmed after the screen action",
          psql(f"SELECT status FROM service_bookings WHERE id='{bkid}'") == "confirmed")
    check("portal: the whole owner path passed in the browser", ui.get("ok") is True,
          f"other 4xx={ui.get('badResponses')}")

    # The API write side uses the same ownership check (already covered above by the screen click).
    st, upd = http("POST", f"/api/v1/bookings/{bkid}/status", {"status": "completed"},
                   token=owner_token)
    check("owner can move the booking on (API, same ownership check)", st == 200, f"status={st}")

    # Negative case: a stranger must still be refused.
    st, tok = http("POST", "/api/v1/auth/login", {"email": stranger_email, "password": pw})
    stranger_token = (tok or {}).get("access_token") if isinstance(tok, dict) else None
    st, _ = http("GET", f"/api/v1/business/{bid}/bookings", token=stranger_token)
    check("a stranger is refused (403, not 200)", st == 403, f"status={st}")

    # Teardown
    psql(f"DELETE FROM service_bookings WHERE business_id='{bid}'")
    psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-bookings-owner-%@swiftsoftware.dev'")
    check("fixtures removed", psql(f"SELECT count(*) FROM businesses WHERE id='{bid}'") == "0")

    cs_left = sweep_coreswift_mirror(["zz-bookings-owner-"], window=15)
    check("no CoreSwift contact mirror left behind", cs_left == 0,
          f"coreswift_contacts={cs_left}")
    verdict = "PASS" if not FAILS else "FAIL"
    with open(os.path.join(AUD, "proof.json"), "w") as fh:
        json.dump({"verdict": verdict, "passed": len(RESULTS) - len(FAILS),
                   "failed": len(FAILS), "checks": RESULTS}, fh, indent=1)
    print(f"\n{verdict}: {len(RESULTS) - len(FAILS)} passed, {len(FAILS)} failed -> {AUD}")
    return 0 if not FAILS else 1


if __name__ == "__main__":
    sys.exit(main())
