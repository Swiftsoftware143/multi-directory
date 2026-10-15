#!/usr/bin/env python3
"""Durable proof — a deal's FINE PRINT is set by the business owner from the screen and read by
a shopper on the public deal page.

The defect this card closes (measured 2026-10-15): `deals.fine_print` was in the schema, was
returned by `GET /api/v1/deals/:id` and `/:id/page`, and the storefront renders it in a dedicated
"Fine Print" tab (frontend/index.html `renderDealDetail`) — but the owner portal had no control for
it AND `POST/PUT /api/v1/deals` accepted `fine_print` and never wrote it (the INSERT/UPDATE column
lists omitted the column; the request field was dead in both handlers). So the tab could only ever
show the Terms text, and the field was unreachable from every screen.

Proves, against the running app and the live database:
  1. the owner has a VISIBLE Fine Print control on the portal's deal form;
  2. clicking the form's own Save Deal persists it (INSERT) — asserted in the database;
  3. a partial PUT (the field present) persists an edit (UPDATE) — asserted in the database;

Run:  python3 scripts/deal-fine-print-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089), AUD, CHROME_PATH, NODE_PATH
Exit: 0 = PASS, 1 = FAIL.  Every probe row (deal, business, claim, account) is removed in teardown.
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
import urllib.error
import urllib.parse
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "deal-fine-print-browser.js")
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-deal-fineprint-{time.strftime('%Y%m%d-%H%M%S')}")
HARNESS = {"X-Swift-Harness": "deal-fineprint-probe"}
MARK = "deal-fineprint-probe"
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


ENV_FILE = os.path.join(APP, ".env")


def mint_operator_token():
    """super_admin JWT, the shape the admin panel uses. Secret never printed."""
    secret = None
    for line in open(ENV_FILE):
        s = line.strip()
        if s.startswith("JWT_SECRET="):
            secret = s.split("=", 1)[1].strip().strip('"')
    if not secret:
        sys.exit("JWT_SECRET not found in " + ENV_FILE)
    b64 = lambda o: base64.urlsafe_b64encode(json.dumps(o).encode()).decode().rstrip("=")
    now = int(time.time())
    msg = b64({"alg": "HS256", "typ": "JWT"}) + "." + b64({
        "sub": "00000000-0000-0000-0000-000000000002",
        "tid": "00000000-0000-0000-0000-000000000001",
        "role": "super_admin", "exp": now + 1200, "iat": now,
        "aud": "multidirectory-api", "iss": "multidirectory"})
    return msg + "." + base64.urlsafe_b64encode(
        hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()).decode().rstrip("=")


PW_DB = db_password()
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT)\b")


def psql(sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW_DB}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    return lines[0] if lines else ""


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def http(method, path, body=None, token=None, extra_headers=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    for k, v in (extra_headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            raw = resp.read().decode()
            try:
                return resp.status, json.loads(raw)
            except json.JSONDecodeError:
                return resp.status, raw
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw)
        except json.JSONDecodeError:
            return e.code, raw


def chrome_path():
    import glob
    if os.environ.get("CHROME_PATH"):
        return os.environ["CHROME_PATH"]
    cands = sorted(glob.glob("/root/.cache/puppeteer/chrome/*/chrome-linux64/chrome"))
    return cands[-1] if cands else "/usr/bin/chromium-browser"


def sweep_stale_probe_rows():
    """A run that died before teardown must not leave junk behind."""
    psql("DELETE FROM deals WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-deal-fineprint-%@swiftsoftware.dev'")


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep_stale_probe_rows()
    ts = str(int(time.time()))
    owner_email = f"zz-deal-fineprint-owner-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    title = f"ZZ Fine Print Probe Deal {ts}"
    fine = f"Fine print proof {ts}: valid for dine-in only, one per customer, not combinable."
    edited = fine + " EDITED: expiring soon."

    code = sh("curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
              BASE + "/api/v1/health").stdout
    check("the app answers /api/v1/health", code == "200", f"status={code}")

    # ── fixture: one owner account owning one disposable listing ───────────────
    st, _ = http("POST", "/api/v1/visitor/register",
                 {"email": owner_email, "password": pw, "name": "ZZ Fine Print Owner"},
                 extra_headers=HARNESS)
    check("owner account can be created (POST /visitor/register)", st in (200, 201), f"status={st}")

    owner_vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{owner_email}'")
    check("the owner account exists in the database", bool(owner_vid))

    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    slug = f"zz-deal-fineprint-probe-{ts}"
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Fine Print Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{MARK}') RETURNING id")
    check("disposable listing created", bool(bid))

    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ Fine Print Owner",
                  "owner_phone": "4075550100"}, extra_headers=HARNESS)
    check("the owner can claim the listing (POST /businesses/:id/claim)", st in (200, 201), f"status={st}")

    # ── the browser arm: the owner sets fine print FROM THE SCREEN ─────────────
    deal_out = os.path.join(AUD, "deal-id.txt")
    env = dict(os.environ, PORTAL_URL=BASE + "/portal", PUBLIC_URL=BASE, OWNER_EMAIL=owner_email,
               PW=pw, FINE_PRINT=fine, TITLE=title, SHOT_DIR=AUD, DEAL_OUT=deal_out,
               CHROME_PATH=chrome_path(), NODE_PATH=NODE_PATH)
    r = sh("node", BROWSER, env=env)
    with open(os.path.join(AUD, "browser.log"), "w") as fh:
        fh.write(r.stdout + r.stderr)
    print(r.stdout.strip())
    if r.stderr.strip():
        print("  (stderr) " + r.stderr.strip()[:400])
    check("the browser flow passed (owner typed + saved fine print on the portal)", r.returncode == 0,
          f"rc={r.returncode}")

    did = psql(f"SELECT id FROM deals WHERE title='{title}' ORDER BY created_at DESC LIMIT 1")
    check("the deal the owner saved exists in the database", bool(did), f"id={did[:8] if did else '-'}")

    # A portal-created deal must land on its listing's directory, or its public page 404s
    # (GET /deals/:id/page INNER JOINs directories) and no city page can show it.
    dir_ok = psql(f"SELECT (d.directory_id IS NOT NULL)::text || '|' || (d.directory_id = b.directory_id)::text "
                  f"FROM deals d JOIN businesses b ON b.id = d.business_id WHERE d.id='{did}'")
    check("the owner-created deal inherits the listing's directory", dir_ok == "true|true", f"got={dir_ok}")

    # ── the write path really stored it (this is what used to be dropped) ──────
    got = psql(f"SELECT coalesce(fine_print,'<null>') FROM deals WHERE id='{did}'")
    check("the database row carries the fine print the owner typed", got == fine,
          f"len={len(got)}")

    st, page = http("GET", f"/api/v1/deals/{did}/page")
    hp = page.get("fine_print") if isinstance(page, dict) else None
    check("the public deal-page API returns the fine print (logged out)", st == 200 and hp == fine,
          f"status={st} match={hp == fine}")

    st, one = http("GET", f"/api/v1/deals/{did}")
    op = one.get("fine_print") if isinstance(one, dict) else None
    check("GET /deals/:id returns the fine print (logged out)", st == 200 and op == fine,
          f"status={st} match={op == fine}")

    # ── the edit path (PUT) must persist it too ────────────────────────────────
    # Through the admin console's identity: assert_deal_admin authorises a platform operator, and
    # the console's deal editor uses the same UPDATE. (A claim made by email alone carries no
    # user_id, so the visitor token cannot satisfy that guard — measured, noted in EVIDENCE.)
    op_token = mint_operator_token()
    st, _ = http("PUT", f"/api/v1/deals/{did}", {"fine_print": edited}, token=op_token)
    check("the operator can edit the fine print (PUT /deals/:id)", st in (200, 201), f"status={st}")
    got2 = psql(f"SELECT coalesce(fine_print,'<null>') FROM deals WHERE id='{did}'")
    check("the edit is stored (the UPDATE path no longer drops it)", got2 == edited, f"len={len(got2)}")

    # ── teardown ───────────────────────────────────────────────────────────────
    psql(f"DELETE FROM deals WHERE id='{did}'")
    psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    if owner_vid:
        psql(f"DELETE FROM visitor_accounts WHERE id='{owner_vid}'")
    left = psql("SELECT (SELECT count(*) FROM deals WHERE business_id='%s')"
                " + (SELECT count(*) FROM businesses WHERE id='%s')"
                " + (SELECT count(*) FROM claimed_businesses WHERE business_id='%s')"
                " + (SELECT count(*) FROM visitor_accounts WHERE lower(email)='%s')"
                % (bid, bid, bid, owner_email))
    check("probe data fully removed", left == "0", f"remaining={left}")

    print()
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)}): " + "; ".join(FAILS))
        return 1
    print("RESULT: PASS")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # noqa: BLE001
        print(f"RESULT: FAIL (exception: {e})")
        sys.exit(1)
