#!/usr/bin/env python3
"""Sector landing pages — BRANDING proof (item 18 cleanup: a dead 401 branding fetch).

What was wrong (measured, not guessed):
  * every sector landing page (`/templates/<sector>.html?slug=<directory>`, 9 files × 10 cities,
    live and indexable) fetched `GET /api/v1/directories/<slug>` for its brand colours. That
    route is the directory ROW — private by default — so an anonymous visitor got **401**, and
    `dir.branding` was read off a JSON object that has no such field either. Two ways dead: the
    fetch could not succeed and the field did not exist. Result: the pages silently kept the
    hardcoded sector palette instead of the directory's own brand colours.
  * `applyBranding` never ran, so a directory's primary/secondary colour was invisible on those
    pages even though every directory HAS a branding row (10/10 in the live DB).

The fix: the page reads the dedicated branding endpoint (`GET /directories/:slug/branding`,
branding is public presentation data — colours/fonts/logo urls/meta templates) which is now
anonymous-readable, and the guard is `brand && brand.primary_color`. The directory ROW stays
private (it carries `google_maps_api_key` and the AI/CoreSwift wiring).

This probe proves, against the running app + a real browser:
  1. the branding endpoint answers 200 to an ANONYMOUS caller with the stored colour;
  2. controls: the directory row still answers 401, and the admin addressee still refuses a write;
  3. a real browser on the sector page shows the DIRECTORY's stored colour (not the sector theme);
  4. a second city + a second sector agree;
  5. the live public hostname agrees (real browser, Cloudflare included);
  6. NOT hardcoded — writing a probe colour into the stored row changes what the page shows, and
     restoring the stored colour brings the original back;
  7. no regression: the page still renders its business cards, with zero console/page errors.

Run:  python3 scripts/sector-template-branding-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import json
import os
import re
import subprocess
import sys
import time

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
PUBLIC = os.environ.get("MD_PUBLIC", "https://zaarhub.com")
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "sector-template-branding-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item18-sector-branding-{time.strftime('%Y%m%d-%H%M%S')}")
FAILS = []
PROBE_COLOUR = "#123456"


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
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""), flush=True)
    if not ok:
        FAILS.append(label)


def http(method, url, token=None, body=None, timeout="20"):
    cmd = ["curl", "-s", "-X", method, "-o", "-", "-w", "\n%{http_code}", url,
           "-H", "Accept: application/json", "-A", "Mozilla/5.0 (compatible; SwiftOps/1.0)"]
    if token:
        cmd += ["-H", f"Authorization: Bearer {token}"]
    if body is not None:
        cmd += ["-H", "Content-Type: application/json", "--data-binary", json.dumps(body)]
    r = sh(*cmd, "--max-time", timeout)
    out = r.stdout.rsplit("\n", 1)
    if len(out) != 2:
        return 0, r.stdout
    return int(out[1] or 0), out[0]


def json_of(txt):
    try:
        return json.loads(txt)
    except Exception:
        return None


def stored(slug):
    row = one(f"SELECT b.primary_color, b.secondary_color, b.accent_color FROM directory_branding b "
              f"JOIN directories d ON d.id = b.directory_id WHERE d.slug = '{slug}'")
    p, s, a = (row.split("|") + ["", "", ""])[:3]
    return p, s, a


def run_browser(targets, slugs, label):
    env = dict(os.environ, MD_BASE=BASE, TARGETS=json.dumps(targets), SLUGS=json.dumps(slugs),
               AUD=AUD, LABEL=label, NODE_PATH=NODE_PATH)
    r = sh("node", BROWSER, env=env)
    proof = json_of(r.stdout)
    if proof is None:
        print(r.stdout[-800:])
        print(r.stderr[-800:])
    return proof


def main():
    os.makedirs(AUD, exist_ok=True)
    print(f"artifact dir: {AUD}")

    # ── 1. the stored values we expect the page to show ────────────────────────────────────────
    pb_p, pb_s, pb_a = stored("palm-bay")
    wg_p, wg_s, _ = stored("winter-garden")
    check("the directory has a stored brand colour to show (palm-bay)", bool(pb_p), f"primary={pb_p}")
    check("...and a secondary colour", bool(pb_s), f"secondary={pb_s}")
    with open(f"{AUD}/stored-branding.txt", "w") as f:
        f.write(f"palm-bay primary={pb_p} secondary={pb_s} accent={pb_a}\n"
                f"winter-garden primary={wg_p} secondary={wg_s}\n")

    # ── 2. the endpoint, anonymous ─────────────────────────────────────────────────────────────
    st, body = http("GET", f"{BASE}/api/v1/directories/palm-bay/branding")
    got = json_of(body) or {}
    check("an ANONYMOUS caller can read the branding endpoint", st == 200, f"status={st}")
    check("...and it returns the stored primary colour", got.get("primary_color") == pb_p,
          f"{got.get('primary_color')!r} == {pb_p!r}")

    # ── 3. controls: we opened ONE presentation read, nothing else ─────────────────────────────
    st_row, _ = http("GET", f"{BASE}/api/v1/directories/palm-bay")
    check("CONTROL the directory ROW is still private (401)", st_row == 401, f"status={st_row}")
    st_put, _ = http("PUT", f"{BASE}/api/v1/admin/branding/00000000-0000-0000-0000-000000000000",
                     body={"primary_color": PROBE_COLOUR})
    st_put2, _ = http("PUT", f"{BASE}/api/v1/branding/00000000-0000-0000-0000-000000000000",
                      body={"primary_color": PROBE_COLOUR})
    check("CONTROL an anonymous WRITE to branding is still refused",
          st_put != 200 and st_put2 != 200, f"admin={st_put} alt={st_put2}")
    st_404, _ = http("GET", f"{BASE}/api/v1/directories/no-such-city-zz/branding")
    check("CONTROL an unknown city is a real 404, not a silent 200", st_404 == 404, f"status={st_404}")

    # ── 4. the pages, in a real browser ────────────────────────────────────────────────────────
    targets = [
        {"url": f"{BASE}/templates/farm.html?slug=palm-bay", "slug": "palm-bay"},
        {"url": f"{BASE}/templates/restaurant.html?slug=winter-garden", "slug": "winter-garden"},
        {"url": f"{PUBLIC}/templates/farm.html?slug=palm-bay", "slug": "palm-bay"},
    ]
    slugs = {"palm-bay": {"primary": pb_p, "secondary": pb_s},
             "winter-garden": {"primary": wg_p, "secondary": wg_s}}
    proof = run_browser(targets, slugs, "base") or {}
    with open(f"{AUD}/browser-base.json", "w") as f:
        json.dump(proof, f, indent=1)

    def arms(prefix):
        return {k: v for k, v in (proof.get("targets") or {}).items() if k.startswith(prefix + ":")}

    internal = arms("base")
    check("a real browser loaded the sector pages (3 arms)", len(internal) == 3, f"n={len(internal)}")
    for key, rec in internal.items():
        slug = rec.get("slug")
        exp = slugs.get(slug, {}).get("primary", "")
        dom = rec.get("dom") or {}
        live = rec["url"].startswith("https://")
        where = "LIVE PUBLIC host" if live else "app"
        check(f"{where} {rec['url'].split('?')[0].split('//')[-1]} page answers 200",
              rec.get("status") == 200, f"status={rec.get('status')}")
        check(f"{where} {slug}: the page shows the directory's stored brand colour",
              dom.get("primary") == exp, f"on screen {dom.get('primary')!r} == stored {exp!r}")
        calls = rec.get("brandingCalls") or []
        check(f"{where} {slug}: the branding request really answered 200 (was 401 before)",
              bool(calls) and all(c.get("status") == 200 for c in calls),
              f"{[c.get('status') for c in calls]}")
        check(f"{where} {slug}: business cards still rendered (no regression)",
              (dom.get("cards") or 0) > 0, f"cards={dom.get('cards')}")
        check(f"{where} {slug}: no console or page errors",
              not rec.get("consoleErrors") and not rec.get("pageErrors"),
              f"console={rec.get('consoleErrors')} page={rec.get('pageErrors')} "
              f"failed={[f.get('url') + ' -> ' + str(f.get('status')) for f in (rec.get('failedResponses') or [])]}")

    # the sector page's own accent comes from the directory's SECONDARY colour (farm.html)
    farm = next((r for r in internal.values()
                 if r.get("slug") == "palm-bay" and not r["url"].startswith("https://")), {})
    check("the page's accent colour comes from the directory's secondary colour",
          ((farm.get("dom") or {}).get("accent") or "") == pb_s,
          f"accent on screen {((farm.get('dom') or {}).get('accent'))!r} == stored {pb_s!r}")

    # ── 5. NOT hardcoded — the stored row drives the page ──────────────────────────────────────
    did = one("SELECT id FROM directories WHERE slug='palm-bay'")
    try:
        psql(f"UPDATE directory_branding SET primary_color='{PROBE_COLOUR}' WHERE directory_id='{did}'")
        check("probe colour written to the stored row",
              one(f"SELECT primary_color FROM directory_branding WHERE directory_id='{did}'") == PROBE_COLOUR)
        probe_targets = [{"url": f"{BASE}/templates/farm.html?slug=palm-bay", "slug": "palm-bay"}]
        probe_slugs = {"palm-bay": {"primary": PROBE_COLOUR, "secondary": pb_s}}
        proof2 = run_browser(probe_targets, probe_slugs, "probe") or {}
        with open(f"{AUD}/browser-probe.json", "w") as f:
            json.dump(proof2, f, indent=1)
        rec2 = next((r for r in (proof2.get("targets") or {}).values()
                     if r.get("slug") == "palm-bay"), {})
        check("NOT hardcoded: the page follows the stored colour when it changes",
              (rec2.get("dom") or {}).get("primary") == PROBE_COLOUR,
              f"on screen {((rec2.get('dom') or {}).get('primary'))!r} == probe {PROBE_COLOUR!r}")
    finally:
        psql(f"UPDATE directory_branding SET primary_color='{pb_p}' WHERE directory_id='{did}'")
        restored = one(f"SELECT primary_color FROM directory_branding WHERE directory_id='{did}'")
        check("the stored brand colour was restored byte-for-byte", restored == pb_p,
              f"{restored!r} == {pb_p!r}")

    # after the restore, the page shows the real colour again
    proof3 = run_browser([{"url": f"{BASE}/templates/farm.html?slug=palm-bay", "slug": "palm-bay"}],
                         {"palm-bay": {"primary": pb_p, "secondary": pb_s}}, "restored") or {}
    with open(f"{AUD}/browser-restored.json", "w") as f:
        json.dump(proof3, f, indent=1)
    rec3 = next((r for r in (proof3.get("targets") or {}).values()
                 if r.get("slug") == "palm-bay"), {})
    check("after the restore the page shows the real brand colour again",
          (rec3.get("dom") or {}).get("primary") == pb_p,
          f"on screen {((rec3.get('dom') or {}).get('primary'))!r} == stored {pb_p!r}")

    # ── 6. every sector template points at the endpoint that answers ───────────────────────────
    bad, noicon = [], []
    for fn in sorted(os.listdir(os.path.join(APP, "frontend", "templates"))):
        if not fn.endswith(".html"):
            continue
        t = open(os.path.join(APP, "frontend", "templates", fn), encoding="utf-8").read()
        if t.count("/directories/${SLUG}/branding") != 1 or "brand.branding" in t:
            bad.append(fn)
        if 'rel="icon"' not in t:
            noicon.append(fn)
    check("all 9 sector templates read the branding endpoint (none left on the 401 one)", not bad,
          f"bad={bad}")
    # The site ships an SVG favicon only, so a page that declares no icon makes the browser
    # auto-request /favicon.ico -> 404 -> a console error on every visit (the assertion above).
    check("all 9 sector templates declare the site's favicon (no phantom 404)", not noicon,
          f"missing={noicon}")

    print()
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)} failed)")
        for f_ in FAILS:
            print(f"  - {f_}")
        return 1
    print("RESULT: PASS (0 failed)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
