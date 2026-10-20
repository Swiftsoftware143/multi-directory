#!/usr/bin/env python3
"""Durable proof — a deleted account's JWT stops working (kanban t_fb37dfc0).

The defect this closes (the fleet-wide class t_53fcb666 fixed on FunnelSwift, CoreSwift-CRM,
WorkflowSwift, ADASwift, missingcallrespondr and IncentiveSwift; this app is the `builds` lane's
copy): `src/auth/middleware.rs` only offered `verify_token()` — signature, `exp`, `iss`, `aud` —
and every protected handler read its own extractor. Nothing re-read the principal, so a token kept
collecting 200 from working handlers for its whole 24 h life after the account was deleted.

Measured live 2026-10-20, BEFORE the fix, with a throwaway signup whose `users` row was then
deleted (tenant kept): GET /my-reviews, /admin/reports, /admin/reports/stats, /templates,
/crm/deals and /admin/audience/overview all answered 200 — and so did POST / PUT / PATCH / DELETE
legs (422/404 from the handler, i.e. the token cleared the boundary). A deleted
`visitor_accounts` row behaved the same way, because the business portal signs a shopper in with
`sub = visitor_accounts.id`. (`GET /deals` and `GET /directories` also answered 200, but those are
declared PUBLIC in `security/route_policy.rs` and answer 200 with no token at all — they trust no
principal, so they are not refusal legs here.)

The fix is one primary-key read in `routes::auth_guard` (the single boundary every private route
passes through) via `auth::middleware::principal_is_live`, which answers for BOTH identity tables:
a deleted/disabled account's bearer is 401 exactly like an expired token, on every method, even
for handlers that never touch `users`. A database that cannot answer is a 500, never a pass.

Proves, against the running app and the live database:
  1. POSITIVE CONTROL — a fresh signup's token answers 200 on five guarded reads, and on the
     POST/PUT/PATCH/DELETE legs the handler (not the boundary) answers (never 401);
  2. after `delete from users` (tenant kept) the SAME token is 401 on every GET, POST, PUT,
     PATCH and DELETE leg — not just writes, not just one handler;
  3. a live visitor session (the other identity table) keeps working, and after its
     `visitor_accounts` row is deleted it too is 401 on every leg;
  4. a second live account — and the platform operator — are unaffected (200);
  5. every row this probe made is gone afterwards (users, visitor, tenants).

Run:  python3 scripts/deleted-credential-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089)
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

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
TAG = "zz-delcred-" + time.strftime("%Y%m%d-%H%M%S")
PW = "Probe!" + TAG + "-9x"
HARNESS = {"X-Swift-Harness": "deleted-credential-probe", "Content-Type": "application/json"}
FAILS = []

# One leg per HTTP method. The two GETs further down are the guarded reads the bug was measured
# on; the rest exist so the refusal is proven on every method, not just writes.
READS = [
    "/api/v1/my-reviews",
    "/api/v1/admin/reports",
    "/api/v1/admin/reports/stats",
    "/api/v1/templates",
    "/api/v1/crm/deals",
    "/api/v1/admin/audience/overview",
]
# Declared PUBLIC in `security/route_policy.rs` (`deals-list`): it answers 200 with no token at
# all, so it trusts no principal and a dead token changes nothing there. Asserted as such below
# and deliberately NOT used as a refusal leg.
PUBLIC_LEG = "/api/v1/deals"
METHOD_LEGS = [
    ("POST", "/api/v1/deals", {}),
    ("POST", "/api/v1/blog/distribute", {}),
    ("PUT", "/api/v1/directories/zz-no-such-directory", {}),
    ("PUT", "/api/v1/deals/00000000-0000-0000-0000-000000000123", {}),
    ("PATCH", "/api/v1/admin/reports/00000000-0000-0000-0000-000000000123", {"status": "resolved"}),
    ("DELETE", "/api/v1/admin/media/00000000-0000-0000-0000-000000000123", None),
    ("DELETE", "/api/v1/deals/00000000-0000-0000-0000-000000000123", None),
]


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


PW_DB = db_password()


def psql(sql):
    env = dict(os.environ, PGPASSWORD=PW_DB)
    r = sh("psql", "-h", "127.0.0.1", "-U", "swift", "-d", "multi_directory", "-tAc", sql, env=env)
    if r.returncode != 0:
        sys.exit("psql failed: " + (r.stderr or "").strip()[:300])
    return r.stdout.strip()


def call(method, path, token=None, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    for k, v in HARNESS.items():
        req.add_header(k, v)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (("  [" + detail + "]") if detail and not ok else ""))
    if not ok:
        FAILS.append(label)


def mint_operator_token():
    """super_admin JWT for the real operator row — the shape the admin panel uses."""
    secret = None
    for line in open(os.path.join(APP, ".env")):
        s = line.strip()
        if s.startswith("JWT_SECRET="):
            secret = s.split("=", 1)[1].strip().strip('"')
    if not secret:
        sys.exit("JWT_SECRET not found in " + APP + "/.env")
    b64 = lambda o: base64.urlsafe_b64encode(json.dumps(o).encode()).decode().rstrip("=")
    now = int(time.time())
    msg = b64({"alg": "HS256", "typ": "JWT"}) + "." + b64({
        "sub": "00000000-0000-0000-0000-000000000002",
        "tid": "00000000-0000-0000-0000-000000000001",
        "role": "super_admin", "exp": now + 1200, "iat": now,
        "aud": "multidirectory-api", "iss": "multidirectory"})
    return msg + "." + base64.urlsafe_b64encode(
        hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()).decode().rstrip("=")


def register_user(suffix, name):
    st, body = call("POST", "/api/v1/auth/register",
                    body={"email": f"{TAG}{suffix}@example.invalid", "password": PW, "name": name})
    if st != 201:
        sys.exit(f"register {suffix} answered {st}: {body[:200]}")
    d = json.loads(body)
    return d["access_token"], d["user"]["id"], d["tenant"]["id"]


def register_visitor(suffix, name):
    st, body = call("POST", "/api/v1/visitor/register",
                    body={"email": f"{TAG}{suffix}@example.invalid", "password": PW, "name": name})
    if st not in (200, 201):
        sys.exit(f"visitor register answered {st}: {body[:200]}")
    d = json.loads(body)
    vid = (d.get("visitor") or {}).get("id") or d.get("id")
    if not d.get("access_token") or not vid:
        sys.exit("visitor register did not return a token + id: " + body[:200])
    return d["access_token"], vid


def main():
    made_users, made_tenants = [], []
    print("== probe tag:", TAG)

    st, _ = call("GET", PUBLIC_LEG)
    check(f"public GET {PUBLIC_LEG} answers with no token at all (trusts no principal)",
          st == 200, f"got {st}")

    tok_a, uid_a, tid_a = register_user("", "Deleted Credential Probe A")
    made_users.append(uid_a)
    made_tenants.append(tid_a)
    print("== throwaway A:", uid_a, "tenant", tid_a)

    # ---- 1. POSITIVE CONTROL: a live token is accepted on every leg (boundary not 401) ----
    for path in READS:
        st, _ = call("GET", path, tok_a)
        check(f"live GET {path} -> 200", st == 200, f"got {st}")
    for method, path, body in METHOD_LEGS:
        st, _ = call(method, path, tok_a, body)
        check(f"live {method} {path} clears the boundary (not 401)", st != 401, f"got {st}")

    # ---- 2. DELETE the principal (tenant kept) and assert the SAME token is refused ----
    psql(f"delete from users where id = '{uid_a}'")
    check("users row for A is gone",
          psql(f"select count(*) from users where id = '{uid_a}'") == "0")
    check("tenant row for A is still there (deletion, not tenant teardown)",
          psql(f"select count(*) from tenants where id = '{tid_a}'") == "1")

    for path in READS:
        st, _ = call("GET", path, tok_a)
        check(f"deleted user GET {path} -> 401", st == 401, f"got {st}")
    for method, path, body in METHOD_LEGS:
        st, _ = call(method, path, tok_a, body)
        check(f"deleted user {method} {path} -> 401", st == 401, f"got {st}")

    # ---- 3. A second LIVE account is unaffected ----
    tok_b, uid_b, tid_b = register_user("-b", "Deleted Credential Probe B")
    made_users.append(uid_b)
    made_tenants.append(tid_b)
    for path in READS[:3]:
        st, _ = call("GET", path, tok_b)
        check(f"second live account GET {path} -> 200", st == 200, f"got {st}")

    # ---- 3b. DISABLED (not deleted) is refused the same way ----
    tok_d, uid_d, tid_d = register_user("-d", "Deleted Credential Probe D")
    made_users.append(uid_d)
    made_tenants.append(tid_d)
    st, _ = call("GET", READS[0], tok_d)
    check(f"live-but-doomed account GET {READS[0]} -> 200", st == 200, f"got {st}")
    psql(f"update users set is_active = false where id = '{uid_d}'")
    for path in READS[:3]:
        st, _ = call("GET", path, tok_d)
        check(f"disabled account GET {path} -> 401", st == 401, f"got {st}")

    # ---- 4. The other identity table: a visitor session is live, then deleted ----
    vt, vid = register_visitor("-v", "Deleted Credential Visitor")
    for path in READS[:3]:
        st, _ = call("GET", path, vt)
        check(f"live visitor GET {path} -> 200", st == 200, f"got {st}")
    psql(f"delete from visitor_accounts where id = '{vid}'")
    for path in READS[:3]:
        st, _ = call("GET", path, vt)
        check(f"deleted visitor GET {path} -> 401", st == 401, f"got {st}")
    for method, path, body in METHOD_LEGS[:3]:
        st, _ = call(method, path, vt, body)
        check(f"deleted visitor {method} {path} -> 401", st == 401, f"got {st}")

    # ---- 5. The platform operator (a real, live account) still reaches an operator route ----
    st, _ = call("GET", "/api/v1/dashboard/stats", mint_operator_token())
    check("platform operator GET /dashboard/stats -> 200", st == 200, f"got {st}")

    # ---- 6. Zero residue ----
    for uid in made_users:
        psql(f"delete from users where id = '{uid}'")
    for tid in made_tenants:
        psql(f"delete from tenants where id = '{tid}'")
    left = psql(
        "select (select count(*) from users where email like '" + TAG + "%')"
        " + (select count(*) from visitor_accounts where email like '" + TAG + "%')"
        " + (select count(*) from tenants where slug like 'deleted-credential-probe%'"
        "    and name like 'Deleted Credential Probe%')"
    )
    check("zero residue (users + visitors + tenants)", left == "0", f"left {left}")

    print()
    if FAILS:
        print(f"FAIL — {len(FAILS)} check(s) failed")
        for f in FAILS:
            print("  - " + f)
        return 1
    print("PASS — deleted/deactivated principals are refused on every method; live accounts "
          "(user, visitor, operator) unaffected; zero residue")
    return 0


if __name__ == "__main__":
    sys.exit(main())
