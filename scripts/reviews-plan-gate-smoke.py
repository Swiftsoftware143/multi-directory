#!/usr/bin/env python3
"""Item 10 (plan capability switches) — prove the "Review responses" plan switch is ENFORCED.

David's rule (entitlements.rs / kanban B113): *every gate must be enforced in the back end*. The
plan editor has always shown a "Review responses" switch, but nothing read it: a business whose plan
turned review responses OFF could still reply to reviews. This probe proves the gate now bites:

  A. a business with an ACTIVE paid plan whose `has_reviews = false` is REFUSED (403) when it replies
  B. the same business, with the subscription removed, IS allowed (200) — i.e. the ~4,000
     unsubscribed listings are never limited and live behaviour is unchanged
  C. the refusal names the plan, so the owner knows why

Everything is minted through the real product routes; the fixture is deleted at the end and the
probe refuses to pass unless the residue count is 0.

Usage: python3 scripts/reviews-plan-gate-smoke.py     (env MD_BASE overrides http://127.0.0.1:8089)
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
MARK = "zz-plan-gate"
FAILS = 0


def sh(*args):
    return subprocess.run(args, capture_output=True, text=True)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


PW_DB = db_password()


def psql(sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW_DB}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    out = r.stdout.strip()
    # `-c` also prints the command tag ("INSERT 0 1") for a RETURNING insert; keep the value line.
    return out.splitlines()[0] if out else ""


def check(name, ok, detail=""):
    global FAILS
    print(("PASS  " if ok else "FAIL  ") + name + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS += 1


def http(method, path, body=None, token=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            raw = r.read().decode(errors="replace")
            try:
                return r.status, json.loads(raw)
            except Exception:
                return r.status, raw
    except urllib.error.HTTPError as e:
        raw = e.read().decode(errors="replace")
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, raw


def cleanup():
    psql("DELETE FROM business_subscriptions WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql("DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{MARK}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{MARK}'")
    psql("DELETE FROM plan_tiers WHERE slug LIKE 'zz-plan-gate-%'")
    psql("DELETE FROM visitor_accounts WHERE email LIKE 'zz-plan-gate-%@swiftsoftware.dev'")


def main():
    cleanup()
    ts = int(time.time())
    pw = "Zz-Plan-Gate-Pw-123!"
    owner_email = f"zz-plan-gate-owner-{ts}@swiftsoftware.dev"

    st, _ = http("POST", "/api/v1/visitor/register",
                 {"email": owner_email, "password": pw, "name": "ZZ Plan Gate Owner"})
    check("the owner account can be created (POST /visitor/register)", st in (200, 201), f"status={st}")
    owner_vid = psql(f"SELECT id FROM visitor_accounts WHERE lower(email)='{owner_email}'")
    check("the owner account exists in the database", bool(owner_vid), f"id={owner_vid[:8]}")

    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, owner_id, "
        "rating, review_count, probe_harness) VALUES "
        f"('{directory_id}', 'ZZ Plan Gate Biz', 'zz-plan-gate-biz-{ts}', 'Palm Bay', 'FL', "
        f"'active', '{owner_vid}', 5, 1, '{MARK}') RETURNING id")
    check("disposable listing created", bool(bid), f"business={bid[:8]}")

    rid = psql(
        "INSERT INTO reviews (business_id, rating, status, reviewer_name) VALUES "
        f"('{bid}', 5, 'approved', 'ZZ Plan Gate Customer') RETURNING id")
    check("a review exists on the listing", bool(rid), f"review={rid[:8]}")

    st, tok = http("POST", "/api/v1/auth/login", {"email": owner_email, "password": pw})
    owner_token = (tok or {}).get("access_token") if isinstance(tok, dict) else None
    check("the owner can sign in", st == 200 and bool(owner_token), f"status={st}")

    # ---- A. a plan with review responses turned OFF must refuse the reply --------------------
    tier_id = psql(
        "INSERT INTO plan_tiers (name, slug, has_reviews) VALUES "
        f"('ZZ Plan Gate Tier', 'zz-plan-gate-{ts}', false) RETURNING id")
    sub_id = psql(
        "INSERT INTO business_subscriptions (id, business_id, tier_id, status, start_date) VALUES "
        f"(gen_random_uuid(), '{bid}', '{tier_id}', 'active', CURRENT_DATE) RETURNING id")
    check("an ACTIVE paid plan (review responses off) is attached", bool(sub_id), f"sub={sub_id[:8]}")

    st, body = http("POST", f"/api/v1/business/reviews/{rid}/respond",
                    {"response": "Thanks for the kind words!"}, token=owner_token)
    detail = json.dumps(body)[:120] if not isinstance(body, str) else body[:120]
    check("A) a plan with review responses OFF refuses the reply (403)", st == 403, f"status={st}")
    check("A) the refusal names the plan limit",
          st == 403 and "plan" in detail.lower(), detail)

    stored = psql(f"SELECT COALESCE(owner_response,'<none>') FROM reviews WHERE id='{rid}'")
    check("A) no reply was written to the row", stored == "<none>", f"owner_response={stored}")

    # ---- B. unsubscribed (no active plan) is NOT limited — live behaviour unchanged -----------
    psql(f"DELETE FROM business_subscriptions WHERE id='{sub_id}'")
    st, body = http("POST", f"/api/v1/business/reviews/{rid}/respond",
                    {"response": "Thanks for the kind words!"}, token=owner_token)
    check("B) with no active plan the same reply is ALLOWED (200)", st == 200, f"status={st}")
    stored = psql(f"SELECT COALESCE(owner_response,'<none>') FROM reviews WHERE id='{rid}'")
    check("B) the reply was written to the row", "Thanks" in stored, f"owner_response={stored[:40]}")

    cleanup()
    left = psql("SELECT (SELECT count(*) FROM businesses WHERE probe_harness='" + MARK + "')"
                "+ (SELECT count(*) FROM visitor_accounts WHERE email LIKE 'zz-plan-gate-%')"
                "+ (SELECT count(*) FROM plan_tiers WHERE slug LIKE 'zz-plan-gate-%')"
                "+ (SELECT count(*) FROM business_subscriptions WHERE business_id='" + bid + "')")
    check("probe data fully removed (residue 0)", left == "0", f"remaining={left}")

    print()
    print(f"RESULT: {'PASS' if FAILS == 0 else 'FAIL (' + str(FAILS) + ')'}")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
