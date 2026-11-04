#!/usr/bin/env python3
"""Item 10 proof — a deal is only public (and only redeemable) inside its validity window.

Before the fix every public deals list ignored start/end dates: 15 featured deals whose
`end_date` had passed months ago were still served, still showed "Valid until <past date>", and
claim/redeem accepted them. This proves both directions with a real deal row and real HTTP.
"""
import json, os, re, subprocess, sys, urllib.request, urllib.error, uuid, datetime

BASE = os.environ.get("MD_BASE_URL", "http://127.0.0.1:8089")
OUT = os.path.dirname(os.path.abspath(__file__))
fails = []
def chk(name, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + name + ("  [" + str(detail) + "]" if detail != "" else ""))
    if not ok: fails.append(name)

url = subprocess.check_output(["docker","exec","multidirectory","printenv","DATABASE_URL"], text=True).strip()
PW = re.match(r"postgres://[^:]+:([^@]+)@", url).group(1)
def sql(q):
    r = subprocess.run(["docker","exec","-i","-e","PGPASSWORD="+PW,"swift-postgres","psql","-U","swift",
                        "-d","multi_directory","-tAc",q], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(r.stderr)
    return r.stdout.strip()

def http(path, method="GET"):
    req = urllib.request.Request(BASE + path, method=method,
                                 data=b"{}" if method == "POST" else None,
                                 headers={"Content-Type":"application/json"})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            body = r.read().decode()
            return r.status, (json.loads(body) if body else None)
    except urllib.error.HTTPError as e:
        body = e.read().decode()
        try: body = json.loads(body)
        except Exception: pass
        return e.code, body

DEAL = str(uuid.uuid4())
TITLE = "ZZ item10 expiry probe " + DEAL[:8]
biz = sql("select b.id from businesses b join directories d on d.id=b.directory_id where d.slug='palm-bay' limit 1")
dirc = sql("select id from directories where slug='palm-bay'")

def lists():
    _, feat = http("/api/v1/deals/featured")
    _, zd = http("/api/v1/zaarhub/deals?limit=100")
    _, home = http("/api/v1/zaarhub/homepage")
    _, city = http("/api/v1/zaarhub/cities/palm-bay")
    _, bd = http("/api/v1/zaarhub/business/palm-bay/" + biz)
    zd_ids = [d.get("id") for d in (zd or {}).get("deals", [])] if isinstance(zd, dict) else [d.get("id") for d in (zd or [])]
    home_ids = [d.get("id") for d in (home or {}).get("featured_deals", [])] if isinstance(home, dict) else []
    city_ids = [d.get("id") for d in (city or {}).get("active_deals", [])] if isinstance(city, dict) else []
    bd_ids = [d.get("id") for d in (bd or {}).get("deals", [])] if isinstance(bd, dict) else []
    return {
        "featured": [d.get("id") for d in (feat or [])],
        "zaarhub_deals": zd_ids,
        "homepage": home_ids,
        "city": city_ids,
        "business": bd_ids,
    }

try:
    before = lists()
    expired_featured_before = sql("select count(*) from deals where featured=true and status='active' and end_date < now()")
    chk("every featured deal in the catalog is already past its end date (the defect this pins)",
        int(expired_featured_before) > 0, expired_featured_before)
    chk("the public featured feed no longer serves those ended deals",
        "/api/v1/deals/featured" and len(before["featured"]) == 0,
        "rows=" + str(len(before["featured"])))

    sql("""INSERT INTO deals (id,title,description,deal_price,original_price,discount_percent,status,
            directory_id,business_id,start_date,end_date,featured,zaarhub_featured,is_active)
           VALUES ('%s','%s','probe',  '$5','$10',50,'active',
            '%s','%s', now() - interval '1 day', now() + interval '30 days', true,true,true)"""
        % (DEAL, TITLE, dirc, biz))
    chk("a fresh deal inside its window was created in the database", sql("select count(*) from deals where id='%s'" % DEAL) == "1")

    vis = lists()
    for surface, key in [("featured feed (/deals/featured)","featured"),
                         ("network deals list (/zaarhub/deals)","zaarhub_deals"),
                         ("homepage featured deals","homepage"),
                         ("city page deals","city"),
                         ("business page deals","business")]:
        chk("the valid deal appears in " + surface, DEAL in vis[key], "found=" + str(DEAL in vis[key]))

    st, body = http("/api/v1/deals/%s/claim" % DEAL, "POST")
    chk("a valid deal can be claimed", st == 200, "status=" + str(st))
    chk("the claim was recorded on the deal", sql("select redemption_count from deals where id='%s'" % DEAL) == "1")
    st, body = http("/api/v1/deals/%s/redeem" % DEAL, "POST")
    chk("a valid deal mints a redemption code", st == 200 and (body or {}).get("redemption_code"), "status=" + str(st))
    chk("the redemption is stored against the real business",
        sql("select count(*) from deal_redemptions where deal_id='%s' and business_id is not null" % DEAL) == "1")

    # Now end the deal and re-check every surface + both write paths.
    sql("update deals set end_date = now() - interval '1 day' where id='%s'" % DEAL)
    gone = lists()
    for surface, key in [("featured feed (/deals/featured)","featured"),
                         ("network deals list (/zaarhub/deals)","zaarhub_deals"),
                         ("homepage featured deals","homepage"),
                         ("city page deals","city"),
                         ("business page deals","business")]:
        chk("the ended deal is NOT served in " + surface, DEAL not in gone[key], "served=" + str(DEAL in gone[key]))

    redemptions_before = sql("select count(*) from deal_redemptions where deal_id='%s'" % DEAL)
    counter_before = sql("select redemption_count from deals where id='%s'" % DEAL)
    st, body = http("/api/v1/deals/%s/claim" % DEAL, "POST")
    chk("claiming an ended deal is refused", st == 400, "status=%s msg=%s" % (st, (body or {}).get("message")))
    chk("the refused claim did not bump the counter", sql("select redemption_count from deals where id='%s'" % DEAL) == counter_before, "before=%s after=%s" % (counter_before, sql("select redemption_count from deals where id='%s'" % DEAL)))
    st, body = http("/api/v1/deals/%s/redeem" % DEAL, "POST")
    chk("redeeming an ended deal is refused", st == 400, "status=%s msg=%s" % (st, (body or {}).get("message")))
    chk("the refused redeem created no redemption row",
        sql("select count(*) from deal_redemptions where deal_id='%s'" % DEAL) == redemptions_before)

    st, body = http("/api/v1/deals/%s/claim" % uuid.uuid4(), "POST")
    chk("claiming a deal id that does not exist is a clean 404", st == 404, "status=" + str(st))
    st, body = http("/api/v1/deals/%s/redeem" % uuid.uuid4(), "POST")
    chk("redeeming a deal id that does not exist is a clean 404", st == 404, "status=" + str(st))
    chk("no orphan redemption row was created by the bogus ids",
        sql("select count(*) from deal_redemptions where deal_id not in (select id from deals)") == "0")
finally:
    sql("delete from deal_redemptions where deal_id='%s'" % DEAL)
    sql("delete from deals where id='%s'" % DEAL)
    chk("probe rows removed", sql("select count(*) from deals where id='%s'" % DEAL) == "0")
    after = lists()
    chk("the public lists are back to their pre-test state", after == before,
        "before=%s after=%s" % ({k: len(v) for k, v in before.items()}, {k: len(v) for k, v in after.items()}))

print()
print("RESULT: " + ("PASS" if not fails else "FAIL (" + ", ".join(fails) + ")"))
sys.exit(0 if not fails else 1)
