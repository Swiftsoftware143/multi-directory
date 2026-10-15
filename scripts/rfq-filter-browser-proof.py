#!/usr/bin/env python3
"""Orchestrator for the supplier RFQ filter browser arm.

Seeds three disposable, clearly-marked RFQ rows, runs `scripts/rfq-filter-browser.js` against the
real served page, then removes the rows on every path.  The browser script itself is the judge.

Usage:  python3 scripts/rfq-filter-browser-proof.py [AUD_DIR]
Exit:   0 = PASS, 1 = FAIL, 2 = could not run.
"""
import os
import re
import subprocess
import sys
import uuid

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(REPO, "scripts"))

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
MARK = "ZZRFQFILTER"
NODE_MODULES = "/opt/swift/browser-run/node_modules"

AUD = sys.argv[1] if len(sys.argv) > 1 else "/opt/swift/audits/zz-item18-rfq-filter-20261015"
os.makedirs(AUD, exist_ok=True)


def sh(*cmd, stdin=None, env=None):
    return subprocess.run(cmd, capture_output=True, text=True, input=stdin, env=env)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit(2)
    return m.group(1)


PW = db_password()
CREATED = []


def psql(sql):
    r = sh("docker", "exec", "-i", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-v", "ON_ERROR_STOP=1", "-tA", "-q",
           stdin=sql)
    if r.returncode != 0:
        raise SystemExit("psql failed: " + r.stderr.strip())
    return r.stdout.strip()


def main():
    biz = psql("SELECT id FROM businesses ORDER BY created_at LIMIT 1;")
    if not biz:
        sys.exit(2)
    a, b, c = str(uuid.uuid4()), str(uuid.uuid4()), str(uuid.uuid4())
    rows = [
        (a, f"{MARK} Alpha Widget", "htmlhint-clean gadget", "zzprobe-cat-a", "high"),
        (b, f"{MARK} Beta Gadget", "a plain standard request", "zzprobe-cat-b", "standard"),
        (c, f"{MARK} Gamma Doohickey", "urgent restock needed", "zzprobe-cat-a", "urgent"),
    ]
    CREATED.extend([a, b, c])
    values = ",\n".join(
        f"('{rid}','{t}','{d}','{cat}','{urg}','{biz}',true,'open')" for (rid, t, d, cat, urg) in rows
    )
    psql("INSERT INTO rfqs (id,title,description,category,urgency,poster_business_id,is_public,status) "
         "VALUES\n" + values + ";")

    env = dict(os.environ)
    env.update({
        "MD_BASE": BASE,
        "MARK": MARK,
        "MARK_IDS": ",".join([a, b, c]),
        "CAT": "zzprobe-cat-a",
        "CAT_IDS": ",".join([a, c]),
        "URGENT_ID": c,
        "AUD": AUD,
        "NODE_PATH": NODE_MODULES,
        "CHROME_PATH": os.environ.get("CHROME_PATH", "/usr/bin/chromium-browser"),
    })
    r = sh("node", os.path.join(REPO, "scripts", "rfq-filter-browser.js"), env=env)
    sys.stdout.write(r.stdout)
    if r.returncode != 0:
        sys.stderr.write(r.stderr)
    return r.returncode


if __name__ == "__main__":
    try:
        rc = main()
    finally:
        if CREATED:
            ids_sql = ",".join(f"'{i}'" for i in CREATED)
            psql(f"DELETE FROM rfqs WHERE id IN ({ids_sql});")
            print("cleanup: probe rows remaining = " +
                  psql(f"SELECT COUNT(*) FROM rfqs WHERE id IN ({ids_sql});"))
    sys.exit(rc)
