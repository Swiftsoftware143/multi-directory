#!/usr/bin/env python3
"""Item 4 — prove a COMPLETE record downloads (and matches the database).

Calls the exact endpoint the admin "This record — JSON/CSV" buttons call, then compares
every related section length with a direct count in Postgres using the same table/FK pairs
the handler declares in src/handlers/entity_export.rs.
"""
import json, re, subprocess, sys, urllib.request

BASE = "http://127.0.0.1:8089"
import os
TOK = os.environ.get("MD_TOKEN") or open("/tmp/mdtok").read().strip()
BIZ = "bb5f6bff-0db4-4d91-a13f-d19dfe64fb37"   # Face It Aesthetics — 2 reviews + listing + meta
SRC = "/opt/swift/apps/multi-directory/src/handlers/entity_export.rs"
fails = []

def chk(ok, label, extra=""):
    print(("  PASS " if ok else "  FAIL ") + label + ((" -> " + extra) if extra else ""))
    if not ok:
        fails.append(label)

def dburl():
    return subprocess.run(["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
                          capture_output=True, text=True).stdout.strip()

def sql(q):
    r = subprocess.run(["docker", "run", "--rm", "--network", "host", "postgres:16",
                        "psql", "-q", "-A", "-t", dburl(), "-c", q], capture_output=True, text=True)
    if r.returncode != 0:
        return None
    return r.stdout.strip()

def fetch(path):
    req = urllib.request.Request(BASE + path)
    req.add_header("Authorization", "Bearer " + TOK)
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.status, dict(r.headers), r.read()

s = open(SRC).read()
block = s.split("const BUSINESS_CHILDREN")[1].split("\n];")[0]
pairs = re.findall(r'\(\s*"([^"]+)",\s*(?:child_q!\(\s*)?"([a-z0-9_]+)"\s*,\s*"([a-z0-9_]+)"', block)
concat_labels = re.findall(r'"([a-z0-9_.]+)"\s*,\s*concat!\(', block)
print(f"declared related sections: {len(pairs)} (+{len(concat_labels)} concat)")

st, hdr, body = fetch(f"/api/v1/export/entity/business/{BIZ}?format=json")
chk(st == 200, "JSON download answers 200", str(st))
chk("attachment" in hdr.get("content-disposition", ""), "JSON is sent as a download",
    hdr.get("content-disposition", ""))
d = json.loads(body)
chk(d.get("entity", {}).get("id") == BIZ, "the record itself is in the download",
    str(d.get("entity", {}).get("name")))
chk(len(d.get("sections", [])) == len(pairs) + len(concat_labels) + 1, "every declared section is present",
    f"{len(d.get('sections', []))} of {len(pairs) + len(concat_labels) + 1}")

mismatch = missing = 0
for label, table, fk in pairs:
    expect = sql(f"SELECT count(*) FROM {table} WHERE {fk} = '{BIZ}';")
    if expect is None:
        missing += 1
        print(f"  SKIP {label}: no such table/column in the live DB")
        continue
    got = len(d.get("related", {}).get(label, []))
    if int(expect) != got:
        mismatch += 1
        print(f"  FAIL {label}: download has {got}, DB has {expect}")
chk(mismatch == 0, "every section count matches the database", f"{mismatch} mismatches")
chk(missing == 0, "no declared section points at a missing table/column", f"{missing} missing")

for label, n in (("reviews", 2), ("business_listings", 1), ("business_meta", 1)):
    chk(len(d["related"].get(label, [])) == n, f"known data present: {label}", str(len(d["related"].get(label, []))))

st2, hdr2, body2 = fetch(f"/api/v1/export/entity/business/{BIZ}?format=csv")
chk(st2 == 200, "CSV download answers 200", str(st2))
chk("attachment" in hdr2.get("content-disposition", "") and ".csv" in hdr2.get("content-disposition", ""),
    "CSV is sent as a .csv download", hdr2.get("content-disposition", ""))
text = body2.decode("utf-8", "replace")
section_lines = [l for l in text.splitlines() if l.startswith("# ")]
chk(len(section_lines) >= len(pairs), "CSV carries a section per related table",
    f"{len(section_lines)} sections")
chk("Face It Aesthetics" in text, "CSV contains the record's data")
chk(len(body2) > 1000, "CSV is a real file, not an empty stub", f"{len(body2)} bytes")

print("RESULT: " + ("PASS" if not fails else f"FAIL ({len(fails)})"))
sys.exit(0 if not fails else 1)
