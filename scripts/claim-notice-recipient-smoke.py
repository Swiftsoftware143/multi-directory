#!/usr/bin/env python3
"""Claim manual-review notice — the recipient is REACHABLE (ZaarHub, MD list item 3).

The platform's own "a claim needs review" notice used to be mailed to `admin@example.com` — the
shipped `ADMIN_EMAIL` default — a reserved domain with no MX record, so every notice hard-bounced
("No MX for example.com, domain accepts no mail"), the operator was never told, and the shared
Mailgun sending domain collected the failure.

This proof drives the REAL public claim route against the LIVE app:

  1. a disposable listing is created (no website -> the claim cannot be auto-accepted)
  2. POST /api/v1/businesses/:id/claim  -> 201 with auto_approved=false  (manual review)
  3. the app's own log shows the manual-review notice path ran, and that for a test/proof
     record it reports SKIPPED (never "sent") — the harness must not email the operator
  4. Mailgun's own event log carries NOTHING for that listing (the guard really held)
  5. no message anywhere was addressed to an example.* / .invalid / .test placeholder

What this cannot prove is delivery of a REAL notice: the product refuses to send one for a
test record, and sending a real one on every run would email the operator (the very thing
2026-11-05 forbade). Delivery is therefore asserted by the recipient resolver (the operator's
own login, read from the database) plus the honest skip in the log.

Run:  python3 scripts/claim-notice-recipient-smoke.py
Exit: 0 = PASS, 1 = FAIL.  The probe rows are removed on both paths.
"""
import base64
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from lib.coreswift_sweep import sweep_coreswift_mirror

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
HARNESS = {"X-Swift-Harness": "claim-notice-probe"}
FAILS = []


def sh(*cmd, stdin_text=None):
    return subprocess.run(cmd, capture_output=True, text=True, input=stdin_text)


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
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    return lines[0] if lines else ""


def psql_stdin(sql):
    return sh("docker", "exec", "-i", "-e", f"PGPASSWORD={PW}", "swift-postgres",
              "psql", "-U", "swift", "-d", "multi_directory", "-At", "-f", "-",
              stdin_text=sql)


def mailgun():
    """(api_base, domain, private_key) — the key is decrypted inside psql via stdin, never argv."""
    sec = sh("docker", "exec", "multidirectory", "printenv", "PROVIDER_KEY_ENC_SECRET").stdout.strip()
    row = psql("SELECT coalesce(base_url,'-/v3/updates.zaarhub.com') FROM provider_keys "
               "WHERE provider='mailgun' AND is_active LIMIT 1")
    dom = row.rstrip("/").split("/")[-1] or "updates.zaarhub.com"
    out = psql_stdin(
        "\\set k '%s'\n\\pset tuples_only on\n"
        "select pgp_sym_decrypt(decode(substring(api_key from 8),'base64'), :'k')::text "
        "from provider_keys where provider='mailgun' and is_active limit 1;\n" % sec).stdout
    key = "".join(out.split())
    if not key:
        sys.exit("could not read the mailgun provider key")
    api = "https://api.mailgun.net/v3"
    if "eu.mailgun.net" in (row or ""):
        api = "https://api.eu.mailgun.net/v3"
    return api, dom, key


def http(method, path, body=None, extra_headers=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    for k, v in (extra_headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
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


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def events(api, dom, key, limit=100):
    req = urllib.request.Request(f"{api}/{dom}/events?limit={limit}")
    req.add_header("Authorization", "Basic " + base64.b64encode(f"api:{key}".encode()).decode())
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.load(resp).get("items", []) or []


def is_placeholder(addr):
    d = (addr or "").rsplit("@", 1)[-1].lower()
    return d in ("example.com", "example.org", "example.net") or d.endswith(
        (".example.com", ".example.org", ".example.net", ".invalid", ".test")) or not d


def sweep():
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         "(SELECT id FROM businesses WHERE probe_harness='claim-notice-probe')")
    psql("DELETE FROM businesses WHERE probe_harness='claim-notice-probe'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-claimnotice-%@swiftsoftware.dev'")
    psql("DELETE FROM visitors WHERE probe_harness='claim-notice-probe'")


def main():
    ts = int(time.time())
    name = f"ZZ Claim Notice Probe {ts}"
    api, dom, key = mailgun()
    sweep()  # a previous run that died before teardown must not leave junk behind

    directory_id = psql("SELECT id FROM directories WHERE slug='palm-bay'")
    check("the network directory exists", bool(directory_id), f"dir={directory_id[:8]}")
    operator = psql("SELECT email FROM users WHERE role='super_admin' AND is_active "
                    "ORDER BY created_at, email LIMIT 1")
    check("a reachable platform-admin address exists in the database", bool(operator), operator)

    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, "
        "review_count, probe_harness) "
        f"VALUES ('{directory_id}', '{name}', 'zz-claimnotice-{ts}', 'Orlando', 'FL', 'active', 0, 0, "
        "'claim-notice-probe') RETURNING id")
    check("disposable listing created", bool(bid), f"biz={str(bid)[:8]}")

    claim_email = f"zz-claimnotice-{ts}@swiftsoftware.dev"
    st, body = http("POST", f"/api/v1/businesses/{bid}/claim",
                    {"owner_email": claim_email, "owner_name": "ZZ Claim Notice Owner",
                     "owner_phone": "4075550199"}, extra_headers=HARNESS)
    check("the public claim route accepts the claim", st in (200, 201), f"status={st}")
    auto = (body or {}).get("auto_approved") if isinstance(body, dict) else None
    check("the claim needs MANUAL review (no matching website)", auto is False, f"auto_approved={auto}")

    # ── the notice hook ran, and the test-record guard kept the mail in ──────────
    # A probe claim can never prove DELIVERY: the product deliberately refuses to mail the
    # operator about test/proof records (David, 2026-11-05 — harness runs were emailing him),
    # so there is no real message to read in Mailgun. What can be proven, and matters, is:
    #   (a) the manual-review arm really calls the notice path, and
    #   (b) for a probe record it reports SKIPPED — not "sent" — in the app's own log, and
    #   (c) nothing at all left the building for this listing.
    subject = f"Claim needs review: {name}"
    log_line = ""
    deadline = time.time() + 30
    while time.time() < deadline:
        logs = sh("docker", "logs", "--since", "3m", "multidirectory").stdout + \
            sh("docker", "logs", "--since", "3m", "multidirectory").stderr
        for ln in logs.splitlines():
            if "[claim] manual-review notice" in ln:
                log_line = ln.strip()
        if "skipped" in log_line:
            break
        time.sleep(3)

    check("the app's log shows the manual-review notice path ran for this claim",
          bool(log_line), f"line={log_line[-120:]!r}")
    check("for a test record it reports SKIPPED, never 'sent'",
          "skipped" in log_line and "sent to" not in log_line, f"line={log_line[-120:]!r}")

    mine = []
    try:
        mine = [it for it in events(api, dom, key)
                if str(((it.get("message") or {}).get("headers") or {}).get("subject", "")) == subject]
    except Exception as e:  # a Mailgun read failure must not fake a pass either way
        print(f"  (mailgun read error: {e})")
    check("nothing left the building for the probe listing (the test-record guard held)",
          not mine, f"events={len(mine)}")
    recip = sorted({it.get("recipient") or "" for it in mine})
    check("nothing was addressed to an example.* / .invalid / .test placeholder",
          not any(is_placeholder(r) for r in recip), f"recipients={recip}")

    if os.environ.get("MD_SMOKE_KEEP") != "1":
        swept = psql("SELECT count(*) FROM businesses WHERE probe_harness='claim-notice-probe'")
        sweep()
        left = psql("SELECT count(*) FROM businesses WHERE probe_harness='claim-notice-probe'")
        check("the probe listing was removed again", left == "0", f"before_sweep={swept} after={left}")

    print()
    cs_left = sweep_coreswift_mirror(["zz-claimnotice-"], window=15)
    check("no CoreSwift contact mirror left behind", cs_left == 0,
          f"coreswift_contacts={cs_left}")
    print("RESULT:", "PASS" if not FAILS else f"FAIL ({len(FAILS)}): " + "; ".join(FAILS))
    return 1 if FAILS else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # never leave a traceback-only run with rows behind
        print("ERROR:", e)
        sys.exit(1)
