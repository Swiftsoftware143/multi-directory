#!/usr/bin/env python3
"""Admin API-Access screen — live end-to-end proof (ZaarHub item 7).

`GET/POST/PUT/DELETE /api/v1/api-keys` (src/handlers/api_complete.rs, operator-guarded) plus the
admin console's "API Access & Webhooks" card are the surface an operator uses to issue credentials
for outside integrations. The defect this card closed (fix d081b39): a key could be created, listed
and verified on the screen but unlocked NOTHING — the global auth guard only looked for a browser
session, so every guarded route refused the key's owner (401). A key is now matched by the SHA-256
of the FULL secret, exchanged for a short-lived session token, and every call is recorded against
the key so the screen's "Last used" column and usage panel tell the truth.

Proves, against the running app and the live database:
  1. anonymous and ordinary-account callers cannot manage keys (401 / 403);
  2. the operator creates a key on the screen route -> 201 + the raw `md_…` secret returned ONCE;
  3. the database stores only the SHA-256 of that secret (the raw value is nowhere on the row) and
     the visible prefix is genuinely a prefix;
  4. THE FIX: the raw key, presented with NO browser session, unlocks operator-guarded routes
     (200) — before d081b39 this was 401. Two arms: the key's own management route AND the real
     integration endpoints an outside system actually calls (`/directories`, `/dashboard/stats`);
  5. the call is recorded: `last_used_at` is stamped and an `api_key_usage` row carries the
     endpoint / method / status, so the screen's usage panel is real;
  6. the prefix alone, or a fabricated `md_…`, authenticates nothing (401);
  7. revoking a key (or an already-expired key) stops it working at once (401);
  8. the live console page really renders this screen and calls the create / list / revoke / delete
     routes (the visible control, not a backend-only capability).

Run:  python3 scripts/api-keys-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Every probe key + the probe signup are removed in `finally`.
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
PANEL_URL = BASE + "/admin-panel.html"
TS = int(time.time())
EMAIL = f"probe-apikeys-{TS}@example.invalid"
PASSWORD = "Probe!apikeys1"
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


def call(method, path, token=None, body=None, raw_auth=False):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", ("Bearer " + token) if not raw_auth else token)
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def main():
    keystore = {"ids": [], "raws": []}

    # ---- anonymous: the global auth guard answers 401 ------------------------
    s, _ = call("GET", "/api/v1/api-keys")
    check("anonymous GET /api/v1/api-keys is refused", s == 401, f"status={s}")

    # ---- a plain free signup: operator_guard must refuse with 403 ------------
    s, body = call("POST", "/api/v1/auth/register", body={
        "email": EMAIL, "password": PASSWORD, "name": f"ZZ ApiKey Probe {TS}"})
    check("a plain account could be created (POST /auth/register)", s in (200, 201), f"status={s}")
    if s not in (200, 201):
        print(body[:300])
        return 1
    reg = json.loads(body)
    plain_token = reg.get("token") or reg.get("access_token")
    tenant_id = reg.get("tenant_id") or (reg.get("user") or {}).get("tenant_id")
    try:
        s, _ = call("GET", "/api/v1/api-keys", plain_token)
        check("an ordinary signed-in account is refused (403)", s == 403, f"status={s}")

        op = mint_operator_token()

        # ---- create a key on the screen route -------------------------------
        s, body = call("POST", "/api/v1/api-keys", op,
                       body={"name": f"ZZ api-key probe {TS}", "scopes": ["read"]})
        check("the operator can create a key (POST /api-keys) -> 201", s == 201, f"status={s}")
        if s != 201:
            print(body[:300])
            return 1
        created = json.loads(body)
        key_id = created.get("id")
        raw_key = created.get("raw_key")
        prefix = created.get("key_prefix")
        keystore["ids"].append(key_id)
        keystore["raws"].append(raw_key)
        check("the raw secret is returned ONCE and is an md_ key",
              bool(raw_key) and raw_key.startswith("md_"), f"raw_prefix={str(raw_key)[:8]}…")
        check("the raw secret is never part of the stored row shape",
              "key_hash" not in json.dumps(created))

        # ---- storage honesty -------------------------------------------------
        if raw_key:
            digest = hashlib.sha256(raw_key.encode()).hexdigest()
            stored_hash = psql(f"select key_hash from api_keys where id='{key_id}'")
            check("the row stores the SHA-256 of the full secret", stored_hash == digest,
                  f"len={len(stored_hash)}")
            check("the raw secret is NOT recoverable from the row",
                  psql(f"select count(*) from api_keys where key_hash='{raw_key}'"
                       f" or name='{raw_key}' or key_prefix='{raw_key}'") == "0")
            check("the visible prefix is a genuine prefix of the secret",
                  bool(prefix) and prefix in raw_key and len(prefix) < len(raw_key),
                  f"prefix={prefix}")

        # ---- THE FIX: the key unlocks a guarded route with NO session --------
        s, body = call("GET", "/api/v1/api-keys", raw_key, raw_auth=True)
        check("the issued key unlocks an operator-guarded route (200, no session)",
              s == 200, f"status={s}")
        listed = json.loads(body) if s == 200 else []
        check("the key's own row is visible through the key",
              any(k.get("id") == key_id for k in listed), f"rows={len(listed)}")

        # ---- THE INTEGRATION ARM: the key alone unlocks the endpoints outside systems call ----
        # (item 7 durable arm: the /tmp use-probe that first proved this was ephemeral and
        #  unmovable — its bearer values were redacted on disk; this arm re-proves it in-repo.)
        for ep in ("/api/v1/directories", "/api/v1/dashboard/stats"):
            s, body = call("GET", ep, raw_key, raw_auth=True)
            check(f"the issued key alone unlocks {ep} (200)", s == 200, f"status={s}")

        # ---- usage bookkeeping is real ---------------------------------------
        last_used = psql(f"select coalesce(last_used_at::text,'') from api_keys where id='{key_id}'")
        check("last_used_at is stamped after a real call", last_used != "", f"last_used={last_used}")
        used = psql(f"select count(*) from api_key_usage where api_key_id='{key_id}'"
                    f" and method='GET' and status_code=200")
        check("an api_key_usage row records the call (endpoint/method/status)",
              int(used) >= 1, f"rows={used}")
        for ep in ("/directories", "/dashboard/stats"):
            n = psql(f"select count(*) from api_key_usage where api_key_id='{key_id}'"
                     f" and endpoint like '%{ep}%' and status_code=200")
            check(f"the usage log records the real {ep} call (screen usage panel is honest)",
                  int(n) >= 1, f"rows={n}")

        # ---- verify endpoint --------------------------------------------------
        s, body = call("POST", "/api/v1/api-keys/verify", raw_key, raw_auth=True)
        check("POST /api-keys/verify accepts the key (200)", s == 200, f"status={s}")
        if s == 200:
            v = json.loads(body)
            check("verify returns the key's own identity",
                  v.get("key_id") == key_id and v.get("valid") is True,
                  f"key_id={v.get('key_id')} valid={v.get('valid')}")

        # ---- lookalikes must authenticate NOTHING ---------------------------
        if prefix:
            s, _ = call("GET", "/api/v1/api-keys", "md_" + prefix, raw_auth=True)
            check("the visible prefix (as md_…) authenticates nothing (401)", s == 401, f"status={s}")
        s, _ = call("GET", "/api/v1/api-keys", "md_deadbeef_" + "x" * 40, raw_auth=True)
        check("a fabricated md_ key authenticates nothing (401)", s == 401, f"status={s}")

        # ---- revoke stops it at once ----------------------------------------
        s, _ = call("PUT", f"/api/v1/api-keys/{key_id}", op, body={"is_active": False})
        check("the operator can revoke the key (PUT is_active=false)", s == 200, f"status={s}")
        s, _ = call("GET", "/api/v1/api-keys", raw_key, raw_auth=True)
        check("a revoked key stops working at once (401)", s == 401, f"status={s}")

        # ---- an already-expired key is refused -------------------------------
        s, body = call("POST", "/api/v1/api-keys", op, body={
            "name": f"ZZ api-key expired probe {TS}",
            "expires_at": "2020-01-01T00:00:00Z"})
        if s == 201:
            exp = json.loads(body)
            keystore["ids"].append(exp.get("id"))
            keystore["raws"].append(exp.get("raw_key"))
            s, _ = call("GET", "/api/v1/api-keys", exp.get("raw_key"), raw_auth=True)
            check("an already-expired key is refused (401)", s == 401, f"status={s}")
        else:
            check("the operator can create an expiring key", s == 201, f"status={s}")

        # ---- the live console really renders this screen ---------------------
        try:
            with urllib.request.urlopen(PANEL_URL, timeout=20) as r:
                html = r.read().decode("utf-8", "replace")
        except Exception as e:  # noqa: BLE001
            html = ""
            check("the live console page is reachable", False, str(e))
        else:
            check("the live console page is reachable", True, f"{len(html)} bytes")
            check("the console has the API Access screen", "developer-api-section" in html)
            check("the screen has a Create-key control", "__akCreate" in html and "ak-name" in html)
            check("the screen lists keys (list control)", "ak-list" in html and "loadApiKeys" in html)
            check("the screen calls POST /api-keys", "apiPost('/api-keys'" in html)
            check("the screen calls PUT /api-keys/:id (revoke)",
                  "apiPut('/api-keys/'" in html)
            check("the screen calls DELETE /api-keys/:id",
                  "apiFetch('DELETE', '/api-keys/'" in html)
            check("the list shows a Last-used column (the usage proof is visible)",
                  "last_used_at" in html)
        return 1 if FAILS else 0
    finally:
        for kid in keystore["ids"]:
            psql(f"DELETE FROM api_keys WHERE id='{kid}'")
        psql(f"DELETE FROM users WHERE email='{EMAIL}'")
        if tenant_id:
            psql(f"DELETE FROM tenants WHERE id='{tenant_id}'")
        left = psql(
            f"SELECT (SELECT count(*) FROM api_keys WHERE name LIKE 'ZZ api-key%{TS}%')"
            f" + (SELECT count(*) FROM users WHERE email='{EMAIL}')"
            f" + (SELECT count(*) FROM api_key_usage u WHERE NOT EXISTS"
            f" (SELECT 1 FROM api_keys k WHERE k.id = u.api_key_id))")
        check("probe keys, signup and their usage rows fully removed", left == "0",
              f"remaining={left}")


if __name__ == "__main__":
    rc = main()
    print("\nRESULT: " + ("PASS" if rc == 0 else "FAIL"))
    sys.exit(rc)
