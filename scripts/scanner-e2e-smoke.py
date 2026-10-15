#!/usr/bin/env python3
"""ZaarHub scanner — end-to-end proof (queue item 2: camera, QR read, amount, points, recorded).

Drives the REAL scanner page in real headless Chromium and proves the whole chain:

  camera    Chromium's camera pipeline is fed a video of the probe card
            (--use-fake-device-for-media-stream + --use-file-for-fake-video-capture), so the
            page's getUserMedia() arm runs and delivers frames (videoWidth > 0).
  QR read   the card is rendered by the app's OWN generator (/qrcode.min.js — the same library a
            member's wallet uses) and read back by the page's OWN decoder (jsQR), so the decode
            is not stubbed: the payload stored by handleQRCode() is compared byte-for-byte.
  modes     all THREE counter modes are driven from the page's own segment buttons and Process
            button: a PURCHASE crediting a real amount, a CHECK-IN, and a REDEMPTION debiting
            units off the balance. Each is submitted with the page's own fresh receipt id.
  amount    the purchase amount (and the redemption's units) are typed into the page.
  points    each success overlay shows the resulting units.
  recorded  every mode is verified in the database: the member balance, the activity ledger
            (purchase / checkin / redemption), the loyalty_checkins row, and one loyalty_scans
            ledger row per mode carrying the amount, the resulting balance and the business.

Run:  python3 scripts/scanner-e2e-smoke.py
Exit: 0 = PASS, 1 = FAIL.  The probe fixture is disposable and torn down either way.
"""
import glob
import json
import math
import os
import re
import subprocess
import sys
import time

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "scanner-e2e-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"
MINT = "/opt/swift/browser-run/md-mint-token.js"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-scanner-e2e-{time.strftime('%Y%m%d-%H%M%S')}")
AMOUNT = "25.00"
REDEEM = 10
HARNESS = "scanner-e2e-probe"
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


def chrome_path():
    """A browser that can actually serve the fake camera video.

    The distro `chromium-browser` is a snap wrapper; the current snap (153) silently drops
    `--use-file-for-fake-video-capture`, so no video device appears and getUserMedia fails with
    NotFoundError. Prefer a real (non-snap) Chrome, fall back to the wrapper.
    """
    cands = sorted(glob.glob("/root/.cache/puppeteer/chrome/*/chrome-linux64/chrome"))
    if os.environ.get("CHROME_PATH"):
        return os.environ["CHROME_PATH"]
    return cands[-1] if cands else "/usr/bin/chromium-browser"


def sweep():
    psql("DELETE FROM businesses WHERE probe_harness='%s'" % HARNESS)
    psql("DELETE FROM visitor_accounts WHERE probe_harness='%s'" % HARNESS)


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep()
    ts = str(int(time.time()))
    payload = f"SCANPROBE-{ts}-CARD"
    pw = "ProbePass123!"

    # the programme + directory the app itself resolves for a card scan
    directory_id = one("SELECT id FROM directories ORDER BY name LIMIT 1")
    program_id = one(
        "SELECT id FROM loyalty_programs WHERE is_active AND network_id IS NOT NULL "
        "ORDER BY created_at LIMIT 1")
    earn_rate = float(one("SELECT earn_rate FROM loyalty_programs WHERE id='%s'" % program_id) or 1.0)
    per_checkin = int(one("SELECT COALESCE(points_per_checkin, 0) FROM loyalty_programs WHERE id='%s'" % program_id) or 0)
    min_redeem = int(one("SELECT COALESCE(min_redeem_balance, 0) FROM loyalty_programs WHERE id='%s'" % program_id) or 0)
    expected = int(math.floor(float(AMOUNT) * earn_rate))
    start_balance = 150
    check("the live programme and directory resolve", bool(directory_id) and bool(program_id),
          f"program={program_id[:8]} earn_rate={earn_rate} checkin={per_checkin} min_redeem={min_redeem}")

    before = {t: one(f"SELECT count(*) FROM {t}") for t in
              ("visitor_accounts", "loyalty_members", "loyalty_activity", "loyalty_scans",
               "loyalty_checkins", "businesses")}

    slug = f"zz-scan-probe-{ts}"
    bid = one(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Scanner Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{HARNESS}') RETURNING id")
    va = one(
        "INSERT INTO visitor_accounts (email, password_hash, name, probe_harness) "
        f"VALUES ('zz-scan-{ts}@swiftsoftware.dev', 'x', 'ZZ Scanner Probe', '{HARNESS}') RETURNING id")
    mem = one(
        "INSERT INTO loyalty_members (program_id, visitor_account_id, points_balance, qr_code, qr_code_generated_at) "
        f"VALUES ('{program_id}', '{va}', 150, '{payload}', now()) RETURNING id")
    check("disposable business, account and card created",
          all([bid, va, mem]), f"member={mem[:8] if mem else ''}")

    # a real scanner session: the platform-operator token the scanner authenticates with
    tok = sh("node", MINT).stdout.strip()
    check("scanner session token minted", tok.count(".") == 2, f"len={len(tok)}")

    env = dict(os.environ, MD_BASE=BASE, PAYLOAD=payload, BIZ_ID=bid,
               BIZ_NAME=f"ZZ Scanner Probe Co {ts}", BIZ_CAT="Home Services",
               TOKEN=tok, AUD=AUD, AMOUNT=AMOUNT, REDEEM_POINTS=str(REDEEM), NODE_PATH=NODE_PATH,
               CHROME_PATH=chrome_path())
    check("a camera-capable browser is available", os.path.exists(env["CHROME_PATH"]),
          os.path.basename(env["CHROME_PATH"]))
    r = sh("node", BROWSER, env=env, timeout=240)
    proof_path = os.path.join(AUD, "browser-proof.json")
    proof = {}
    if os.path.exists(proof_path):
        proof = json.load(open(proof_path))
    else:
        print(r.stdout[-800:]); print(r.stderr[-800:])

    # ── the page itself ──────────────────────────────────────────────────────────
    check("the live scanner page loaded", proof.get("httpStatus") == 200,
          f"status={proof.get('httpStatus')} ts={ts}")
    check("the page ran with no script errors", proof.get("pageErrors") == [],
          f"errors={proof.get('pageErrors')}")
    gen = proof.get("generator", {})
    check("the app's own QR generator rendered the card", bool(gen.get("hasCanvas")),
          f"canvas_w={gen.get('w')}")

    # ── camera + QR read ────────────────────────────────────────────────────────
    ds = proof.get("decodeState", {})
    check("the camera delivered real frames (getUserMedia)", (ds.get("videoW") or 0) > 0,
          f"videoW={ds.get('videoW')}")
    check("the page decoded the QR from the live frame", proof.get("decoded") is True,
          f"status={(ds.get('status') or '')[:40]}")
    check("the decoded payload matches the card", proof.get("payloadMatches") is True,
          f"stored={(ds.get('storedQR') or '')[:24]}")

    # ── the three counter modes: on screen and on the wire ──────────────────────
    modes = proof.get("modes", {})
    pur, chi, red = modes.get("purchase", {}), modes.get("checkin", {}), modes.get("redemption", {})
    finals = start_balance + expected + per_checkin - REDEEM

    check("the purchase was submitted from the page", pur.get("clicked") is True)
    check("the purchase overlay reports a credit", pur.get("title") == "Purchase Credited!",
          f"title={pur.get('title')} label={pur.get('label')}")
    check("the purchase overlay shows the points earned", pur.get("points") == f"+{expected}",
          f"points={pur.get('points')} expected=+{expected}")
    check("the check-in was submitted from the page", chi.get("clicked") is True)
    check("the check-in overlay reports a check-in", chi.get("title") == "Checked In!",
          f"title={chi.get('title')} label={chi.get('label')} details={chi.get('details')}")
    check("the check-in overlay shows the check-in points", chi.get("points") == f"+{per_checkin}",
          f"points={chi.get('points')} expected=+{per_checkin}")
    check("the redemption was submitted from the page", red.get("clicked") is True)
    check("the redemption overlay reports a redemption", red.get("title") == "Redeemed!",
          f"title={red.get('title')} label={red.get('label')} details={red.get('details')}")
    check("the redemption overlay shows the units taken off", red.get("points") == f"-{REDEEM}",
          f"points={red.get('points')} expected=-{REDEEM}")

    scans = proof.get("scanReqs") or []
    bodies = {}
    for s in scans:
        try:
            b = json.loads(s.get("body") or "{}")
        except Exception:
            b = {}
        if b.get("status"):
            bodies[b["status"]] = (s.get("status"), b)
    check("the page made three real scan calls, all 200",
          len(scans) == 3 and all(s.get("status") == 200 for s in scans),
          f"calls={[s.get('status') for s in scans]}")
    check("the API credited the expected purchase points",
          bodies.get("credited", (None, {}))[1].get("points_awarded") == expected,
          f"body={bodies.get('credited', (None, {}))[1]}")
    check("the API awarded the programme's check-in points",
          bodies.get("checked_in", (None, {}))[1].get("points_awarded") == per_checkin,
          f"body={bodies.get('checked_in', (None, {}))[1]}")
    check("the API redeemed exactly the units asked for",
          bodies.get("redeemed", (None, {}))[1].get("points_redeemed") == REDEEM
          and bodies.get("redeemed", (None, {}))[1].get("points_awarded") == -REDEEM,
          f"body={bodies.get('redeemed', (None, {}))[1]}")

    # ── recorded in the database ────────────────────────────────────────────────
    bal = one(f"SELECT points_balance FROM loyalty_members WHERE id='{mem}'")
    check("the balance is start + purchase + check-in − redemption", bal == str(finals),
          f"balance={bal} expected={finals} ({start_balance}+{expected}+{per_checkin}-{REDEEM})")
    acts = psql("SELECT activity_type||'|'||points_earned FROM loyalty_activity "
                f"WHERE member_id='{mem}' ORDER BY activity_type")
    want = sorted([f"purchase|{expected}", f"checkin|{per_checkin}", f"redemption|{-REDEEM}"])
    check("the activity ledger holds all three moves", acts == want, f"rows={acts}")
    nci = one("SELECT COUNT(*) FROM loyalty_checkins "
              f"WHERE member_id='{mem}' AND points_awarded={per_checkin} AND method='counter_scan'")
    check("the check-in is recorded as a counter check-in", nci == "1", f"rows={nci}")
    row_p = one("SELECT scan_type||'|'||points_awarded||'|'||COALESCE(transaction_amount::text,'')"
                f"||'|'||business_id FROM loyalty_scans WHERE member_id='{mem}' AND scan_type='purchase'")
    check("the purchase scan is recorded with its amount and the business",
          row_p == f"purchase|{expected}|{AMOUNT}|{bid}", f"row={row_p}")
    row_c = one("SELECT scan_type||'|'||points_awarded||'|'||business_id FROM loyalty_scans "
                f"WHERE member_id='{mem}' AND scan_type='checkin'")
    check("the check-in scan is recorded against the business",
          row_c == f"checkin|{per_checkin}|{bid}", f"row={row_c}")
    row_r = one("SELECT scan_type||'|'||points_awarded||'|'||points_balance||'|'||business_id "
                f"FROM loyalty_scans WHERE member_id='{mem}' AND scan_type='redemption'")
    check("the redemption scan is recorded (debit, resulting balance, the business)",
          row_r == f"redemption|{-REDEEM}|{finals}|{bid}", f"row={row_r}")
    nscan = one(f"SELECT COUNT(*) FROM loyalty_scans WHERE member_id='{mem}'")
    check("all three scans are on the ledger", nscan == "3", f"rows={nscan}")
    uniq = one(f"SELECT COUNT(DISTINCT transaction_id) FROM loyalty_scans WHERE member_id='{mem}'")
    check("each page-driven scan carries its own receipt id", uniq == "3", f"distinct={uniq}")

    # ── teardown ────────────────────────────────────────────────────────────────
    psql(f"DELETE FROM loyalty_checkins WHERE member_id='{mem}'")
    psql(f"DELETE FROM loyalty_scans WHERE member_id='{mem}'")
    psql(f"DELETE FROM loyalty_activity WHERE member_id='{mem}'")
    psql(f"DELETE FROM loyalty_members WHERE id='{mem}'")
    psql(f"DELETE FROM visitor_accounts WHERE id='{va}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    after = {t: one(f"SELECT count(*) FROM {t}") for t in before}
    drift = {k: (before[k], after[k]) for k in before if before[k] != after[k]}
    check("probe data fully removed (no residue)", not drift, f"drift={drift}")
    with open(os.path.join(AUD, "residue.txt"), "w") as f:
        f.write("before=" + json.dumps(before) + "\nafter=" + json.dumps(after) + "\n")

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
