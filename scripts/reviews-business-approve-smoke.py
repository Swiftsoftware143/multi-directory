#!/usr/bin/env python3
"""Business-side review moderation — live end-to-end proof (REVIEWS item, ZaarHub).

Proves, with real API calls + real rows, that a listing owner can see a pending review
and approve it, that the approval flips the row, recomputes the business aggregates, makes
the review publicly visible, and that a STRANGER cannot approve it.

Everything is created through the public endpoints (no direct writes for the flow itself):
  1. POST /api/v1/visitor/register   -> owner account   (real visitor_account + password)
  2. POST /api/v1/businesses/:id/claim -> owner claims the listing (public claim route)
  3. POST /api/v1/reviews            -> a customer leaves a review (lands status='pending')
  4. owner signs in (POST /api/v1/auth/login, visitor_account fallback branch)
  5. GET  /api/v1/business/reviews/pending          -> the pending review is listed
  6. POST /api/v1/business/reviews/:id/approve      -> owner approves
  7. anonymous GET /api/v1/reviews?business_id=...  -> review is publicly visible
  8. a different account POSTing approve            -> 403 (owner scoping holds)

The only direct DB writes are the probe's disposable business row and teardown.
Run:  python3 scripts/reviews-business-approve-smoke.py
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
HARNESS = {"X-Swift-Harness": "review-approve-probe"}
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


def psql(sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    # -At still prints the command tag after a RETURNING result; drop it.
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    return lines[0] if lines else ""


def sweep_stale_probe_rows():
    """A previous run that died before teardown must not leave junk behind."""
    psql("DELETE FROM reviews WHERE business_id IN "
         "(SELECT id FROM businesses WHERE probe_harness='review-approve-probe')")
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         "(SELECT id FROM businesses WHERE probe_harness='review-approve-probe')")
    psql("DELETE FROM businesses WHERE probe_harness='review-approve-probe'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-review-%@swiftsoftware.dev'")


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
    owner_email = f"zz-review-owner-{ts}@swiftsoftware.dev"
    other_email = f"zz-review-other-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    slug = f"zz-review-probe-{ts}"
    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    created = {"visitors": [], "business": None}

    # ── 1. the owner's real account (public signup) ──────────────────────────────
    st, body = http("POST", "/api/v1/visitor/register",
                    {"email": owner_email, "password": pw, "name": "ZZ Probe Owner"})
    check("owner account can be created (POST /visitor/register) 201/200", st in (200, 201), f"status={st}")
    st2, _ = http("POST", "/api/v1/visitor/register",
                  {"email": other_email, "password": pw, "name": "ZZ Probe Other"})
    check("second account can be created", st2 in (200, 201), f"status={st2}")
    for em in (owner_email, other_email):
        vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{em}'")
        if vid:
            created["visitors"].append(vid)
    check("both accounts exist in the database", len(created["visitors"]) == 2,
          f"found={len(created['visitors'])}")

    # ── 2. a disposable listing the owner will claim ─────────────────────────────
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Review Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'review-approve-probe') RETURNING id")
    created["business"] = bid
    check("disposable listing created", bool(bid))

    st, body = http("POST", f"/api/v1/businesses/{bid}/claim",
                    {"owner_email": owner_email, "owner_name": "ZZ Probe Owner",
                     "owner_phone": "4075550100"}, extra_headers=HARNESS)
    check("owner can claim the listing (POST /businesses/:id/claim)", st in (200, 201), f"status={st}")

    # ── 3. a customer leaves a review (lands pending) ────────────────────────────
    st, tok_other = http("POST", "/api/v1/auth/login", {"email": other_email, "password": pw})
    check("second account can sign in", st == 200 and "access_token" in (tok_other or {}), f"status={st}")
    other_token = (tok_other or {}).get("access_token")

    st, rev = http("POST", "/api/v1/reviews",
                   {"business_id": bid, "rating": 5, "title": "Great service",
                    "content": "Probe review left by a customer."}, token=other_token)
    check("a customer review can be created", st in (200, 201), f"status={st}")
    rid = (rev or {}).get("id") if isinstance(rev, dict) else None
    check("the new review starts as pending", (rev or {}).get("status") == "pending",
          f"status={(rev or {}).get('status')}")

    # ── 4. the OWNER signs in ────────────────────────────────────────────────────
    st, tok_owner = http("POST", "/api/v1/auth/login", {"email": owner_email, "password": pw})
    check("the listing owner can sign in", st == 200 and "access_token" in (tok_owner or {}), f"status={st}")
    owner_token = (tok_owner or {}).get("access_token")

    # ── 5. the pending queue shows it ────────────────────────────────────────────
    st, pend = http("GET", "/api/v1/business/reviews/pending", token=owner_token)
    ids = [r.get("id") for r in ((pend or {}).get("data") or [])] if isinstance(pend, dict) else []
    check("the owner's pending list returns 200", st == 200, f"status={st}")
    check("the pending review is listed for the owner", rid in ids, f"ids={len(ids)}")

    # the owner's review feed (what business-portal.html renders Approve on)
    st, feed = http("GET", f"/api/v1/reviews?business_id={bid}&per_page=100", token=owner_token)
    feed_ids = [r.get("id") for r in ((feed or {}).get("data") or [])] if isinstance(feed, dict) else []
    check("the owner's review feed shows the pending review (Approve button source)",
          rid in feed_ids, f"status={st} ids={len(feed_ids)}")

    # ── 6. a stranger cannot approve ─────────────────────────────────────────────
    st, _ = http("POST", f"/api/v1/business/reviews/{rid}/approve", {}, token=other_token)
    check("a different account CANNOT approve it (403)", st == 403, f"status={st}")

    # ── 7. the owner approves ────────────────────────────────────────────────────
    st, appr = http("POST", f"/api/v1/business/reviews/{rid}/approve", {}, token=owner_token)
    check("the owner CAN approve the review (200)", st == 200, f"status={st}")
    check("the approved review is returned as approved",
          isinstance(appr, dict) and appr.get("status") == "approved",
          f"status={(appr or {}).get('status') if isinstance(appr, dict) else appr}")

    row = psql(f"SELECT status FROM reviews WHERE id='{rid}'")
    check("the database row is approved", row == "approved", f"row={row}")

    agg = psql(f"SELECT review_count||'|'||(rating=5) FROM businesses WHERE id='{bid}'")
    check("the business rating/review_count recomputed", agg == "1|true", f"agg={agg}")

    # ── 8. it is publicly visible ────────────────────────────────────────────────
    st, pub = http("GET", f"/api/v1/reviews?business_id={bid}")
    pub_ids = [r.get("id") for r in ((pub or {}).get("data") or [])] if isinstance(pub, dict) else []
    check("the approved review is visible to a logged-out visitor", rid in pub_ids,
          f"status={st} ids={len(pub_ids)}")

    # ── teardown ────────────────────────────────────────────────────────────────
    psql(f"DELETE FROM reviews WHERE business_id='{bid}'")
    psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    for vid in created["visitors"]:
        psql(f"DELETE FROM visitor_accounts WHERE id='{vid}'")
    left = psql(f"SELECT (SELECT count(*) FROM businesses WHERE id='{bid}')"
                f"+ (SELECT count(*) FROM visitor_accounts WHERE email IN ('{owner_email}','{other_email}'))"
                f"+ (SELECT count(*) FROM claimed_businesses WHERE business_id='{bid}')")
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
