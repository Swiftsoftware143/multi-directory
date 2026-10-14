#!/usr/bin/env python3
"""Durable proof of queue item 11 — per-directory head/body/footer code injection (ZaarHub).

  "code injection body+footer"

Two levels, both against the REAL running app (no stubs):
  A. SERVER — the code an operator enters is really injected into the live public city pages,
     served through the real edge (https://zaarhub.com), and placed correctly:
     head marker inside <head>, body marker after <body>, footer marker before </body>.
     Checked on two city routes: /palm-bay and /palm-bay/businesses.
  B. SCREEN — the owner-portal SEO screen really shows three labelled code boxes, loads the
     saved code into them, and its Save button really asks the API to store all three
     (real headless Chromium; window.fetch is replaced, so no production write).

The live directory's three code values are captured first and restored on EVERY exit path
(net-zero is asserted). Run:  python3 scripts/code-injection-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import os
import re
import subprocess
import sys
import time

APP = "/opt/swift/apps/multi-directory"
BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
EDGE = os.environ.get("MD_EDGE", "https://zaarhub.com")
UA = "Mozilla/5.0 (compatible; SwiftOps/1.0)"
SLUG = os.environ.get("MD_CITY", "palm-bay")
MINT = "/opt/swift/bin/md-mint-token.py"
OWNER = "swiftsoftware143@yahoo.com"
BROWSER = os.path.join(APP, "scripts", "code-injection-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"

HEAD = "ZZHEAD4471"
BODY = "ZZBODY4471"
FOOT = "ZZFOOT4471"
H_HTML = f'<meta name="{HEAD.lower()}" content="{HEAD}">'
B_HTML = f'<div id="zz-body">{BODY}</div>'
F_HTML = f'<div id="zz-foot">{FOOT}</div>'

FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def db_url():
    r = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL")
    return r.stdout.strip()


URL = db_url()
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT)\b")


def psql(sql):
    r = sh("docker", "exec", "-i", "swift-postgres", "psql", URL, "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql[:200]}")
    return [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]


def one(sql):
    rows = psql(sql)
    return rows[0] if rows else ""


def http(method, path, token=None, body=None, extra=None):
    cmd = ["curl", "-s", "-X", method, BASE + path, "-H", "Accept: application/json"]
    if token:
        cmd += ["-H", f"Authorization: Bearer {token}"]
    if body is not None:
        cmd += ["-H", "Content-Type: application/json", "--data-binary", body]
    if extra:
        cmd += extra
    r = sh(*cmd, "--max-time", "25")
    return r.stdout


DIR_ID = one(f"select id from directories where slug='{SLUG}'")
check("directory exists", bool(DIR_ID), f"{SLUG}={DIR_ID}")

# --- binary parity: the artifact under test is the live one ---
repo = sh("sha256sum", f"{APP}/target/release/multidirectory").stdout.split()[0]
cont = sh("docker", "exec", "multidirectory", "sha256sum", "/app/multidirectory").stdout.split()[0]
check("live container runs the repo's freshly-built binary", repo == cont, f"{repo[:16]} vs {cont[:16]}")

# --- capture originals (base64, chr(30)-joined) so restore is exact ---
CAPTURE = (
    "select encode(convert_to(coalesce(head_injection,'')||chr(30)||coalesce(body_injection,'')||"
    f"chr(30)||coalesce(footer_injection,''),'UTF8'),'base64') from directories where id='{DIR_ID}'"
)
BEFORE = one(CAPTURE)


def restore():
    psql(
        "update directories set "
        f"head_injection=nullif(split_part(convert_from(decode('{BEFORE}','base64'),'UTF8'),chr(30),1),''), "
        f"body_injection=nullif(split_part(convert_from(decode('{BEFORE}','base64'),'UTF8'),chr(30),2),''), "
        f"footer_injection=nullif(split_part(convert_from(decode('{BEFORE}','base64'),'UTF8'),chr(30),3),'') "
        f"where id='{DIR_ID}'"
    )


restored = {"done": False}


def main():
    try:
        # --- A. server-side injection on the live edge ---
        psql(
            "update directories set "
            f"head_injection='{H_HTML}', body_injection='{B_HTML}', footer_injection='{F_HTML}' "
            f"where id='{DIR_ID}'"
        )
        time.sleep(1.5)
        for path in (f"/{SLUG}", f"/{SLUG}/businesses"):
            r = subprocess.run(["curl", "-s", "-A", UA, EDGE + path], capture_output=True, text=True, timeout=30)
            page = r.stdout
            check(f"{path} head code rendered", HEAD in page)
            check(f"{path} body code rendered", BODY in page)
            check(f"{path} footer code rendered", FOOT in page)
            head_part = page.split("</head>", 1)[0]
            check(f"{path} head code sits in <head>", HEAD in head_part)
            m = re.search(r"<body[^>]*>", page, re.I)
            tail = page[m.end():] if m else ""
            check(f"{path} body code sits after <body>", BODY in tail)
            fb = page.rfind(FOOT)
            cb = page.lower().rfind("</body>")
            check(f"{path} footer code sits before </body>", fb != -1 and cb != -1 and fb < cb)

        # --- restore + net-zero ---
        restore()
        restored["done"] = True
        time.sleep(1.0)
        AFTER = one(CAPTURE)
        check("net-zero (city code boxes restored)", BEFORE == AFTER, f"before={BEFORE[:24]} after={AFTER[:24]}")
        live = sh("curl", "-s", "-A", UA, f"{EDGE}/{SLUG}", "--max-time", "25").stdout
        check("probe removed from live page", "ZZHEAD4471" not in live and "ZZBODY4471" not in live)

        # --- save endpoint serves all three fields ---
        tok = sh("python3", MINT, OWNER).stdout.strip()
        r = http("GET", f"/api/v1/directories/{SLUG}", token=tok)
        for k in ("head_injection", "body_injection", "footer_injection"):
            check(f"save endpoint serves {k}", f'"{k}"' in r)

        # --- B. the real owner-portal screen in a real browser ---
        env = dict(os.environ, NODE_PATH=NODE_PATH)
        br = sh("node", BROWSER, env=env)
        print((br.stdout or "") + (br.stderr or ""))
        check("owner-portal code-injection screen (browser)", "RESULT: PASS" in (br.stdout or ""))
    finally:
        if not restored["done"]:
            try:
                restore()
                print("restored after early exit")
            except Exception as e:  # noqa: BLE001
                print(f"RESTORE FAILED: {e}")


if __name__ == "__main__":
    main()
    print("RESULT: " + ("PASS" if not FAILS else f"FAIL ({len(FAILS)})"))
    sys.exit(1 if FAILS else 0)
