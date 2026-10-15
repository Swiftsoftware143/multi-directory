#!/usr/bin/env python3
"""Reviews (item 1 surface) — the reviewer's EMAIL must not be served to the public.

The defect class: `reviews.reviewer_email` is real PII (the signed-in customer's account
address, captured by `create_review`). The three public/any-account read paths all serialise
the WHOLE `reviews` row, so an anonymous visitor — or any free signup — reading an approved
review over the API receives the reviewer's email address. business-detail.html renders only
name/title/content; nothing on any public screen needs the address.

The code itself already states the intent (`reviews.rs`, the list-route moderation gate:
"read EVERY business's pending/rejected review text together with the reviewer's email" is
the hole it closes) — but only the STATUS was gated, never the email FIELD on a row that is
approved and therefore public.

This probe proves, on the live build:
  1. a review created through the real API really does carry the customer's account email;
  2. no read path hands that email to a logged-out visitor or to a stranger account —
     list, single-read and the per-business route alike;
  3. a moderator of that business (its owner) still sees the address, so the fix is scoped,
     not a blanket deletion of the column from the API.

Run:  python3 scripts/reviews-public-email-leak-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Nothing is left behind on either path.
"""
import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:8089"
HARNESS = {"X-Swift-Harness": "review-email-leak-probe"}
MARK = "review-email-leak-probe"
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


def rows_of(payload):
    if isinstance(payload, dict):
        return payload.get("data") or []
    if isinstance(payload, list):
        return payload
    return []


def email_seen(payload, rid):
    """Return the reviewer_email the API handed back for review `rid` ('' when absent)."""
    for r in rows_of(payload):
        if isinstance(r, dict) and r.get("id") == rid:
            return r.get("reviewer_email") or ""
    if isinstance(payload, dict) and payload.get("id") == rid:
        return payload.get("reviewer_email") or ""
    return ""


def raw_exposes(payload, target):
    """Fallback sweep: the address must not appear anywhere in the serialised response."""
    return bool(target) and target.lower() in json.dumps(payload).lower()


def sweep():
    psql(f"DELETE FROM reviews WHERE business_id IN (SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM claimed_businesses WHERE business_id IN (SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-reviewmail-%@swiftsoftware.dev'")


def main():
    sweep()
    ts = str(int(time.time()))
    owner_email = f"zz-reviewmail-owner-{ts}@swiftsoftware.dev"
    cust_email = f"zz-reviewmail-cust-{ts}@swiftsoftware.dev"
    stranger_email = f"zz-reviewmail-stranger-{ts}@swiftsoftware.dev"
    pw = "ProbePass123!"
    slug = f"zz-reviewmail-probe-{ts}"
    directory = psql("SELECT id||'|'||slug FROM directories ORDER BY name LIMIT 1")
    directory_id, directory_slug = directory.split("|") if "|" in directory else ("", "")
    accounts = []

    for em, nm in ((owner_email, "ZZ Mail Probe Owner"), (cust_email, "ZZ Mail Probe Customer"),
                   (stranger_email, "ZZ Mail Probe Stranger")):
        st, _ = http("POST", "/api/v1/visitor/register",
                     {"email": em, "password": pw, "name": nm})
        check(f"account can be created ({nm})", st in (200, 201), f"status={st}")
        vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{em}'")
        if vid:
            accounts.append(vid)

    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, review_count, probe_harness) "
        f"VALUES ('{directory_id}', 'ZZ Review Mail Probe {ts}', '{slug}', 'Orlando', 'FL', 'active', 0, 0, "
        f"'{MARK}') RETURNING id")
    check("disposable listing created", bool(bid))

    st, _ = http("POST", f"/api/v1/businesses/{bid}/claim",
                 {"owner_email": owner_email, "owner_name": "ZZ Mail Probe Owner",
                  "owner_phone": "4075550100"}, extra_headers=HARNESS)
    check("the owner claims the listing", st in (200, 201), f"status={st}")

    st, tok_cust = http("POST", "/api/v1/auth/login", {"email": cust_email, "password": pw})
    cust_token = (tok_cust or {}).get("access_token") if isinstance(tok_cust, dict) else None
    check("the customer can sign in", st == 200 and bool(cust_token), f"status={st}")

    st, rev = http("POST", "/api/v1/reviews",
                   {"business_id": bid, "rating": 5, "title": "Great service",
                    "content": "Probe review for the public-email check."}, token=cust_token)
    rid = (rev or {}).get("id") if isinstance(rev, dict) else None
    check("the customer's review is accepted", st in (200, 201) and bool(rid), f"status={st}")

    stored = psql(f"SELECT coalesce(reviewer_email,'') FROM reviews WHERE id='{rid}'")
    check("the stored review really carries the customer's address (real PII)",
          stored.lower() == cust_email.lower(), f"stored={stored}")

    st, tok_owner = http("POST", "/api/v1/auth/login", {"email": owner_email, "password": pw})
    owner_token = (tok_owner or {}).get("access_token") if isinstance(tok_owner, dict) else None
    st, _ = http("POST", f"/api/v1/business/reviews/{rid}/approve", {}, token=owner_token)
    check("the owner approves it (it is public from here on)",
          st == 200 and psql(f"SELECT status FROM reviews WHERE id='{rid}'") == "approved", f"status={st}")

    st, tok_str = http("POST", "/api/v1/auth/login", {"email": stranger_email, "password": pw})
    stranger_token = (tok_str or {}).get("access_token") if isinstance(tok_str, dict) else None
    check("the stranger can sign in", st == 200 and bool(stranger_token), f"status={st}")

    # ── the leak arms ────────────────────────────────────────────────────────────
    st, anon = http("GET", f"/api/v1/reviews?business_id={bid}&status=approved")
    check("logged-out: the list route serves the approved review", st == 200 and bool(rows_of(anon)), f"status={st}")
    check("logged-out: the list route does NOT serve the reviewer's address",
          email_seen(anon, rid) == "" and not raw_exposes(anon, cust_email),
          f"leaked={email_seen(anon, rid) or 'none'}")

    st, anon_all = http("GET", "/api/v1/reviews?per_page=50")
    check("logged-out: the platform feed does NOT carry the address anywhere",
          not raw_exposes(anon_all, cust_email), f"status={st}")

    st, stranger = http("GET", f"/api/v1/reviews?business_id={bid}&status=approved", token=stranger_token)
    check("a stranger account: the list route does NOT serve the address",
          not raw_exposes(stranger, cust_email), f"status={st} leaked={email_seen(stranger, rid) or 'none'}")

    st, one = http("GET", f"/api/v1/reviews/{rid}", token=stranger_token)
    check("a stranger account: the single-read route does NOT serve the address",
          not raw_exposes(one, cust_email), f"status={st} leaked={email_seen(one, rid) or 'none'}")

    st, anon_one = http("GET", f"/api/v1/reviews/{rid}")
    check("logged-out: the single-read route does NOT serve the address",
          not raw_exposes(anon_one, cust_email), f"status={st}")

    st, per_biz = http("GET", f"/api/v1/directories/{directory_slug}/businesses/{bid}/reviews",
                       token=stranger_token)
    check("a stranger account: the per-business route does NOT serve the address",
          not raw_exposes(per_biz, cust_email), f"status={st} leaked={email_seen(per_biz, rid) or 'none'}")

    # ── the moderator arm: the owner must still see it (scoped, not blanket) ─────
    st, own = http("GET", f"/api/v1/reviews?business_id={bid}&per_page=100", token=owner_token)
    check("the business owner (moderator) still sees the reviewer's address",
          email_seen(own, rid).lower() == cust_email.lower(),
          f"owner_view={email_seen(own, rid) or 'none'}")

    # ── teardown ────────────────────────────────────────────────────────────────
    psql(f"DELETE FROM reviews WHERE business_id='{bid}'")
    psql(f"DELETE FROM claimed_businesses WHERE business_id='{bid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    for vid in accounts:
        psql(f"DELETE FROM visitor_accounts WHERE id='{vid}'")
    left = psql("SELECT (SELECT count(*) FROM businesses WHERE probe_harness='" + MARK + "')"
                " + (SELECT count(*) FROM visitor_accounts WHERE email LIKE 'zz-reviewmail-%')")
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
