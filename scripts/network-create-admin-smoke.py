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
  7. cleanup is net-zero: every probe network is gone and the live network count is back to before;
  8. the CROSS-APP residue is cleaned and proven zero: creating a network provisions a CoreSwift
     workspace, and deleting the MD network does not touch it, so each run used to leave two
     orphan workspaces ("zz api probe" + "zz builds item 13 probe") behind in the tenant list
     (kanban t_8bea040a). Both workspaces are deleted by their exact reserved address through
     `/opt/swift/scripts/probe-identity-cleanup.py --app coreswift_crm`, and the probe asserts no
     `zz` workspace and no `md-zz*` user is left in the CoreSwift database;
  9. the minted workspaces carry the fleet marker: the probe sends `X-Swift-Harness: <MD_HARNESS
     or t_8bea040a>` on its own calls AND the product forwards it onto the cross-app register call,
     so `tenants.probe_harness` on the CoreSwift workspace is non-NULL and names this card.

The minted address must stay `md-<slug>@local.coreswift`: that domain is what the fleet residue
instrument (`scripts/fleet-probe-residue.py` LOCAL_APP_PROBE_DOM / `probe-residue-classify.py`
LOCAL_APP_PROBE) recognises as non-routable, so the sweep PAGES on a leaked workspace. Changing the
shape re-blinds the guard until the new `<x>@local.<app>` domain is added to BOTH constants
(`/opt/swift/docs/fleet-harness-probe-address-convention-2026-09-28.md`).

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
CLEANUP = "/opt/swift/scripts/probe-identity-cleanup.py"
CS_ENV = "/etc/swift/env/coreswift.env"
# Fleet probe attribution (kanban t_8bea040a). The reserved address `md-<slug>@local.coreswift` is
# convention (a); the marker below is layer (c) — the tag CoreSwift stores on the workspace it mints
# from our register call, so a sweep can tell this probe's workspace from a customer's. Override
# with MD_HARNESS when another card drives this probe.
HARNESS = os.environ.get("MD_HARNESS", "t_8bea040a")
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
    # the fleet probe marker travels on EVERY call this harness makes (kanban t_8bea040a): the
    # product forwards it onto the cross-app CoreSwift register call, which is where the row that
    # marks the residue is actually written.
    req.add_header("X-Swift-Harness", HARNESS)
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


def coreswift_psql(sql):
    """One read-only query against CoreSwift's database (the OTHER app this probe touches).

    The probe's own database is the MD one, so the cross-app assertions (the workspace marker and
    the workspace residue) need their own connection. `None` = could not read (treated as a failure
    by the caller, never as "clean").
    """
    url = ""
    try:
        with open(CS_ENV) as fh:
            for line in fh:
                if line.startswith("DATABASE_URL="):
                    url = line.strip().split("=", 1)[1]
    except OSError:
        return None
    m = re.search(r"://[^:]+:([^@]+)@[^/]+/([A-Za-z0-9_]+)", url)
    if not m:
        return None
    r = sh("docker", "exec", "-e", f"PGPASSWORD={m.group(1)}", "swift-postgres",
           "psql", "-U", "swift", "-d", m.group(2), "-At", "-c", sql)
    if r.returncode != 0:
        return None
    return r.stdout.strip()


def cs_workspace_markers():
    """`name|probe_harness` for every zz-named CoreSwift workspace (the marker evidence)."""
    out = coreswift_psql("SELECT name || '|' || COALESCE(probe_harness,'NULL') FROM tenants "
                         "WHERE name ILIKE '%zz%' ORDER BY name")
    return [l for l in (out or "").splitlines() if l]


def cs_zz_residue():
    """(users, tenants) still matching this probe's zz naming — the cross-app residue count."""
    users = coreswift_psql("SELECT count(*) FROM users WHERE email LIKE 'md-zz%local.coreswift'")
    tenants = coreswift_psql("SELECT count(*) FROM tenants WHERE name ILIKE '%zz%'")
    return users, tenants


def cs_outbound():
    """Rows CoreSwift queued to send to a probe address (must be 0 — nothing may be mailed)."""
    return coreswift_psql("SELECT count(*) FROM outbound_messages "
                          "WHERE to_address ILIKE '%local.coreswift%'")


def delete_cs_workspaces(slugs, tries=10):
    """Delete the CoreSwift workspaces minted for `slugs` by their EXACT reserved addresses and prove
    the cross-app residue is zero.

    Both addresses are deleted in each pass and the residue is asserted GLOBALLY (no `zz` workspace,
    no `md-zz*` user), never per-slug: a per-slug tenant count reads the other workspace still sitting
    there and can never reach zero (measured 2026-10-14). `provision_tenant` also runs in a detached
    task, so a workspace can still be landing when the MD row is already gone — hence the re-run
    rather than a single pass.
    """
    detail = ""
    for _ in range(tries):
        rcs = []
        for slug in slugs:
            email = f"md-{slug}@local.coreswift"
            r = sh("python3", CLEANUP, "--app", "coreswift_crm", "--email", email)
            rcs.append(r.returncode)
            out = (r.stdout + r.stderr).strip()
            if out:
                detail = f"{slug}: {out.splitlines()[-1]}"
        users, tenants = cs_zz_residue()
        # both signals: the tool must have succeeded (its own RESIDUE=0 -> rc 0) AND the cross-app
        # counts must read zero (a detached provisioning task can land after the delete).
        if set(rcs) == {0} and users == "0" and tenants == "0":
            return True, detail
        time.sleep(3)
    return False, detail


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
        env = dict(os.environ, MD_BASE=BASE, TOKEN=tok, AUD=AUD, MD_HARNESS=HARNESS,
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
        # ---- the freed row was never the whole residue (kanban t_8bea040a) -------------------
        # Both creates above provisioned a CORE SWIFT WORKSPACE (the API create and the console
        # create), and deleting the MD network does not touch it — so read the marker and the
        # cross-app residue while the workspaces still exist, then delete them.
        rows = cs_workspace_markers()
        names = [rec.split("|", 1)[0] for rec in rows]
        check("both harness paths minted a CoreSwift workspace",
              any("zz api probe" in n for n in names)
              and any("zz builds item 13 probe" in n for n in names), str(rows))
        check("every minted CoreSwift workspace carries the harness marker",
              bool(rows) and all(rec.split("|", 1)[1] == HARNESS for rec in rows), str(rows))
        check("CoreSwift queued no mail to a probe address", cs_outbound() == "0",
              f"outbound={cs_outbound()}")

        # ---- cleanup: net zero ------------------------------------------------
        psql(f"DELETE FROM networks WHERE slug LIKE '{SLUG_PREFIX}%'")
        left = int(one(f"SELECT count(*) FROM networks WHERE slug LIKE '{SLUG_PREFIX}%'"))
        check("no probe network remains in the database", left == 0, f"remaining={left}")
        after = db_networks()
        check("the live networks are back to their before state", after == before,
              f"before={db_nets} after={len(after)}")

        # ---- cleanup: the cross-app workspace --------------------------------
        deleted, detail = delete_cs_workspaces([api_slug, probe_slug])
        check("the CoreSwift workspaces both harness creates minted were deleted", deleted, detail)
        users_left, tenants_left = cs_zz_residue()
        check("no probe workspace or user remains in CoreSwift",
              users_left == "0" and tenants_left == "0",
              f"users={users_left} tenants={tenants_left}")

    # ---- the screen exists in the served console -----------------------------
    html = open(PANEL, encoding="utf-8", errors="ignore").read()
    for hook in ("networks-section", 'id="nw-new"', 'id="nw-save"', "data-nw-edit", "data-nw-del"):
        check(f"the console carries the {hook} control", hook in html)

    print("\nRESULT: " + ("PASS" if not FAILS else "FAIL"))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
