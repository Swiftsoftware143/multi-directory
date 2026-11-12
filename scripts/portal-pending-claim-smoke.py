#!/usr/bin/env python3
"""A business that has claimed its listing must SEE that claim while it waits for review.

Defect this pins (measured 2026-11-12): since a claim whose e-mail domain does not match the
listing is stored `is_active = false` until an operator approves it, `GET /portal/business/profile`
returned an EMPTY `claimed_businesses` list and the Business Portal greeted the owner with
"No claimed businesses yet — claim your business listing". The owner had just claimed it and was
told to do the thing they had already done; there was no queue, no status, no next step.

Flow (all through product routes, only the disposable fixture rows are written directly):
  1. POST /visitor/register            -> the owner's real account
  2. a disposable listing              -> probe_harness='pending-claim-portal'
  3. POST /businesses/:id/claim         -> claim lands INACTIVE (pending a human)
  4. GET  /portal/business/profile      -> claimed_businesses empty, pending_claims carries it
  5. a STRANGER's profile               -> no pending claim leaks to another account
  6. browser (business-portal.html)     -> the owner reads "Your claim is in the queue" on screen
  7. POST /admin/claims/:id/approve      -> the claim becomes active
  8. GET  /portal/business/profile       -> the listing moves into claimed_businesses, queue empty
  9. teardown -> 0 residue

Run: python3 scripts/portal-pending-claim-smoke.py     Exit: 0 PASS, 1 FAIL
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
import urllib.parse
import urllib.request
from lib.coreswift_sweep import sweep_coreswift_mirror

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
PORTAL_URL = os.environ.get("PORTAL_URL", BASE + "/portal")
HARNESS = {"X-Swift-Harness": "pending-claim-probe"}
HARNESS_TAG = "pending-claim-portal"
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


def mint_operator_token():
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


def sweep_stale_probe_rows():
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{HARNESS_TAG}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{HARNESS_TAG}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-pending-%@swiftsoftware.dev'")


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


def profile(token):
    st, body = http("GET", "/api/v1/portal/business/profile", token=token)
    return st, (body if isinstance(body, dict) else {})


def main():
    sweep_stale_probe_rows()
    ts = str(int(time.time()))
    owner_email = f"zz-pending-owner-{ts}@swiftsoftware.dev"
    stranger_email = f"zz-pending-stranger-{ts}@swiftsoftware.dev"
    slug = f"zz-pending-claim-{ts}"
    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    owner_id = stranger_id = bid = claim_id = None

    try:
        # ── 1. two real accounts through the public signup ──────────────────────
        st, _ = http("POST", "/api/v1/visitor/register",
                     {"email": owner_email, "password": PW_APP, "name": "ZZ Pending Owner"})
        check("owner account can be created", st in (200, 201), f"status={st}")
        st, _ = http("POST", "/api/v1/visitor/register",
                     {"email": stranger_email, "password": PW_APP, "name": "ZZ Pending Stranger"})
        check("a second (stranger) account can be created", st in (200, 201), f"status={st}")
        owner_id = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)=lower('{owner_email}')")
        stranger_id = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)=lower('{stranger_email}')")
        check("both accounts exist in the database", bool(owner_id) and bool(stranger_id))

        # ── 2. a disposable listing the owner claims ────────────────────────────
        bid = psql(
            "INSERT INTO businesses (directory_id, name, slug, city, state, website, status, "
            "rating, review_count, probe_harness) "
            f"VALUES ('{directory_id}', 'ZZ Pending Claim Co {ts}', '{slug}', 'Orlando', 'FL', "
            f"'https://zz-pending-{ts}.example.com', 'active', 0, 0, '{HARNESS_TAG}') RETURNING id")
        check("disposable listing created", bool(bid), f"business={bid}")

        # ── 3. claim it (free-mail-style address, so the claim cannot auto-approve) ──
        st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                     {"owner_email": owner_email, "owner_name": "ZZ Pending Owner",
                      "owner_phone": "4075550199"}, extra_headers=HARNESS)
        check("the owner can submit a claim", st in (200, 201), f"status={st}")
        claim_id = psql(f"SELECT id FROM claimed_businesses WHERE business_id='{bid}'")
        inactive = psql(f"SELECT is_active::text FROM claimed_businesses WHERE id='{claim_id}'")
        check("the claim waits for a human (is_active=false)", inactive == "false",
              f"is_active={inactive}")

        st, tok = http("POST", "/api/v1/auth/login", {"email": owner_email, "password": PW_APP})
        owner_token = (tok or {}).get("access_token") if isinstance(tok, dict) else None
        check("the owner can sign in to the portal", st == 200 and bool(owner_token), f"status={st}")
        st, tok2 = http("POST", "/api/v1/auth/login", {"email": stranger_email, "password": PW_APP})
        stranger_token = (tok2 or {}).get("access_token") if isinstance(tok2, dict) else None

        # ── 4. the portal answers with the claim IN THE QUEUE ───────────────────
        st, prof = profile(owner_token)
        claimed = prof.get("claimed_businesses") or []
        pending = prof.get("pending_claims") or []
        check("the portal profile answers", st == 200, f"status={st}")
        check("nothing is granted yet (no active listing)", len(claimed) == 0,
              f"claimed={len(claimed)}")
        check("the portal reports the claim as WAITING (pending_claims)", len(pending) == 1,
              f"pending={len(pending)}")
        if pending:
            p = pending[0]
            check("the queued claim names the listing", p.get("business_name", "").startswith("ZZ Pending Claim Co"),
                  f"name={p.get('business_name')}")
            check("the queued claim carries the id the dashboard links to",
                  p.get("business_id") == bid, f"business_id={p.get('business_id')}")
            check("the queued claim shows the address it was made with",
                  (p.get("owner_email") or "").lower() == owner_email.lower(),
                  f"email={p.get('owner_email')}")
            check("the queued claim carries a submitted time", bool(p.get("submitted_at")),
                  f"submitted_at={p.get('submitted_at')}")

        # ── 5. another account must not see it ──────────────────────────────────
        st, sprof = profile(stranger_token)
        sp = sprof.get("pending_claims") or []
        check("a stranger's portal shows NO pending claim of this business",
              all(x.get("business_id") != bid for x in sp), f"stranger_pending={len(sp)}")

        # ── 6. the screen the owner actually sees ───────────────────────────────
        shot_dir = os.environ.get("SHOT_DIR", "/opt/swift/audits/zz-pending-claim-portal-20261112")
        os.makedirs(shot_dir, exist_ok=True)
        env = dict(os.environ, PORTAL_URL=PORTAL_URL, OWNER_EMAIL=owner_email, PW=PW_APP,
                   BUSINESS_ID=bid, SHOT_DIR=shot_dir,
                   NODE_PATH=os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules"))
        node = sh("node", os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                       "portal-pending-claim-browser.js"), env=env)
        took = node.returncode == 0
        for line in (node.stdout or "").splitlines():
            print("  " + line)
        if not took:
            for line in (node.stderr or "").strip().splitlines()[-4:]:
                print("  ! " + line)
        check("browser: the portal tells the owner the claim is in the queue", took,
              (node.stderr or "").strip().splitlines()[-1] if (not took and (node.stderr or "").strip()) else "")

        # ── 7. the operator approves, and the portal opens up ───────────────────
        op = mint_operator_token()
        st, _ = http("POST", f"/api/v1/admin/claims/{claim_id}/approve", {}, token=op)
        check("the operator can approve the claim", st in (200, 201), f"status={st}")
        st, prof2 = profile(owner_token)
        claimed2 = prof2.get("claimed_businesses") or []
        pending2 = prof2.get("pending_claims") or []
        check("the approved listing now appears as the owner's business",
              any((c.get("business") or {}).get("id") == bid for c in claimed2),
              f"claimed={len(claimed2)}")
        check("the queue empties once the claim is approved",
              all(x.get("business_id") != bid for x in pending2), f"pending={len(pending2)}")
    finally:
        # ── 8. teardown ────────────────────────────────────────────────────────
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
                           "WHERE email LIKE 'zz-pending-%@swiftsoftware.dev'")
        check("probe data fully removed",
              left in ("0", "") and left_claims in ("0", "") and left_people in ("0", ""),
              f"businesses={left} claims={left_claims} accounts={left_people}")
    cs_left = sweep_coreswift_mirror(["zz-pending-"], window=15)
    check("no CoreSwift contact mirror left behind", cs_left == 0,
          f"coreswift_contacts={cs_left}")

    print("\nRESULT: " + ("PASS" if not FAILS else f"FAIL ({len(FAILS)} checks)"))
    if FAILS:
        for f in FAILS:
            print("  - " + f)
    sys.exit(0 if not FAILS else 1)


if __name__ == "__main__":
    main()
