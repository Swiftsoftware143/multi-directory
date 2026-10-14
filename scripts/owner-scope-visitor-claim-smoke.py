#!/usr/bin/env python3
"""Durable proof — a self-serve business owner (an account that lives in `visitor_accounts`,
created by the PUBLIC signup form and linked to their listing by an EMAIL-ONLY claim) can reach
the owner-scoped business console routes.

Why this exists. The Business Portal's ad-earnings card silently hid itself for exactly these
owners: `tenant_scope::assert_business_admin_or_claimant` resolved the caller's email only from
`users`, while the public claim route records `owner_email` and (before any account is linked) no
account id at all — so every route behind that guard answered 404 for the owners the claim form
creates (measured live 2026-10-14: GET /businesses/:id/ads/earnings -> 404 while the listing list
route, which resolves ownership differently, answered 200). The fix landed in `14cd367` with the
item-1 review work but had NO test. This script is that test: it mints the fixture through the real
public routes, asserts the owner gets 200 while a stranger gets 404 and anonymous gets 401 on three
different handlers behind the guard, then proves the anti-shadow rule (a claim already linked to
somebody else's account can never be reached by an email match), and tears everything down.

Run:  python3 scripts/owner-scope-visitor-claim-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089), AUD
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
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-owner-scope-{time.strftime('%Y%m%d-%H%M%S')}")
HARNESS = {"X-Swift-Harness": "owner-scope-probe"}
MARK = "owner-scope-probe"
FAILS = []
PW_ACCT = "ProbePass123!"


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
        FAILS.append(label + (f" ({detail})" if detail else ""))


def sweep_stale():
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-scope-%@swiftsoftware.dev'")


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep_stale()
    ts = str(int(time.time()))
    owner_email = f"zz-scope-owner-{ts}@swiftsoftware.dev"
    other_email = f"zz-scope-other-{ts}@swiftsoftware.dev"
    stranger_email = f"zz-scope-stranger-{ts}@swiftsoftware.dev"
    slug = f"zz-scope-probe-{ts}"

    st, _ = http("GET", "/api/v1/health")
    check("the app answers /api/v1/health", st == 200, f"status={st}")

    # ── 1. three real accounts through the public signup form ───────────────────
    for em, nm in ((owner_email, "ZZ Scope Owner"), (other_email, "ZZ Scope Other"),
                   (stranger_email, "ZZ Scope Stranger")):
        st, _ = http("POST", "/api/v1/visitor/register", {"email": em, "password": PW_ACCT, "name": nm})
        check(f"public signup works for {em.split('-')[2]}", st in (200, 201), f"status={st}")
    ids = {em: psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{em}'")
           for em in (owner_email, other_email, stranger_email)}
    check("all three accounts exist in visitor_accounts (not users)",
          all(ids.values()) and psql(f"SELECT count(*) FROM users WHERE lower(email)='{owner_email}'") == "0",
          f"ids={list(ids.values())}")
    tok = {}
    for em in (owner_email, other_email, stranger_email):
        st, body = http("POST", "/api/v1/auth/login", {"email": em, "password": PW_ACCT})
        tok[em] = (body or {}).get("access_token") if isinstance(body, dict) else None
        check(f"signin works for {em.split('-')[2]}", st == 200 and bool(tok[em]), f"status={st}")

    # ── 2. a disposable listing claimed the way the public form claims it ───────
    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Owner Scope Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{MARK}') RETURNING id")
    check("disposable listing created", bool(bid))
    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ Scope Owner",
                  "owner_phone": "4075550100"}, extra_headers=HARNESS)
    check("the owner claims the listing with their email (POST /businesses/:id/claim)",
          st in (200, 201), f"status={st}")
    link = psql("SELECT coalesce(visitor_account_id::text,'NULL')||'|'||coalesce(user_id::text,'NULL') "
                f"FROM claimed_businesses WHERE business_id='{bid}' AND is_active")
    check("the fresh claim carries NO account id (email-only claim, as the public form creates)",
          link == "NULL|NULL", f"claim={link}")

    # ── 3. the three owner-scoped handlers behind the guard ────────────────────
    routes = {
        "ad earnings (the card that vanished)": f"/api/v1/businesses/{bid}/ads/earnings",
        "business call log": f"/api/v1/businesses/{bid}/call-logs",
        "business visitor analytics": f"/api/v1/visitors/business/{bid}",
    }
    for label, path in routes.items():
        st, _ = http("GET", path, token=tok[owner_email])
        check(f"the claiming OWNER reaches {label}", st == 200, f"status={st} path={path}")
        st, body = http("GET", path, token=tok[stranger_email])
        check(f"a signed-in STRANGER is refused {label}",
              st == 404 and "not found" in json.dumps(body).lower(), f"status={st}")
        st, _ = http("GET", path)
        check(f"anonymous is refused {label}", st == 401, f"status={st}")

    # ── 4. anti-shadow: an email match can never reach a claim that already has an owner account ──
    psql(f"UPDATE claimed_businesses SET visitor_account_id='{ids[other_email]}', user_id=NULL "
         f"WHERE business_id='{bid}'")
    st_o, _ = http("GET", routes["ad earnings (the card that vanished)"], token=tok[owner_email])
    st_x, _ = http("GET", routes["ad earnings (the card that vanished)"], token=tok[other_email])
    check("once the claim is linked to ANOTHER account, the email-only match no longer opens it",
          st_o == 404, f"status={st_o}")
    check("the account the claim is actually linked to does open it", st_x == 200, f"status={st_x}")

    # ── 5. teardown, nothing left behind ───────────────────────────────────────
    psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    psql("DELETE FROM visitor_accounts WHERE email IN "
         f"('{owner_email}','{other_email}','{stranger_email}')")
    left = psql(f"SELECT (SELECT count(*) FROM businesses WHERE id='{bid}')"
                f"+ (SELECT count(*) FROM claimed_businesses WHERE business_id='{bid}')"
                " + (SELECT count(*) FROM visitor_accounts WHERE email LIKE 'zz-scope-%@swiftsoftware.dev')")
    check("probe data fully removed", left == "0", f"remaining={left}")

    print()
    with open(os.path.join(AUD, "run.log"), "w") as fh:
        fh.write(f"fails={len(FAILS)}\n" + "\n".join(FAILS) + "\n")
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
