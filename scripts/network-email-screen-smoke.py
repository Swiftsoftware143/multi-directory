#!/usr/bin/env python3
"""Network email — "ONE setup for all cities" — live end-to-end proof (ZaarHub queue item 3).

The send path was already proven (scripts/network-email-send-smoke.py: a city that owns no row
sends through the ONE network setup). This driver closes the FRONT-END MANDATE half: the operator
can SEE and use that one setup from the console, in every scope, and a city is never offered a copy
of its own.

Evidence, against the RUNNING app + a real browser:
  API arm (live DB + real HTTP):
    1. the database holds EXACTLY ONE mail setup (the premise of "one setup for all cities");
    2. the carrier directory reports own=true and the setup's identity;
    3. a NON-primary city in the same network reports own=false, inherited_from=<carrier>, and the
       SAME from-email / transport — i.e. it genuinely inherits the one setup;
    4. a bogus city slug does not silently return the network's setup (no wildcard inheritance).
  UI arm (real Chromium, the page's own controls): scripts/network-email-screen-browser.js

Run:  python3 scripts/network-email-screen-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089), AUD, CHROME_PATH
Exit: 0 = PASS, 1 = FAIL.  Read-only: it never saves settings.
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
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "network-email-screen-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"
MINT = "/opt/swift/browser-run/md-mint-token.js"
AUD = f"/opt/swift/audits/zz-item3-network-email-screen-{time.strftime('%Y%m%d-%H%M%S')}"
FAILS = []


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
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-F", "|", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    return [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]


def one(sql):
    rows = psql(sql)
    return rows[0] if rows else ""


def call(method, path, token=None, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + "/api/v1" + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, json.loads(r.read().decode() or "null")
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw or "null")
        except json.JSONDecodeError:
            return e.code, None


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def main():
    os.makedirs(AUD, exist_ok=True)

    # ── 1. the premise: exactly ONE mail setup in the whole database ─────────────
    n_rows = int(one("SELECT count(*) FROM directory_email_settings") or 0)
    check("the database holds exactly ONE mail setup for the network", n_rows == 1, f"rows={n_rows}")

    row = one("SELECT directory_id||'|'||transport||'|'||coalesce(from_email,'')||'|'||"
              "coalesce(from_name,'') FROM directory_email_settings LIMIT 1")
    carrier_id, transport, from_email, from_name = (row.split("|") + ["", "", "", ""])[:4]
    carrier_slug = one(f"SELECT slug FROM directories WHERE id='{carrier_id}'")
    net_id = one(f"SELECT coalesce(network_id::text,'') FROM directories WHERE id='{carrier_id}'")
    net_name = one(f"SELECT name FROM networks WHERE id='{net_id}'")
    n_cities = int(one(f"SELECT count(*) FROM directories WHERE network_id='{net_id}'") or 0)
    city = one(f"SELECT id||'|'||slug FROM directories WHERE network_id='{net_id}' "
               f"AND coalesce(is_primary,false)=false LIMIT 1")
    city_id, city_slug = (city.split("|") + ["", ""])[:2]
    print(f"DB truth: carrier={carrier_slug} ({carrier_id})  transport={transport}  "
          f"from={from_email}  network={net_name} ({n_cities} cities)  non-primary city={city_slug}")
    check("a non-primary city exists in the network to inherit the one setup",
          bool(city_id) and bool(carrier_slug), f"city={city_slug} carrier={carrier_slug}")

    tok = sh("node", MINT).stdout.strip()
    check("platform-operator session minted", tok.count(".") == 2, f"len={len(tok)}")

    # ── 2. the carrier owns the setup ────────────────────────────────────────────
    s, body = call("GET", f"/directories/{carrier_slug}/email-settings", tok)
    body = body or {}
    check("the carrier directory reports the setup as its own", s == 200 and body.get("own", True) is not False,
          f"status={s} own={body.get('own')}")
    check("the carrier's sender is the one saved setup",
          body.get("from_email") == from_email and body.get("transport") == transport,
          f"api={body.get('from_email')}/{body.get('transport')}")

    # ── 3. the non-primary city INHERITS the same one setup (no copy of its own) ──
    s, body = call("GET", f"/directories/{city_slug}/email-settings", tok)
    body = body or {}
    check("the non-primary city reports it does NOT own a setup", body.get("own") is False,
          f"status={s} own={body.get('own')} configured={body.get('configured')}")
    check("the city names the carrier it inherits from",
          body.get("inherited_from") == carrier_slug, f"inherited_from={body.get('inherited_from')}")
    check("the city sends with the SAME one setup (identical sender + transport)",
          body.get("from_email") == from_email and body.get("transport") == transport,
          f"city={body.get('from_email')}/{body.get('transport')} vs setup={from_email}/{transport}")

    # ── 4. no wildcard inheritance: a bogus city must not resolve to the network's setup ─────────
    s, body = call("GET", "/directories/zz-no-such-city-zz/email-settings", tok)
    body = body or {}
    wild = (body.get("configured") is True) and (body.get("from_email") == from_email)
    check("a city that does not exist does NOT silently inherit the network's setup",
          s != 200 or not wild, f"status={s} configured={body.get('configured')}")

    # ── the screen itself (real Chromium, the page's own controls) ───────────────
    chrome = os.environ.get("CHROME_PATH", "/usr/bin/chromium-browser")
    env = dict(os.environ, MD_BASE=BASE, TOKEN=tok, AUD=AUD, NODE_PATH=NODE_PATH, CHROME_PATH=chrome,
               NET_ID=net_id, CITY_ID=city_id, EXPECT_FROM=from_email,
               EXPECT_TRANSPORT=transport, EXPECT_NETNAME=net_name or "")
    r = sh("node", BROWSER, env=env, timeout=300)
    proof_path = os.path.join(AUD, "em-screen-proof.json")
    proof = {}
    if os.path.exists(proof_path):
        proof = json.load(open(proof_path))
    else:
        print(r.stdout[-1500:])
        print(r.stderr[-1500:])
    bchecks = proof.get("checks", [])
    check("the console walk produced a proof file", bool(bchecks), proof_path)
    for c in bchecks:
        check("browser: " + c.get("name", "?"), bool(c.get("ok")), c.get("detail", ""))
    check("the console walk had no failures (exit 0)", r.returncode == 0,
          f"rc={r.returncode} pass={proof.get('pass')} fail={proof.get('fail')}")

    print()
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)} failed)")
        for f in FAILS:
            print("  - " + f)
        sys.exit(1)
    print("RESULT: PASS")
    sys.exit(0)


if __name__ == "__main__":
    main()
