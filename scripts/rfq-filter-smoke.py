#!/usr/bin/env python3
"""B2B RFQ list filters — live end-to-end proof that `q`, `category` and `urgency` actually filter.

The supplier page `frontend/rfq.html` renders a Search / Category / Urgency filter bar and sends
`?q=…&category=…&urgency=…` to `GET /api/v1/b2b/rfqs`. The handler ignored all three (they were
flagged `never read` by the dead-code sweep), so every filter silently returned the whole list
while the page still printed "No open RFQs match these filters right now."

This probe inserts three disposable, clearly-marked RFQ rows through SQL, drives the real HTTP
endpoint, and asserts:
  * no filter      -> all three probe rows come back;
  * ?q=            -> title/description search is honoured, non-matching probes are excluded;
  * ?category=     -> case-insensitive exact category match;
  * ?urgency=      -> exact urgency match;
  * no-match query -> empty body AND total == 0.
Every returned row is also checked against the predicate it was asked for (no false positives),
which is what fails on an unfiltered server.

Run:  python3 scripts/rfq-filter-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Probe rows are removed on every path.
"""
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
import uuid

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
MARK = "ZZRFQFILTER"
FAILS = []
CREATED = []


def sh(*cmd, stdin=None):
    return subprocess.run(cmd, capture_output=True, text=True, input=stdin)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


PW = db_password()


def psql(sql, tuples=True):
    cmd = ["docker", "exec", "-i", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-v", "ON_ERROR_STOP=1", "-tA"]
    if tuples:
        cmd.append("-tA")
    r = sh(*cmd, stdin=sql)
    if r.returncode != 0:
        raise SystemExit(f"psql failed: {r.stderr.strip()}")
    return r.stdout.strip()


def get(path):
    req = urllib.request.Request(BASE + path, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.loads(resp.read().decode())


def check(name, ok, detail):
    tag = "PASS" if ok else "FAIL"
    print(f"{tag}  {name} — {detail}")
    if not ok:
        FAILS.append(name)


def ids(payload):
    return {r["id"] for r in (payload.get("rfqs") or [])}


def install_probe_rows():
    biz = psql("SELECT id FROM businesses ORDER BY created_at LIMIT 1;")
    if not biz:
        sys.exit("no business row available to own the probe RFQs")
    rows = {
        "A": (str(uuid.uuid4()), f"{MARK} Alpha Widget", "htmlhint-clean gadget", "zzprobe-cat-a", "high"),
        "B": (str(uuid.uuid4()), f"{MARK} Beta Gadget", "a plain standard request", "zzprobe-cat-b", "standard"),
        "C": (str(uuid.uuid4()), f"{MARK} Gamma Doohickey", "urgent restock needed", "zzprobe-cat-a", "urgent"),
    }
    values = ",\n".join(
        f"('{rid}','{title}','{desc}','{cat}','{urg}','{biz}',true,'open')"
        for (rid, title, desc, cat, urg) in rows.values()
    )
    psql(
        "INSERT INTO rfqs (id,title,description,category,urgency,poster_business_id,is_public,status) VALUES\n"
        + values
        + ";"
    )
    for rid, *_ in rows.values():
        CREATED.append(rid)
    return rows


def cleanup():
    if not CREATED:
        return
    ids_sql = ",".join(f"'{i}'" for i in CREATED)
    psql(f"DELETE FROM rfqs WHERE id IN ({ids_sql});")
    left = psql(f"SELECT COUNT(*) FROM rfqs WHERE id IN ({ids_sql});")
    print(f"cleanup: probe rows remaining = {left}")


def main():
    rows = install_probe_rows()
    a, b, c = (rows[k][0] for k in ("A", "B", "C"))

    # 1. no filter — the probe rows must be visible at all
    p = get("/api/v1/b2b/rfqs?per_page=100")
    got = ids(p)
    check("unfiltered list contains the probe rows",
          {a, b, c} <= got,
          f"returned {len(got)} row(s), probe rows present = {sorted({a, b, c} & got)}")

    # 2. q — title/description search
    p = get(f"/api/v1/b2b/rfqs?per_page=100&q={MARK}")
    got = ids(p)
    check("q=<marker> returns exactly the matching rows",
          {a, b, c} <= got and all(MARK.lower() in (r["title"] + " " + (r["description"] or "")).lower()
                                   for r in p["rfqs"]),
          f"total={p.get('total')}, ids={len(got)}")

    p = get("/api/v1/b2b/rfqs?per_page=100&q=Alpha")
    got = ids(p)
    check("q=Alpha excludes the non-matching probe rows",
          a in got and b not in got and c not in got,
          f"Alpha present={a in got}, Beta present={b in got}, Gamma present={c in got}")
    check("q=Alpha every returned row matches 'alpha'",
          all("alpha" in (r["title"] + " " + (r["description"] or "")).lower() for r in p["rfqs"]),
          f"rows checked={len(p['rfqs'])}")

    # 3. category — case-insensitive exact match
    p = get("/api/v1/b2b/rfqs?per_page=100&category=ZZPROBE-CAT-A")
    got = ids(p)
    check("category= (upper-cased) honours case-insensitivity and excludes other categories",
          a in got and c in got and b not in got,
          f"A={a in got} C={c in got} B={b in got}")
    check("category= every returned row has that category",
          all((r.get("category") or "").lower() == "zzprobe-cat-a" for r in p["rfqs"]),
          f"rows checked={len(p['rfqs'])}")

    # 4. urgency
    p = get("/api/v1/b2b/rfqs?per_page=100&urgency=urgent")
    got = ids(p)
    check("urgency=urgent excludes standard/high rows",
          c in got and a not in got and b not in got,
          f"C={c in got} A={a in got} B={b in got}")
    check("urgency= every returned row is urgent",
          all(r.get("urgency") == "urgent" for r in p["rfqs"]),
          f"rows checked={len(p['rfqs'])}")

    # 5. combined filters
    p = get(f"/api/v1/b2b/rfqs?per_page=100&q={MARK}&category=zzprobe-cat-a&urgency=urgent")
    got = ids(p)
    check("combined q+category+urgency narrows to the single matching row",
          got == {c},
          f"ids={sorted(got)}")

    # 6. no-match query — empty body AND honest total
    p = get("/api/v1/b2b/rfqs?per_page=100&q=zz-no-such-rfq-6f2a1c")
    check("no-match query returns an empty page and total 0",
          (p.get("rfqs") or []) == [] and p.get("total") == 0,
          f"rows={len(p.get('rfqs') or [])} total={p.get('total')}")

    # 7. total respects the filter
    p = get(f"/api/v1/b2b/rfqs?per_page=100&category=zzprobe-cat-a")
    check("total is computed under the active filter",
          p.get("total") == len(p.get("rfqs") or []),
          f"total={p.get('total')} rows={len(p.get('rfqs') or [])}")

    print(f"\nRFQ FILTER RESULT: {'all passed' if not FAILS else str(len(FAILS)) + ' failure(s): ' + ', '.join(FAILS)}")
    return 1 if FAILS else 0


if __name__ == "__main__":
    try:
        rc = main()
    finally:
        cleanup()
    sys.exit(rc)
