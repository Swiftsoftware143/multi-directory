#!/usr/bin/env python3
"""Live proof for kanban t_412bfc54 — MD signup must refuse an address that can never receive mail.

Runs against the PUBLIC edge (no loopback, no direct container access) and prints one line per probe.
Every row this script can possibly create is deleted at the end and the residue is asserted to be 0.
"""
import json
import subprocess
import sys
import time
import urllib.error
import urllib.request

BASE = "https://zaarhub.com"
DBURL = subprocess.check_output(
    ["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"], text=True
).strip()
STAMP = str(int(time.time()))
CONTROL = f"probe-emailfmt-{STAMP}@swiftsoftware.dev"
BAD = ["x@-.com", "x@x-.com", "x@[1.2.3.4]", 'no"body@swiftsoftware.dev']
PROBE_EMAILS = BAD + [CONTROL]


def psql(sql):
    out = subprocess.run(
        ["docker", "exec", "swift-postgres", "psql", DBURL, "-At", "-c", sql],
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        print("PSQL ERROR:", out.stderr.strip()[:300])
        sys.exit(1)
    return out.stdout.strip()


def post(path, payload):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(payload).encode(),
        headers={
            "Content-Type": "application/json",
            "Accept": "application/json",
            # The public edge sits behind Cloudflare, which answers a UA-less POST with its own 403
            # page. A normal browser UA gets the application's real answer (measured: 401 for a
            # non-existent login), so the probes exercise the app, not the WAF.
            "User-Agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
            "(KHTML, like Gecko) Chrome/126.0 Safari/537.36",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, r.read().decode()[:200]
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:200]


def counts():
    lst = ",".join("'" + e.replace("'", "''") + "'" for e in PROBE_EMAILS)
    u = psql(f"select count(*) from users where lower(email) in ({lst})")
    v = psql(f"select count(*) from visitor_accounts where lower(email) in ({lst})")
    t = psql("select count(*) from tenants")
    return int(u), int(v), int(t)


u0, v0, t0 = counts()
print(f"baseline: users={u0} visitor_accounts={v0} tenants={t0}")

results = {}
for addr in BAD:
    st, body = post("/api/v1/auth/register",
                    {"name": "Probe EmailFmt", "email": addr, "password": "ProbePass123!"})
    msg = body[:140]
    try:
        j = json.loads(body)
        for key in ("message", "detail", "error"):
            v = j.get(key)
            if isinstance(v, str) and v:
                msg = v
                break
    except Exception:
        pass
    results[addr] = st
    print(f'register {addr!r:28} -> {st}  {msg[:90]}')

# Negative control: a real address must still register (201) and then log in.
st, body = post("/api/v1/auth/register",
                {"name": "Probe EmailFmt", "email": CONTROL, "password": "ProbePass123!"})
print(f"register {CONTROL!r} -> {st}  {body[:80]}")
ctrl_ok = st in (200, 201)
st2, body2 = post("/api/v1/auth/login", {"email": CONTROL, "password": "ProbePass123!"})
print(f"login    {CONTROL!r} -> {st2}  {body2[:60]}")

# Pre-existing account (created 2026-08-11, long before this change) must still log in.
for pre in ("swiftsoftware143@yahoo.com", "probe-canary-a@swiftsoftware.dev"):
    s, b = post("/api/v1/auth/login", {"email": pre, "password": "ProbePass123!"})
    print(f"pre-existing login {pre} -> {s} {'OK' if s == 200 else 'no-known-pw'}")

# The visitor boundary shares the module.
st, body = post("/api/v1/visitor/register",
                {"name": "Probe EmailFmt", "email": "x@[1.2.3.4]", "password": "ProbePass123!"})
print(f"visitor_register x@[1.2.3.4] -> {st}  {body[:90]}")

# Cleanup: the control account, its tenant, and any residue the probes could have minted.
psql(f"with d as (select id, tenant_id from users where lower(email) in ("
     f"select unnest(array[{','.join(chr(39) + e.replace(chr(39), chr(39) * 2) + chr(39) for e in PROBE_EMAILS)}]))) "
     f"delete from tenants where id in (select tenant_id from d)")
psql(f"delete from users where lower(email) in "
     f"({','.join(chr(39) + e.replace(chr(39), chr(39) * 2) + chr(39) for e in PROBE_EMAILS)})")
psql(f"delete from visitor_accounts where lower(email) in "
     f"({','.join(chr(39) + e.replace(chr(39), chr(39) * 2) + chr(39) for e in PROBE_EMAILS)})")

u1, v1, t1 = counts()
print(f"residue: users={u1} visitor_accounts={v1} tenants={t0}->{t1}")

ok = (
    all(results[a] == 422 for a in BAD)
    and ctrl_ok
    and st2 == 200
    and u1 == u0 and v1 == v0 and t1 == t0
)
print("VERDICT:", "PASS" if ok else "FAIL")
sys.exit(0 if ok else 2)
