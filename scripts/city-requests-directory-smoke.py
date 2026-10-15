#!/usr/bin/env python3
"""City-request directory attribution — live end-to-end proof (item 18, ZaarHub).

The defect this pins: `POST /api/v1/city-requests` accepted a `directory_id` and threw it away,
so every vote landed with `directory_id` NULL. The operator console's "Requests for a directory"
screen (`GET /api/v1/admin/directories/:id/city-requests`) filters on exactly that column, so the
screen could never show a request made from a city page — measured on the live build: the row
landed NULL and the scoped list answered `[]`.

Arms (all against the running app, all through the public/operator endpoints):
  1. a vote submitted with a real `directory_id` is stored WITH it;
  2. that vote is visible on the directory-scoped operator screen;
  3. the platform-wide list still shows it;
  4. voting again keeps the first attribution and bumps the count;
  5. a directory id that names no directory is ignored (the vote still succeeds, not a 500);
  6. teardown removes every probe row.

Run:  python3 scripts/city-requests-directory-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Nothing is left behind on either path.
"""
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
API = BASE + "/api/v1"
MINT = "/opt/swift/browser-run/md-mint-token.js"
MARK = "ZZQ-Probe"
FAILS = []


def sh(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


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
    return [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]


def one(sql):
    rows = psql(sql)
    return rows[0] if rows else ""


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def call(method, path, token=None, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(API + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def main():
    slug, dir_id = one("SELECT slug || '|' || id FROM directories ORDER BY slug LIMIT 1").split("|")
    print(f"# directory under test: {slug} {dir_id}")

    st, body = call("GET", "/health")
    check("the app answers /api/v1/health", st == 200, f"status={st}")

    tok = sh("node", MINT).stdout.strip()
    if not tok:
        tok = open("/tmp/.mdtok").read().strip()
    check("an operator token can be minted", tok.count(".") == 2)

    city = f"{MARK} City"
    psql(f"DELETE FROM city_requests WHERE city_name LIKE '{MARK}%'")

    # 1. vote WITH a real directory
    st, body = call("POST", "/city-requests",
                    body={"city_name": city, "state": "TS", "directory_id": dir_id})
    check("a city vote can be submitted with a directory", st in (200, 201), f"status={st}")
    stored = one(f"SELECT COALESCE(directory_id::text,'NULL') FROM city_requests WHERE city_name='{city}'")
    check("the stored row carries the directory (defect: it used to be NULL)",
          stored == dir_id, f"stored={stored}")

    # 2. the directory-scoped operator screen
    st, body = call("GET", f"/admin/directories/{dir_id}/city-requests", token=tok)
    ids = [r.get("city_name") for r in (json.loads(body).get("requests") or [])] if st == 200 else []
    check("the directory-scoped screen shows the vote", st == 200 and city in ids,
          f"status={st} names={ids[:4]}")

    # 3. the platform-wide list
    st, body = call("GET", "/city-requests")
    names = [r.get("city_name") for r in (json.loads(body).get("requests") or [])] if st == 200 else []
    check("the platform-wide list still shows the vote", st == 200 and city in names,
          f"status={st}")

    # 4. voting again keeps the first attribution and bumps the count
    st, body = call("POST", "/city-requests",
                    body={"city_name": city, "state": "TS", "directory_id": dir_id})
    votes = one(f"SELECT votes::text FROM city_requests WHERE city_name='{city}'")
    stored2 = one(f"SELECT COALESCE(directory_id::text,'NULL') FROM city_requests WHERE city_name='{city}'")
    check("a second vote bumps the count and keeps the attribution",
          st in (200, 201) and votes == "2" and stored2 == dir_id, f"votes={votes} stored={stored2}")

    # 5. a bogus directory id is ignored, not fatal
    bogus = "00000000-0000-0000-0000-00000000dead"
    st, body = call("POST", "/city-requests",
                    body={"city_name": f"{MARK} Bogus", "state": "TS", "directory_id": bogus})
    bog = one(f"SELECT COALESCE(directory_id::text,'NULL') FROM city_requests WHERE city_name='{MARK} Bogus'")
    check("an unknown directory id cannot turn a vote into a 500",
          st in (200, 201) and bog == "NULL", f"status={st} stored={bog}")

    # teardown
    psql(f"DELETE FROM city_requests WHERE city_name LIKE '{MARK}%'")
    left = one(f"SELECT COUNT(*)::text FROM city_requests WHERE city_name LIKE '{MARK}%'")
    check("probe data fully removed", left == "0", f"remaining={left}")

    print()
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)})")
        return 1
    print("RESULT: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
