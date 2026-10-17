#!/usr/bin/env python3
"""DELETE /api/v1/directories/:slug — the operator door works end to end, or refuses honestly.

Surfaced by the fixtures of t_bf740579: `DELETE /api/v1/directories/:slug` was a bare
`DELETE FROM directories WHERE slug = $1`, while `create_directory` seeds the directory's own
children (`ad_zones` = 12, `email_templates` = 10, both `NO ACTION` FKs). The door therefore
returned **500 {"code":500,"message":"Database error"}** for a directory created seconds earlier —
unconditionally, not data-dependently (kanban t_e6f4e0aa).

Proves, against the RUNNING app and the live database:
  1. anonymous `DELETE /api/v1/directories/:slug` is refused (401) and destroys nothing;
  2. a valid NON-operator session is refused (404, no existence leak) and destroys nothing;
  3. a fresh operator create seeds the exact children that used to make the delete 500
     (`ad_zones`, `email_templates` — read from the DB, not assumed);
  4. `DELETE` on that directory answers **200** (regression: 500) and reports what it swept;
  5. the `directories` row is gone;
  6. NO child row anywhere still points at the deleted directory — swept dynamically over every
     foreign key that references `directories`, so a table nobody remembered is caught here;
  7. a directory that holds a CUSTOMER record (`crm_contacts`) is REFUSED with **409**, the body
     names it in plain English, and NOTHING was deleted — not the row, not its `ad_zones`;
  8. once the blocking record is gone, the SAME door succeeds (200) — the refusal is a door, not a wall;
  9. the cross-app workspace goes WITH the row (kanban t_d4b8aad7): each create provisions a
     CoreSwift workspace (`md-<slug>@local.coreswift` + 3 lists) and stores its id on the row; the
     probe reads that id, proves the workspace is REALLY live in CoreSwift, then requires the delete
     to answer `coreswift_workspace.status = "deprovisioned"` AND the tenant / user / lists to be
     gone from the RUNNING CoreSwift database (not a name grep, not the app's own word);
 10. a REFUSED delete (409, a customer record holds it) does not touch that workspace either;
 11. the NETWORK door does the same: `DELETE /api/v1/networks/:id` deprovisions the network's own
     workspace, while deleting a CITY that lives inside a network leaves the network's workspace
     alone (a city has no workspace of its own — the network's is not the city's to destroy);
 12. the doors leave ZERO cross-app residue ON THEIR OWN, before any cleanup script runs;
 13. cleanup is net zero: the sweep by reserved address is still run as the safety net, no probe
     directory/network/CRM fixture is left, and the live rows are back to their before state.


Run:  python3 scripts/delete-directory-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Writes only into $AUD (default /opt/swift/audits/zz-del-door-...).
"""
import hashlib
import hmac
import json
import os
import re
import subprocess
import sys
import time

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
MINT = "/opt/swift/browser-run/md-mint-token.js"
CLEANUP = "/opt/swift/scripts/probe-identity-cleanup.py"
MD_ENV = "/etc/swift/env/multi-directory.env"
CS_ENV = "/etc/swift/env/coreswift.env"
# Fleet probe attribution (kanban t_8bea040a): the reserved address `md-<slug>@local.coreswift` is
# the convention, this marker is the tag CoreSwift stores on the workspace it mints from our call.
HARNESS = os.environ.get("MD_HARNESS", "t_e6f4e0aa")
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-del-door-{time.strftime('%Y%m%d-%H%M%S')}")
SLUG_PREFIX = "zz-del-door"
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def env_value(path, key):
    try:
        with open(path) as fh:
            for line in fh:
                s = line.strip()
                if s.startswith(key + "="):
                    return s.split("=", 1)[1].strip().strip('"')
    except OSError:
        return ""
    return ""


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
    # The fleet marker travels on every call this harness makes (kanban t_8bea040a): the product
    # forwards it onto the cross-app CoreSwift register call, which is where the residue row is written.
    req.add_header("X-Swift-Harness", HARNESS)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def mint_non_operator():
    """A structurally valid session that is NOT the platform operator (role `directory_admin`)."""
    secret = env_value(MD_ENV, "JWT_SECRET")
    if not secret:
        return ""
    b64 = lambda o: __import__("base64").urlsafe_b64encode(  # noqa: E731
        json.dumps(o).encode()).decode().rstrip("=")
    now = int(time.time())
    msg = b64({"alg": "HS256", "typ": "JWT"}) + "." + b64({
        "sub": "00000000-0000-0000-0000-000000000003",
        "tid": "00000000-0000-0000-0000-000000000001",
        "role": "directory_admin", "exp": now + 600, "iat": now,
        "aud": "multidirectory-api", "iss": "multidirectory",
    })
    sig = hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()
    import base64
    return msg + "." + base64.urlsafe_b64encode(sig).decode().rstrip("=")


def child_fk_tables():
    """Every (table, column) that references directories — read from the catalog, never a hand list."""
    rows = psql(
        "SELECT c.conrelid::regclass::text || '|' || a.attname "
        "FROM pg_constraint c JOIN LATERAL unnest(c.conkey) k ON true "
        "JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k "
        "WHERE c.confrelid = 'directories'::regclass AND c.contype = 'f' ORDER BY 1")
    out = []
    for r in rows:
        t, col = r.split("|", 1)
        out.append((t.split(".")[-1], col))
    return out


def child_rows_for(directory_id):
    """Total rows across ALL child tables that still point at `directory_id`."""
    total = 0
    detail = []
    for table, col in child_fk_tables():
        n = int(one(f'SELECT count(*) FROM "{table}" WHERE "{col}" = \'{directory_id}\''))
        if n:
            total += n
            detail.append(f"{table}={n}")
    return total, detail


def cs_residue():
    """(users, tenants) matching this probe's zz naming in CoreSwift — the cross-app residue."""
    url = env_value(CS_ENV, "DATABASE_URL")
    m = re.search(r"://[^:]+:([^@]+)@[^/]+/([A-Za-z0-9_]+)", url)
    if not m:
        return None, None
    def q(sql):
        r = sh("docker", "exec", "-e", f"PGPASSWORD={m.group(1)}", "swift-postgres",
               "psql", "-U", "swift", "-d", m.group(2), "-At", "-c", sql)
        return r.stdout.strip() if r.returncode == 0 else None
    return (q("SELECT count(*) FROM users WHERE email LIKE 'md-zz-del-door%local.coreswift'"),
            q("SELECT count(*) FROM tenants WHERE name ILIKE '%zz delete door%'"))


def cs_q(sql):
    """One value out of the RUNNING CoreSwift database — the cross-app proof, not an app claim."""
    url = env_value(CS_ENV, "DATABASE_URL")
    m = re.search(r"://[^:]+:([^@]+)@[^/]+/([A-Za-z0-9_]+)", url)
    if not m:
        return ""
    r = sh("docker", "exec", "-e", f"PGPASSWORD={m.group(1)}", "swift-postgres",
           "psql", "-U", "swift", "-d", m.group(2), "-At", "-c", sql)
    return r.stdout.strip() if r.returncode == 0 else ""


def wait_workspace_id(select_sql, tries=60):
    """The row's `coreswift_tenant_id`. Provisioning is a spawned task, so it lands a beat after
    the create; poll instead of racing it (an empty id would read as 'never provisioned')."""
    for _ in range(tries):
        tid = one(select_sql)
        if tid:
            return tid
        time.sleep(0.5)
    return ""


def cs_workspace_state(tid, slug):
    """(tenant rows, md users, lists) this workspace still owns in CoreSwift, read live."""
    if not tid:
        return "?", "?", "?"
    return (cs_q(f"SELECT count(*) FROM tenants WHERE id='{tid}'"),
            cs_q(f"SELECT count(*) FROM users WHERE email='md-{slug}@local.coreswift'"),
            cs_q(f"SELECT count(*) FROM lists WHERE tenant_id='{tid}'"))


def swept_cs(body):
    """The `coreswift_workspace` object the delete door answered with."""
    try:
        return json.loads(body).get("coreswift_workspace") or {}
    except Exception:
        return {}


def sweep_cs_workspaces(slugs, tries=10):
    detail = ""
    for _ in range(tries):
        rcs = []
        for slug in slugs:
            r = sh("python3", CLEANUP, "--app", "coreswift_crm", "--email",
                   f"md-{slug}@local.coreswift")
            rcs.append(r.returncode)
            out = (r.stdout + r.stderr).strip()
            if out:
                detail = f"{slug}: {out.splitlines()[-1]}"
        users, tenants = cs_residue()
        if set(rcs) == {0} and users == "0" and tenants == "0":
            return True, detail
        time.sleep(3)
    return False, detail


def db_directories():
    return psql("SELECT slug FROM directories ORDER BY slug")


def preclean():
    """Remove anything a previous failed run left behind — children first, then the parents.

    Deleting the parents first would trip the very `NO ACTION` foreign keys this card is about, so
    the sweep walks the catalog exactly like the probe's own residue check does.
    """
    ids = psql(f"SELECT id FROM directories WHERE slug LIKE '{SLUG_PREFIX}%'")
    if ids:
        quoted = ", ".join(f"'{i}'" for i in ids)
        for table, col in child_fk_tables():
            psql(f'DELETE FROM "{table}" WHERE "{col}" IN ({quoted})')
        psql(f"DELETE FROM directories WHERE slug LIKE '{SLUG_PREFIX}%'")
    psql("DELETE FROM businesses WHERE slug LIKE 'zz-delete-door%'")
    # Networks last: `directories.network_id` is ON DELETE SET NULL, so the directories above must
    # already be gone or a stale probe network would be left holding nothing.
    psql(f"DELETE FROM networks WHERE slug LIKE '{SLUG_PREFIX}%'")


def main():
    os.makedirs(AUD, exist_ok=True)
    preclean()
    stamp = str(int(time.time()))
    main_slug = f"{SLUG_PREFIX}-{stamp}"
    block_slug = f"{SLUG_PREFIX}-blk-{stamp}"
    other_slug = f"{SLUG_PREFIX}-nonop-{stamp}"
    minted = []

    before = db_directories()
    print("DB truth: directories =", len(before), "|", ", ".join(before))

    # -- 1. anonymous refused ------------------------------------------------
    s, _ = call("DELETE", "/directories/anonymous-probe")
    check("anonymous DELETE is refused", s == 401, f"status={s}")

    tok = sh("node", MINT).stdout.strip()
    check("platform-operator session minted", tok.count(".") == 2, f"len={len(tok)}")

    # -- 2. create probe A (the common case: a directory that was just created) -----
    s, body = call("POST", "/directories", tok,
                   {"name": "zz delete door probe", "slug": main_slug})
    check("operator POST /directories creates the probe directory", s in (200, 201), f"status={s}")
    minted.append(main_slug)
    did = one(f"SELECT id FROM directories WHERE slug='{main_slug}'")
    check("the probe directory landed in the database", bool(did), did)

    # -- 2b. it owns a REAL CoreSwift workspace (the thing a delete used to orphan) ------
    main_tid = wait_workspace_id(
        f"SELECT coreswift_tenant_id FROM directories WHERE slug='{main_slug}'")
    check("the create provisioned a CoreSwift workspace and MD stored its id", bool(main_tid),
          main_tid or "no coreswift_tenant_id after 30s")
    mt, mu, ml = cs_workspace_state(main_tid, main_slug)
    check("the workspace is genuinely LIVE in CoreSwift before the delete",
          mt == "1" and mu == "1", f"tenants={mt} users={mu} lists={ml}")

    # -- 3. the seeded children that used to make the delete a 500 -----------
    zones = int(one(f"SELECT count(*) FROM ad_zones WHERE directory_id='{did}'") or 0)
    tmpl = int(one(f"SELECT count(*) FROM email_templates WHERE directory_id='{did}'") or 0)
    check("the fresh create seeded ad zones and email templates (the old 500 cause)",
          zones > 0 and tmpl > 0, f"ad_zones={zones} email_templates={tmpl}")

    # -- 4. a non-operator session cannot destroy it -------------------------
    other_tok = mint_non_operator()
    s, _ = call("DELETE", f"/directories/{main_slug}", other_tok or None)
    check("a non-operator session is refused", s in (403, 404), f"status={s}")
    check("the refused non-operator delete destroyed nothing",
          one(f"SELECT count(*) FROM directories WHERE slug='{main_slug}'") == "1")

    # -- 5. THE DOOR: the delete used to be a hard 500 -----------------------
    s, body = call("DELETE", f"/directories/{main_slug}", tok)
    check("operator DELETE answers 200 (regression: 500)", s == 200, f"status={s} body={body[:200]}")
    swept = {}
    try:
        swept = json.loads(body).get("children_removed") or {}
    except Exception:
        pass
    check("the delete reports what it swept", bool(swept), json.dumps(swept)[:200])
    check("the directories row is gone",
          one(f"SELECT count(*) FROM directories WHERE slug='{main_slug}'") == "0")
    left, detail = child_rows_for(did)
    check("NO child row anywhere still points at the deleted directory", left == 0,
          ", ".join(detail) or "0 rows in every child table")

    # -- 5b. THE ORPHAN: the CoreSwift workspace goes with the directory (t_d4b8aad7) -----
    ws = swept_cs(body)
    check("the delete answer reports what became of the CoreSwift workspace", bool(ws),
          json.dumps(ws)[:220])
    check("the workspace is reported deprovisioned, not silently left", ws.get("status") == "deprovisioned",
          str(ws.get("status")) + " " + str(ws.get("detail"))[:160])
    check("the answer says the cross-app side is clean", ws.get("clean") is True)
    mt2, mu2, ml2 = cs_workspace_state(main_tid, main_slug)
    check("the CoreSwift tenant row is GONE from the running CoreSwift", mt2 == "0", f"tenants={mt2}")
    check("the workspace's md-<slug>@local.coreswift account is GONE", mu2 == "0", f"users={mu2}")
    check("nothing hangs off that workspace id any more (its lists)", ml2 == "0", f"lists={ml2}")

    # -- 6. a directory holding a customer record is REFUSED, not cascaded ----
    s, _ = call("POST", "/directories", tok,
                {"name": "zz delete door blocked probe", "slug": block_slug})
    check("second probe directory created", s in (200, 201), f"status={s}")
    minted.append(block_slug)
    bid = one(f"SELECT id FROM directories WHERE slug='{block_slug}'")
    btid = wait_workspace_id(
        f"SELECT coreswift_tenant_id FROM directories WHERE slug='{block_slug}'")
    check("the blocked probe owns a live CoreSwift workspace too", bool(btid) and
          cs_q(f"SELECT count(*) FROM tenants WHERE id='{btid}'") == "1", btid)
    psql(f"INSERT INTO crm_contacts (directory_id, first_name, last_name, email, source) "
         f"VALUES ('{bid}', 'ZZ', 'DeleteDoorProbe', 'zz-del-door-probe@example.invalid', "
         f"'zz-delete-door-probe')")
    bzones = int(one(f"SELECT count(*) FROM ad_zones WHERE directory_id='{bid}'") or 0)
    s, body = call("DELETE", f"/directories/{block_slug}", tok)
    check("a directory holding a CRM contact is REFUSED with 409", s == 409, f"status={s}")
    check("the refusal names what blocks it, in plain English",
          "CRM contact" in body and "crm_contacts" not in body, body[:300])
    check("the refused delete left the directory intact",
          one(f"SELECT count(*) FROM directories WHERE slug='{block_slug}'") == "1")
    check("the refused delete swept nothing (its ad zones are untouched)",
          int(one(f"SELECT count(*) FROM ad_zones WHERE directory_id='{bid}'") or 0) == bzones,
          f"ad_zones={bzones}")
    crm_blocks = int(one(f"SELECT count(*) FROM crm_contacts WHERE directory_id='{bid}'") or 0)
    check("the refused delete destroyed no customer record", crm_blocks == 1, f"rows={crm_blocks}")
    check("the REFUSED delete did not touch the CoreSwift workspace either",
          cs_q(f"SELECT count(*) FROM tenants WHERE id='{btid}'") == "1", btid)

    # -- 7. the same door succeeds once the blocking record is gone ----------
    psql(f"DELETE FROM crm_contacts WHERE directory_id='{bid}' AND source='zz-delete-door-probe'")
    s, body = call("DELETE", f"/directories/{block_slug}", tok)
    check("the same door succeeds once the blocking record is gone", s == 200,
          f"status={s} body={body[:200]}")
    check("the second directory row is gone",
          one(f"SELECT count(*) FROM directories WHERE slug='{block_slug}'") == "0")
    left, detail = child_rows_for(bid)
    check("no child row anywhere still points at the second directory", left == 0,
          ", ".join(detail) or "0 rows in every child table")
    bws = swept_cs(body)
    check("the second directory's workspace is deprovisioned with it",
          bws.get("status") == "deprovisioned", str(bws.get("status")) + " " + str(bws.get("detail"))[:160])
    bt, bu, _ = cs_workspace_state(btid, block_slug)
    check("the second workspace is really gone from CoreSwift", bt == "0" and bu == "0",
          f"tenants={bt} users={bu}")

    # -- 7b. a directory that holds its OWN content (listings + their claims) -----
    #      The schema cascades listings and this door sweeps the proofs, so a real directory must
    #      still delete cleanly — a guard that only handles an empty directory is not a door.
    cas_slug = f"{SLUG_PREFIX}-cas-{stamp}"
    s, _ = call("POST", "/directories", tok,
                {"name": "zz delete door cascade probe", "slug": cas_slug})
    check("third probe directory created", s in (200, 201), f"status={s}")
    minted.append(cas_slug)
    cid = one(f"SELECT id FROM directories WHERE slug='{cas_slug}'")
    ctid = wait_workspace_id(
        f"SELECT coreswift_tenant_id FROM directories WHERE slug='{cas_slug}'")
    psql(f"INSERT INTO businesses (directory_id, name, slug) "
         f"VALUES ('{cid}', 'ZZ Delete Door Probe Business', 'zz-delete-door-probe-business')")
    cbid = one(f"SELECT id FROM businesses WHERE directory_id='{cid}'")
    psql(f"INSERT INTO business_verifications (business_id, directory_id, method, status) "
         f"VALUES ('{cbid}', '{cid}', 'manual', 'pending')")
    s, body = call("DELETE", f"/directories/{cas_slug}", tok)
    check("a directory holding its own listings deletes cleanly (cascade + sweep)", s == 200,
          f"status={s} body={body[:200]}")
    check("the listings were never orphaned",
          one(f"SELECT count(*) FROM businesses WHERE directory_id='{cid}'") == "0")
    left, detail = child_rows_for(cid)
    check("no child row anywhere still points at the listing directory", left == 0,
          ", ".join(detail) or "0 rows in every child table")
    ct, cu, _ = cs_workspace_state(ctid, cas_slug)
    check("the listing directory's workspace went with it too", ct == "0" and cu == "0",
          f"status={swept_cs(body).get('status')} tenants={ct} users={cu}")

    # -- 8. the NETWORK door owns a workspace too (t_d4b8aad7) ---------------
    #      A network provisions its own CoreSwift tenant; deleting the network used to leave it
    #      live for ever, and there was no door at all that would ever reach it.
    net_slug = f"{SLUG_PREFIX}-net-{stamp}"
    s, nbody = call("POST", "/networks", tok,
                    {"name": "zz delete door network", "slug": net_slug})
    check("operator POST /networks creates the probe network", s in (200, 201),
          f"status={s} body={nbody[:160]}")
    nid = one(f"SELECT id FROM networks WHERE slug='{net_slug}'")
    net_tid = wait_workspace_id(f"SELECT coreswift_tenant_id FROM networks WHERE id='{nid}'")
    check("the network create provisioned its own CoreSwift workspace", bool(net_tid),
          net_tid or "no coreswift_tenant_id on the network after 30s")
    check("the network workspace is live in CoreSwift",
          cs_q(f"SELECT count(*) FROM tenants WHERE id='{net_tid}'") == "1", net_tid)

    # a CITY inside that network: it has no workspace of its own, and the network's is not its to
    # destroy — this is the bleed the scoped-credential rule exists for, one layer up.
    city_slug = f"{SLUG_PREFIX}-city-{stamp}"
    s, cbody = call("POST", "/directories", tok,
                    {"name": "zz delete door city", "slug": city_slug, "network_mode": "connect",
                     "parent_network_id": nid, "url_type": "subfolder"})
    check("a city can be created inside the probe network", s in (200, 201),
          f"status={s} body={cbody[:160]}")
    minted.append(city_slug)
    check("the city was linked to the network",
          one(f"SELECT network_id FROM directories WHERE slug='{city_slug}'") == nid)
    s, cbody = call("DELETE", f"/directories/{city_slug}", tok)
    check("the city deletes cleanly", s == 200, f"status={s} body={cbody[:160]}")
    cws = swept_cs(cbody)
    check("a city with no workspace of its own reports 'not_provisioned'",
          cws.get("status") == "not_provisioned", str(cws.get("status")) + " " + str(cws.get("detail"))[:160])
    check("deleting a CITY left the NETWORK's workspace untouched",
          cs_q(f"SELECT count(*) FROM tenants WHERE id='{net_tid}'") == "1", net_tid)

    s, nbody = call("DELETE", f"/networks/{nid}", tok)
    check("operator DELETE /networks/:id answers 200", s == 200,
          f"status={s} body={nbody[:200]}")
    nws = swept_cs(nbody)
    check("the network delete reports its workspace deprovisioned",
          nws.get("status") == "deprovisioned",
          str(nws.get("status")) + " " + str(nws.get("detail"))[:160])
    check("the network's CoreSwift tenant is GONE from the running CoreSwift",
          cs_q(f"SELECT count(*) FROM tenants WHERE id='{net_tid}'") == "0")
    check("the network's md-<slug>@local.coreswift account is GONE",
          cs_q(f"SELECT count(*) FROM users WHERE email='md-{net_slug}@local.coreswift'") == "0")
    check("the network row itself is gone",
          one(f"SELECT count(*) FROM networks WHERE slug='{net_slug}'") == "0")

    # -- 9. nothing of the probe is left behind ------------------------------
    leftover = int(one(f"SELECT count(*) FROM directories WHERE slug LIKE '{SLUG_PREFIX}%'") or 0)
    check("no probe directory remains in the database", leftover == 0, f"remaining={leftover}")
    check("no probe network remains in the database",
          one(f"SELECT count(*) FROM networks WHERE slug LIKE '{SLUG_PREFIX}%'") == "0")
    check("no probe CRM fixture remains",
          one("SELECT count(*) FROM crm_contacts WHERE source='zz-delete-door-probe'") == "0")
    after = db_directories()
    check("the live directories are back to their before state", after == before,
          f"before={len(before)} after={len(after)}")

    # -- 10. the doors left NOTHING in CoreSwift on their own (t_d4b8aad7) ----
    #       This runs BEFORE any cleanup script: the sweep further down is the safety net, so if it
    #       fires here the door did not do its job and this check — not the sweep — says so.
    users, tenants = cs_residue()
    check("CoreSwift was reachable for the residue check", users is not None and tenants is not None,
          f"users={users} tenants={tenants}")
    check("the delete doors left ZERO CoreSwift residue ON THEIR OWN (no sweep yet)",
          users == "0" and tenants == "0", f"users={users} tenants={tenants}")

    # -- 11. the cross-app workspace (CoreSwift) -----------------------------
    deleted, detail = sweep_cs_workspaces(minted)
    users, tenants = cs_residue()
    check("every CoreSwift workspace this probe minted was swept by address", deleted, detail)
    check("no md-zz-del-door residue is left in CoreSwift",
          users == "0" and tenants == "0", f"users={users} tenants={tenants}")

    print()
    if FAILS:
        print(f"FAIL — {len(FAILS)} check(s) failed:")
        for f in FAILS:
            print("  -", f)
        return 1
    print(f"PASS — every check green. Workspace: {AUD}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
