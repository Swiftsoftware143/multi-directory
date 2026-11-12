#!/usr/bin/env python3
"""A pending-claim owner must get a CLEAR 403 (not a dead 404) on business-scoped routes.

Defect this pins (2026-11-12, t_fcf467ca): since a claim whose e-mail domain does not match the
listing is stored `is_active = false` until an operator approves it, the business-scoped guard
`assert_business_admin_or_claimant` answered 404 "business not found" for the very owner who had
just submitted the claim. On the Photos screen (`GET /businesses/:id/images`) that left a
just-claimed owner staring at a "not found" with no hint that approval was the missing step.

The fix adds ONE exception to that guard: when the caller holds a claim on the listing that is
not yet ACTIVE, it answers 403 with the real reason instead of 404 — nothing is leaked (the
caller submitted the claim) and a stranger still gets 404 so a listing's existence stays hidden.

Flow (all through product routes; only the disposable fixture rows are written directly):
  1. POST /visitor/register            -> the owner's real account
  2. a disposable listing              -> probe_harness='pending-claim-photos403'
  3. POST /businesses/:id/claim         -> claim lands INACTIVE (pending a human)
  4. GET  /businesses/:id/images        -> 403 + "awaiting approval" (NOT 404)
  5. a STRANGER hits the same route     -> 404 (no existence leak)
  6. POST /admin/claims/:id/approve     -> claim becomes active
  7. GET  /businesses/:id/images        -> 200 (the owner can now see their photos)
  8. teardown -> 0 residue

Run: python3 scripts/pending-claim-photos-403-smoke.py     Exit: 0 PASS, 1 FAIL
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from lib.coreswift_sweep import sweep_coreswift_mirror

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
HARNESS = {"X-Swift-Harness": "pending-claim-photos403"}
HARNESS_TAG = "pending-claim-photos403"
PW_APP = "ProbePass123!"
FAILS = []


def sh(*cmd, env=None):
    return subprocess.run(cmd, capture_output=True, text=True, env=env)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    parsed = urllib.parse.urlsplit(url)
    if not parsed.password:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return urllib.parse.unquote(parsed.password)


PW = db_password()
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT)\b")


def psql(sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    return lines[0] if lines else ""


def sweep_stale_probe_rows():
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{HARNESS_TAG}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{HARNESS_TAG}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-photos403-%@swiftsoftware.dev'")


def http(method, path, body=None, token=None, extra_headers=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
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


def login(email):
    st, tok = http("POST", "/api/v1/auth/login", {"email": email, "password": PW_APP})
    return st, (tok or {}).get("access_token") if isinstance(tok, dict) else None


def main():
    sweep_stale_probe_rows()
    ts = str(int(time.time()))
    owner_email = f"zz-photos403-owner-{ts}@swiftsoftware.dev"
    stranger_email = f"zz-photos403-stranger-{ts}@swiftsoftware.dev"
    slug = f"zz-photos403-{ts}"
    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    owner_id = stranger_id = bid = claim_id = None

    try:
        st, _ = http("POST", "/api/v1/visitor/register",
                     {"email": owner_email, "password": PW_APP, "name": "ZZ Photos403 Owner"})
        check("owner account created", st in (200, 201), f"status={st}")
        st, _ = http("POST", "/api/v1/visitor/register",
                     {"email": stranger_email, "password": PW_APP, "name": "ZZ Photos403 Stranger"})
        check("stranger account created", st in (200, 201), f"status={st}")
        owner_id = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)=lower('{owner_email}')")
        stranger_id = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)=lower('{stranger_email}')")
        check("both accounts exist", bool(owner_id) and bool(stranger_id))

        bid = psql(
            "INSERT INTO businesses (directory_id, name, slug, city, state, website, status, "
            "rating, review_count, probe_harness) "
            f"VALUES ('{directory_id}', 'ZZ Photos403 Co {ts}', '{slug}', 'Orlando', 'FL', "
            f"'https://zz-photos403-{ts}.example.com', 'active', 0, 0, '{HARNESS_TAG}') RETURNING id")
        check("disposable listing created", bool(bid), f"business={bid}")

        st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                     {"owner_email": owner_email, "owner_name": "ZZ Photos403 Owner",
                      "owner_phone": "4075550199"}, extra_headers=HARNESS)
        check("owner submits a claim", st in (200, 201), f"status={st}")
        claim_id = psql(f"SELECT id FROM claimed_businesses WHERE business_id='{bid}'")
        inactive = psql(f"SELECT is_active::text FROM claimed_businesses WHERE id='{claim_id}'")
        check("the claim waits for a human (is_active=false)", inactive == "false",
              f"is_active={inactive}")

        st, owner_token = login(owner_email)
        check("owner signs in", st == 200 and bool(owner_token), f"status={st}")
        st, stranger_token = login(stranger_email)

        st, body = http("GET", f"/api/v1/businesses/{bid}/images", token=owner_token)
        check("pending owner gets 403 (not 404) on the photos route", st == 403, f"status={st}")
        msg = body.get("message") if isinstance(body, dict) else body
        check("the 403 says why (awaiting approval)",
              isinstance(msg, str) and "awaiting approval" in msg, f"body={body!r}")

        st, body = http("GET", f"/api/v1/businesses/{bid}/images", token=stranger_token)
        check("a stranger still gets 404 (listing existence hidden)", st == 404, f"status={st}")

        import base64
        import hashlib
        import hmac
        secret = sh("docker", "exec", "multidirectory", "printenv", "JWT_SECRET").stdout.strip()
        b64 = lambda o: base64.urlsafe_b64encode(json.dumps(o).encode()).decode().rstrip("=")
        now = int(time.time())
        msg = b64({"alg": "HS256", "typ": "JWT"}) + "." + b64({
            "sub": "00000000-0000-0000-0000-000000000002",
            "tid": "00000000-0000-0000-0000-000000000001",
            "role": "super_admin", "exp": now + 1200, "iat": now,
            "aud": "multidirectory-api", "iss": "multidirectory"})
        op = msg + "." + base64.urlsafe_b64encode(
            hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()).decode().rstrip("=")
        st, _ = http("POST", f"/api/v1/admin/claims/{claim_id}/approve", {}, token=op)
        check("operator approves the claim", st in (200, 201), f"status={st}")

        st, _ = http("GET", f"/api/v1/businesses/{bid}/images", token=owner_token)
        check("after approval the owner reaches the photos route", st == 200, f"status={st}")

    finally:
        if bid:
            psql(f"DELETE FROM reviews WHERE business_id='{bid}'")
            psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
            psql(f"DELETE FROM businesses WHERE id='{bid}'")
        if owner_id:
            psql(f"DELETE FROM visitor_accounts WHERE id='{owner_id}'")
        if stranger_id:
            psql(f"DELETE FROM visitor_accounts WHERE id='{stranger_id}'")
        left = psql(f"SELECT count(*) FROM businesses WHERE probe_harness='{HARNESS_TAG}'")
        left_claims = psql("SELECT count(*) FROM claimed_businesses cb JOIN businesses b "
                           f"ON b.id=cb.business_id WHERE b.probe_harness='{HARNESS_TAG}'")
        left_people = psql("SELECT count(*) FROM visitor_accounts "
                           "WHERE email LIKE 'zz-photos403-%@swiftsoftware.dev'")
        check("probe data fully removed",
              left in ("0", "") and left_claims in ("0", "") and left_people in ("0", ""),
              f"businesses={left} claims={left_claims} accounts={left_people}")
    cs_left = sweep_coreswift_mirror(["zz-photos403-"], window=15)
    check("no CoreSwift contact mirror left behind", cs_left == 0, f"coreswift_contacts={cs_left}")

    print("\nRESULT: " + ("PASS" if not FAILS else f"FAIL ({len(FAILS)} checks)"))
    if FAILS:
        for f in FAILS:
            print("  - " + f)
    sys.exit(0 if not FAILS else 1)


if __name__ == "__main__":
    main()
