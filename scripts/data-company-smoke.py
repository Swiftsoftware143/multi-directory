#!/usr/bin/env python3
"""Data Company tab — live end-to-end proof (ZaarHub queue item 14).

The admin console's "Data Company" card surfaces the `data_company.rs` toolbelt that had NO
operator screen at all: Google Places autocomplete + details, Yelp lookup, the business-verification
queue, and bulk export. This script proves, against the RUNNING app and a real browser, that the
card is a working screen — not a section that merely exists in the HTML.

API arm (real HTTP, live DB):
  1. anonymous GET /verifications -> 401 (operator-guarded);
  2. operator GET /verifications -> 200 and returns EXACTLY the rows the database holds;
  3. GET /enrich/logs -> 200, an array;
  4. GET /export/bulk?entity_type=businesses&format=json -> 200, a count and a data array;
  5. GET /export/bulk?entity_type=reviews&format=csv -> 200, a real header row;
  6. GET /yelp/search -> answers HONESTLY: either real results, or a plain "not configured" refusal
     (never a crash / never fake data);
  7. GET /places/autocomplete -> 200 with results; GET /places/details -> ONE place object carrying
     a name and an address (the historical bug returned the autocomplete ARRAY);
  8. PUT /verifications/:id with a bogus status -> refused (422) and the row is left unchanged.

UI arm (real Chromium, the page's own controls): `scripts/data-company-browser.js`.

Run:  python3 scripts/data-company-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089), AUD, CHROME_PATH
Exit: 0 = PASS, 1 = FAIL.  Read-only: no probe rows are created.
"""
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
BROWSER = os.path.join(APP, "scripts", "data-company-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"
MINT = "/opt/swift/browser-run/md-mint-token.js"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item14-datacompany-smoke-{time.strftime('%Y%m%d-%H%M%S')}")
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


def call(method, path, token=None, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + "/api/v1" + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read().decode()
            try:
                return r.status, json.loads(raw), raw
            except json.JSONDecodeError:
                return r.status, None, raw
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw), raw
        except json.JSONDecodeError:
            return e.code, None, raw


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def main():
    os.makedirs(AUD, exist_ok=True)

    db_verifs = int(one("SELECT count(*) FROM business_verifications") or 0)
    any_row = one("SELECT id FROM business_verifications ORDER BY created_at DESC NULLS LAST LIMIT 1")
    any_status = one(f"SELECT status FROM business_verifications WHERE id='{any_row}'") if any_row else ""
    print(f"DB truth: business_verifications = {db_verifs} | sample row status = {any_status}")

    # ── anonymous refused ───────────────────────────────────────────────────────
    s, _, _ = call("GET", "/verifications")
    check("anonymous GET /verifications is refused", s == 401, f"status={s}")

    tok = sh("node", MINT).stdout.strip()
    check("platform-operator session minted", tok.count(".") == 2, f"len={len(tok)}")

    # ── the verification queue matches the database ─────────────────────────────
    s, body, _ = call("GET", "/verifications", tok)
    check("operator GET /verifications answers", s == 200, f"status={s}")
    api_n = len(body) if isinstance(body, list) else None
    check("the queue returns exactly the rows the database holds",
          api_n == db_verifs, f"api={api_n} db={db_verifs}")

    s, body, _ = call("GET", "/enrich/logs", tok)
    check("enrichment log answers with a list", s == 200 and isinstance(body, list), f"status={s}")

    # ── bulk export (JSON + CSV) ────────────────────────────────────────────────
    s, body, _ = call("GET", "/export/bulk?entity_type=businesses&format=json", tok)
    ok = s == 200 and isinstance(body, dict) and isinstance(body.get("data"), list) and isinstance(body.get("count"), int)
    check("bulk export (businesses, JSON) returns a count and a data array", ok,
          f"status={s} count={(body or {}).get('count') if isinstance(body, dict) else None}")

    s, body, _ = call("GET", "/export/bulk?entity_type=reviews&format=csv", tok)
    d = body.get("data") if isinstance(body, dict) else None
    ok = s == 200 and isinstance(d, list) and bool(d) and isinstance(d[0], str) and "," in d[0]
    check("bulk export (reviews, CSV) returns a real header row", ok,
          f"status={s} first={(d or [''])[0][:60] if isinstance(d, list) and d else ''}")

    # ── Yelp: answers honestly (results, or a plain not-configured refusal) ─────
    s, body, raw = call("GET", "/yelp/search?term=coffee&location=Orlando", tok)
    msg = str((body or {}).get("message") or raw or "")
    honest = s == 200 or (s in (400, 422, 503) and "configur" in msg.lower())
    check("the Yelp lookup answers honestly (results or a plain not-configured refusal)",
          honest, f"status={s} msg={msg[:60]}")

    # ── Google Places: results + ONE detail object ──────────────────────────────
    s, body, _ = call("GET", "/places/autocomplete?input=coffee%20miami", tok)
    results = body.get("results") if isinstance(body, dict) else None
    check("Google Places autocomplete returns results", s == 200 and isinstance(results, list) and len(results) > 0,
          f"status={s} results={len(results) if isinstance(results, list) else None}")

    pid = (results[0].get("place_id") if isinstance(results, list) and results else "") or ""
    s, body, _ = call("GET", "/places/details?place_id=" + urllib.parse.quote(pid), tok)
    place = body.get("place") if isinstance(body, dict) else None
    one_place = isinstance(place, dict) and not isinstance(place, list)
    check("Places details returns ONE place object (not the autocomplete array)",
          s == 200 and one_place, f"status={s} place_type={type(place).__name__}")
    pl = place if isinstance(place, dict) else {}
    check("the place detail carries a name and an address",
          bool(pl.get("name")) and bool(pl.get("formatted_address")),
          f"name={pl.get('name')}")

    # ── the update route validates and writes nothing on a bad value ────────────
    if any_row:
        s, _, _ = call("PUT", f"/verifications/{any_row}", tok, {"status": "bogus"})
        check("a bogus verification status is refused", s == 422, f"status={s}")
        after = one(f"SELECT status FROM business_verifications WHERE id='{any_row}'")
        check("the refused update left the row unchanged", after == any_status,
              f"before={any_status} after={after}")

    # ── the screen itself (real Chromium, the page's own controls) ──────────────
    chrome = os.environ.get("CHROME_PATH", "/usr/bin/chromium-browser")
    env = dict(os.environ, MD_BASE=BASE, TOKEN=tok, AUD=AUD, NODE_PATH=NODE_PATH, CHROME_PATH=chrome)
    r = sh("node", BROWSER, env=env, timeout=240)
    proof_path = os.path.join(AUD, "dc-ui-proof.json")
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
    check("the console walk had no failures (exit 0)", r.returncode == 0,
          f"rc={r.returncode} pass={proof.get('pass')} fail={proof.get('fail')}")

    print()
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)} failed)")
        for f in FAILS:
            print("  - " + f)
        sys.exit(1)
    print("RESULT: PASS")
    sys.exit(0)


if __name__ == "__main__":
    main()
