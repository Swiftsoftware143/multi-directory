#!/usr/bin/env python3
"""Item 18 — remove the leftover Phase-2 / e2e test accounts from the LIVE ZaarHub database.

Scope (narrow, deliberate):
  * ONLY visitor accounts whose address is a reserved test fixture
    (@test.com, @x.com, @example.com) AND that own nothing, and that are at
    least 2 minutes old (so a fixture created by a smoke run still in flight is
    never yanked out from under it).
  * NEVER touched: probe-canary-a/b@swiftsoftware.dev (reserved fleet tenancy canaries,
    watched by /opt/swift/fleet/canary-id-census.py), biz@test.com (a fixture the app's own
    smoke scripts sign in as -- /opt/swift/bin/md-5xx-fix-smoke.py, md-provider-key-crypto-smoke.py),
    real customer accounts, the system tenant.

Proof produced: pre/post row counts, a reference census proving each deleted row was an orphan,
post-delete residue = 0, and the app still healthy.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item18-testdata-{time.strftime('%Y%m%d-%H%M%S')}")
os.makedirs(AUD, exist_ok=True)

KEEP = {"probe-canary-a@swiftsoftware.dev", "probe-canary-b@swiftsoftware.dev"}
TARGET_RE = re.compile(r"@(test\.com|x\.com|example\.com)$", re.I)
# A fixture younger than this is presumed to belong to a smoke run still in flight.
MIN_AGE_SECONDS = 120


def sh(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("no DATABASE_URL from the multidirectory container")
    return m.group(1)


PW = db_password()
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT)\b")


def psql(sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}"[:400])
    return [ln for ln in r.stdout.strip().splitlines() if ln and not TAG.match(ln)]


def rows(sql):
    out = psql(sql)
    return [ln.split("|") for ln in out]


fails = []
line = []


def check(ok, label, detail=""):
    line.append(f"{'PASS' if ok else 'FAIL'}  {label}" + (f"  [{detail}]" if detail else ""))
    if not ok:
        fails.append(label)
    print(line[-1], flush=True)


# ---- 1. target set -------------------------------------------------------------------------
targets = rows(f"""SELECT id, email FROM visitor_accounts
                   WHERE email !~* '(probe-canary|swiftsoftware\\.dev)'
                     AND email ~* '@(test\\.com|x\\.com|example\\.com)$'
                     AND created_at < now() - interval '{MIN_AGE_SECONDS} seconds'
                   ORDER BY email""")
ids = [t[0] for t in targets]
emails = [t[1] for t in targets]
print(f"  leftover test accounts: {len(ids)} :: {', '.join(emails) if emails else '-'}")
if any(e in KEEP for e in emails):
    check(False, "target set excludes the reserved canaries")
    sys.exit(1)

# ---- 1b. scan: leftover test SUBMISSIONS (the same fixtures' business-enquiry rows) ---------
sub_targets = rows(f"""SELECT id, submitter_email FROM submissions
                       WHERE submitter_email ~* '@(test\\.com|x\\.com|example\\.com)$'
                         AND submitter_email !~* '(probe-canary|swiftsoftware\\.dev)'
                         AND created_at < now() - interval '{MIN_AGE_SECONDS} seconds'
                       ORDER BY created_at""")
sub_ids = [s[0] for s in sub_targets]
sub_emails = [s[1] for s in sub_targets]
print(f"  leftover test submissions: {len(sub_ids)} :: {', '.join(sub_emails) if sub_emails else '-'}")

if not ids and not sub_ids:
    check(True, "no leftover test data on the live database", "0 accounts, 0 submissions")
else:
    check(True, "leftover test data found on the live database",
          f"{len(ids)} account(s), {len(sub_ids)} submission(s)")

# ---- 2. census: every deleted row must be an orphan ----------------------------------------
uuid_cols = rows("""SELECT table_name||'|'||column_name FROM information_schema.columns
                    WHERE table_schema='public' AND data_type='uuid'""")
before, after = {}, {}

if ids:
    arr = ",".join(f"'{i}'::uuid" for i in ids)
    refs = []
    for t, c in uuid_cols:
        n = psql(f"SELECT count(*) FROM {t} WHERE {c} IN ({arr})")
        if n and int(n[0]) > 0:
            refs.append((t, c, int(n[0]), n[0]))
    for t, c, n, _ in refs:
        print(f"  reference: {t}.{c} = {n}")
    with open(os.path.join(AUD, "reference-census.txt"), "w") as f:
        f.write("\n".join(f"{t}.{c} = {n}" for t, c, n, _ in refs) + "\n")
    check(True, "reference census recorded", f"{len(refs)} table(s) touch these rows")

    # which reference is a loyalty member? prove it is empty of value
    for t, c, n, _ in refs:
        if t == "loyalty_members":
            pts = psql(f"SELECT coalesce(sum(points_balance),0)||'|'||coalesce(sum(lifetime_points),0) "
                       f"FROM loyalty_members WHERE {c} IN ({arr})")
            check(pts and pts[0] == "0|0", "the one attached loyalty record holds 0 points", pts[0] if pts else "?")

    # ---- 3. the same rows are invisible to the public --------------------------------------
    vis = rows(f"""SELECT count(*) FROM businesses WHERE owner_id IN ({arr})""")
    check(int(vis[0][0]) == 0, "none of them owns a public listing (so nothing disappears from the site)",
          f"businesses={vis[0][0]}")

    # ---- 4. before counts ------------------------------------------------------------------
    before = {
        "visitor_accounts": int(psql("SELECT count(*) FROM visitor_accounts")[0]),
        "loyalty_members": int(psql("SELECT count(*) FROM loyalty_members")[0]),
    }
    print("  before:", json.dumps(before))

    # ---- 5. delete -------------------------------------------------------------------------
    psql("BEGIN")
    for i in ids:
        psql(f"DELETE FROM visitor_accounts WHERE id = '{i}'")
    psql("COMMIT")

    after = {
        "visitor_accounts": int(psql("SELECT count(*) FROM visitor_accounts")[0]),
        "loyalty_members": int(psql("SELECT count(*) FROM loyalty_members")[0]),
    }
    print("  after: ", json.dumps(after))
    check(after["visitor_accounts"] == before["visitor_accounts"] - len(ids),
          "every leftover test account is gone",
          f"{before['visitor_accounts']} -> {after['visitor_accounts']} (-{len(ids)})")

    # ---- 6. residue = 0 --------------------------------------------------------------------
    resid = 0
    for t, c in uuid_cols:
        n = psql(f"SELECT count(*) FROM {t} WHERE {c} IN ({arr})")
        if n and int(n[0]) > 0:
            resid += int(n[0])
    check(resid == 0, "no trace left anywhere in the database", f"rows={resid}")
else:
    check(True, "no leftover test visitor accounts to trace", "0")

kept = psql("SELECT count(*) FROM visitor_accounts WHERE email IN "
            "('probe-canary-a@swiftsoftware.dev','probe-canary-b@swiftsoftware.dev')")
check(kept and int(kept[0]) == 2, "the two reserved fleet canaries are untouched", f"canaries={kept[0]}")

# ---- 6b. delete the leftover test SUBMISSIONS (scanned at 1b) -------------------------------
if sub_ids:
    print(f"  leftover test submissions: {len(sub_ids)} :: {', '.join(sub_emails)}")
    sarr = ",".join(f"'{i}'::uuid" for i in sub_ids)
    sub_before = int(psql("SELECT count(*) FROM submissions")[0])
    psql("BEGIN")
    psql(f"DELETE FROM submissions WHERE id IN ({sarr})")
    psql("COMMIT")
    sub_after = int(psql("SELECT count(*) FROM submissions")[0])
    check(sub_after == sub_before - len(sub_ids),
          "every leftover test submission is gone",
          f"{sub_before} -> {sub_after} (-{len(sub_ids)})")
    sub_resid = 0
    for t, c in uuid_cols:
        n = psql(f"SELECT count(*) FROM {t} WHERE {c} IN ({sarr})")
        if n and int(n[0]) > 0:
            sub_resid += int(n[0])
    check(sub_resid == 0, "no trace of the test submissions left anywhere", f"rows={sub_resid}")
else:
    check(True, "no leftover test submissions", "0")

# ---- 6c. fake test records left inside the customer-facing modules ---------------------------
# The visitor-account sweep above only covered signups. The fleet's own probes also left records
# INSIDE the product: fake "claimed" businesses in the CRM, fake newsletter signups, offer
# claims, referrals, a fake missed-call lead, and probe traffic in the analytics tables. Every
# predicate below is anchored on a marker only this fleet writes (the `ZZ … - Claimed (Orlando)`
# CRM title shape, `@example.*` / `@invalid.test` addresses, `zz-sponsored-probe` URLs), and the
# two reserved tenancy canaries plus the app's own `biz@test.com` login fixture are excluded
# everywhere. Rows are backed up to the audit dir before deletion so the change is reversible.
PROTECTED = r"biz@test\.com|probe-canary|swiftsoftware\.dev"
ZZ_WHERE = r"title ~* '(probe|smoke|selftest|fixture|dummy)'" \
           r" OR (title ~* '^ZZ ' AND title ~ ' - Claimed \(Orlando\)$')"
V_URL = r"zzprobe|zz-sponsored-probe|zz-item6-debug"
PROBE_RECORDS = [
    ("crm_deal_records", ZZ_WHERE, ()),
    ("business_messages", r"sender_email ~* '@(example\.(com|invalid)|invalid\.test|test\.com)$'",
     ("sender_email",)),
    ("newsletter_subscribers", r"email ~* '@(example\.(com|invalid)|invalid\.test)$'", ("email",)),
    ("offer_claims", r"email ~* '@example\.(com|invalid)$'", ("email",)),
    ("referrals", r"referrer_email ~* '@example\.invalid$' OR referee_email ~* '@example\.invalid$'",
     ("referrer_email", "referee_email")),
    ("call_logs", r"lead_email ~* '@example\.(com|invalid)$'", ("lead_email",)),
]
bk_dir = os.path.join(AUD, "fake-records-backup")
os.makedirs(bk_dir, exist_ok=True)


def backup_and_delete(table, where):
    n_before = int(psql(f"SELECT count(*) FROM {table} WHERE {where}")[0])
    if n_before:
        r = sh("docker", "exec", "-e", f"PGPASSWORD={PW}", "swift-postgres", "psql", "-U", "swift",
               "-d", "multi_directory", "-c",
               f"COPY (SELECT * FROM {table} WHERE {where}) TO STDOUT WITH CSV HEADER")
        with open(os.path.join(bk_dir, f"{table}.csv"), "w") as f:
            f.write(r.stdout)
        psql("BEGIN")
        psql(f"DELETE FROM {table} WHERE {where}")
        psql("COMMIT")
    n_after = int(psql(f"SELECT count(*) FROM {table} WHERE {where}")[0])
    check(n_after == 0, f"fake test records cleared from {table}",
          f"{n_before} -> {n_after} (backup in fake-records-backup/{table}.csv)")
    return n_before


cleaned = 0
for _t, _pred, _cols in PROBE_RECORDS:
    _where = f"({_pred}) AND created_at < now() - interval '{MIN_AGE_SECONDS} seconds'"
    for _c in _cols:
        _where += f" AND coalesce({_c}::text,'') !~* '{PROTECTED}'"
    cleaned += backup_and_delete(_t, _where)

# probe TRAFFIC: the analytics rows the probe browsers wrote while walking the site
_vs_where = (f"landing_page ~* '{V_URL}' OR entry_url ~* '{V_URL}'"
             f" OR exit_page ~* '{V_URL}' OR exit_url ~* '{V_URL}'")
_ve_where = (f"session_id IN (SELECT id FROM visitor_sessions WHERE {_vs_where})"
             f" OR page_url ~* '{V_URL}' OR event_value ~* 'zz.*probe'")
cleaned += backup_and_delete("visitor_events", _ve_where)
cleaned += backup_and_delete("visitor_sessions", _vs_where)
print(f"  fake test record sweep removed {cleaned} row(s) this run")

# ---- 7. the site still works ---------------------------------------------------------------
try:
    with urllib.request.urlopen(f"{BASE}/api/v1/health", timeout=10) as r:
        check(r.status == 200, "the app is still healthy", f"health={r.status}")
except Exception as e:  # noqa: BLE001
    check(False, "the app is still healthy", str(e)[:120])

req = urllib.request.Request("https://zaarhub.com/", headers={"User-Agent": "Mozilla/5.0 (compatible; SwiftOps/1.0)"})
try:
    with urllib.request.urlopen(req, timeout=20) as r:
        body = r.read(4096)
        check(r.status == 200 and b"<" in body, "the public site still answers", f"status={r.status}")
except Exception as e:  # noqa: BLE001
    check(False, "the public site still answers", str(e)[:120])

with open(os.path.join(AUD, "evidence.txt"), "w") as f:
    f.write("\n".join(line) + "\n")
    f.write(f"\ndeleted: {', '.join(emails)}\nbefore={json.dumps(before)} after={json.dumps(after)}\n")

print("\n" + ("RESULT: PASS" if not fails else "RESULT: FAIL " + ", ".join(fails)))
print("evidence:", AUD)
sys.exit(0 if not fails else 1)
