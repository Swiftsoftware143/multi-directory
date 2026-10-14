#!/usr/bin/env python3
"""SHADOW GUARD: an email match must NEVER resolve a claim that is already linked to another
account. Builds a business whose ONLY claim carries a different account id but the probe owner's
email, and asserts /portal/business/payouts still answers 403 for that owner.
"""
import json, os, subprocess, sys, time, urllib.error, urllib.request

BASE = "http://127.0.0.1:8089/api/v1"
TS = int(time.time())
HARNESS = f"payouts-shadow-{TS}"
H = {"Content-Type": "application/json", "X-Swift-Harness": HARNESS}
OWNER = f"probe-shadow-owner-{TS}@swiftsoftware.dev"
OTHER = f"probe-shadow-other-{TS}@swiftsoftware.dev"
PW = "ProbePass123!"
OUT = []

def call(method, path, body=None, token=None):
    req = urllib.request.Request(BASE + path,
                                 data=json.dumps(body).encode() if body is not None else None,
                                 method=method)
    for k, v in H.items():
        req.add_header(k, v)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            code, raw = r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        code, raw = e.code, e.read().decode()
    try:
        return code, json.loads(raw)
    except Exception:
        return code, raw[:200]

def psql(sql):
    url = subprocess.check_output(["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"]).decode().strip()
    pw = url.split("://", 1)[1].split("@", 1)[0].split(":", 1)[1]
    r = subprocess.run(["psql", "-h", "127.0.0.1", "-U", "swift", "-d", "multi_directory",
                        "-t", "-A", "-F", "|", "-c", sql],
                       capture_output=True, text=True, env=dict(os.environ, PGPASSWORD=pw))
    if r.returncode != 0:
        print("PSQL FAIL", r.stderr[:300]); sys.exit(4)
    return r.stdout.strip()

def rec(ok, name, detail=""):
    OUT.append((name, bool(ok), detail))
    print(("PASS " if ok else "** ") + name + ((" :: " + detail) if detail else ""))

def login(email):
    c, r = call("POST", "/auth/login", {"email": email, "password": PW})
    return r.get("access_token") if isinstance(r, dict) else None

def main():
    # the owner (the account whose email matches) and a different account that holds the claim
    c, r = call("POST", "/visitor/register", {"email": OWNER, "password": PW, "name": "Shadow Owner"})
    rec(c in (200, 201), "owner account created", f"http={c}")
    c, r = call("POST", "/visitor/register", {"email": OTHER, "password": PW, "name": "Shadow Other"})
    rec(c in (200, 201), "other account created", f"http={c}")
    otok, stok = login(OWNER), login(OTHER)
    rec(bool(otok) and bool(stok), "both accounts sign in")

    # a business + ONE claim linked to the OTHER account but carrying the OWNER's email
    c, r = call("POST", "/b2b/register", {"email": f"biz-{TS}@swiftsoftware.dev", "password": PW,
                                          "name": f"Shadow Biz {TS}", "phone": "4075550322",
                                          "business_type": "local"})
    rec(c in (200, 201), "probe business registered", f"http={c}")
    bid = psql(f"SELECT id FROM businesses WHERE email = 'biz-{TS}@swiftsoftware.dev' ORDER BY created_at DESC LIMIT 1;")
    psql(f"DELETE FROM claimed_businesses WHERE business_id = '{bid}';")
    other_id = psql(f"SELECT id FROM visitor_accounts WHERE email = '{OTHER}' LIMIT 1;")
    psql(f"INSERT INTO claimed_businesses (business_id, visitor_account_id, owner_email, owner_name, is_active, probe_harness) "
         f"VALUES ('{bid}', '{other_id}', '{OWNER}', 'Shadow Other', true, '{HARNESS}');")
    shape = psql(f"SELECT visitor_account_id IS NOT NULL, owner_email FROM claimed_businesses WHERE business_id = '{bid}';")
    rec(shape.startswith("t|" + OWNER), "claim is linked to ANOTHER account, carrying the owner's email", shape)

    c, body = call("GET", "/portal/business/payouts", token=otok)
    rec(c == 403, "the email must NOT shadow another account's claim (403)", f"http={c} body={str(body)[:110]}")
    c, body = call("GET", "/portal/business/payouts", token=stok)
    rec(c == 200, "the account that actually holds the claim IS resolved", f"http={c}")

    psql(f"DELETE FROM claimed_businesses WHERE business_id = '{bid}';")
    psql(f"DELETE FROM businesses WHERE id = '{bid}';")
    psql(f"DELETE FROM visitor_accounts WHERE probe_harness = '{HARNESS}';")
    left = psql(f"SELECT (SELECT COUNT(*) FROM businesses WHERE email = 'biz-{TS}@swiftsoftware.dev') + (SELECT COUNT(*) FROM visitor_accounts WHERE probe_harness = '{HARNESS}') + (SELECT COUNT(*) FROM claimed_businesses WHERE owner_email = '{OWNER}');")
    rec(left == "0", "cleanup removed every probe row", f"left={left}")

    bad = [n for n, ok, _ in OUT if not ok]
    print("\nHARNESS", HARNESS)
    print("RESULT:", "PASS" if not bad else "FAIL -> " + "; ".join(bad))
    return 0 if not bad else 1

sys.exit(main())
