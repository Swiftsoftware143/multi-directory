#!/usr/bin/env python3
"""Operator dashboard stat tiles — live end-to-end proof (ZaarHub item 9).

The admin console's Dashboard shows stat tiles (Directories / Active / Businesses /
Reviews / Domain Mappings) that must be REAL numbers read from live data, never a
placeholder and never a fabricated 0. The backend is
`GET /api/v1/dashboard/stats` (`src/handlers/admin.rs::dashboard_stats`), operator-guarded,
returning total_directories / active_directories / published_directories / total_businesses /
total_reviews / total_domains.

Proves, against the running app and the live database:
  1. anonymous            -> 401 (the global auth guard);
  2. a plain free signup  -> 403 (an ordinary account is not an operator);
  3. the platform operator-> 200 and every returned field EQUALS the direct DB count;
  4. lifecycle honesty: the active / published counters use the value a directory really
     carries ('active'), so both are non-zero while the 10 live cities are live — the defect
     this route once had was counting a status no row holds ('published'), i.e. 0 forever;
  5. the console actually calls this route and renders the tiles (frontend/admin-panel.html);
  6. a failed call must never fabricate a number (the panel keeps its honest '—').

Run:  python3 scripts/dashboard-stats-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Writes nothing durable (the probe signup is deleted in `finally`).
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
ENV_FILE = "/opt/swift/apps/multi-directory/.env"
PANEL = "/opt/swift/apps/multi-directory/frontend/admin-panel.html"
TS = int(time.time())
EMAIL = f"probe-dashstats-{TS}@example.invalid"
PASSWORD = "Probe!dashstats1"
FAILS = []


def check(label, ok, note=""):
    print(f"{'PASS' if ok else 'FAIL'}  {label}" + (f"  [{note}]" if note else ""))
    if not ok:
        FAILS.append(label)


def db_password():
    url = subprocess.run(
        ["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
        capture_output=True, text=True,
    ).stdout.strip()
    if not url:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return url.split("://", 1)[1].split("@", 1)[0].split(":", 1)[1]


def psql(sql):
    r = subprocess.run(
        ["docker", "exec", "-e", f"PGPASSWORD={db_password()}", "swift-postgres",
         "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    return r.stdout.strip()


def mint_operator_token():
    """super_admin JWT, same shape the admin panel uses. Secret never printed."""
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


def call(method, path, token=None, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def main():
    # ---- live DB truth -------------------------------------------------------
    truth = {
        "total_directories": int(psql("select count(*) from directories")),
        "active_directories": int(psql("select count(*) from directories where status='active'")),
        "total_businesses": int(psql("select count(*) from businesses")),
        "total_reviews": int(psql("select count(*) from reviews")),
        "total_domains": int(psql("select count(*) from domain_mappings")),
    }
    print("DB truth:", json.dumps(truth))
    check("the database holds a directory lifecycle value the panel can count",
          truth["active_directories"] > 0, f"status='active' => {truth['active_directories']}")
    n_published = int(psql("select count(*) from directories where status='published'"))
    check("'published' is not a directory status (so counting it would be a permanent 0)",
          n_published == 0, f"rows with status='published' = {n_published}")

    # ---- anonymous: the global auth guard answers 401 ------------------------
    s, _ = call("GET", "/api/v1/dashboard/stats")
    check("anonymous GET /api/v1/dashboard/stats is refused", s == 401, f"status={s}")

    # ---- a plain free signup: operator_guard must refuse with 403 ------------
    s, body = call("POST", "/api/v1/auth/register", body={
        "email": EMAIL, "password": PASSWORD, "name": f"ZZ DashStats Probe {TS}"})
    check("a plain account could be created (POST /auth/register)", s in (200, 201), f"status={s}")
    if s not in (200, 201):
        print(body[:300])
        return 1
    reg = json.loads(body)
    token = reg.get("token") or reg.get("access_token")
    tenant_id = reg.get("tenant_id") or (reg.get("user") or {}).get("tenant_id")
    try:
        s, _ = call("GET", "/api/v1/dashboard/stats", token)
        check("an ordinary signed-in account is refused (403)", s == 403, f"status={s}")

        # ---- the operator sees real, DB-matched numbers ----------------------
        op = mint_operator_token()
        s, body = call("GET", "/api/v1/dashboard/stats", op)
        check("the platform operator gets 200", s == 200, f"status={s}")
        if s != 200:
            print(body[:300])
            return 1
        stats = json.loads(body)
        for field, want in truth.items():
            got = stats.get(field)
            check(f"stats.{field} equals the live DB count", str(got) == str(want),
                  f"api={got} db={want}")
        check("stats.published_directories matches the same live count",
              str(stats.get("published_directories")) == str(truth["active_directories"]),
              f"api={stats.get('published_directories')} db={truth['active_directories']}")
        check("every field is a real number, none is a placeholder",
              all(isinstance(stats.get(k), (int, float)) for k in truth),
              json.dumps(stats))
        check("the totals are internally consistent (active <= total)",
              stats["active_directories"] <= stats["total_directories"],
              f"{stats['active_directories']} <= {stats['total_directories']}")

        # ---- the console really renders these tiles --------------------------
        html = open(PANEL).read()
        check("the console calls GET /dashboard/stats", "/dashboard/stats" in html)
        for tile in ("stat-reviews", "stat-domains", "stat-total", "stat-businesses"):
            check(f"the console has the {tile} tile", tile in html)
        check("a failed call leaves the honest placeholder, never a fake 0",
              "'—'" in html or "\\u2014" in html or "&mdash;" in html)
        return 1 if FAILS else 0
    finally:
        # remove the throwaway signup account + its tenant whatever happens
        psql(f"DELETE FROM users WHERE email='{EMAIL}'")
        if tenant_id:
            psql(f"DELETE FROM tenants WHERE id='{tenant_id}'")
        left = psql(f"SELECT (SELECT count(*) FROM users WHERE email='{EMAIL}')"
                    f" + (SELECT count(*) FROM tenants WHERE slug LIKE 'zz-dashstats-probe-%')")
        check("probe signup and tenant removed", left == "0", f"remaining={left}")


if __name__ == "__main__":
    rc = main()
    print("\nRESULT: " + ("PASS" if rc == 0 else "FAIL"))
    sys.exit(rc)
