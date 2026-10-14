#!/usr/bin/env python3
"""Networks screen — create / edit / delete a network from the admin console (ZaarHub operator item 13).

The operator must be able to create a directory network from the admin console — a capability that
exists only as an endpoint is a defect by David's rule. The network CRUD handlers existed in
`handlers/networks.rs` but were UNROUTED, so a network could only be created with SQL; the console's
Networks card is the control, and this proves it end to end.

Proves, against the running app and the live database:
  1. anonymous `GET/POST /api/v1/networks` is refused (the global auth guard);
  2. the networks the API returns are exactly the rows the database holds (count + values);
  3. the API itself refuses a root domain that is not a bare domain, and creates no row for it;
  4. the API normalises a scheme-bearing root domain to its bare form (and the disposable row is
     removed again — net zero);
  5. the SERVED console (`frontend/admin-panel.html`, real browser through the page's own controls,
     `scripts/network-create-admin-browser.js`): the card and nav link exist; "＋ New network" opens
     the editor; a junk root domain is refused ON SCREEN with the server's own message and writes no
     row; the card's own "Save network" creates a REAL row (read back from DB + API); the new
     network shows in the table and exposes its id; the row's own "Edit" opens pre-filled and its
     Save writes the new name through; a network that still has cities attached is REFUSED on screen
     (nothing orphaned); the row's own "Delete" removes the network (API + table);
  6. no console or auth errors during the walk;
  7. cleanup is net-zero: every probe network is gone and the live network count is back to before.

Run:  python3 scripts/network-create-admin-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Writes only into $AUD (default /opt/swift/audits/zz-item13-netcreate-...).
"""
import json
import os
import re
import subprocess
import sys
import time

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "network-create-admin-browser.js")
PANEL = os.path.join(APP, "frontend", "admin-panel.html")
NODE_PATH = "/opt/swift/browser-run/node_modules"
MINT = "/opt/swift/browser-run/md-mint-token.js"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item13-netcreate-{time.strftime('%Y%m%d-%H%M%S')}")
SLUG_PREFIX = "zz-nw-probe"
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


def db_networks():
    return psql("SELECT id, name, slug, COALESCE(root_domain,''), COALESCE(status,'') "
                "FROM networks ORDER BY slug")


def main():
    os.makedirs(AUD, exist_ok=True)
    psql(f"DELETE FROM networks WHERE slug LIKE '{SLUG_PREFIX}%'")

    stamp = str(int(time.time()))
    probe_slug = f"{SLUG_PREFIX}-{stamp}"
    probe_root = f"zz-nw-{stamp}.example"

    # ---- live DB truth -------------------------------------------------------
    before = db_networks()
    db_nets = len(before)
    print("DB truth: networks =", db_nets, "|", "; ".join(r.split("|")[1] for r in before))
    check("the database holds at least one network", db_nets >= 1, str(db_nets))

    # ---- anonymous refused ---------------------------------------------------
    s, _ = call("GET", "/networks")
    check("anonymous GET /networks is refused", s == 401, f"status={s}")
    s, _ = call("POST", "/networks", None,
                {"name": "zz anonymous", "slug": probe_slug + "-anon", "root_domain": probe_root})
    check("anonymous POST /networks is refused", s == 401, f"status={s}")

    # ---- operator sees exactly what the DB holds -----------------------------
    tok = sh("node", MINT).stdout.strip()
    check("platform-operator session minted", tok.count(".") == 2, f"len={len(tok)}")
    s, body = call("GET", "/networks", tok)
    check("operator GET /networks answers", s == 200, f"status={s}")
    api_nets = []
    if s == 200:
        try:
            api_nets = sorted(
                f"{n.get('id')}|{n.get('name')}|{n.get('slug')}|{n.get('root_domain') or ''}|{n.get('status') or ''}"
                for n in json.loads(body))
        except Exception as e:
            print("  parse error:", e)
    check("the API returns exactly the networks the database holds",
          api_nets == sorted(before), f"api={len(api_nets)} db={len(before)}")

    # ---- the API refuses a junk root domain and writes nothing ---------------
    api_slug = f"{SLUG_PREFIX}-api-{stamp}"
    s, body = call("POST", "/networks", tok,
                   {"name": "zz api probe", "slug": api_slug, "root_domain": "not a domain"})
    check("the API refuses a root domain that is not a bare domain",
          s not in (200, 201), f"status={s}")
    check("the refused create wrote no row",
          int(one(f"SELECT count(*) FROM networks WHERE slug='{api_slug}'")) == 0, api_slug)

    # ---- the API normalises a scheme-bearing root domain ---------------------
    s, body = call("POST", "/networks", tok,
                   {"name": "zz api probe", "slug": api_slug, "root_domain": "https://ZzNetProbe.example/"})
    check("the API accepts a valid root domain", s in (200, 201), f"status={s}")
    stored = one(f"SELECT COALESCE(root_domain,'') FROM networks WHERE slug='{api_slug}'")
    check("a scheme-bearing root domain is stored bare", stored == "zznetprobe.example", stored)
    api_id = one(f"SELECT id FROM networks WHERE slug='{api_slug}'")
    ds, _ = call("DELETE", "/networks/" + api_id, tok) if api_id else (0, "")
    check("the disposable API network was removed", bool(api_id) and ds in (200, 204), f"status={ds}")

    # ---- the SCREEN itself (real browser, the page's own controls) -----------
    chrome = os.environ.get("CHROME_PATH", "/usr/bin/chromium-browser")
    try:
        env = dict(os.environ, MD_BASE=BASE, TOKEN=tok, AUD=AUD,
                   PROBE_NAME="zz builds item 13 probe", PROBE_SLUG=probe_slug,
                   PROBE_ROOT=probe_root, NODE_PATH=NODE_PATH, CHROME_PATH=chrome)
        r = sh("node", BROWSER, env=env, timeout=300)
        proof_path = os.path.join(AUD, "networks-proof.json")
        proof = json.load(open(proof_path)) if os.path.exists(proof_path) else {}
        if not proof.get("checks"):
            print(r.stdout[-1500:])
            print(r.stderr[-1500:])
        bchecks = proof.get("checks", [])
        check("the console walk produced a proof file", bool(bchecks), proof_path)
        for c in bchecks:
            check("browser: " + c.get("name", "?"), bool(c.get("ok")), c.get("detail", ""))
        check("the console walk had no browser failures", proof.get("fail", 1) == 0,
              f"pass={proof.get('pass')} fail={proof.get('fail')}")
    finally:
        # ---- cleanup: net zero ------------------------------------------------
        psql(f"DELETE FROM networks WHERE slug LIKE '{SLUG_PREFIX}%'")
        left = int(one(f"SELECT count(*) FROM networks WHERE slug LIKE '{SLUG_PREFIX}%'"))
        check("no probe network remains in the database", left == 0, f"remaining={left}")
        after = db_networks()
        check("the live networks are back to their before state", after == before,
              f"before={db_nets} after={len(after)}")

    # ---- the screen exists in the served console -----------------------------
    html = open(PANEL, encoding="utf-8", errors="ignore").read()
    for hook in ("networks-section", 'id="nw-new"', 'id="nw-save"', "data-nw-edit", "data-nw-del"):
        check(f"the console carries the {hook} control", hook in html)

    print("\nRESULT: " + ("PASS" if not FAILS else "FAIL"))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
