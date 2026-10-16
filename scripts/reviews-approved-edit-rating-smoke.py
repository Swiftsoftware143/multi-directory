#!/usr/bin/env python3
"""Durable proof — item 1 (reviews, business side): editing an APPROVED review's rating
must move the listing's public average with it.

The moderation paths (approve / reject / delete) each recompute the listing aggregate from
its imported baseline PLUS our approved rows. The documented edit path
(PUT /api/v1/reviews/:id, frontend/guide.html line 447) can change `rating` on the SAME row —
if it does not recompute, the listing keeps advertising a number no longer backed by any
review, and the public business page shows the stale average to every visitor.

Run:  python3 scripts/reviews-approved-edit-rating-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089), ADMIN_EMAIL
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
ADMIN_EMAIL = os.environ.get("ADMIN_EMAIL", "swiftsoftware143@yahoo.com")
MARK = "review-edit-rating-probe"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-review-edit-rating-{time.strftime('%Y%m%d-%H%M%S')}")
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


PW_DB = db_password()
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT)\b")


def psql(sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW_DB}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    return lines[0] if lines else ""


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def http(method, path, body=None, token=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
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


def sweep_stale():
    psql("DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")


def agg_of(bid):
    """(review_count, rating-or-None) straight off the public listing row."""
    raw = psql(f"SELECT review_count||'|'||COALESCE(rating::text,'NULL') FROM businesses WHERE id='{bid}'")
    cnt, rat = (raw.split("|", 1) + [""])[:2]
    return int(cnt or 0), (None if rat in ("", "NULL") else round(float(rat), 1))


def main():
    os.makedirs(AUD, exist_ok=True)
    sweep_stale()
    ts = str(int(time.time()))

    code = sh("curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", BASE + "/api/v1/health").stdout
    check("the app answers /api/v1/health", code == "200", f"status={code}")

    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    slug = f"zz-review-edit-probe-{ts}"
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Review Edit Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{MARK}') RETURNING id")
    check("disposable listing created", bool(bid), f"id={bid}")

    token = sh("python3", "/opt/swift/bin/md-mint-token.py", ADMIN_EMAIL, "900").stdout.strip()
    check("an operator token was minted", token.count(".") == 2, f"len={len(token)}")

    rid = psql(
        "INSERT INTO reviews (business_id, rating, title, content, reviewer_name, status, is_verified) "
        f"VALUES ('{bid}', 5, 'Edit probe', 'rating-edit proof {ts}', 'ZZ Probe', 'pending', false) RETURNING id")
    check("a pending review was seeded", bool(rid), f"id={rid}")

    # the moderation path owes us a recompute — establish the baseline it produces
    st, _ = http("POST", f"/api/v1/reviews/{rid}/approve", token=token)
    check("the operator can approve the review (POST /reviews/:id/approve)", st == 200, f"status={st}")
    cnt, rat = agg_of(bid)
    check("the listing aggregate follows the approval (5.0 / 1)", (cnt, rat) == (1, 5.0), f"agg={cnt}|{rat}")

    st, pub = http("GET", f"/api/v1/reviews?business_id={bid}")
    ids = [x.get("id") for x in ((pub or {}).get("data") or [])] if isinstance(pub, dict) else []
    check("the approved review is public", rid in ids, f"status={st} n={len(ids)}")

    # THE CLAIM UNDER TEST: edit that same approved review's rating.
    st, _ = http("PUT", f"/api/v1/reviews/{rid}", {"rating": 1}, token=token)
    check("the moderator can edit the review (PUT /reviews/:id)", st == 200, f"status={st}")
    row_rating = psql(f"SELECT rating::text FROM reviews WHERE id='{rid}'")
    check("the review row itself now reads 1", row_rating == "1", f"row={row_rating}")

    cnt2, rat2 = agg_of(bid)
    check("the listing average MOVED with the edit (1.0 / 1)", (cnt2, rat2) == (1, 1.0), f"agg={cnt2}|{rat2}")

    # and the public read a visitor gets must show the corrected number, not the stale one
    st, page = http("GET", f"/api/v1/zaarhub/business/z/{bid}")
    pub_rating = (page or {}).get("rating") if isinstance(page, dict) else None
    check("the public business read reports 1.0",
          st == 200 and pub_rating is not None and round(float(pub_rating), 1) == 1.0,
          f"status={st} rating={pub_rating}")

    psql(f"DELETE FROM reviews WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    left = psql(f"SELECT (SELECT count(*) FROM businesses WHERE id='{bid}')"
                f" + (SELECT count(*) FROM reviews WHERE business_id='{bid}')")
    check("probe data fully removed", left == "0", f"remaining={left}")

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
