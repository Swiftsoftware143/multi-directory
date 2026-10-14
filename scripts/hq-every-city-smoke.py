#!/usr/bin/env python3
"""ZaarHub Headquarters — end-to-end proof (queue item 15).

  "Headquarters field (city+state, NOT hardcoded) — set Orlando, Florida, show on every city page"

What it proves, against the REAL running app (no stubs):
  1. the company Headquarters is ONE stored value (zaarhub_site_config.headquarters);
  2. every one of the platform's cities serves that same value from the public city API;
  3. every city PAGE renders it on screen ("🏢 Headquarters: …"), read from the live DOM
     in real headless Chromium at the app's own city deep link (/city/<slug>);
  4. it is NOT hardcoded: writing a new value through the real admin endpoint changes what the
     city page shows, and restoring the original brings it back.

Run:  python3 scripts/hq-every-city-smoke.py
Exit: 0 = PASS, 1 = FAIL.  The live config is restored on every exit path.
"""
import glob
import json
import os
import re
import subprocess
import sys
import time

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "hq-every-city-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"
MINT = "/opt/swift/browser-run/md-mint-token.js"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item15-hq-{time.strftime('%Y%m%d-%H%M%S')}")
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


def http_json(method, path, token=None, body=None):
    """Minimal curl-based JSON call (no external deps)."""
    cmd = ["curl", "-s", "-X", method, BASE + path,
           "-H", "Accept: application/json"]
    if token:
        cmd += ["-H", f"Authorization: Bearer {token}"]
    if body is not None:
        cmd += ["-H", "Content-Type: application/json", "--data-binary", json.dumps(body)]
    r = sh(*cmd, "--max-time", "20")
    try:
        return r.stdout
    except Exception:
        return ""


def json_of(txt):
    try:
        return json.loads(txt)
    except Exception:
        return None


def run_browser(cities, expected):
    env = dict(os.environ, MD_BASE=BASE, CITIES=json.dumps(cities), HQ=expected,
               AUD=AUD, NODE_PATH=NODE_PATH)
    r = sh("node", BROWSER, env=env)
    return json_of(r.stdout)


def main():
    os.makedirs(AUD, exist_ok=True)
    tok = sh("node", MINT).stdout.strip()
    check("platform-operator session minted", tok.count(".") == 2, f"len={len(tok)}")

    cfg = json_of(http_json("GET", "/api/v1/zaarhub/admin/config", tok))
    check("the admin config endpoint answers", isinstance(cfg, dict), f"keys={len(cfg or {})}")
    if not isinstance(cfg, dict):
        return finish()
    original = cfg.get("headquarters") or ""
    check("the company Headquarters is set (one stored value)", bool(original.strip()),
          repr(original))

    cities = psql("SELECT slug FROM directories ORDER BY slug")
    check("the platform has cities to check", len(cities) > 0, f"cities={len(cities)}")

    # ── API arm: every city serves the one value ──
    api_bad = []
    for slug in cities:
        d = json_of(http_json("GET", f"/api/v1/zaarhub/cities/{slug}"))
        got = ((d or {}).get("directory") or {}).get("headquarters")
        if got != original:
            api_bad.append(f"{slug}={got!r}")
    check("every city's public API carries the ONE Headquarters value",
          not api_bad, f"{len(cities)} cities" + (f" WRONG: {api_bad}" if api_bad else ""))

    # ── Browser arm: every city page shows it on screen ──
    out = run_browser(cities, original)
    check("the real city pages answered", bool(out), "")
    if not out:
        return finish()
    bad = []
    for slug in cities:
        c = out["cities"].get(slug, {})
        txt = c.get("text", "")
        if not (c.get("found") and c.get("visible") and original in txt):
            bad.append(f"{slug}(found={c.get('found')},visible={c.get('visible')},text={txt!r})")
    check("every city page renders 'Headquarters: <value>' on screen",
          not bad, f"{len(cities)} pages" + (f" MISSING: {bad}" if bad else ""))
    check("no script errors on the city pages",
          all(not c.get("pageErrors") for c in out["cities"].values()), "")

    # ── Not-hardcoded arm: change the stored value, watch the page change ──
    probe = "HQ Probe City, TS"
    payload = {k: cfg.get(k) for k in (
        "site_name", "site_tagline", "primary_color", "secondary_color", "logo_url",
        "favicon_url", "google_analytics_id", "facebook_app_id", "twitter_handle",
        "contact_email", "contact_phone", "copyright_year", "headquarters")}
    try:
        payload["headquarters"] = probe
        http_json("PATCH", "/api/v1/zaarhub/admin/config", tok, payload)
        after = json_of(http_json("GET", "/api/v1/zaarhub/admin/config", tok)) or {}
        check("a NEW Headquarters value is accepted by the admin endpoint",
              after.get("headquarters") == probe, repr(after.get("headquarters")))

        pout = run_browser([cities[0]], probe)
        ptext = ((pout or {}).get("cities", {}).get(cities[0], {}) or {}).get("text", "")
        check("the city page shows the NEW value (so it is NOT hardcoded)",
              probe in ptext, f"{cities[0]} -> {ptext!r}")
    finally:
        # Always restore, whatever happened above.
        payload["headquarters"] = original
        http_json("PATCH", "/api/v1/zaarhub/admin/config", tok, payload)
        back = json_of(http_json("GET", "/api/v1/zaarhub/admin/config", tok)) or {}
        check("the original Headquarters value is restored",
              back.get("headquarters") == original, repr(back.get("headquarters")))

    rout = run_browser([cities[0]], original)
    rtext = ((rout or {}).get("cities", {}).get(cities[0], {}) or {}).get("text", "")
    check("the city page shows the ORIGINAL value again", original in rtext, repr(rtext))

    # Evidence note.
    with open(os.path.join(AUD, "EVIDENCE.md"), "w") as fh:
        fh.write(
            "# Item 15 — Headquarters (city+state, NOT hardcoded; shown on every city page)\n\n"
            f"Date: {time.strftime('%Y-%m-%d %H:%M %Z')} | App: multi-directory (ZaarHub)\n\n"
            f"Stored value (`zaarhub_site_config.headquarters`): `{original}`\n\n"
            f"Cities checked: {len(cities)} — {', '.join(cities)}\n\n"
            "Arms, all against the real running app:\n"
            "1. every city's public city API (`GET /api/v1/zaarhub/cities/<slug>`) returns the same\n"
            "   single stored value in `directory.headquarters`;\n"
            "2. every city PAGE (`/city/<slug>`, real headless Chromium, live DOM) renders\n"
            "   `🏢 Headquarters: <value>` visibly;\n"
            "3. NOT hardcoded — writing `HQ Probe City, TS` through the real admin endpoint\n"
            "   (`PATCH /api/v1/zaarhub/admin/config`) changed what the city page showed, and\n"
            "   restoring the stored value brought the original back.\n\n"
            f"RESULTS: {'PASS' if not FAILS else 'FAIL ' + str(FAILS)}\n")
    return finish()


def finish():
    if FAILS:
        print("\nRESULT: FAIL — " + "; ".join(FAILS))
        return 1
    print("\nRESULT: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
