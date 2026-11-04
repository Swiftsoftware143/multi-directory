#!/usr/bin/env python3
"""Remove OUR OWN probe traffic from the visitor analytics tables.

Every browser proof run loads real pages from http://127.0.0.1:8089, and the page's
own tracker faithfully records those loads as visitor page views / clicks.  A live
site can never be visited over 127.0.0.1, so any visitor_events row whose page_url
host is localhost is by definition test traffic — it inflates the operator's
dashboard stats (queue item 9) and is swept by queue item 18.

Only visitor_events rows are touched, and only those whose page_url starts with
http://127.0.0.1: or http://localhost:.  Real traffic (zaarhub.com, directory.
swiftsoftware.net, any customer domain) is never matched.

Run:
    python3 scripts/clean-probe-analytics.py --dry-run
    python3 scripts/clean-probe-analytics.py          # delete, prints what it did

Exit: 0 = clean (or nothing to clean), 1 = failure.
"""
import os
import re
import subprocess
import sys

PROBE = ("page_url LIKE 'http://127.0.0.1:%' "
         "OR page_url LIKE 'http://localhost:%'")


def sh(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


def psql(pw, sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={pw}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        sys.exit(f"psql failed: {r.stderr.strip()}")
    return r.stdout.strip()


def main():
    dry = "--dry-run" in sys.argv
    pw = db_password()
    probe = psql(pw, f"SELECT count(*) FROM visitor_events WHERE {PROBE};")
    real = psql(pw, f"SELECT count(*) FROM visitor_events WHERE NOT ({PROBE});")
    print(f"before: probe={probe} real={real}")
    if probe == "0":
        print("nothing to clean")
        return 0
    if dry:
        print("dry run — nothing deleted")
        return 0
    psql(pw, f"DELETE FROM visitor_events WHERE {PROBE};")
    probe_after = psql(pw, f"SELECT count(*) FROM visitor_events WHERE {PROBE};")
    real_after = psql(pw, f"SELECT count(*) FROM visitor_events WHERE NOT ({PROBE});")
    print(f"after:  probe={probe_after} real={real_after}")
    if probe_after != "0" or real_after != real:
        print("RESULT: FAIL — real rows changed or probe rows remain")
        return 1
    print("RESULT: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
