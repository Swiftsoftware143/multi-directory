#!/usr/bin/env python3
"""SEO / sitemap admin surface — caller gate, live (ZaarHub item 12 domain).

The seo.rs handlers check NO caller, and the app's flat role model hands every public signup
the tenant role `admin`, so before this change any free account could read every city's
sitemap config, rewrite ANOTHER city's config and force a global regeneration. The routes now
carry the same `operator_guard` as the rest of the operation console (the sibling
/seo/submission-settings was already operator-gated inside its handler).

Proves, against the running app:
  * anonymous            -> 401 on every route (the global auth guard);
  * a plain free signup  -> 403 on every route (the hole: was 200/200/200);
  * the platform operator-> 200 on the read, and a NET-ZERO write still saves;
  * control: the same plain signup already got 403 on /seo/submission-settings (proving the
    signup token is a legitimate, non-operator session and not a broken token).

Writes nothing durable: the signup account + tenant are deleted in `finally`, and the operator
write re-sends the row's own current values (COALESCE keeps them).
Run:  python3 scripts/seo-admin-gate-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import base64
import hashlib
import hmac
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
ENV_FILE = "/opt/swift/apps/multi-directory/.env"
NIL = "00000000-0000-0000-0000-000000000000"
TS = int(time.time())
EMAIL = f"probe-seogate-{TS}@example.invalid"
PASSWORD = "Probe!seogate1"
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
    env = dict(os.environ, PGPASSWORD=db_password())
    return subprocess.run(
        ["psql", "-h", "127.0.0.1", "-U", "swift", "-d", "multi_directory", "-tAc", sql],
        capture_output=True, text=True, env=env,
    ).stdout.strip()


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
    dir_id = psql("select id from directories where slug='apopka'")
    row_sql = ("select auto_generate||'|'||priority||'|'||change_freq from sitemap_config "
               f"where directory_id='{dir_id}'")
    before = psql(row_sql)
    check("a city to probe exists with a sitemap config row", bool(dir_id) and bool(before),
          f"apopka row='{before}'")

    routes = [
        ("GET", "/api/v1/seo/sitemap-config", None),
        ("PUT", f"/api/v1/seo/sitemap-config/{dir_id}", {}),
        ("GET", f"/api/v1/seo/sitemap-config/{dir_id}", None),
        ("POST", "/api/v1/seo/regenerate-sitemap", {}),
        ("GET", f"/api/v1/seo/website/{NIL}", None),
        ("PUT", f"/api/v1/seo/website/{NIL}", {}),
    ]

    # ---- anonymous: the global auth guard answers 401 -------------------------
    for method, path, body in routes:
        s, _ = call(method, path, None, body)
        check(f"anonymous {method} {path} is refused", s == 401, f"status={s}")

    # ---- a plain free signup: operator_guard must refuse with 403 -------------
    s, body = call("POST", "/api/v1/auth/register", body={
        "email": EMAIL, "password": PASSWORD, "name": f"ZZ SEO Gate Probe {TS}"})
    check("a plain account could be created (POST /auth/register)", s in (200, 201), f"status={s}")
    if s not in (200, 201):
        print(body[:300])
        return 1
    reg = json.loads(body)
    token = reg.get("token") or reg.get("access_token")
    tenant_id = reg.get("tenant_id") or (reg.get("user") or {}).get("tenant_id")
    role = reg.get("role") or (reg.get("user") or {}).get("role")
    check("the signup is a normal signed-in account, not an operator", bool(token) and role != "super_admin",
          f"role={role}")
    try:
        # control: a sibling route that was ALREADY operator-gated answers the same token with 403
        s, _ = call("PUT", "/api/v1/seo/submission-settings", token, {})
        check("control: the same account is refused where the gate already existed", s == 403, f"status={s}")

        for method, path, body in routes:
            s, _ = call(method, path, token, body)
            check(f"a free signup cannot {method} {path}", s == 403, f"status={s}")

        # ---- the operator keeps working --------------------------------------
        op = mint_operator_token()
        s, body = call("GET", "/api/v1/seo/sitemap-config", op)
        n = len(json.loads(body)) if s == 200 else "n/a"
        check("the platform operator can still list every sitemap config", s == 200, f"status={s} configs={n}")
        s, body = call("PUT", f"/api/v1/seo/sitemap-config/{dir_id}", op, {})
        check("the platform operator can still save a sitemap config (net-zero body)", s == 200, f"status={s}")
    finally:
        after = psql(row_sql)
        check("the probed sitemap row is unchanged (net zero)", after == before, f"'{before}' -> '{after}'")
        psql(f"delete from users where email='{EMAIL}'")
        if tenant_id:
            psql(f"delete from tenants where id='{tenant_id}'")
        left = psql(f"select count(*) from users where email='{EMAIL}'")
        check("probe account removed", left == "0", f"users_left={left}")

    if FAILS:
        print(f"\nRESULT: FAIL ({len(FAILS)}): " + "; ".join(FAILS))
        return 1
    print("\nRESULT: PASS")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # noqa: BLE001
        print(f"RESULT: FAIL (exception: {e})")
        sys.exit(1)
