#!/usr/bin/env python3
"""Plan-tier payment provider — set, read back, clear, read back (ZaarHub, item 10/16 prep).

The portal's tier form lets the operator pick a gateway (Stripe / PayPal / NMI / EPD /
Square / Paddle, or "Not set — use the active gateway"). This proves the API behind that
control actually persists a choice AND can clear it again, which the update handler used to
silently refuse (a null was treated as "field omitted", so "Not set" was a no-op).

Runs against the live app and the live database, and restores the row to its exact
before-state (net-zero). Exit 0 = PASS.

Run: python3 scripts/plan-tier-provider-roundtrip-smoke.py
"""
import json
import os
import subprocess
import sys
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
TOKEN = os.environ.get("MD_TOKEN") or subprocess.check_output(
    ["node", "/opt/swift/browser-run/md-mint-token.js"], text=True
).strip()

out = []


def rec(ok, name, detail=""):
    out.append(("PASS " if ok else "FAIL ") + name + ((" :: " + str(detail)) if detail else ""))


def call(method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(BASE + "/api/v1" + path, data=data, method=method)
    req.add_header("Authorization", "Bearer " + TOKEN)
    req.add_header("Accept", "application/json")
    if data:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=20) as r:
        return r.status, json.loads(r.read().decode())


def db(sql):
    url = subprocess.check_output(["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"], text=True).strip()
    url = url.replace("localhost", "127.0.0.1")
    return subprocess.check_output(
        ["docker", "exec", "-e", "PGURL=" + url, "swift-postgres", "bash", "-lc",
         'psql "$PGURL" -tAc ' + json.dumps(sql)],
        text=True,
    ).strip()


def main():
    _, tiers = call("GET", "/monetization/tiers")
    rows = tiers if isinstance(tiers, list) else tiers.get("data", [])
    rec(len(rows) > 0, "the live tiers are readable", "n=%d" % len(rows))
    tier = next((t for t in rows if t.get("name") == "Featured"), rows[0])
    tid = tier["id"]
    before = tier.get("payment_provider")
    before_name = tier.get("name")
    rec(before is None, "the probe tier starts with no provider", before)

    try:
        st, saved = call("PUT", "/monetization/tiers/" + tid, {"payment_provider": "nmi"})
        rec(st == 200, "saving NMI on a tier returns 200", st)
        rec(saved.get("payment_provider") == "nmi", "the API echoes NMI", saved.get("payment_provider"))
        rowval = db("select coalesce(payment_provider,'(null)') from plan_tiers where id='%s';" % tid)
        rec(rowval == "nmi", "the database row holds nmi", rowval)
        rec(saved.get("name") == before_name, "the save did not disturb the other fields", saved.get("name"))

        st, cleared = call("PUT", "/monetization/tiers/" + tid, {"payment_provider": None})
        rec(st == 200, "clearing the provider returns 200", st)
        rec(cleared.get("payment_provider") is None, "the API echoes no provider", cleared.get("payment_provider"))
        rowval = db("select coalesce(payment_provider,'(null)') from plan_tiers where id='%s';" % tid)
        rec(rowval == "(null)", "the database row is cleared", rowval)
        rec(cleared.get("name") == before_name, "the clear did not disturb the other fields", cleared.get("name"))

        # An omitted field must still mean "leave it alone" — the opposite of clearing.
        call("PUT", "/monetization/tiers/" + tid, {"payment_provider": "epd"})
        st, kept = call("PUT", "/monetization/tiers/" + tid, {"price_monthly": float(tier.get("price_monthly") or 0)})
        rec(kept.get("payment_provider") == "epd", "a save that omits the provider keeps it", kept.get("payment_provider"))
    finally:
        st, restored = call("PUT", "/monetization/tiers/" + tid, {"payment_provider": before})
        rec(st == 200 and restored.get("payment_provider") == before, "the probe tier is restored", restored.get("payment_provider"))
        final = db("select coalesce(payment_provider,'(null)') from plan_tiers where id='%s';" % tid)
        rec(final == "(null)", "net-zero: no provider left behind", final)

    print("\n".join(out))
    failed = sum(1 for l in out if l.startswith("FAIL"))
    print("\nRESULT: " + ("PASS" if failed == 0 else "FAIL (%d)" % failed))
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
