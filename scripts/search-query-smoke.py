#!/usr/bin/env python3
"""Durable proof — the public search returns the businesses it matches, on the API and ON THE SCREEN.

The defect this pins (measured 2026-10-15): `GET /api/v1/zaarhub/search` answered 200 with
`{"listings": [...]}` while every search screen (`frontend/index.html`, `frontend/zaarhub.html`,
`frontend/claim.html`) reads `results` / `deals` / `total` and renders the card fields `name`,
`rating`, `review_count`, `latitude`, `longitude`, `directory_slug` — and links each card to
`#/city/<directory_slug>/biz/<id>`, the public business-detail route, which resolves the row in
`businesses`. The screen therefore showed "No results found" for EVERY query, and the `category` /
`lat` / `lng` / `radius` parameters the screen sends were ignored entirely.

This probe seeds ONE clearly-marked probe business (`ZZPROBE-SEARCH-<ts>`) in a live directory and
proves, against the RUNNING app:

  HTTP  1. a plain search returns it: it is in `results`, `total` >= 1, and the row carries the
           card fields the screen reads (name / rating / review_count / latitude / longitude /
           directory_slug);
        2. the `category` filter is honoured: a wrong category returns 0, the right one returns it;
        3. the `radius` filter is honoured: the probe is EXCLUDED when the caller is ~111 km away
           and INCLUDED when the caller is on top of it;
        4. `deals` is present and is a list, and the legacy `listings` alias is still there;
  BROWSER 5. the real page /#/search/<marker> renders a card for the probe, links it at
           #/city/<directory_slug>/biz/<id>, shows the "<N> results found" header, raises no
           console error and the search API answered 200.

Nothing is left behind on either path (the probe row is deleted and its absence asserted).

Run:  python3 scripts/search-query-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089), DATABASE_URL override, CHROME_PATH, NODE_PATH, AUD
Exit: 0 = PASS, 1 = FAIL.
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "search-query-browser.js")
NODE_PATH = os.environ.get("NODE_PATH", "/opt/swift/browser-run/node_modules")
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-search-{time.strftime('%Y%m%d-%H%M%S')}")
MARKER = f"ZZPROBE-SEARCH-{int(time.time())}"
FAILS = []


def ok(m):
    print("PASS  " + m)


def bad(m):
    FAILS.append(m)
    print("FAIL  " + m)


def check(cond, m):
    (ok if cond else bad)(m)
    return cond


# ── DB access (the ONLY direct write is the disposable probe row + its teardown) ──────────────
def _pg_url():
    if os.environ.get("DATABASE_URL"):
        return os.environ["DATABASE_URL"]
    out = subprocess.run(["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
                         capture_output=True, text=True).stdout.strip()
    return out


def psql(sql):
    url = _pg_url()
    # -tA: tuples only, unaligned; -q: no "INSERT 0 1" command-status lines on stdout.
    p = subprocess.run(
        ["docker", "exec", "-i", "-e", f"PGURL={url}", "swift-postgres", "sh", "-c", 'psql "$PGURL" -tAq'],
        input=sql, capture_output=True, text=True,
    )
    if p.returncode != 0:
        raise RuntimeError(f"psql failed: {p.stderr.strip()[:300]}")
    return p.stdout.strip()


def call(method, path, token=None):
    req = urllib.request.Request(BASE + "/api/v1" + path, method=method)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.status, json.loads(r.read().decode() or "null")


def search(query, **params):
    qs = {"q": query}
    qs.update({k: v for k, v in params.items() if v is not None})
    return call("GET", "/zaarhub/search?" + urllib.parse.urlencode(qs))


def main():
    # 1. Pick a live directory + a real category to seed the probe under.
    row = psql(
        "SELECT b.directory_id||'|'||COALESCE(b.category_id::text,'')||'|'||COALESCE(dc.name,'')"
        "||'|'||COALESCE(b.city,'')||'|'||COALESCE(b.state,'')||'|'||b.latitude||'|'||b.longitude"
        "||'|'||d.slug FROM businesses b"
        " JOIN directories d ON d.id = b.directory_id"
        " LEFT JOIN directory_categories dc ON dc.id = b.category_id"
        " WHERE b.category_id IS NOT NULL AND b.is_active AND b.latitude IS NOT NULL"
        " AND b.longitude IS NOT NULL AND (d.status = 'active' OR d.status IS NULL)"
        " ORDER BY b.review_count DESC NULLS LAST LIMIT 1;"
    )
    if not row:
        bad("no live directory+category row to hang the probe off — cannot prove search")
        return finish(None)
    dir_id, cat_id, cat_name, city, state, lat, lng, dir_slug = row.split("|")
    lat, lng = float(lat), float(lng)
    ok(f"live directory resolved  [slug={dir_slug} category={cat_name}]")

    # 2. Seed the probe business (unique name => the marker query can only match it).
    probe_slug = MARKER.lower()
    pid = psql(
        "INSERT INTO businesses (directory_id, name, slug, description, city, state, latitude,"
        " longitude, category_id, is_active, rating, review_count)"
        f" VALUES ('{dir_id}', '{MARKER}', '{probe_slug}', 'search probe listing',"
        f" '{city}', '{state}', {lat}, {lng}, '{cat_id}', true, 5.0, 999) RETURNING id;"
    )
    pid_lines = [ln.strip() for ln in (pid or "").splitlines() if ln.strip()]
    pid = pid_lines[0] if pid_lines else ""
    if not pid:
        bad("probe business insert returned no id")
        return finish(None)
    ok(f"disposable probe business created  [id={pid[:8]} name={MARKER}]")

    # ── HTTP arm ─────────────────────────────────────────────────────────────────────────────
    st, body = search(MARKER)
    check(st == 200, f"search answers 200 for the probe marker  [status={st}]")
    if isinstance(body, dict):
        check("results" in body, "the response carries the `results` key the screen reads")
        check("deals" in body, "the response carries the `deals` key the screen reads")
        check("total" in body, "the response carries `total`")
        check("listings" in body, "the legacy `listings` alias is kept")
        res = body.get("results") or []
        ids = [r.get("id") for r in res]
        check(pid in ids, f"the probe business is in `results`  [total={body.get('total')}]")
        check(isinstance(body.get("deals"), list), "`deals` is a list")
        row1 = next((r for r in res if r.get("id") == pid), {})
        check(row1.get("name") == MARKER, "the row carries the card field `name`")
        check(row1.get("review_count") == 999, "the row carries the card field `review_count`")
        check(row1.get("rating") == 5.0, "the row carries the card field `rating`")
        check(row1.get("directory_slug") == dir_slug, "the row carries `directory_slug` the card links on")
        check(abs((row1.get("latitude") or 0) - lat) < 1e-6, "the row carries `latitude`")
        check(abs((row1.get("longitude") or 0) - lng) < 1e-6, "the row carries `longitude`")
    else:
        bad(f"search body was not an object: {str(body)[:120]}")

    # category filter — a wrong category must exclude the probe
    _, wrong = search(MARKER, category="zzprobe-no-such-category")
    check(isinstance(wrong, dict) and (wrong.get("total") or 0) == 0,
          "a WRONG category returns nothing (the filter is really applied)")
    _, right = search(MARKER, category=cat_name.upper())
    check(isinstance(right, dict) and any(r.get("id") == pid for r in (right.get("results") or [])),
          "the probe's OWN category (upper-cased) still returns it")

    # radius filter — metres; ~1 degree of latitude away must exclude it
    far = lat + 1.0
    _, far_body = search(MARKER, lat=far, lng=lng, radius=20000)
    check(isinstance(far_body, dict) and (far_body.get("total") or 0) == 0,
          "a caller ~111 km away with a 20 km radius does NOT see it")
    _, near_body = search(MARKER, lat=lat, lng=lng, radius=5000)
    check(isinstance(near_body, dict) and any(r.get("id") == pid for r in (near_body.get("results") or [])),
          "a caller on top of it with a 5 km radius DOES see it")

    # search that matches nothing
    _, none_body = search("zzprobe-no-such-business-" + MARKER)
    check(isinstance(none_body, dict) and (none_body.get("total") or 0) == 0,
          "a query that matches nothing returns 0 (no spurious rows)")

    # ── Browser arm ──────────────────────────────────────────────────────────────────────────
    os.makedirs(AUD, exist_ok=True)
    env = dict(os.environ)
    env.update({
        "MD_BASE": BASE, "MARKER": MARKER, "EXPECT_SLUG": dir_slug, "EXPECT_ID": pid,
        "NODE_PATH": NODE_PATH, "SHOT_DIR": AUD,
    })
    if os.environ.get("CHROME_PATH"):
        env["CHROME_PATH"] = os.environ["CHROME_PATH"]
    p = subprocess.run(["node", BROWSER], capture_output=True, text=True, env=env, cwd=APP)
    for line in p.stdout.strip().splitlines():
        print("  " + line)
    if p.stderr.strip():
        print("  [stderr] " + p.stderr.strip()[:300])
    check(p.returncode == 0, f"the browser flow passed (a real search card rendered on the screen)  [rc={p.returncode}]")

    return finish(pid)


def finish(pid):
    # Teardown + absence assertion, always.
    if pid:
        try:
            psql(f"DELETE FROM businesses WHERE id = '{pid}';")
            left = psql(f"SELECT COUNT(*) FROM businesses WHERE id = '{pid}';")
            check(left == "0", f"probe business fully removed  [remaining={left}]")
        except Exception as e:  # noqa: BLE001
            bad(f"teardown failed: {e}")
    print()
    if FAILS:
        print(f"RESULT: FAIL  ({len(FAILS)} check(s) failed)")
        for f in FAILS:
            print("  - " + f)
        sys.exit(1)
    print("RESULT: PASS")
    sys.exit(0)


if __name__ == "__main__":
    try:
        main()
    except urllib.error.URLError as e:
        bad(f"could not reach the app at {BASE}: {e}")
        finish(None)
    except Exception as e:  # noqa: BLE001
        bad(f"probe threw: {e}")
        finish(None)
