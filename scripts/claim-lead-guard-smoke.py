#!/usr/bin/env python3
"""Item 18 — a fixture claim must not leave a sales lead in the operator's CRM.

A browser-driven proof cannot send the `X-Swift-Harness` header, so its claim reached the app
looking exactly like a real owner's and minted a permanent "<business> - Claimed (<city>)" row in
`crm_deal_records`. Twenty-seven of those had accumulated in the live pipeline by 2026-11-05.

The fix: a claim whose address is on a reserved fixture domain never mirrors into the pipeline.
This proof drives the REAL public claim route twice, with no harness header on either arm:

  A. reserved fixture address (@swiftsoftware.dev)  -> claim 201, ZERO deal rows
  B. ordinary address (@swiftsoftware.net)          -> claim 201, exactly ONE deal row

B is what stops the fix from being "leads silently stopped working". Teardown removes the
listings, claims, visitor accounts and both deal rows; the run fails if anything is left.

Run:  python3 scripts/claim-lead-guard-smoke.py
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
MARK = "claim-lead-guard"
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
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}"[:600])
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    return lines[0] if lines else ""


def cssql(q):
    """Run SQL against CoreSwift (coreswift_crm) — the cross-app mirror target."""
    r = sh("/opt/swift/bin/swift-psql.sh", "coreswift_crm", "-t", "-A", "-F", "|", "-c", q)
    if r.returncode != 0:
        raise RuntimeError(f"coreswift psql failed: {r.stderr.strip()}"[:400])
    return [ln for ln in r.stdout.strip().splitlines() if ln]


def http(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=45) as resp:
            raw = resp.read().decode()
            return resp.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw)
        except json.JSONDecodeError:
            return e.code, raw


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def sweep():
    psql("DELETE FROM crm_deal_records WHERE title LIKE 'ZZ Claim Lead Guard%'")
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-guard-%'")


def sweep_coreswift(window=0):
    """Delete the CoreSwift contact mirror the claim route fans the owner out to.

    A claim reaches CoreSwift (`coreswift::push_claimed_business` + tag_sync) as a contact on
    the directory's tenant — the multi_directory deletes in sweep() never touch it. The push is
    fire-and-forget server-side (per-call timeouts up to 10s), so a single delete can run before
    the contact lands: pass a `window` and poll-delete across it. tag_assignments is a
    polymorphic (entity_type, entity_id) join with NO FK cascade, so it is removed first; the
    contact delete then cascades list_members and the rest.
    """
    deadline = time.time() + window
    while True:
        cssql("DELETE FROM tag_assignments WHERE entity_type='contact' AND entity_id IN "
              "(SELECT id FROM contacts WHERE email LIKE 'zz-guard-%');")
        cssql("DELETE FROM contacts WHERE email LIKE 'zz-guard-%';")
        if time.time() >= deadline:
            return
        time.sleep(2)


def make_listing(directory_id, name, slug):
    return psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, "
        "review_count, probe_harness) "
        f"VALUES ('{directory_id}', '{name}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{MARK}') RETURNING id")


def deals_for(name):
    n = psql("SELECT count(*) FROM crm_deal_records WHERE title LIKE "
             f"'%{name}%' AND title LIKE '% - Claimed (%'")
    return int(n or 0)


def main():
    ts = int(time.time())
    # a previous run that died before teardown must not leave junk behind (multi_directory rows
    # AND the CoreSwift contact its claim fanned out to — one pass is enough for a landed mirror)
    sweep()
    sweep_coreswift()

    directory_id = psql("SELECT id FROM directories WHERE slug='palm-bay'")
    check("the network directory exists", bool(directory_id), f"dir={str(directory_id)[:8]}")

    # ---- arm A: reserved fixture address -> no sales lead -------------------------------
    name_a = f"ZZ Claim Lead Guard A {ts}"
    bid_a = make_listing(directory_id, name_a, f"zz-guard-a-{ts}")
    check("disposable listing A created", bool(bid_a), f"biz={str(bid_a)[:8]}")
    email_a = f"zz-guard-a-{ts}@swiftsoftware.dev"
    st, body = http("POST", f"/api/v1/businesses/{bid_a}/claim",
                    {"owner_email": email_a, "owner_name": "ZZ Guard Owner A",
                     "owner_phone": "4075550181"})
    check("arm A: the public claim route accepts a fixture claim", st in (200, 201), f"status={st}")
    check("arm A: the claim is stored", int(psql("SELECT count(*) FROM claimed_businesses "
                                                 f"WHERE business_id='{bid_a}'") or 0) == 1)
    check("arm A: NO sales lead was created for the fixture claim",
          deals_for(name_a) == 0, f"deals={deals_for(name_a)}")

    # ---- arm B: ordinary address -> the lead still appears (nothing was switched off) ---
    name_b = f"ZZ Claim Lead Guard B {ts}"
    bid_b = make_listing(directory_id, name_b, f"zz-guard-b-{ts}")
    check("disposable listing B created", bool(bid_b), f"biz={str(bid_b)[:8]}")
    email_b = f"zz-guard-b-{ts}@swiftsoftware.net"
    st, body = http("POST", f"/api/v1/businesses/{bid_b}/claim",
                    {"owner_email": email_b, "owner_name": "ZZ Guard Owner B",
                     "owner_phone": "4075550182"})
    check("arm B: the public claim route accepts an ordinary claim", st in (200, 201), f"status={st}")
    check("arm B: exactly ONE sales lead is created for a real-looking claim",
          deals_for(name_b) == 1, f"deals={deals_for(name_b)}")

    # ---- teardown ----------------------------------------------------------------------
    sweep()
    left = psql("SELECT (SELECT count(*) FROM businesses WHERE probe_harness='" + MARK + "')"
                "||'|'||(SELECT count(*) FROM claimed_businesses WHERE business_id IN "
                "(SELECT id FROM businesses WHERE name LIKE 'ZZ Claim Lead Guard%'))"
                "||'|'||(SELECT count(*) FROM crm_deal_records "
                "WHERE title LIKE 'ZZ Claim Lead Guard%')"
                "||'|'||(SELECT count(*) FROM visitor_accounts WHERE email LIKE 'zz-guard-%')")
    check("teardown left no residue", left == "0|0|0|0", f"businesses|claims|deals|accounts={left}")

    st, health = http("GET", "/api/v1/health")
    check("the app is still healthy", st == 200 and isinstance(health, dict)
          and health.get("status") == "ok", f"status={st}")

    print("\nRESULT: " + ("PASS" if not FAILS else f"FAIL ({len(FAILS)})"))
    return 0 if not FAILS else 1


if __name__ == "__main__":
    sys.exit(main())
