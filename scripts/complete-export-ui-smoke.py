#!/usr/bin/env python3
"""Item 4 — DURABLE driver for the "a complete record downloads from the admin UI" proof.

Why this exists: `scripts/complete-export-ui-browser.js` was committed (4b48a59) but NOTHING
invoked it — it was an orphaned proof. An orphaned proof cannot be re-run, so a later regression
would go unnoticed and the only item-4 evidence would be the direct-endpoint verifier
(`complete-export-verify.py`), which never renders the admin page or clicks the page's own
buttons. This driver makes the screen half re-runnable:

  1. mints a platform-operator session (no dependency on a stale /tmp/mdtok),
  2. runs the real Chromium arm against the live admin console: pick a real business, click the
     page's OWN "This record — JSON" and "— CSV" buttons, capture the actual downloaded files,
  3. fails unless the browser arm exits 0 AND reports every check passed,
  4. independently asserts both downloaded files exist, are non-trivial, and carry the record's
     full declared section set (so a driver run cannot pass on a screenshot alone),
  5. keeps evidence under /opt/swift/audits/<run>/.

Run:  python3 scripts/complete-export-ui-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import glob
import json
import os
import shutil
import subprocess
import sys
import time

APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "complete-export-ui-browser.js")
MINT = "/opt/swift/browser-run/md-mint-token.js"
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
AUD = os.environ.get(
    "AUD", f"/opt/swift/audits/zz-item4-export-ui-{time.strftime('%Y%m%d-%H%M%S')}"
)
BIZ = "bb5f6bff-0db4-4d91-a13f-d19dfe64fb37"
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
    for p in glob.glob("/root/.cache/puppeteer/chrome/*/chrome-linux*/chrome"):
        return p
    return ""


def main():
    os.makedirs(AUD, exist_ok=True)

    chrome = find_chrome()
    check("a Chromium browser is available for the screen proof", bool(chrome), chrome or "none")
    if not chrome:
        return finish()

    tok = subprocess.run(["node", MINT], capture_output=True, text=True).stdout.strip()
    check("platform-operator session minted", tok.count(".") == 2, f"len={len(tok)}")
    if tok.count(".") != 2:
        return finish()

    env = dict(os.environ, MD_TOKEN=tok, AUD=AUD, NODE_PATH=NODE_PATH, CHROME_PATH=chrome)
    r = subprocess.run(
        ["node", BROWSER], capture_output=True, text=True, env=env, timeout=240
    )
    out = (r.stdout or "") + (r.stderr or "")
    print(out)
    check("the browser arm exited cleanly", r.returncode == 0, f"rc={r.returncode}")
    check("the browser arm reported every check passed", "RESULT: PASS" in out)

    dl = os.path.join(AUD, "downloads")
    js = os.path.join(AUD, "downloaded-record.json")
    csv = os.path.join(AUD, "downloaded-record.csv")
    # The browser arm writes the JSON to AUD/downloads/<name>.json and the CSV next to it.
    js_files = [p for p in glob.glob(os.path.join(dl, "*.json"))] + ([js] if os.path.exists(js) else [])
    check("a JSON record file was actually produced", bool(js_files), js_files[0] if js_files else "none")
    if js_files:
        size = os.path.getsize(js_files[0])
        check("the JSON download is a real file, not a stub", size > 4000, f"{size} bytes")
        try:
            doc = json.load(open(js_files[0]))
            # Shape: {"sections": [ {name,count,...} ... ], "related": {...} } — one section per
            # declared child table plus the entity row itself.
            sections = doc.get("sections") or doc.get("related") or []
            n = len(sections)
            check("the JSON carries all declared sections", n >= 61, str(n))
            if isinstance(sections, list) and sections and isinstance(sections[0], dict):
                named = all("name" in s for s in sections)
                check("each JSON section is named", named)
            check("the JSON carries this record", BIZ in json.dumps(doc)[:200000] or "Face It Aesthetics" in json.dumps(doc), "record present")
        except Exception as e:  # noqa: BLE001
            check("the JSON download parses", False, str(e)[:120])

    check("a CSV record file was actually produced", os.path.exists(csv), csv if os.path.exists(csv) else "none")
    if os.path.exists(csv):
        size = os.path.getsize(csv)
        text = open(csv, encoding="utf-8", errors="replace").read()
        secs = sum(1 for ln in text.splitlines() if ln.startswith("# "))
        check("the CSV is a real file, not a stub", size > 1000, f"{size} bytes")
        check("the CSV has a section per related table", secs >= 61, str(secs))
        check("the CSV contains the record", "Face It Aesthetics" in text)

    check("the proof kept its evidence", os.path.exists(os.path.join(AUD, "ui-proof.txt")))
    return finish()


def finish():
    global fails
    print(f"\nRESULT: {'PASS' if fails == 0 else 'FAIL'}  evidence: {AUD}")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
