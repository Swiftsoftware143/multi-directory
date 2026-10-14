#!/usr/bin/env python3
"""Register / slug-walk regression proof — the NINTH signup under a shared name must still work.

Defect pinned (measured 2026-10-14, item 8's own proof died of it): `POST /api/v1/auth/register`
derives `tenants.slug` from the display name and, on a UNIQUE violation, walked a FIXED candidate
list (`base`, then `base-2 … base-8`). `probe-webhooks` … `probe-webhooks-8` had accumulated in the
live `tenants` table (eight leaked runs), so the ninth signup collided on every candidate and the
loop returned the unique violation: a bare `500 {"message":"Database error"}` — a real person could
not create an account. The walk now tries the readable `-2 … -9` and then a random suffix, so it
converges instead of running out.

This drives the RUNNING app: nine real signups sharing one display name, then asserts
  1. every one answers 200/201 (no 500 anywhere, including the ninth);
  2. every signup got a DISTINCT tenant slug, and all nine rows exist in the database;
  3. the ninth slug is still derived from the base name (readable or random-suffixed, never blank).
Then it removes every row it created (FK-safe order) and asserts no residue.

Run:  python3 scripts/register-slug-exhaustion-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
TS = int(time.time())
NAME = f"ZZ Slug Walk {TS}"
TNAME = f"{NAME}'s Directory"  # what register stores in tenants.name
EMAIL_FMT = f"zz-slug-walk-{TS}-{{}}@example.invalid"
PASSWORD = "Probe!slugwalk1"
WANT = 12  # 9 collide with the readable list (base + -2 … -9), 3 more must fall through to a
#            random suffix — the old fixed list died at the ninth
FAILS = []


def check(label, ok, note=""):
    print(f"{'PASS' if ok else 'FAIL'}  {label}" + (f"  [{note}]" if note else ""))
    if not ok:
        FAILS.append(label)


def db_password():
    url = subprocess.run(["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
                         capture_output=True, text=True).stdout.strip()
    if not url:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return url.split("://", 1)[1].split("@", 1)[0].split(":", 1)[1]


def psql(sql):
    r = subprocess.run(["docker", "exec", "-e", f"PGPASSWORD={db_password()}", "swift-postgres",
                        "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql],
                       capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    return r.stdout.strip()


def call(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def main():
    print(f"=== register slug-walk proof — {time.strftime('%Y-%m-%d %H:%M:%S')} ===")
    codes = []
    try:
        for i in range(WANT):
            s, body = call("POST", "/api/v1/auth/register",
                           body={"email": EMAIL_FMT.format(i), "password": PASSWORD, "name": NAME})
            codes.append(s)
            if s not in (200, 201):
                print(f"      signup #{i + 1} -> {s} {body[:160]}")

        check(f"all {WANT} signups sharing one display name succeeded (no 500)",
              all(c in (200, 201) for c in codes), f"codes={codes}")
        check("the ninth signup (the old fixed list's dead end) answered 200/201",
              codes[8] in (200, 201) if len(codes) > 8 else False,
              f"code={codes[8] if len(codes) > 8 else 'none'}")
        check("the signups past the readable list (the random-suffix arm) answered 200/201",
              all(c in (200, 201) for c in codes[9:]), f"codes={codes[9:]}")

        esc = TNAME.replace("'", "''")
        slugs = psql(f"SELECT string_agg(slug, ',') FROM tenants WHERE name = '{esc}'")
        got = [x for x in (slugs or "").split(",") if x]
        check(f"all {WANT} tenants exist in the database", len(got) == WANT, f"rows={len(got)}")
        check("every tenant slug is distinct", len(set(got)) == len(got), f"slugs={got}")
        check("the ninth slug still derives from the base name (not blank)",
              bool(got) and len(got[-1]) > 0 and "zz-slug-walk" in got[-1], f"last={got[-1] if got else None}")
    finally:
        esc = TNAME.replace("'", "''")
        tid_rows = psql(f"SELECT id::text FROM tenants WHERE name = '{esc}'")
        tids = [x for x in tid_rows.splitlines() if x.strip()]
        if tids:
            inlist = ",".join(f"'{t}'" for t in tids)
            for tbl in ("directory_events", "webhooks", "api_keys", "provider_keys",
                        "tracked_links", "tag_rules"):
                psql(f"DELETE FROM {tbl} WHERE tenant_id IN ({inlist})")
            psql(f"DELETE FROM users WHERE tenant_id IN ({inlist})")
            psql(f"DELETE FROM tenants WHERE id IN ({inlist})")
        psql(f"DELETE FROM visitor_accounts WHERE email LIKE 'zz-slug-walk-{TS}-%'")
        left = psql(f"SELECT COUNT(*) FROM tenants WHERE name = '{esc}'")
        check("the probe removed every tenant and account it created (no residue)",
              left == "0", f"tenants_left={left}")

    print()
    print("RESULT: " + ("PASS" if not FAILS else "FAIL"))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
