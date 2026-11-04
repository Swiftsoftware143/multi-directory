#!/usr/bin/env python3
"""
Item 8 + item 9 — DURABLE driver for the admin "API Access & Webhooks" screen and the
operator Dashboard tiles, proven in a REAL browser.

Why this exists: `scripts/admin-dashboard-webhooks-browser.js` was written (commit 98f83c9) but
NOTHING invoked it. An orphaned proof is not a proof — it cannot be re-run, so a later regression
would go unnoticed, and the crash it actually had (a missing screenshot directory aborting the run
before the webhooks half was ever reached) stayed invisible.

This driver:
  1. runs the browser proof against the live console as a real operator session;
  2. fails unless the browser arm itself exits 0 AND reports every check passed;
  3. independently asserts the probe left NO residue in `webhooks`;
  4. keeps the screenshots + JSON evidence under /opt/swift/audits/<run>/.

What the browser arm proves on screen (not by grepping source):
  item 9 — every Dashboard tile shows a REAL number equal to the live DB count (never "—").
  item 8 — the Webhooks screen renders its controls; a webhook can be REGISTERED through the
           screen and lands in the DB; the signing secret is stored but never echoed back or
           rendered; firing a real directory event delivers a signed POST; the Deliveries button
           shows that delivery on screen; deleting through the screen removes the row.

Run:  python3 scripts/admin-dashboard-webhooks-smoke.py
Exit: 0 = all checks passed, 1 = something failed.
"""
import json
import os
import shutil
import subprocess
import sys
import time

APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "admin-dashboard-webhooks-browser.js")
BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
AUD = os.environ.get(
    "AUD", f"/opt/swift/audits/zz-item8-item9-webhooks-{time.strftime('%Y%m%d-%H%M%S')}"
)

fails = 0


def check(name, ok, detail=None):
    global fails
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail is not None else ""))
    if not ok:
        fails += 1


def find_chrome():
    for cand in ("google-chrome", "chromium", "chromium-browser"):
        p = shutil.which(cand)
        if p:
            return p
    import glob
    for p in glob.glob("/root/.cache/puppeteer/chrome/*/chrome-linux*/chrome"):
        return p
    return ""


def psql(sql):
    pw = subprocess.run(
        ["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
        capture_output=True, text=True,
    ).stdout.strip()
    password = pw.split("://", 1)[1].split(":", 1)[1].split("@", 1)[0]
    env = dict(os.environ, PGPASSWORD=password)
    r = subprocess.run(
        ["psql", "-h", "127.0.0.1", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql],
        capture_output=True, text=True, env=env,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip() or "psql failed")
    return r.stdout.strip()


def main():
    os.makedirs(AUD, exist_ok=True)

    chrome = find_chrome()
    check("a Chromium browser is available for the screen proof", bool(chrome), chrome or "none")
    if not chrome:
        return finish()

    # The browser arm must be re-runnable from scratch: a fresh screenshot dir is exactly the
    # condition that used to abort it, so prove the driver works with one that does not exist yet.
    shots = os.path.join(AUD, "screens")
    shutil.rmtree(shots, ignore_errors=True)
    if os.path.isdir(shots):
        raise SystemExit("could not clear the screenshot directory")

    env = dict(
        os.environ,
        NODE_PATH=NODE_PATH,
        CHROME_PATH=chrome,
        ADMIN_URL=f"{BASE}/admin-panel.html",
        MD_BASE=BASE,
        SHOT_DIR=shots,
    )
    r = subprocess.run(["node", BROWSER], capture_output=True, text=True, env=env, cwd=APP)

    out = (r.stdout or "") + (r.stderr or "")
    for line in out.splitlines():
        s = line.strip()
        if s.startswith("[PASS]") or s.startswith("[FAIL]") or s.startswith("BROWSER"):
            print("  " + s)

    n_pass = out.count("[PASS]")
    n_fail = out.count("[FAIL]")
    check("the browser proof ran and reported checks", n_pass > 0, f"pass={n_pass} fail={n_fail}")
    check("no browser check failed", n_fail == 0, f"fail={n_fail}")
    check(
        "the browser arm reported every check passed",
        "BROWSER RESULT: all passed" in out,
        (out.strip().splitlines() or ["<no output>"])[-1][:120],
    )
    check("the browser arm exited cleanly", r.returncode == 0, f"rc={r.returncode}")

    # The proof is only meaningful if it left nothing behind.
    try:
        residue = int(psql("SELECT COUNT(*) FROM webhooks WHERE url LIKE '%zz-wh-screen-%'") or "0")
        check("the proof left no webhook residue", residue == 0, f"rows={residue}")
    except Exception as e:  # noqa: BLE001
        check("the proof left no webhook residue", False, str(e)[:120])

    evidence = os.path.join(shots, "dashboard-webhooks-proof.json")
    if os.path.exists(evidence):
        with open(evidence) as fh:
            data = json.load(fh)
        check("the screen registered a webhook that reached the database", data.get("db_row") == 1,
              f"db_row={data.get('db_row')}")
        check("a real event was DELIVERED to the registered webhook", data.get("event_status") == 200
              or data.get("receiver_hits") == 1,
              f"status={data.get('event_status')} hits={data.get('receiver_hits')}")
        check("the delivery is visible ON SCREEN", bool(data.get("deliveries_on_screen")))
        check("deleting through the screen removed the row", data.get("row_after_delete") == 0)
        check("the signing secret was stored but never rendered back", bool(data.get("secret_encrypted")))
    else:
        check("the browser arm wrote its evidence file", False, evidence)

    print(f"\nEvidence: {AUD}")
    return finish()


def finish():
    print("\nRESULT: " + ("PASS" if fails == 0 else f"FAIL ({fails} failed)"))
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
