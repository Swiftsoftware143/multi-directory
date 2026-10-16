#!/usr/bin/env python3
"""Business-side review moderation, part 3 — a listing that carries an IMPORTED reputation AND
already has approved reviews of ours (the 74 live listings migration 171 deliberately skipped).

`reviews-business-moderation-smoke.py` proves the guard for a listing whose imported rating/count
has ZERO rows of ours behind it (imported_* captured by migration 171, so the recompute adds to it).

But migration 171's backfill is guarded by `NOT EXISTS (approved review of ours)`, so every listing
that already had an approved review when it ran kept `imported_rating`/`imported_review_count` NULL —
while still carrying its imported `rating`/`review_count` in the base columns. Measured on the live
database 2026-10-16: 74 such listings (e.g. rating 4.7 / 780 reviews, 4.6 / 5238 reviews) with
exactly one approved review of ours.

For those, the recompute's baseline is COALESCE(imported_review_count, 0) = 0, so ANY moderation event
(approve a new review, reject an approved one, delete an approved one) rewrites the listing as
"our own rows only" — 5238 reviews collapse to 2 and the imported average is replaced. One-way loss.

This probe reproduces that state on a disposable listing and asserts the moderation INCORPORATES the
listing's imported reputation instead of destroying it:
  * listing: rating 4.6, review_count 5238, imported_* NULL, one pre-existing approved review of ours
  * a customer leaves a second review -> owner approves it from the business route
  * the listing must read ~4.6 / 5239 afterwards (baseline captured + both of our rows), never 5.0 / 2

and that a listing with NO imported reputation of its own (its count equals its own approved rows) is
untouched by the same rule (no invented baseline).

Run:  python3 scripts/reviews-imported-baseline-with-our-reviews-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Nothing is left behind on either path.
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
HARNESS = {"X-Swift-Harness": "review-impbaseline-probe"}
MARK = "review-impbaseline-probe"
FAILS = []


def sh(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


PW = db_password()
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT)\b")


def psql(sql, all_rows=False):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    if all_rows:
        return lines
    return lines[0] if lines else ""


def sweep_stale_probe_rows():
    psql("DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-impbase-%@swiftsoftware.dev'")


def http(method, path, body=None, token=None, extra_headers=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    for k, v in (extra_headers or {}).items():
        req.add_header(k, v)
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


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def main():
    sweep_stale_probe_rows()
    ts = str(int(time.time()))
    owner_email = f"zz-impbase-owner-{ts}@swiftsoftware.dev"
    cust_email = f"zz-impbase-cust-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    slug = f"zz-impbase-probe-{ts}"
    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    created = {"visitors": [], "businesses": [], "reviews": []}

    # ── reachability: the live state this probe reproduces ──────────────────────
    vuln = psql("SELECT count(*) FROM businesses b WHERE b.imported_rating IS NULL "
                "AND COALESCE(b.review_count,0) > (SELECT count(*) FROM reviews r "
                "WHERE r.business_id=b.id AND r.status='approved') "
                "AND EXISTS (SELECT 1 FROM reviews r2 WHERE r2.business_id=b.id AND r2.status='approved')")
    print(f"INFO  live listings carrying an imported count AND our approved reviews: {vuln}")

    for email, name in ((owner_email, "ZZ ImpBase Owner"), (cust_email, "ZZ ImpBase Customer")):
        st, _ = http("POST", "/api/v1/visitor/register", {"email": email, "password": pw, "name": name})
        check(f"account can be created ({email.split('@')[0]})", st in (200, 201), f"status={st}")
        vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{email}'")
        if vid:
            created["visitors"].append(vid)
    check("both accounts exist in the database", len(created["visitors"]) == 2,
          f"found={len(created['visitors'])}")

    # ── the vulnerable listing: imported 4.6/5238 + one approved review of ours ──
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ ImpBase Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', 4.6, 5238, "
        f"'{MARK}') RETURNING id")
    created["businesses"].append(bid)
    check("the vulnerable listing is created (imported 4.6 / 5238, no imported_* baseline)", bool(bid))

    legacy_rid = psql(
        "INSERT INTO reviews (business_id, directory_id, rating, title, content, reviewer_name, status, source) "
        f"VALUES ('{bid}', '{directory_id}', 5, 'Legacy approved review', 'Pre-existing row of ours.', "
        f"'Legacy Reviewer', 'approved', 'direct') RETURNING id")
    created["reviews"].append(legacy_rid)
    check("it already carries one approved review of ours (migration 171 skipped exactly this shape)",
          bool(legacy_rid))

    row = psql(f"SELECT rating||'|'||review_count||'|'||COALESCE(imported_rating::text,'NULL') "
               f"FROM businesses WHERE id='{bid}'")
    check("the fixture starts in the live shape (4.6|5238|NULL)", row == "4.6|5238|NULL", f"row={row}")

    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ ImpBase Owner",
                  "owner_phone": "4075550101"}, extra_headers=HARNESS)
    check("the owner can claim the listing (POST /businesses/:id/claim)", st in (200, 201), f"status={st}")

    st, tok_owner = http("POST", "/api/v1/auth/login", {"email": owner_email, "password": pw})
    check("the owner can sign in", st == 200 and "access_token" in (tok_owner or {}), f"status={st}")
    owner_token = (tok_owner or {}).get("access_token")
    st, tok_cust = http("POST", "/api/v1/auth/login", {"email": cust_email, "password": pw})
    cust_token = (tok_cust or {}).get("access_token")
    check("the customer can sign in", st == 200 and bool(cust_token), f"status={st}")

    # ── a customer leaves a second review; the owner approves it ────────────────
    st, rev = http("POST", "/api/v1/reviews",
                   {"business_id": bid, "rating": 5, "title": "Imported-baseline probe",
                    "content": "Left by the imported-baseline probe customer."}, token=cust_token)
    rid = (rev or {}).get("id") if isinstance(rev, dict) else None
    if rid:
        created["reviews"].append(rid)
    check("a second review lands pending on the listing",
          st in (200, 201) and (rev or {}).get("status") == "pending",
          f"status={st} review={(rev or {}).get('status') if isinstance(rev, dict) else rev}")

    st, _ = http("POST", f"/api/v1/business/reviews/{rid}/approve", {}, token=owner_token)
    check("the owner approves it from the business route (200)", st == 200, f"status={st}")

    agg = psql(f"SELECT review_count||'|'||COALESCE(round(rating::numeric,2)::text,'NULL') "
               f"FROM businesses WHERE id='{bid}'")
    count = agg.split("|")[0]
    rating = float(agg.split("|")[1]) if agg.split("|")[1] != "NULL" else None
    # The captured baseline is the listing's own count MINUS whatever of ours was already approved
    # (5238 - 1 = 5237): a stored imported count may or may not already include our row and the
    # conservative read never overstates a listing's reputation, so 5237 + our 2 approved rows = 5239.
    # Before the fix this read 2|5.00.
    check("approving INCORPORATES the imported reputation (5237 baseline + our 2 = 5239)",
          count == "5239", f"agg={agg} (destroyed value would be 2)")
    check("the imported average survives the approval (still ~4.6, not a jump to 5.0)",
          rating is not None and 4.55 <= rating <= 4.65, f"agg={agg}")

    st, pub = http("GET", f"/api/v1/reviews?business_id={bid}&status=approved")
    shown = len(((pub or {}).get("data") or [])) if isinstance(pub, dict) else 0
    check("the listing's approved reviews are still shown publicly", st == 200 and shown >= 2,
          f"status={st} shown={shown}")

    # deleting one of ours must also fall back to the captured baseline, not to zero (the DELETE arm
    # of the same recompute)
    st, _ = http("DELETE", f"/api/v1/reviews/{rid}", token=owner_token)
    check("the owner can delete one of the listing's reviews (200)", st in (200, 204), f"status={st}")
    agg3 = psql(f"SELECT review_count||'|'||COALESCE(round(rating::numeric,2)::text,'NULL') "
                f"FROM businesses WHERE id='{bid}'")
    check("deleting it falls back to the imported baseline too (5237 + 1 = 5238, ~4.6)",
          agg3.split("|")[0] == "5238" and 4.55 <= float(agg3.split("|")[1] or 0) <= 4.65,
          f"agg={agg3}")

    # ── a listing with no imported reputation of its own keeps its plain semantics ──
    plain = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ ImpBase Plain Co {ts}', '{slug}-plain', 'Orlando', 'FL', 'active', NULL, 0, "
        f"'{MARK}') RETURNING id")
    created["businesses"].append(plain)
    st, _ = http("POST", f"/api/v1/businesses/{plain}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ ImpBase Owner",
                  "owner_phone": "4075550101"}, extra_headers=HARNESS)
    st, prev = http("POST", "/api/v1/reviews",
                    {"business_id": plain, "rating": 4, "title": "Plain probe",
                     "content": "Plain listing probe."}, token=cust_token)
    prid = (prev or {}).get("id") if isinstance(prev, dict) else None
    if prid:
        created["reviews"].append(prid)
    st, _ = http("POST", f"/api/v1/business/reviews/{prid}/approve", {}, token=owner_token)
    agg2 = psql(f"SELECT review_count||'|'||COALESCE(round(rating::numeric,1)::text,'NULL') "
                f"FROM businesses WHERE id='{plain}'")
    check("a listing with no imported reputation still reads its own single review (1|4.0)",
          agg2 == "1|4.0", f"agg={agg2}")

    # ── teardown ────────────────────────────────────────────────────────────────
    psql("DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    for vid in created["visitors"]:
        psql(f"DELETE FROM visitor_accounts WHERE id='{vid}'")
    bl = ",".join(f"'{b}'" for b in created["businesses"]) or "NULL"
    rl = ",".join(f"'{r}'" for r in created["reviews"]) or "NULL"
    left = psql(f"SELECT (SELECT count(*) FROM businesses WHERE probe_harness='{MARK}')"
                f"+ (SELECT count(*) FROM visitor_accounts WHERE email IN ('{owner_email}','{cust_email}'))"
                f"+ (SELECT count(*) FROM claimed_businesses WHERE business_id IN ({bl}))"
                f"+ (SELECT count(*) FROM reviews WHERE id IN ({rl}))")
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
