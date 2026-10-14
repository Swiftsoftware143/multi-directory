#!/usr/bin/env python3
"""REPRO: a listing owner whose claim came from the PUBLIC claim route (owner_email only, no
account id) is shown their listing by the business portal, but the portal's own payouts surface
cannot resolve the caller's business -> 403 'No claimed business is linked to this account yet.'

Run against the live API (127.0.0.1:8089). Creates probe rows through REAL routes; cleans up.
Exit 0 only if the owner IS resolved (200). Prints FAIL while the defect stands.
"""
import json, os, subprocess, sys, time, urllib.error, urllib.request

BASE = "http://127.0.0.1:8089/api/v1"
TS = int(time.time())
HARNESS = f"payouts-claim-gap-{TS}"
H = {"Content-Type": "application/json", "X-Swift-Harness": HARNESS}
OWNER = f"probe-payout-owner-{TS}@swiftsoftware.dev"
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
    c, r = call("POST", "/b2b/register", {"email": OWNER, "password": PW,
                                          "name": f"Probe Payout Owner {TS}",
                                          "phone": "4075550311", "business_type": "local"})
    rec(c in (200, 201), "probe listing registered", f"http={c}")
    bid = psql(f"SELECT id FROM businesses WHERE email = '{OWNER}' ORDER BY created_at DESC LIMIT 1;")
    rec(bool(bid), "listing row exists", bid)
    psql(f"DELETE FROM claimed_businesses WHERE business_id = '{bid}';")
    c, r = call("POST", f"/businesses/{bid}/claim", {"owner_email": OWNER,
                                                     "owner_name": f"Probe Payout Owner {TS}",
                                                     "owner_phone": "4075550311"})
    rec(c in (200, 201), "public claim route accepted", f"http={c}")
    shape = psql(f"SELECT (visitor_account_id IS NULL)::text || '/' || (user_id IS NULL)::text || '/' || is_active::text FROM claimed_businesses WHERE business_id = '{bid}';")
    rec(shape.startswith("true/true/t"), "claim is linked by email ONLY (no account id)", shape)

    tok = login(OWNER)
    rec(bool(tok), "owner signed in", "")

    c, prof = call("GET", "/portal/business/profile", token=tok)
    ids = [x.get("business", {}).get("id") for x in (prof.get("claimed_businesses") or [])] if isinstance(prof, dict) else []
    rec(c == 200 and bid in ids, "portal LISTS the owner's listing", f"http={c} n={len(ids)}")

    for path in ["/portal/business/payouts"]:
        c, body = call("GET", path, token=tok)
        rec(c == 200, f"owner CAN reach {path}", f"http={c} body={str(body)[:140]}")

    # a stranger must still be refused
    c, r = call("POST", "/visitor/register", {"email": f"stranger-{TS}@swiftsoftware.dev",
                                              "password": PW, "name": "Stranger"})
    stok = login(f"stranger-{TS}@swiftsoftware.dev")
    c, body = call("GET", "/portal/business/payouts", token=stok)
    rec(c in (403, 404), "a stranger is still refused", f"http={c}")

    # cleanup
    psql(f"DELETE FROM claimed_businesses WHERE business_id = '{bid}';")
    psql(f"DELETE FROM businesses WHERE id = '{bid}';")
    psql(f"DELETE FROM visitor_accounts WHERE probe_harness = '{HARNESS}';")
    left = psql(f"SELECT (SELECT COUNT(*) FROM businesses WHERE email = '{OWNER}') + (SELECT COUNT(*) FROM visitor_accounts WHERE probe_harness = '{HARNESS}');")
    rec(left == "0", "cleanup removed every probe row", f"left={left}")

    bad = [n for n, ok, _ in OUT if not ok]
    print("\nHARNESS", HARNESS)
    print("RESULT:", "PASS" if not bad else "FAIL -> " + "; ".join(bad))
    return 0 if not bad else 1

sys.exit(main())
