#!/usr/bin/env python3
"""Item 3 — one network email setup serves every city.

Read-only. For every directory in the network, GET /api/v1/directories/<slug>/email-settings
and assert: configured, same from_email as the network setup, and either own=true (the one
row) or inherited_from=<network primary>. Also asserts exactly one DB row.
"""
import json, subprocess, sys, urllib.request

BASE = "http://127.0.0.1:8089"
import os
TOK = os.environ.get("MD_TOKEN") or open("/tmp/mdtok").read().strip()
fails = []

def get(path):
    req = urllib.request.Request(BASE + path)
    req.add_header("Authorization", "Bearer " + TOK)
    with urllib.request.urlopen(req, timeout=20) as r:
        return r.status, json.loads(r.read().decode())

def sql(q):
    url = subprocess.run(["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
                         capture_output=True, text=True).stdout.strip()
    r = subprocess.run(["docker", "run", "--rm", "--network", "host", "postgres:16",
                        "psql", "-q", "-A", "-t", url, "-c", q], capture_output=True, text=True)
    return [l for l in (r.stdout or "").strip().splitlines() if l]

def chk(ok, label, extra=""):
    print(("  PASS " if ok else "  FAIL ") + label + ((" -> " + extra) if extra else ""))
    if not ok:
        fails.append(label)

cities = sql("SELECT slug FROM directories WHERE network_id = "
             "'aaaaaaaa-0000-0000-0000-000000000001' ORDER BY is_primary DESC, slug;")
rows = sql("SELECT count(*) FROM directory_email_settings;")
print(f"cities={len(cities)}  email_rows={rows}")

chk(len(cities) == 10, "all ten cities belong to the network", str(len(cities)))
chk(rows == ["1"], "exactly ONE email setup row exists", str(rows))

base_from = None
for slug in cities:
    st, d = get(f"/api/v1/directories/{slug}/email-settings")
    ok = st == 200 and d.get("configured") is True and bool(d.get("from_email")) and d.get("mail_ready") is True
    chk(ok, f"{slug}: email configured and mail-ready",
        f"from={d.get('from_email')} own={d.get('own')} inherited={d.get('inherited_from')} ready={d.get('mail_ready')}")
    if base_from is None:
        base_from = d.get("from_email")
    chk(d.get("from_email") == base_from, f"{slug}: same sender as the network setup", str(d.get("from_email")))
    if slug == cities[0]:
        chk(not d.get("inherited_from"), "primary is the source of the single setup",
            f"inherited_from={d.get('inherited_from')} id={d.get('id')}")
    else:
        chk(d.get("own") is False and d.get("inherited_from") == cities[0],
            f"{slug}: inherits the network setup (no own row)",
            f"inherited_from={d.get('inherited_from')}")

print("RESULT: " + ("PASS" if not fails else f"FAIL ({len(fails)})"))
sys.exit(0 if not fails else 1)
