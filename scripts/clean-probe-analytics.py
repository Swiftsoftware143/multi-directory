#!/usr/bin/env python3
"""Remove OUR OWN probe traffic from the visitor analytics tables.

Every browser proof run loads real pages — often over the REAL hostname (Cloudflare
and cookie behaviour only exist there), not just http://127.0.0.1:8089.  The page's
own tracker faithfully records those loads as visitor sessions / events, so both
hosts have to be swept, and sessions have to be swept as well as events.

Test traffic is identified by SHAPE, never by host alone:
  * the page URL host is 127.0.0.1 / localhost  (a live customer can never arrive there), or
  * the FINAL path segment of the URL is a harness slug — it contains "probe" or starts
    with "zz-" (the fleet's fixture naming convention).
Real customer pages (zaarhub.com/palm-bay/businesses/<slug>, any directory domain) never
match: their last segment is a business/article slug, not a probe marker.

Deletion order respects FKs: events (by page_url and by probe event_value) -> sessions
(by any of their four URL columns, and by the events just removed) -> visitors left with
neither a session nor an event AND marked probe_harness.

Real rows are counted before and after and must not change.

Run:
    python3 scripts/clean-probe-analytics.py --dry-run
    python3 scripts/clean-probe-analytics.py          # delete, prints what it did

Exit: 0 = clean (or nothing to clean), 1 = failure.
"""
import os
import re
import subprocess
import sys

# A URL is a probe URL when its host is local, or its last path segment is a harness slug.
PROBE_URL = ("(page_url LIKE 'http://127.0.0.1:%' OR page_url LIKE 'http://localhost:%' "
             "OR page_url ~* '/[^/?]*probe[^/?]*$' OR page_url ~* '/zz-[^/?]*$')")
PROBE_URL_COL = ("({c} LIKE 'http://127.0.0.1:%' OR {c} LIKE 'http://localhost:%' "
                 "OR {c} ~* '/[^/?]*probe[^/?]*$' OR {c} ~* '/zz-[^/?]*$')")
PROBE_VALUE = "(event_value ~* '^ZZ ' OR event_value ~* 'probe')"
SESS_COLS = ("entry_url", "exit_url", "landing_page", "exit_page")


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


def sess_pred():
    return " OR ".join(PROBE_URL_COL.format(c=c) for c in SESS_COLS)


def counts(pw):
    """Probe/real counts. An event is only "real" when it is not a probe event AND does not
    belong to a probe session — an event recorded inside our own localhost session is ours,
    whatever its own page_url says."""
    ev_probe = psql(pw, f"SELECT count(*) FROM visitor_events ve WHERE {PROBE_URL} OR {PROBE_VALUE} "
                        f"OR ve.session_id IN (SELECT id FROM visitor_sessions WHERE {sess_pred()});")
    ev_real = psql(pw, f"SELECT count(*) FROM visitor_events ve WHERE NOT ({PROBE_URL} OR {PROBE_VALUE}) "
                       f"AND (ve.session_id IS NULL OR ve.session_id NOT IN "
                       f"(SELECT id FROM visitor_sessions WHERE {sess_pred()}));")
    ss_probe = psql(pw, f"SELECT count(*) FROM visitor_sessions WHERE {sess_pred()};")
    ss_real = psql(pw, f"SELECT count(*) FROM visitor_sessions WHERE NOT ({sess_pred()});")
    return int(ev_probe), int(ev_real), int(ss_probe), int(ss_real)


def main():
    dry = "--dry-run" in sys.argv
    pw = db_password()
    ev_probe, ev_real, ss_probe, ss_real = counts(pw)
    print(f"before: probe_events={ev_probe} real_events={ev_real} "
          f"probe_sessions={ss_probe} real_sessions={ss_real}")
    if ev_probe == 0 and ss_probe == 0:
        print("nothing to clean")
        return 0
    if dry:
        print("dry run — nothing deleted")
        return 0
    sql = (
        "BEGIN;"
        f"DELETE FROM visitor_events WHERE {PROBE_URL} OR {PROBE_VALUE};"
        f"DELETE FROM visitor_events WHERE session_id IN "
        f"(SELECT id FROM visitor_sessions WHERE {sess_pred()});"
        f"DELETE FROM visitor_sessions WHERE {sess_pred()};"
        "DELETE FROM visitors v WHERE v.probe_harness IS NOT NULL"
        " AND NOT EXISTS (SELECT 1 FROM visitor_sessions s WHERE s.visitor_id = v.id)"
        " AND NOT EXISTS (SELECT 1 FROM visitor_events e WHERE e.visitor_id = v.id);"
        "COMMIT;"
    )
    psql(pw, sql)
    ev_probe2, ev_real2, ss_probe2, ss_real2 = counts(pw)
    print(f"after:  probe_events={ev_probe2} real_events={ev_real2} "
          f"probe_sessions={ss_probe2} real_sessions={ss_real2}")
    if ev_probe2 or ss_probe2:
        print("RESULT: FAIL — probe rows remain")
        return 1
    if ev_real2 != ev_real or ss_real2 != ss_real:
        print("RESULT: FAIL — real rows changed")
        return 1
    print("RESULT: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
