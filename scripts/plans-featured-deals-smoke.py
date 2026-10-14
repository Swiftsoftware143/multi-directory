#!/usr/bin/env python3
"""Plans (tiers) + Featured Deals screens — live end-to-end proof (ZaarHub operator item 10).

The admin console has two screens the operator must be able to see and operate:
  * **Plan Tiers** — the three subscription tiers with their prices, allowances and limits,
    each row editable and savable (`GET/PUT /api/v1/tiers`, table `plan_tiers`);
  * **Featured Deals** — the network's deals list with a count and a per-row "Feature"
    control (`GET /api/v1/deals`, `PUT /api/v1/deals/:id`, table `deals`).

A capability that exists only as an endpoint is a defect by David's rule, so this proves the
SCREENS: the API is read back against the live database AND the served console is driven in a
real browser (`scripts/plans-featured-deals-browser.js`), through the page's own controls.

Proves, against the running app and the live database:
  1. anonymous `GET /api/v1/tiers` is refused (the global auth guard);
  2. the tiers the API returns are exactly the rows the database holds (count + values);
  3. the console renders the three tiers from that live API, each with its own Save control,
     and pressing Save persists the edit (read back) — then the original value is restored;
  4. a disposable deal is created, is visible on the deals screen, and its own Feature control
     toggles the deal both ways with the change persisted (read back);
  5. no console or auth errors during the walk;
  6. cleanup is net-zero: the disposable deal is gone and the tiers are back to their before state.

Run:  python3 scripts/plans-featured-deals-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Writes only into $AUD (default /opt/swift/audits/zz-item10-...).
"""
import json
import os
import re
import subprocess
import sys
import time

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "plans-featured-deals-browser.js")
PANEL = os.path.join(APP, "frontend", "admin-panel.html")
NODE_PATH = "/opt/swift/browser-run/node_modules"
MINT = "/opt/swift/browser-run/md-mint-token.js"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item10-plans-deals-{time.strftime('%Y%m%d-%H%M%S')}")
TITLE_PREFIX = "zz-plans-deals-probe"
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


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
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    return [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]


def one(sql):
    rows = psql(sql)
    return rows[0] if rows else ""


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def call(method, path, token=None, body=None):
    import urllib.error
    import urllib.request
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(BASE + "/api/v1" + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def sweep_stray_deals():
    """Remove any disposable deal a crashed prior run may have left behind."""
    psql("DELETE FROM deals WHERE title LIKE '%s%%'" % TITLE_PREFIX)


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep_stray_deals()

    # ---- live DB truth -------------------------------------------------------
    db_tiers_rows = psql("SELECT name, monthly_point_allowance FROM plan_tiers "
                         "ORDER BY price_monthly ASC")
    db_tiers = [tuple(r.split("|", 1)) for r in db_tiers_rows]
    db_deals = int(one("SELECT count(*) FROM deals"))
    print("DB truth: plan_tiers rows =", len(db_tiers), "| deals =", db_deals)
    check("the database holds subscription tiers", len(db_tiers) >= 1,
          ", ".join("|".join(t) for t in db_tiers))

    # ---- anonymous refused ---------------------------------------------------
    s, _ = call("GET", "/tiers")
    check("anonymous GET /tiers is refused", s == 401, f"status={s}")

    # ---- operator sees exactly what the DB holds -----------------------------
    tok = sh("node", MINT).stdout.strip()
    check("platform-operator session minted", tok.count(".") == 2, f"len={len(tok)}")
    s, body = call("GET", "/tiers", tok)
    check("operator GET /tiers answers", s == 200, f"status={s}")
    api_tiers = []
    if s == 200:
        try:
            api_tiers = [(t.get("name"), str(t.get("monthly_point_allowance")))
                         for t in json.loads(body)]
        except Exception as e:
            print("  parse error:", e)
    check("the API returns exactly the tiers the database holds",
          sorted(api_tiers) == sorted([(n, str(a)) for n, a in db_tiers]),
          f"api={api_tiers} db={db_tiers}")
    check("the three live tiers are visible to the operator", len(api_tiers) == 3, str(len(api_tiers)))

    # ---- the screens themselves (real browser, the page's own controls) ------
    biz = one("SELECT id FROM businesses ORDER BY created_at LIMIT 1")
    title = f"{TITLE_PREFIX}-{int(time.time())}"
    s, body = call("POST", "/deals", tok, {"title": title, "business_id": biz})
    check("a disposable deal could be created for the proof", s == 201, f"status={s}")
    deal_id = ""
    if s == 201:
        try:
            deal_id = json.loads(body).get("id") or ""
        except Exception:
            pass
    check("the disposable deal has an id", bool(deal_id), deal_id[:8])

    chrome = os.environ.get("CHROME_PATH", "/usr/bin/chromium-browser")
    try:
        env = dict(os.environ, MD_BASE=BASE, TOKEN=tok, AUD=AUD,
                   TEMP_DEAL_TITLE=title, TEMP_DEAL_ID=deal_id, BIZ_ID=biz,
                   NODE_PATH=NODE_PATH, CHROME_PATH=chrome)
        r = sh("node", BROWSER, env=env, timeout=300)
        proof_path = os.path.join(AUD, "plans-deals-proof.json")
        proof = {}
        if os.path.exists(proof_path):
            proof = json.load(open(proof_path))
        else:
            print(r.stdout[-1200:])
            print(r.stderr[-1200:])
        bchecks = proof.get("checks", [])
        check("the console walk produced a proof file", bool(bchecks), proof_path)
        for c in bchecks:
            check("browser: " + c.get("name", "?"), bool(c.get("ok")), c.get("detail", ""))
        check("the console walk had no browser failures",
              proof.get("fail", 1) == 0, f"pass={proof.get('pass')} fail={proof.get('fail')}")
    finally:
        # ---- cleanup: net zero ------------------------------------------------
        if deal_id:
            ds, _ = call("DELETE", "/deals/" + deal_id, tok)
            check("the disposable deal was removed", ds in (200, 204), f"status={ds}")
        left = int(one("SELECT count(*) FROM deals WHERE title LIKE '%s%%'" % TITLE_PREFIX))
        check("no disposable deal remains", left == 0, f"remaining={left}")
        now_tiers = [tuple(r.split("|", 1)) for r in
                     psql("SELECT name, monthly_point_allowance FROM plan_tiers "
                          "ORDER BY price_monthly ASC")]
        check("the tiers are back to their before state", now_tiers == db_tiers,
              f"before={db_tiers} after={now_tiers}")
        check("no real deal was lost", int(one("SELECT count(*) FROM deals")) == db_deals,
              f"before={db_deals} after={one('SELECT count(*) FROM deals')}")

    # ---- the screens exist in the served console -----------------------------
    html = open(PANEL, encoding="utf-8", errors="ignore").read()
    for hook in ("plan-tiers-section", "plan-tier-row", "deals-section", "dl-list"):
        check(f"the console carries the {hook} control", hook in html)

    print("\nRESULT: " + ("PASS" if not FAILS else "FAIL"))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
