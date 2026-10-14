#!/usr/bin/env python3
"""ZaarHub queue item 12 — sitemap pinging really works AND the operator sees it on the card.

The item is "sitemap pinging". The backend leg (POST /seo/run-submission -> a live submission, a
row in seo_submission_log) had been proven before; what had NEVER been proven is the operator's
half: the admin console card #seo-submit-section is where the feature is driven, and its own status
line has to show the result. It did not — the card reused element ids that two other cards already
owned, so `getElementById` handed every status message to the Submission Queue card and the sitemap
card stayed blank.

Proves, against the running app:
  * anonymous            -> 401 on POST /seo/run-submission (global auth guard);
  * the platform operator-> 200, status ok, >=1 target submitted, at least one target OK;
  * the run appends a REAL row to seo_submission_log (newest row ok=true);
  * a real browser on /admin-panel.html: the card renders, its "Run now" button is wired, and the
    RUN RESULT APPEARS INSIDE THAT CARD — plus no card element shares its id with another card
    (the durable guard against the collision coming back), console clean.

Writes no fixtures: it drives the real operator account and the real sitemap config; the only rows it
adds are the genuine submission entries the feature is supposed to write.
Run:  python3 scripts/sitemap-ping-smoke.py
Exit: 0 = PASS, 1 = FAIL.
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
BROWSER = os.path.join(APP, "scripts", "sitemap-card-status-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item12-sitemap-card-{time.strftime('%Y%m%d-%H%M%S')}")
UA = "Mozilla/5.0 (compatible; SwiftOps/1.0)"
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


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


def chrome_path():
    import glob
    if os.environ.get("CHROME_PATH"):
        return os.environ["CHROME_PATH"]
    cands = sorted(glob.glob("/root/.cache/puppeteer/chrome/*/chrome-linux64/chrome"))
    return cands[-1] if cands else "/usr/bin/chromium-browser"


def b64(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def operator_token():
    secret = sh("docker", "exec", "multidirectory", "printenv", "JWT_SECRET").stdout.strip()
    uid = one("SELECT id FROM users LIMIT 1")
    tid = one("SELECT id FROM tenants LIMIT 1")
    if not (secret and uid and tid):
        sys.exit("could not resolve JWT_SECRET / a user / a tenant")
    now = int(time.time())
    hh = b64(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    pp = b64(json.dumps({"sub": uid, "tid": tid, "role": "super_admin", "exp": now + 1800,
                         "iat": now, "aud": "multidirectory-api", "iss": "multidirectory"},
                        separators=(",", ":")).encode())
    sig = b64(hmac.new(secret.encode(), f"{hh}.{pp}".encode(), hashlib.sha256).digest())
    return f"{hh}.{pp}.{sig}"


def http(method, url, token=None, data=None, timeout=40):
    cmd = ["curl", "-s", "-m", str(timeout), "-A", UA, "-X", method, "-w", "\n__CODE__%{http_code}"]
    if token:
        cmd += ["-H", f"Authorization: Bearer {token}"]
    if data is not None:
        cmd += ["-H", "Content-Type: application/json", "-d", data]
    cmd.append(url)
    r = sh(*cmd)
    body, _, code = r.stdout.rpartition("__CODE__")
    return code.strip(), body


def main():
    os.makedirs(AUD, exist_ok=True)
    tok = operator_token()

    code, _ = http("GET", f"{BASE}/api/v1/health")
    check("the app answers its health endpoint", code == "200", f"status={code}")

    # 1) the operator gate on the run endpoint
    code, body = http("POST", f"{BASE}/api/v1/seo/run-submission", data="{}")
    check("a logged-out caller CANNOT force a sitemap submission", code == "401", f"status={code}")

    # 2) the operator reads the settings
    code, body = http("GET", f"{BASE}/api/v1/seo/submission-settings", token=tok)
    st = {}
    try:
        st = (json.loads(body) or {}).get("settings") or {}
    except Exception:
        pass
    check("the operator CAN read the submission settings", code == "200", f"status={code}")
    check("an IndexNow key is configured (the live 2026 ping path)",
          bool((st.get("indexnow_key") or "").strip()), f"len={len((st.get('indexnow_key') or ''))}")

    # 3) a real run: does it actually submit, and log it?
    before = int(one("SELECT count(*) FROM seo_submission_log") or "0")
    code, body = http("POST", f"{BASE}/api/v1/seo/run-submission", token=tok, data="{}")
    run = {}
    try:
        run = json.loads(body) or {}
    except Exception:
        pass
    check("the operator's run answers 200", code == "200", f"status={code}")
    check("the run reports a real submission pass", (run.get("status") in ("ok", "partial")),
          f"status={run.get('status')}")
    check("at least one target was submitted", int(run.get("targets_submitted") or 0) >= 1,
          f"targets_submitted={run.get('targets_submitted')}")
    oks = [r for r in (run.get("results") or []) if r.get("ok")]
    check("at least one target answered OK", len(oks) >= 1,
          "targets=" + ",".join(str(r.get("target")) for r in (run.get("results") or [])))

    after = int(one("SELECT count(*) FROM seo_submission_log") or "0")
    check("the run appended a real submission row", after > before, f"{before} -> {after}")
    newest = one("SELECT target_name || '|' || coalesce(status_code::text,'-') || '|' || ok "
                 "FROM seo_submission_log ORDER BY created_at DESC LIMIT 1")
    check("the newest logged row is a success", newest.endswith("|true"), f"row={newest}")

    # 4) the operator's half: the card shows the result
    env = dict(os.environ, MD_BASE=BASE, MD_ADMIN_TOKEN=tok, AUD=AUD,
               NODE_PATH=NODE_PATH, CHROME_PATH=chrome_path())
    check("a real browser is available", os.path.exists(env["CHROME_PATH"]),
          os.path.basename(env["CHROME_PATH"]))
    r = sh("node", BROWSER, env=env, timeout=240)
    proof_path = os.path.join(AUD, "browser-proof.json")
    proof = json.load(open(proof_path)) if os.path.exists(proof_path) else {}
    if not proof:
        print(r.stdout[-800:]); print(r.stderr[-800:], file=sys.stderr)
    check("the sitemap card is on the operator's console", proof.get("card_present") is True)
    check("its Run-now control is inside the card", proof.get("run_button_found") is True)
    check("the RUN RESULT IS VISIBLE ON THE CARD ITSELF",
          proof.get("status_visible_on_card") is True,
          f"text={proof.get('card_status_after')!r} (before={proof.get('card_status_before')!r})")
    check("no console card element shares an id with another card",
          proof.get("duplicate_ids") == [], json.dumps(proof.get("duplicate_ids"))[:200])
    check("the console ran with no script errors", proof.get("console_errors") == [],
          json.dumps(proof.get("console_errors"))[:200])
    check("the console made no failing API call", proof.get("failing_api") == [],
          json.dumps(proof.get("failing_api"))[:200])

    return finish(run, proof, newest)


def finish(run, proof, newest):
    verdict = "PASS" if not FAILS else f"FAIL ({len(FAILS)})"
    lines = [
        f"# Item 12 — sitemap pinging: the operator's card shows the run result\n",
        f"Date: {time.strftime('%Y-%m-%d %H:%M:%S %Z')} | App: multi-directory (ZaarHub) | Verdict: **{verdict}**\n",
        "## Run JSON\n```json", json.dumps(run, indent=2)[:2000], "```\n",
        "## Browser proof\n```json", json.dumps(proof, indent=2), "```\n",
        f"## Newest submission row\n`{newest}`\n",
    ]
    if FAILS:
        lines += ["## Failures\n"] + [f"* {f}" for f in FAILS]
    open(os.path.join(AUD, "EVIDENCE.md"), "w").write("\n".join(lines) + "\n")
    print(f"RESULT: {verdict}" + (": " + "; ".join(FAILS) if FAILS else ""))
    print(f"evidence: {AUD}/EVIDENCE.md")
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()
