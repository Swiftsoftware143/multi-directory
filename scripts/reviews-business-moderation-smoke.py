#!/usr/bin/env python3
"""Business-side review moderation, part 2 — REJECT + REPLY (REVIEWS item, ZaarHub).

`reviews-business-approve-smoke.py` proves the APPROVE half of the business-side review
surface. The product's own business console (`frontend/business-portal.html`) also renders a
**Reject** button on every review and a **public reply** box ("Post reply" / "Update reply" /
"Remove reply"), wired to `POST /api/v1/business/reviews/:id/reject` and
`POST /api/v1/reviews/:id/respond`. Neither had a durable end-to-end proof.

This probe proves, with real API calls + real rows, that the listing owner can:
  1. reject a pending review          -> row 'rejected', invisible publicly, aggregates UNCHANGED
                                        (a reject that changes nothing visible must not recompute —
                                         the guard that keeps an imported 4.8/120 listing intact)
  2. re-approve it (the reject is reversible) -> row 'approved', aggregate 1|5
  3. reject it again from the approved state (what the console offers) -> aggregates back to 0
  4. post a public reply              -> owner_response/owner_responded_at persisted, shown publicly
  5. update the reply                 -> the new text replaces the old
  6. remove the reply (empty body)    -> owner_response back to NULL
  7. a whitespace-only reply          -> treated as "remove" (trimmed), not stored

and that a STRANGER (any signed-in account that does not own the business) cannot reject a
review or post an owner reply on it (403) — the same ownership gate the approve path uses.

The only direct DB writes are the disposable probe listing and teardown.
Run:  python3 scripts/reviews-business-moderation-smoke.py
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
HARNESS = {"X-Swift-Harness": "review-moderation-probe"}
MARK = "review-moderation-probe"
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
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    return lines[0] if lines else ""


def sweep_stale_probe_rows():
    """A previous run that died before teardown must not leave junk behind."""
    psql("DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql("DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-revmod-%@swiftsoftware.dev'")


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


def public_ids(bid, query=""):
    st, pub = http("GET", f"/api/v1/reviews?business_id={bid}{query}")
    rows = ((pub or {}).get("data") or []) if isinstance(pub, dict) else []
    return st, rows


def main():
    sweep_stale_probe_rows()
    ts = str(int(time.time()))
    owner_email = f"zz-revmod-owner-{ts}@swiftsoftware.dev"
    cust_email = f"zz-revmod-cust-{ts}@swiftsoftware.dev"
    stranger_email = f"zz-revmod-stranger-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    slug = f"zz-revmod-probe-{ts}"
    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    created = {"visitors": [], "business": None}

    # ── accounts ────────────────────────────────────────────────────────────────
    for email, name in ((owner_email, "ZZ Revmod Owner"), (cust_email, "ZZ Revmod Customer"),
                        (stranger_email, "ZZ Revmod Stranger")):
        st, _ = http("POST", "/api/v1/visitor/register", {"email": email, "password": pw, "name": name})
        check(f"account can be created ({email.split('-')[2].split('@')[0]})", st in (200, 201), f"status={st}")
    for em in (owner_email, cust_email, stranger_email):
        vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{em}'")
        if vid:
            created["visitors"].append(vid)
    check("all three accounts exist in the database", len(created["visitors"]) == 3,
          f"found={len(created['visitors'])}")

    # ── disposable listing the owner claims ─────────────────────────────────────
    # rating is NULL, not 0: the app's own recompute writes NULL for "no approved reviews"
    # (see step 3 below), so NULL is how this schema represents "no rating". A no-op reject
    # (step 1) no longer recomputes at all, so this listing must already start at the state
    # the app would leave it in.
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Revmod Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', NULL, 0, "
        f"'{MARK}') RETURNING id")
    created["business"] = bid
    check("disposable listing created", bool(bid))

    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ Revmod Owner",
                  "owner_phone": "4075550101"}, extra_headers=HARNESS)
    check("owner can claim the listing (POST /businesses/:id/claim)", st in (200, 201), f"status={st}")

    # ── a customer leaves a review (lands pending) ──────────────────────────────
    st, tok_cust = http("POST", "/api/v1/auth/login", {"email": cust_email, "password": pw})
    check("customer can sign in", st == 200 and "access_token" in (tok_cust or {}), f"status={st}")
    cust_token = (tok_cust or {}).get("access_token")

    st, rev = http("POST", "/api/v1/reviews",
                   {"business_id": bid, "rating": 5, "title": "Reject/reply probe",
                    "content": "Left by the moderation probe customer."}, token=cust_token)
    check("a customer review can be created", st in (200, 201), f"status={st}")
    rid = (rev or {}).get("id") if isinstance(rev, dict) else None
    check("the new review starts as pending", (rev or {}).get("status") == "pending",
          f"status={(rev or {}).get('status')}")

    # ── sign the owner and a stranger in ────────────────────────────────────────
    st, tok_owner = http("POST", "/api/v1/auth/login", {"email": owner_email, "password": pw})
    check("the listing owner can sign in", st == 200 and "access_token" in (tok_owner or {}), f"status={st}")
    owner_token = (tok_owner or {}).get("access_token")
    st, tok_str = http("POST", "/api/v1/auth/login", {"email": stranger_email, "password": pw})
    check("the stranger can sign in", st == 200 and "access_token" in (tok_str or {}), f"status={st}")
    stranger_token = (tok_str or {}).get("access_token")

    # ── A stranger cannot reject or reply ───────────────────────────────────────
    st, _ = http("POST", f"/api/v1/business/reviews/{rid}/reject", {}, token=stranger_token)
    check("a stranger CANNOT reject the review (403)", st == 403, f"status={st}")
    st, _ = http("POST", f"/api/v1/business/reviews/{rid}/reject", {}, token=cust_token)
    check("the customer CANNOT reject their own review (403)", st == 403, f"status={st}")
    st, _ = http("POST", f"/api/v1/reviews/{rid}/respond", {"response": "STRANGER REPLY"}, token=stranger_token)
    check("a stranger CANNOT post an owner reply (403)", st == 403, f"status={st}")
    row = psql(f"SELECT status||'|'||COALESCE(owner_response,'NULL') FROM reviews WHERE id='{rid}'")
    check("neither attempt changed the row", row == "pending|NULL", f"row={row}")

    # ── 1. the OWNER rejects ────────────────────────────────────────────────────
    st, rej = http("POST", f"/api/v1/business/reviews/{rid}/reject", {}, token=owner_token)
    check("the owner CAN reject the review (200)", st == 200, f"status={st}")
    check("the rejected review is returned as rejected",
          isinstance(rej, dict) and rej.get("status") == "rejected",
          f"status={(rej or {}).get('status') if isinstance(rej, dict) else rej}")
    row = psql(f"SELECT status FROM reviews WHERE id='{rid}'")
    check("the database row is rejected", row == "rejected", f"row={row}")
    agg = psql(f"SELECT review_count||'|'||(rating IS NULL) FROM businesses WHERE id='{bid}'")
    check("the business aggregates stay at zero after a reject", agg == "0|true", f"agg={agg}")
    st, rows = public_ids(bid)
    check("a rejected review is NOT visible to a logged-out visitor",
          rid not in [r.get("id") for r in rows], f"status={st} ids={len(rows)}")
    st, pend = http("GET", "/api/v1/business/reviews/pending", token=owner_token)
    pend_ids = [r.get("id") for r in ((pend or {}).get("data") or [])] if isinstance(pend, dict) else []
    check("a rejected review leaves the owner's 'awaiting reply' list", rid not in pend_ids,
          f"ids={len(pend_ids)}")

    # ── 1b. a no-op reject must NOT destroy a listing's own (imported) rating ───
    # An imported listing carries a rating/review_count of its own with zero rows of ours behind
    # it (3,914 of 3,988 listings; measured 2026-10-15). Moderation that changes nothing visible
    # must leave those numbers ALONE — recomputing from our own (empty) reviews flipped 4.8/120
    # to NULL/0. This is the regression the step-1 rule above exists to prevent.
    imp_slug = f"{slug}-imported"
    imp_bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Revmod Imported Co {ts}', '{imp_slug}', 'Orlando', 'FL', 'active', 4.8, 120, "
        f"'{MARK}') RETURNING id")
    check("an imported-style listing (its own 4.8 / 120) created", bool(imp_bid))
    st, _ = http("POST", f"/api/v1/businesses/{imp_bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ Revmod Owner",
                  "owner_phone": "4075550101"}, extra_headers=HARNESS)
    check("the owner can claim the imported listing", st in (200, 201), f"status={st}")
    st, imp_rev = http("POST", "/api/v1/reviews",
                       {"business_id": imp_bid, "rating": 2, "title": "Imported-listing probe",
                        "content": "Left by the moderation probe customer."}, token=cust_token)
    imp_rid = (imp_rev or {}).get("id") if isinstance(imp_rev, dict) else None
    imp_status = imp_rev.get("status") if isinstance(imp_rev, dict) else None
    check("a second review lands pending on the imported listing",
          st in (200, 201) and imp_status == "pending", f"status={st}")
    st, _ = http("POST", f"/api/v1/business/reviews/{imp_rid}/reject", {}, token=owner_token)
    imp_agg = psql(f"SELECT review_count||'|'||rating FROM businesses WHERE id='{imp_bid}'")
    check("a no-op reject leaves the imported rating untouched (4.8 / 120)",
          st == 200 and imp_agg == "120|4.8", f"status={st} agg={imp_agg}")

    # ── 2. reject is reversible: the owner approves ─────────────────────────────
    st, appr = http("POST", f"/api/v1/business/reviews/{rid}/approve", {}, token=owner_token)
    check("the owner can approve after rejecting (reversible)", st == 200 and appr.get("status") == "approved",
          f"status={st}")
    agg = psql(f"SELECT review_count||'|'||(rating=5) FROM businesses WHERE id='{bid}'")
    check("the business aggregates recompute to 1|5 on approve", agg == "1|true", f"agg={agg}")
    st, rows = public_ids(bid)
    check("the approved review IS visible to a logged-out visitor",
          rid in [r.get("id") for r in rows], f"ids={len(rows)}")

    # ── 3. reject from the approved state (what the console offers) ─────────────
    st, _ = http("POST", f"/api/v1/business/reviews/{rid}/reject", {}, token=owner_token)
    agg = psql(f"SELECT review_count||'|'||(rating IS NULL) FROM businesses WHERE id='{bid}'")
    check("rejecting an approved review pulls the aggregate back to 0", st == 200 and agg == "0|true",
          f"status={st} agg={agg}")
    st, _ = http("POST", f"/api/v1/business/reviews/{rid}/approve", {}, token=owner_token)
    check("re-approved again (ready for the reply half)", st == 200, f"status={st}")

    # ── 4. the OWNER posts a public reply ───────────────────────────────────────
    reply = "Thanks for the kind words — see you again soon!"
    st, resp = http("POST", f"/api/v1/reviews/{rid}/respond", {"response": reply}, token=owner_token)
    check("the owner CAN post a public reply (200)", st == 200, f"status={st}")
    check("the reply is returned on the updated review",
          isinstance(resp, dict) and resp.get("owner_response") == reply,
          f"owner_response={(resp or {}).get('owner_response') if isinstance(resp, dict) else resp}")
    row = psql(f"SELECT owner_response||'|'||(owner_responded_at IS NOT NULL)||'|'||(owner_response_by IS NOT NULL) "
               f"FROM reviews WHERE id='{rid}'")
    check("the reply, its timestamp and its author are persisted", row == f"{reply}|true|true", f"row={row}")
    st, rows = public_ids(bid)
    shown = next((r.get("owner_response") for r in rows if r.get("id") == rid), None)
    check("the reply travels on the PUBLIC review payload", shown == reply, f"shown={shown}")
    st, pend = http("GET", "/api/v1/business/reviews/pending", token=owner_token)
    pend_ids = [r.get("id") for r in ((pend or {}).get("data") or [])] if isinstance(pend, dict) else []
    check("a replied review leaves the 'awaiting reply' list", rid not in pend_ids, f"ids={len(pend_ids)}")

    # ── 5. update the reply ─────────────────────────────────────────────────────
    reply2 = "Updated: apologies for the wait, and thank you."
    st, resp = http("POST", f"/api/v1/reviews/{rid}/respond", {"response": reply2}, token=owner_token)
    row = psql(f"SELECT owner_response FROM reviews WHERE id='{rid}'")
    check("the owner can UPDATE the reply in place", st == 200 and row == reply2, f"row={row}")

    # ── 6. remove the reply (empty body) ────────────────────────────────────────
    st, _ = http("POST", f"/api/v1/reviews/{rid}/respond", {"response": ""}, token=owner_token)
    row = psql(f"SELECT COALESCE(owner_response,'NULL')||'|'||(owner_responded_at IS NULL) FROM reviews WHERE id='{rid}'")
    check("an empty reply REMOVES the reply and its metadata", st == 200 and row == "NULL|true", f"row={row}")

    # ── 7. whitespace-only reply == remove (trimmed, never stored) ──────────────
    st, _ = http("POST", f"/api/v1/reviews/{rid}/respond", {"response": "   \n  "}, token=owner_token)
    row = psql(f"SELECT COALESCE(owner_response,'NULL') FROM reviews WHERE id='{rid}'")
    check("a whitespace-only reply is trimmed to nothing", st == 200 and row == "NULL", f"row={row}")

    # ── 8. over-long reply is refused, not silently stored ──────────────────────
    st, _ = http("POST", f"/api/v1/reviews/{rid}/respond", {"response": "x" * 4001}, token=owner_token)
    row = psql(f"SELECT COALESCE(owner_response,'NULL') FROM reviews WHERE id='{rid}'")
    check("a >4000-character reply is refused and nothing is stored",
          st in (400, 422) and row == "NULL", f"status={st} row={row}")

    # ── teardown ────────────────────────────────────────────────────────────────
    bids = [b for b in (bid, imp_bid) if b]
    revs = [r for r in (rid, imp_rid) if r]
    psql(f"DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM claimed_businesses WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    for vid in created["visitors"]:
        psql(f"DELETE FROM visitor_accounts WHERE id='{vid}'")
    bid_list = ",".join(f"'{b}'" for b in bids) or "NULL"
    rev_list = ",".join(f"'{r}'" for r in revs) or "NULL"
    left = psql(f"SELECT (SELECT count(*) FROM businesses WHERE probe_harness='{MARK}')"
                f"+ (SELECT count(*) FROM visitor_accounts WHERE email IN ('{owner_email}','{cust_email}','{stranger_email}'))"
                f"+ (SELECT count(*) FROM claimed_businesses WHERE business_id IN ({bid_list}))"
                f"+ (SELECT count(*) FROM reviews WHERE id IN ({rev_list}))")
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
