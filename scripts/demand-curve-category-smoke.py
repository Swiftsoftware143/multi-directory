#!/usr/bin/env python3
"""demand-curve category filter — proves the advertised ?category= filter is honoured.

Queue item 18 (dead-code / accepted-but-ignored inputs). GET /api/v1/analytics/demand-curve
documents "optional filtering by directory_id, city, category" — before this fix `category` was
deserialised and silently dropped, so a caller filtering by category got the *unfiltered* curve
(measured: bogus category and a real category both returned the full 6 rows).

This smoke cross-checks the API against a direct SQL aggregation and asserts:
  1. unfiltered returns at least one row
  2. a category that exists in no row returns ZERO rows      <- the fix
  3. a category that does have rows returns only rows of that category_id
  4. filtering by the same category by UUID gives the same count
  5. the filtered count never exceeds the unfiltered count

Exit 0 = PASS, 1 = FAIL. Read-only: no fixtures, no writes.
"""
import json
import os
import subprocess
import sys
import urllib.parse
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
TOKEN = os.environ.get("MD_ADMIN_TOKEN") or open("/tmp/.mdtok").read().strip()
BOGUS = "zzz-nope-9999"
fails = []


def db_url():
    return subprocess.run(
        ["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
        capture_output=True, text=True, timeout=30,
    ).stdout.strip()


def pw():
    u = urllib.parse.urlparse(db_url())
    return urllib.parse.unquote(u.password or "")


def sql(q):
    r = subprocess.run(
        ["docker", "exec", "-i", "-e", "PGPASSWORD=" + pw(), "swift-postgres",
         "psql", "-U", "swift", "-d", "multi_directory", "-tAc", q],
        capture_output=True, text=True, timeout=60,
    )
    return r.stdout.strip()


def api(**qs):
    url = BASE + "/api/v1/analytics/demand-curve?" + urllib.parse.urlencode(qs)
    req = urllib.request.Request(url, headers={"Authorization": "Bearer " + TOKEN})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def check(ok, name, detail=""):
    print(("PASS  " if ok else "FAIL  ") + name + ((" :: " + str(detail)) if detail else ""))
    if not ok:
        fails.append(name)


# Pick the category that actually has demand events in the lookback window.
row = sql(
    "SELECT b.category_id||'|'||dc.name FROM visitor_events ve "
    "JOIN businesses b ON b.id = ve.business_id "
    "JOIN directory_categories dc ON dc.id = b.category_id "
    "WHERE ve.created_at >= NOW() - INTERVAL '365 days' AND b.category_id IS NOT NULL "
    "GROUP BY 1 ORDER BY COUNT(*) DESC LIMIT 1"
)
check(bool(row), "a category with demand rows exists (fixture premise)", row)
if not row:
    print("\nRESULT: FAIL")
    sys.exit(1)
cat_id, cat_name = row.split("|", 1)

unf = api(days=365)
check(unf.get("count", 0) >= 1, "unfiltered curve returns rows", "count=%s" % unf.get("count"))

bogus = api(days=365, category=BOGUS)
check(bogus.get("count", 0) == 0,
      "a non-existent category returns ZERO rows (filter is honoured)",
      "count=%s (before the fix this was the unfiltered count)" % bogus.get("count"))

by_name = api(days=365, category=cat_name)
rows = by_name.get("data", [])
check(by_name.get("count", 0) >= 1, "filtering by a real category name returns rows",
      "category=%r count=%s" % (cat_name, by_name.get("count")))
check(all(str(r.get("category_id")) == cat_id for r in rows),
      "every row belongs to the requested category", "cat_id=%s" % cat_id)

by_id = api(days=365, category=cat_id)
check(by_id.get("count") == by_name.get("count"),
      "filtering by the category UUID gives the same count",
      "id=%s name=%s" % (by_id.get("count"), by_name.get("count")))

check(by_name.get("count", 0) <= unf.get("count", 0),
      "filtered count never exceeds the unfiltered count",
      "%s <= %s" % (by_name.get("count"), unf.get("count")))

print("\nRESULT: " + ("PASS" if not fails else "FAIL (%d)" % len(fails)))
sys.exit(0 if not fails else 1)
