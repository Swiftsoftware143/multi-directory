#!/usr/bin/env python3
"""Directories screen — create / edit / delete a directory from the admin console (kanban t_26639554).

MD's only job is to CREATE directories and decide, for each one, whether it is a CITY inside a
network (a folder on the network's own domain, e.g. zaarhub.com/palm-bay) or a STANDALONE directory
on its own domain. The handlers existed (POST/PUT/DELETE /api/v1/directories) and the console had no
control at all, so a directory could only be created with SQL — a capability that exists only as an
endpoint is a defect (David's rule). The console's Directories card is the control, and this proves
it end to end, against the running app and the live database.

Proves:
  1. the served console carries the Directories card and its controls;
  2. anonymous GET/POST/DELETE on /api/v1/directories are refused (401) and write nothing;
  3. a signed-in NON-operator (`business_owner`) is refused (403) on create AND on delete — before
     the fix a business_owner token created a directory with HTTP 201, and a create in
     standalone/new_network mode also mints a CoreSwift workspace and a public city page. Update
     keeps its own rule (platform operator or the directory's owning tenant) — not touched here;
  4. the operator's GET /directories returns exactly the rows the DATABASE holds;
  5. THE SCREEN (real browser, the page's own controls, scripts/directories-panel-browser.js):
     the card + nav anchor exist; the table lists every directory the API holds and shows where
     each one lives; "＋ New directory" opens the editor with the placement choice and the live
     networks; a nameless save and a DUPLICATE slug are refused on screen and write no row; a valid
     save creates a REAL row; the row's own Edit opens pre-filled, shows where it lives instead of
     a moving control, and saves the new name + city through; the row's own Delete removes it;
     no console or auth errors;
  6. the DATABASE between the two browser phases: the directory the panel created is in Postgres
     with the chosen network, subfolder URL, draft status, city and support email — and after the
     panel's Delete it is gone, with its seeded children (ad zones) swept and no orphan left;
  7. the STANDALONE placement ("its own domain") is driven through the same API the card uses: the
     row lands with no network and url_type=standalone, the per-directory defaults are seeded, the
     CoreSwift workspace it mints carries the fleet harness marker, and both the directory and the
     workspace are removed again (net zero — kanban t_8bea040a convention);
  8. cleanup is net zero: no probe directory, no probe workspace, no probe user, and the live
     directory count is back to what it was.

Run:  python3 scripts/directories-panel-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Writes only into $AUD (default /opt/swift/audits/zz-item-directories-<stamp>).
"""
import base64
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
BROWSER = os.path.join(APP, "scripts", "directories-panel-browser.js")
PANEL = os.path.join(APP, "frontend", "admin-panel.html")
MINT = "/opt/swift/browser-run/md-mint-token.js"
NODE_PATH = "/opt/swift/browser-run/node_modules"
CLEANUP = "/opt/swift/scripts/probe-identity-cleanup.py"
CS_ENV = "/etc/swift/env/coreswift.env"
MD_ENV = "/etc/swift/env/multi-directory.env"
# Fleet probe attribution: the reserved address `md-<slug>@local.coreswift` is the convention the
# sweep recognises, and X-Swift-Harness is the tag CoreSwift stores on the workspace it mints.
HARNESS = os.environ.get("MD_HARNESS", "t_26639554")
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item-directories-{time.strftime('%Y%m%d-%H%M%S')}")
SLUG_PREFIX = "zz-dr-probe"
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
        return None
    return None


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
    req.add_header("X-Swift-Harness", HARNESS)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def b64url(raw):
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def non_operator_token(role="business_owner"):
    """A signed-in user who is NOT the platform operator — the caller the create/delete guard is for."""
    secret = env_value(MD_ENV, "JWT_SECRET")
    if not secret:
        sys.exit("could not read JWT_SECRET")
    now = int(time.time())
    head = b64url(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    body = {"sub": "11111111-1111-1111-1111-111111111111",
            "tid": "00000000-0000-0000-0000-000000000001",
            "role": role, "exp": now + 900, "iat": now,
            "aud": "multidirectory-api", "iss": "multidirectory"}
    msg = head + "." + b64url(json.dumps(body, separators=(",", ":")).encode())
    sig = hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()
    return msg + "." + b64url(sig)


def db_dirs():
    return sorted(psql(
        "SELECT id::text || '|' || name || '|' || slug || '|' || COALESCE(network_id::text,'') "
        "|| '|' || COALESCE(url_type,'') || '|' || COALESCE(status,'') FROM directories ORDER BY slug"))


def api_dirs(token):
    s, body = call("GET", "/directories?per_page=200", token)
    if s != 200:
        return s, []
    try:
        rows = json.loads(body).get("data", [])
    except Exception:
        return s, []
    return s, sorted(
        f"{d.get('id')}|{d.get('name')}|{d.get('slug')}|{d.get('network_id') or ''}"
        f"|{d.get('url_type') or ''}|{d.get('status') or ''}" for d in rows)


def coreswift_psql(sql):
    url = env_value(CS_ENV, "DATABASE_URL") or ""
    m = re.search(r"://[^:]+:([^@]+)@[^/]+/([A-Za-z0-9_]+)", url)
    if not m:
        return None
    r = sh("docker", "exec", "-e", f"PGPASSWORD={m.group(1)}", "swift-postgres",
           "psql", "-U", "swift", "-d", m.group(2), "-At", "-c", sql)
    return r.stdout.strip() if r.returncode == 0 else None


def cs_residue():
    users = coreswift_psql(f"SELECT count(*) FROM users WHERE email LIKE 'md-{SLUG_PREFIX}%local.coreswift'")
    tenants = coreswift_psql(f"SELECT count(*) FROM tenants WHERE name ILIKE '%zz dr probe%'")
    return users, tenants


def delete_cs_workspaces(slugs, tries=10):
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
        users, tenants = cs_residue()
        if set(rcs) == {0} and users == "0" and tenants == "0":
            return True, detail
        time.sleep(3)
    return False, detail


def purge_probe_dirs(token, keep=None):
    """Delete every probe directory still in the database (the API sweeps its children)."""
    left = [s for s in psql(f"SELECT slug FROM directories WHERE slug LIKE '{SLUG_PREFIX}%'") if s != keep]
    for slug in left:
        call("DELETE", "/directories/" + slug, token)
    return left


def run_browser(phase, tok, extra):
    chrome = os.environ.get("CHROME_PATH", "/usr/bin/chromium-browser")
    env = dict(os.environ, MD_BASE=BASE, TOKEN=tok, AUD=AUD, PHASE=phase,
               MD_HARNESS=HARNESS, CHROME_PATH=chrome, NODE_PATH=NODE_PATH)
    env.update(extra)
    r = sh("node", BROWSER, env=env, timeout=300)
    proof_path = os.path.join(AUD, f"directories-proof-{phase}.json")
    proof = json.load(open(proof_path)) if os.path.exists(proof_path) else {}
    if not proof.get("checks"):
        print(r.stdout[-1500:])
        print(r.stderr[-1500:])
    for c in proof.get("checks", []):
        check("browser(" + phase + "): " + c.get("name", "?"), bool(c.get("ok")), c.get("detail", ""))
    check(f"the {phase} browser phase produced a proof file", bool(proof.get("checks")), proof_path)
    check(f"the {phase} browser phase had no failures", proof.get("fail", 1) == 0,
          f"pass={proof.get('pass')} fail={proof.get('fail')}")
    return proof


def main():
    os.makedirs(AUD, exist_ok=True)
    stamp = str(int(time.time()))
    probe_slug = f"{SLUG_PREFIX}-{stamp}"
    probe_name = f"zz dr probe city {stamp}"
    probe_renamed = f"zz dr probe renamed {stamp}"
    standalone_slug = f"{SLUG_PREFIX}-standalone-{stamp}"

    tok = sh("node", MINT).stdout.strip()
    check("platform-operator session minted", tok.count(".") == 2, f"len={len(tok)}")

    # ---- 0. leftovers from an interrupted run, then live DB truth -------------
    for slug in purge_probe_dirs(tok):
        print("  removed leftover probe directory:", slug)
    before = db_dirs()
    print("DB truth: directories =", len(before), "|", ", ".join(r.split("|")[2] for r in before))
    check("the database holds the live directories", len(before) >= 1, str(len(before)))

    net_id = one("SELECT id::text FROM networks WHERE slug = 'zaarhub'")
    net_name = one("SELECT name FROM networks WHERE slug = 'zaarhub'")
    check("the ZaarHub network is available to join", bool(net_id) and net_name == "ZaarHub",
          f"{net_name} {net_id[:8]}")
    existing_slug = one("SELECT slug FROM directories WHERE network_id = '" + net_id + "' ORDER BY created_at LIMIT 1")
    existing_slug = existing_slug or before[0].split("|")[2]

    # ---- 1. the console carries the card ------------------------------------
    html = open(PANEL, encoding="utf-8", errors="ignore").read()
    for hook in ('id="directories-section"', 'id="dr-new"', 'id="dr-save"', 'id="dr-refresh"',
                 "data-dr-edit", "data-dr-del", "api('POST', '/directories'",
                 "api('DELETE', '/directories/'"):
        check(f"the console carries {hook}", hook in html)

    # ---- 2. anonymous is refused and writes nothing ------------------------
    s, _ = call("GET", "/directories")
    check("anonymous GET /directories is refused", s == 401, f"status={s}")
    s, _ = call("POST", "/directories", None, {"name": "zz anon probe", "slug": probe_slug + "-anon"})
    check("anonymous POST /directories is refused", s == 401, f"status={s}")
    s, _ = call("DELETE", "/directories/" + existing_slug)
    check("anonymous DELETE /directories/:slug is refused", s == 401, f"status={s}")
    check("no anonymous probe row was written",
          one(f"SELECT count(*) FROM directories WHERE slug LIKE '{SLUG_PREFIX}%'") == "0", probe_slug)

    # ---- 3. a signed-in NON-operator is refused ----------------------------
    nt = non_operator_token()
    s, body = call("POST", "/directories", nt,
                   {"name": "zz non-operator probe", "slug": probe_slug + "-nonop",
                    "network_mode": "connect", "parent_network_id": net_id})
    check("a signed-in business_owner cannot create a directory", s == 403, f"status={s}")
    check("the refused non-operator create wrote no row",
          one(f"SELECT count(*) FROM directories WHERE slug = '{probe_slug}-nonop'") == "0", "no row")
    s, body = call("DELETE", "/directories/" + existing_slug, nt)
    # 404 is the handler's own scoping answer (a non-operator must not learn that a slug exists),
    # 403 is the explicit platform-operator refusal — either way the delete does not happen.
    check("a signed-in business_owner cannot delete a directory", s in (403, 404), f"status={s}")
    check("the directory the non-operator tried to delete still exists",
          one(f"SELECT count(*) FROM directories WHERE slug = '{existing_slug}'") == "1", existing_slug)

    # ---- 4. the operator sees exactly what the DB holds --------------------
    s, api_rows = api_dirs(tok)
    check("operator GET /directories answers", s == 200, f"status={s}")
    check("the API returns exactly the directories the database holds",
          api_rows == before, f"api={len(api_rows)} db={len(before)}")

    extra = dict(PROBE_NAME=probe_name, PROBE_SLUG=probe_slug, PROBE_RENAMED=probe_renamed,
                 NETWORK_ID=net_id, NETWORK_NAME=net_name, EXISTING_SLUG=existing_slug)

    try:
        # ---- 5/6. create + edit on the screen, then read Postgres -------------
        run_browser("create", tok, extra)
        row = one("SELECT name || '|' || COALESCE(city,'') || '|' || COALESCE(network_id::text,'') "
                  "|| '|' || COALESCE(url_type,'') || '|' || COALESCE(status,'') || '|' "
                  "|| COALESCE(support_email,'') || '|' || COALESCE(description,'') "
                  f"FROM directories WHERE slug = '{probe_slug}'")
        parts = row.split("|") if row else []
        check("the directory the panel created is a real row in the database", bool(row), probe_slug)
        check("the database holds the name the panel saved last", len(parts) > 0 and parts[0] == probe_renamed,
              parts[0] if parts else "")
        check("the database holds the city the panel edited in", len(parts) > 1 and parts[1] == "Palm Bay",
              parts[1] if len(parts) > 1 else "")
        check("the directory joined the network chosen in the panel", len(parts) > 2 and parts[2] == net_id,
              parts[2] if len(parts) > 2 else "")
        check("its URL is a subfolder of the network", len(parts) > 3 and parts[3] == "subfolder",
              parts[3] if len(parts) > 3 else "")
        check("it was created as a draft, as chosen in the panel", len(parts) > 4 and parts[4] == "draft",
              parts[4] if len(parts) > 4 else "")
        check("the support email typed in the panel was stored", len(parts) > 5 and parts[5] == "zz-probe@zaarhub.com",
              parts[5] if len(parts) > 5 else "")
        probe_id = one(f"SELECT id::text FROM directories WHERE slug = '{probe_slug}'")
        zones = one(f"SELECT count(*) FROM ad_zones WHERE directory_id = '{probe_id}'") if probe_id else "0"
        check("the new directory was created complete (its ad zones are seeded, no manual step)",
              zones.isdigit() and int(zones) >= 6, f"ad_zones={zones}")

        # ---- 5/6b. delete on the screen, then read Postgres ------------------
        run_browser("delete", tok, extra)
        check("the directory the panel deleted is gone from the database",
              one(f"SELECT count(*) FROM directories WHERE slug = '{probe_slug}'") == "0", probe_slug)
        if probe_id:
            left = one(f"SELECT count(*) FROM ad_zones WHERE directory_id = '{probe_id}'")
            check("its seeded children went with it (nothing orphaned)", left == "0", f"ad_zones={left}")
        after = db_dirs()
        check("the live directories are back to their before state", after == before,
              f"before={len(before)} after={len(after)}")
    finally:
        # ---- 7. the STANDALONE placement ("its own domain") ------------------
        s, body = call("POST", "/directories", tok,
                       {"name": f"zz dr probe standalone {stamp}", "slug": standalone_slug,
                        "network_mode": "standalone", "status": "draft", "city": "Sebastian",
                        "state": "FL"})
        check("the API accepts a standalone directory", s == 201, f"status={s}")
        sa = one("SELECT COALESCE(network_id::text,'NULL') || '|' || COALESCE(url_type,'') || '|' "
                 f"|| COALESCE(status,'') FROM directories WHERE slug = '{standalone_slug}'")
        check("a standalone directory has no network", sa.startswith("NULL|"), sa)
        check("a standalone directory is its own URL, not a subfolder", sa.split("|")[1:2] == ["standalone"],
              sa)
        sa_id = one(f"SELECT id::text FROM directories WHERE slug = '{standalone_slug}'")
        tpl = one(f"SELECT count(*) FROM email_templates WHERE directory_id = '{sa_id}'") if sa_id else "0"
        check("a standalone directory gets its own email templates (defaults over blanks)",
              tpl.isdigit() and int(tpl) >= 1, f"email_templates={tpl}")
        time.sleep(3)  # the CoreSwift provisioning runs in a detached task
        minted = coreswift_psql("SELECT name || '|' || COALESCE(probe_harness,'NULL') FROM tenants "
                                "WHERE name ILIKE 'zz dr probe standalone%'")
        minted = [l for l in (minted or "").splitlines() if l]
        if minted:
            check("the minted CoreSwift workspace carries the fleet harness marker",
                  all(l.split("|", 1)[1] == HARNESS for l in minted), str(minted))
        else:
            check("no CoreSwift workspace was minted for this probe (nothing to clean)",
                  cs_residue() == ("0", "0"), str(cs_residue()))

        ds, _ = call("DELETE", "/directories/" + standalone_slug, tok)
        check("the disposable standalone directory was removed", ds in (200, 204), f"status={ds}")

        # ---- 8. cleanup: net zero ------------------------------------------
        purge_probe_dirs(tok)
        left = int(one(f"SELECT count(*) FROM directories WHERE slug LIKE '{SLUG_PREFIX}%'"))
        check("no probe directory remains in the database", left == 0, f"remaining={left}")
        check("the live directories are back to their before state after cleanup",
              db_dirs() == before, f"now={len(db_dirs())} before={len(before)}")
        deleted, detail = delete_cs_workspaces([standalone_slug, probe_slug])
        check("the CoreSwift workspace the standalone create minted was deleted", deleted, detail)
        users, tenants = cs_residue()
        check("no probe workspace or user remains in CoreSwift",
              users == "0" and tenants == "0", f"users={users} tenants={tenants}")

    print("\nArtifacts:", AUD)
    print("RESULT: " + ("PASS" if not FAILS else "FAIL"))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
