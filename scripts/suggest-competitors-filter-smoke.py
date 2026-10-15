#!/usr/bin/env python3
"""Answer-first "Suggest competitors" — the category filter is honoured (queue item 18, ZaarHub).

`POST /api/v1/admin/articles/suggest-competitors` is documented in the operator guide
(frontend/guide.html: "Suggest competitors from same directory") and its handler's own contract is
"competitors from the same directory, category and city". The request carried a `category` that the
handler dropped on the floor, so an operator asking for ONE category got every business in the
directory back — and the endpoint had no control in the operator console at all.

This probe proves, against the running app and real rows, that:
  1. no category  -> every business in the directory is still returned (no regression),
  2. category=X   -> exactly the businesses whose category is X come back, each labelled X,
  3. category=""  -> whitespace is no filter at all (same as (1)),
  4. category=Y   -> a category with NO businesses in that directory returns ZERO (the
                     discriminating case: before the fix this returned the whole directory),
  5. business_name -> the caller's own listing is still skipped,
  6. a logged-out caller is refused (401),
  7. the operator console now carries the control (its own `category` field).

Read-only against the live data — it writes nothing and asserts the row counts are unchanged on
either path. Run: python3 scripts/suggest-competitors-filter-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
CONSOLE = os.path.join(APP, "frontend", "admin-ops.html")
MINT = "/opt/swift/browser-run/md-mint-token.js"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item18-suggest-competitors-{time.strftime('%Y%m%d-%H%M%S')}")
PATH = "/api/v1/admin/articles/suggest-competitors"
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


def http(method, path, body=None, token=None, timeout=60):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
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


def main():
    print(f"== suggest-competitors category filter — {BASE} ==")

    st, health = http("GET", "/api/v1/health")
    check("the app answers /api/v1/health", st == 200, f"status={st}")

    # ── pick a real directory with categorised businesses ─────────────────────────
    directory_id = one(
        "SELECT b.directory_id FROM businesses b WHERE b.category_id IS NOT NULL "
        "GROUP BY 1 HAVING count(*) >= 20 ORDER BY count(*) DESC LIMIT 1")
    check("a live directory with categorised businesses resolves", bool(directory_id),
          f"dir={directory_id[:8] if directory_id else ''}")

    total_all = int(one(f"SELECT count(*) FROM businesses WHERE directory_id = '{directory_id}'"))
    total_cat = int(one(
        "SELECT count(*) FROM businesses b JOIN directory_categories dc ON dc.id = b.category_id "
        f"WHERE b.directory_id = '{directory_id}' AND b.category_id IS NOT NULL"))
    check("the directory has businesses (a real data set)", total_all > 20,
          f"total={total_all} categorised={total_cat}")

    # the category the handler must keep (>= 2 businesses, exact-name count)
    row = psql(
        "SELECT dc.name, count(*) FROM businesses b "
        "JOIN directory_categories dc ON dc.id = b.category_id "
        f"WHERE b.directory_id = '{directory_id}' GROUP BY dc.name "
        "HAVING count(*) >= 2 ORDER BY 2 DESC, 1 LIMIT 1")
    keep_cat = row[0].split("|")[0]
    keep_n = int(row[0].split("|")[1])
    check("a category with >= 2 live businesses resolves", bool(keep_cat), f"{keep_cat}={keep_n}")

    # a category name that has NO business in this directory (deduplicated by NAME, because the
    # handler compares names, not ids)
    drop_cat = one(
        "SELECT dc.name FROM directory_categories dc WHERE dc.name IS NOT NULL AND NOT EXISTS ("
        "  SELECT 1 FROM businesses b JOIN directory_categories d2 ON d2.id = b.category_id "
        f"  WHERE b.directory_id = '{directory_id}' AND lower(d2.name) = lower(dc.name)) "
        "GROUP BY dc.name ORDER BY dc.name LIMIT 1")
    check("a category with NO business in this directory resolves", bool(drop_cat), f"{drop_cat}")

    tok = sh("node", MINT).stdout.strip()
    check("an operator session token minted", tok.count(".") == 2, f"len={len(tok)}")

    before = int(one("SELECT count(*) FROM businesses"))

    # 1. no category — unchanged, whole directory
    st, r1 = http("POST", PATH, {"directory_id": directory_id}, tok)
    check("no category: the endpoint answers 200", st == 200, f"status={st}")
    got1 = r1.get("total") if isinstance(r1, dict) else None
    check("no category: every business in the directory is returned (no regression)",
          got1 == total_all, f"api={got1} db={total_all}")

    # 2. category = keep_cat — exactly that category, each labelled with it
    st, r2 = http("POST", PATH, {"directory_id": directory_id, "category": keep_cat}, tok)
    check("category filter: the endpoint answers 200", st == 200, f"status={st}")
    got2 = r2.get("total") if isinstance(r2, dict) else None
    sugg2 = r2.get("suggestions", []) if isinstance(r2, dict) else []
    check("category filter: exactly the businesses of that category come back",
          got2 == keep_n, f"api={got2} db={keep_n} cat={keep_cat!r}")
    labels = sorted({s.get("category") for s in sugg2 if s.get("category")})
    check("category filter: every suggestion is labelled with that category",
          bool(sugg2) and all(l.lower() == keep_cat.lower() for l in labels),
          f"labels={labels}")
    check("category filter: this is a real narrowing, not the whole directory",
          got2 is not None and got2 < total_all, f"filtered={got2} total={total_all}")

    # 3. whitespace is no filter
    st, r3 = http("POST", PATH, {"directory_id": directory_id, "category": "   "}, tok)
    got3 = r3.get("total") if isinstance(r3, dict) else None
    check("whitespace category is no filter at all (same as no category)",
          st == 200 and got3 == total_all, f"api={got3} db={total_all}")

    # 4. the discriminating case: a category with no businesses in this directory -> ZERO
    st, r4 = http("POST", PATH, {"directory_id": directory_id, "category": drop_cat}, tok)
    got4 = r4.get("total") if isinstance(r4, dict) else None
    check("a category with no businesses here returns ZERO (the filter is really applied)",
          st == 200 and got4 == 0, f"api={got4} cat={drop_cat!r} (whole directory = {total_all})")

    # 5. the caller's own listing is still skipped
    names = psql(
        f"SELECT min(name), count(*) FROM businesses WHERE directory_id = '{directory_id}' "
        "GROUP BY lower(name) ORDER BY count(*) DESC LIMIT 1")
    self_name = names[0].split("|")[0]
    self_n = int(names[0].split("|")[1])
    st, r5 = http("POST", PATH, {"directory_id": directory_id, "business_name": self_name}, tok)
    got5 = r5.get("total") if isinstance(r5, dict) else None
    check("passing business_name skips the caller's own listing",
          st == 200 and got5 == total_all - self_n,
          f"api={got5} expected={total_all - self_n} name={self_name!r}")

    # 6. a logged-out caller is refused
    st, body = http("POST", PATH, {"directory_id": directory_id})
    check("a logged-out caller cannot use the helper (401)", st == 401, f"status={st}")

    # 7. the operator console now carries the control with its own category field
    html = open(CONSOLE, encoding="utf-8").read()
    idx = html.find("'/admin/articles/suggest-competitors'")
    check("the operator console carries the Suggest competitors control", idx > 0, f"at={idx}")
    if idx > 0:
        block = html[idx:idx + 700]
        check("the console's control offers the category field",
              "k: 'category'" in block, "")
        check("the console's control offers the directory picker",
              "k: 'directory_id'" in block, "")

    # ── read-only: nothing was written ───────────────────────────────────────────
    after = int(one("SELECT count(*) FROM businesses"))
    check("the probe wrote nothing (business count unchanged)", before == after,
          f"before={before} after={after}")

    os.makedirs(AUD, exist_ok=True)
    json.dump({"directory_id": directory_id, "total_all": total_all, "keep_cat": keep_cat,
               "keep_n": keep_n, "drop_cat": drop_cat,
               "api": {"none": got1, "filtered": got2, "whitespace": got3, "empty": got4,
                       "self_skipped": got5}},
              open(os.path.join(AUD, "proof.json"), "w"), indent=2)
    print(f"evidence: {AUD}")


if __name__ == "__main__":
    try:
        main()
    except Exception as e:  # noqa: BLE001
        print(f"FAIL  probe crashed: {e}")
        FAILS.append("crash")
    print()
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)} failed) -> {FAILS}")
        sys.exit(1)
    print("RESULT: PASS")
    sys.exit(0)
