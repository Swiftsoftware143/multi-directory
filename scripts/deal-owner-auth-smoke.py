#!/usr/bin/env python3
"""Durable proof — a business owner can edit and delete their OWN deal from the portal.

The defect this closes (measured live 2026-10-15, carried over from the deal fine-print card):
the business portal signs a listing owner in as a `visitor` account (`POST /visitor/login` mints
`sub = visitor_accounts.id`, `tid = nil`, `role = "visitor"`), and the claim linking them to the
listing is written by the public claim route (`POST /businesses/:id/claim`) which records
`owner_email` — and, on the flows that knew the account, `visitor_account_id`. `assert_deal_admin`
matched `claimed_businesses.user_id` ALONE, which no self-serve owner ever populates, so the
owner's own Deal Edit / Delete answered "Deal not found" (404). The same owner could already use
every business-scoped route, because those go through `assert_business_admin_or_claimant`
(`can_admin_business` checks `visitor_account_id`). The guard now applies the same ownership rule.

Proves, against the running app and the live database:
  1. the owner's token EDITS its own deal (PUT /api/v1/deals/:id -> 200) — email-only claim shape;
  2. same with an account-linked claim (the live row's shape) -> 200;
  3. a STRANGER's token still gets 404 (the endpoint must not confirm the id exists);
  4. the platform operator still gets 200;
  5. the owner's token DELETES its own deal (DELETE /api/v1/deals/:id -> 204) and the row is gone.

Run:  python3 scripts/deal-owner-auth-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089), AUD
Exit: 0 = PASS, 1 = FAIL.  Every probe row (deal, business, claims, accounts) is removed.
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

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-deal-owner-auth-{time.strftime('%Y%m%d-%H%M%S')}")
HARNESS = {"X-Swift-Harness": "deal-owner-auth-probe"}
MARK = "deal-owner-auth-probe"
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


ENV_FILE = os.path.join(APP, ".env")


def mint_operator_token():
    """super_admin JWT, the shape the admin panel uses. Secret never printed."""
    secret = None
    for line in open(ENV_FILE):
        s = line.strip()
        if s.startswith("JWT_SECRET="):
            secret = s.split("=", 1)[1].strip().strip('"')
    if not secret:
        sys.exit("JWT_SECRET not found in " + ENV_FILE)
    b64 = lambda o: base64.urlsafe_b64encode(json.dumps(o).encode()).decode().rstrip("=")
    now = int(time.time())
    msg = b64({"alg": "HS256", "typ": "JWT"}) + "." + b64({
        "sub": "00000000-0000-0000-0000-000000000002",
        "tid": "00000000-0000-0000-0000-000000000001",
        "role": "super_admin", "exp": now + 1200, "iat": now,
        "aud": "multidirectory-api", "iss": "multidirectory"})
    return msg + "." + base64.urlsafe_b64encode(
        hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()).decode().rstrip("=")


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


def sweep_stale():
    psql("DELETE FROM deals WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-deal-owner-%@swiftsoftware.dev'")


def signup(email, pw, name):
    st, _ = http("POST", "/api/v1/visitor/register",
                 {"email": email, "password": pw, "name": name}, extra_headers=HARNESS)
    if st not in (200, 201):
        return None
    st, body = http("POST", "/api/v1/visitor/login", {"email": email, "password": pw})
    if st != 200 or not isinstance(body, dict):
        return None
    return body.get("access_token")


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep_stale()
    ts = str(int(time.time()))
    owner_email = f"zz-deal-owner-half-{ts}@swiftsoftware.dev"
    stranger_email = f"zz-deal-owner-stranger-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    title = f"ZZ Deal Owner Auth Deal {ts}"

    code = sh("curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
              BASE + "/api/v1/health").stdout
    check("the app answers /api/v1/health", code == "200", f"status={code}")

    # ── fixture: an owner account + a disposable listing it claims ─────────────
    owner_token = signup(owner_email, pw, "ZZ Deal Owner")
    check("the owner signs in through the real visitor login (POST /visitor/login)",
          bool(owner_token))
    if not owner_token:
        print("\nRESULT: FAIL (no owner token)")
        return 1
    owner_vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{owner_email}'")

    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    slug = f"zz-deal-owner-auth-{ts}"
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Deal Owner Auth Co {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{MARK}') RETURNING id")
    check("disposable listing created", bool(bid))

    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ Deal Owner",
                  "owner_phone": "4075550101"}, extra_headers=HARNESS)
    check("the owner claims the listing through the real public claim route",
          st in (200, 201), f"status={st}")
    claim_shape = psql(
        "SELECT (visitor_account_id IS NOT NULL)::text || '|' || (user_id IS NOT NULL)::text "
        f"FROM claimed_businesses WHERE business_id='{bid}' AND is_active ORDER BY created_at LIMIT 1")
    check("the claim is active", bool(claim_shape), f"shape={claim_shape}")

    # the deal the portal's own form would save: business_id, no directory_id, owner token
    st, created = http("POST", "/api/v1/deals",
                       {"title": title, "business_id": bid, "status": "active",
                        "fine_print": "owner auth probe"}, token=owner_token)
    check("the owner's own token saves a deal (POST /deals)", st in (200, 201), f"status={st}")
    did = psql(f"SELECT id FROM deals WHERE title='{title}' ORDER BY created_at DESC LIMIT 1")
    check("the deal exists in the database", bool(did), f"id={did[:8] if did else '-'}")
    if not did:
        print("\nRESULT: FAIL (no deal created; nothing to authorise)")
        return 1

    # ── arm 1: the email-only claim shape (what the public claim route writes) ─
    edit1 = "owner auth edit one " + ts
    st, _ = http("PUT", f"/api/v1/deals/{did}", {"fine_print": edit1}, token=owner_token)
    check("the owner's token EDITS its own deal (PUT /deals/:id)", st in (200, 201),
          f"status={st} claim_shape={claim_shape}")
    got = psql(f"SELECT coalesce(fine_print,'<null>') FROM deals WHERE id='{did}'")
    check("the edit really landed in the database", got == edit1, f"len={len(got)}")

    # ── arm 2: the account-linked claim shape (the live row's shape) ───────────
    psql(f"UPDATE claimed_businesses SET visitor_account_id='{owner_vid}' "
         f"WHERE business_id='{bid}' AND visitor_account_id IS NULL")
    shape2 = psql("SELECT (visitor_account_id IS NOT NULL)::text FROM claimed_businesses "
                  f"WHERE business_id='{bid}' AND is_active ORDER BY created_at LIMIT 1")
    edit2 = "owner auth edit two " + ts
    st, _ = http("PUT", f"/api/v1/deals/{did}", {"fine_print": edit2}, token=owner_token)
    check("an account-linked claim owner also EDITS its own deal", st in (200, 201),
          f"status={st} linked={shape2}")
    got2 = psql(f"SELECT coalesce(fine_print,'<null>') FROM deals WHERE id='{did}'")
    check("the second edit really landed in the database", got2 == edit2, f"len={len(got2)}")

    # ── arm 3: a stranger must still be refused ───────────────────────────────
    stranger_token = signup(stranger_email, pw, "ZZ Deal Stranger")
    check("a second account signs in (the stranger arm)", bool(stranger_token))
    if stranger_token:
        st, _ = http("PUT", f"/api/v1/deals/{did}", {"fine_print": "stranger"}, token=stranger_token)
        check("a STRANGER's token cannot edit the deal (404, not 403/200)", st == 404,
              f"status={st}")
        st, _ = http("DELETE", f"/api/v1/deals/{did}", token=stranger_token)
        check("a STRANGER's token cannot delete the deal", st == 404, f"status={st}")
    else:
        FAILS.append("stranger arm could not run")

    st, _ = http("PUT", f"/api/v1/deals/{did}", {"fine_print": "logged out"}, token=None)
    check("a logged-out caller cannot edit the deal", st in (401, 403), f"status={st}")

    # ── arm 4: the platform operator still works ──────────────────────────────
    op_token = mint_operator_token()
    st, _ = http("PUT", f"/api/v1/deals/{did}", {"fine_print": "operator edit"}, token=op_token)
    check("the platform operator can still edit the deal (PUT /deals/:id)", st in (200, 201),
          f"status={st}")

    # ── arm 5: the owner's own DELETE ─────────────────────────────────────────
    st, _ = http("DELETE", f"/api/v1/deals/{did}", token=owner_token)
    check("the owner's token DELETES its own deal (DELETE /deals/:id)", st in (200, 204),
          f"status={st}")
    left_deal = psql(f"SELECT count(*) FROM deals WHERE id='{did}'")
    check("the deal row is really gone", left_deal == "0", f"rows={left_deal}")

    # ── teardown ───────────────────────────────────────────────────────────────
    psql(f"DELETE FROM deals WHERE business_id='{bid}'")
    psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    psql("DELETE FROM visitor_accounts WHERE lower(email) IN "
         f"('{owner_email}','{stranger_email}')")
    left = psql(
        f"SELECT (SELECT count(*) FROM deals WHERE business_id='{bid}')"
        f" + (SELECT count(*) FROM businesses WHERE id='{bid}')"
        f" + (SELECT count(*) FROM claimed_businesses WHERE business_id='{bid}')"
        f" + (SELECT count(*) FROM visitor_accounts WHERE lower(email) IN ('{owner_email}','{stranger_email}'))")
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
