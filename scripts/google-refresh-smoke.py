#!/usr/bin/env python3
"""W25 — REFRESH FROM GOOGLE: prove the batched run against the RUNNING app.

What it proves (each line PASS/FAIL, no invented results):
  1. the status card answers for a city (schedule + counts + last run),
  2. a CLAIMED listing is never touched — its enriched_at does not move across a run,
  3. the UPDATE arm refreshes real unclaimed listings in place from Google: the same record
     (same id) carries the new data, its name is not rewritten, and Google's place id is stamped
     so the identity is durable,
  4. the DISCOVER arm adds only businesses that were not listed (dedupe by place id), and a
     SECOND run cannot create a second copy of any of them,
  5. the run is recorded (added/updated/unchanged/unreachable) so the panel can report it,
  6. the schedule is an admin setting that saves and reads back,
  7. nobody else can read a city's refresh state.

Run:  python3 scripts/google-refresh-smoke.py
Env:  MD_BASE (default http://127.0.0.1:8089)
"""
import json
import os
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
MINT = "/opt/swift/browser-run/md-mint-token.js"
PALM_BAY = "5c0fde84-4c32-4aa3-84f6-bf2ba0f51334"
ST_PETE = "4c71a30c-278a-4bbb-b0ec-e1d912f7c8f1"

results = []


def check(name, ok, detail=""):
    results.append((bool(ok), name, detail))
    print(("PASS  " if ok else "FAIL  ") + name + ((" | " + str(detail)) if detail else ""))
    return ok


def psql(sql):
    url = subprocess.run(
        ["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
        capture_output=True, text=True, check=True).stdout.strip()
    pwd = url.split("://", 1)[1].split(":", 1)[1].split("@", 1)[0]
    env = dict(os.environ, PGPASSWORD=pwd)
    out = subprocess.run(
        ["psql", "-h", "127.0.0.1", "-U", "swift", "-d", "multi_directory", "-Atc", sql],
        capture_output=True, text=True, env=env)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip()[:300])
    return out.stdout.strip()


def q1(sql):
    out = psql(sql)
    return out.splitlines()[0] if out else ""


def call(path, token=None, method="GET", body=None, timeout=300):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + "/api/v1" + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read().decode() or "{}")
    except urllib.error.HTTPError as e:
        raw = e.read().decode(errors="replace")
        try:
            return e.code, json.loads(raw or "{}")
        except Exception:
            return e.code, {"raw": raw[:200]}


def snapshot(directory_id):
    """id -> (name, phone, website, address, rating, lat, lng, reviews, place_id)."""
    raw = psql(
        "select id::text || E'\\t' || coalesce(name,'') || E'\\t' || coalesce(phone,'') || "
        "E'\\t' || coalesce(website,'') || E'\\t' || coalesce(address,'') || E'\\t' || "
        "coalesce(rating::text,'') || E'\\t' || coalesce(latitude::text,'') || E'\\t' || "
        "coalesce(longitude::text,'') || E'\\t' || coalesce(review_count::text,'') || E'\\t' || "
        "coalesce(place_id,'') from businesses "
        "where directory_id='%s' and not coalesce(claimed,false)" % directory_id)
    out = {}
    for line in raw.splitlines():
        parts = line.split("\t")
        if len(parts) == 10:
            out[parts[0]] = tuple(parts[1:])
    return out


def main():
    token = subprocess.run(["node", MINT], capture_output=True, text=True, check=True)
    token = token.stdout.strip().splitlines()[-1]
    check("operator token minted", len(token) > 100, f"len={len(token)}")

    # ── 1. status card ──────────────────────────────────────────────────────
    st, body = call(f"/zaarhub/admin/google-refresh/status?directory_id={PALM_BAY}", token)
    check("status answers for the city", st == 200 and "unclaimed" in body, f"HTTP {st}")
    check("status reports a schedule setting",
          body.get("schedule") in ("manual", "daily", "weekly", "monthly"), body.get("schedule"))
    unclaimed_before = int(body.get("unclaimed") or 0)
    check("status counts the city's unclaimed listings", unclaimed_before > 100,
          f"{unclaimed_before} unclaimed, {body.get('claimed')} claimed")

    # ── 2. a CLAIMED listing is never touched ───────────────────────────────
    claimed_id = q1("select id from businesses where claimed limit 1") or ""
    if claimed_id:
        before = q1("select coalesce(enriched_at::text,'NULL') from businesses where id='%s'" % claimed_id)
        st, run = call("/zaarhub/admin/google-refresh/run", token, "POST",
                       {"directory_id": ST_PETE, "update": True, "discover": False, "max_update": 2})
        check("a run over the city that holds the claimed listing succeeds", st == 200,
              f"HTTP {st} updated={run.get('updated')}")
        after = q1("select coalesce(enriched_at::text,'NULL') from businesses where id='%s'" % claimed_id)
        check("the claimed listing was NOT refreshed (the owner's data is untouched)",
              after == before, f"{before} -> {after}")
    else:
        check("a claimed listing exists to test the skip rule", False, "none in this install")

    # ── 3. UPDATE arm: real data, same record, identity stamped ─────────────
    before_snap = snapshot(PALM_BAY)
    before_count = int(q1("select count(*) from businesses where directory_id='%s'" % PALM_BAY))

    st, run = call("/zaarhub/admin/google-refresh/run", token, "POST",
                   {"directory_id": PALM_BAY, "update": True, "discover": True,
                    "max_update": 3, "max_add": 2})
    check("the refresh run answers for the city", st == 200 and run.get("success") is True,
          f"HTTP {st} {str(run.get('error') or '')[:160]}")
    print("      run summary: added=%s updated=%s unchanged=%s unreachable=%s skipped_claimed=%s notes=%s" % (
        run.get("added"), run.get("updated"), run.get("unchanged"), run.get("unreachable"),
        run.get("skipped_claimed"), (run.get("notes") or [])[:2]))

    updated = [s for s in (run.get("samples") or [])
               if s.get("result") == "updated" and s.get("business_id")]
    check("the update arm refreshed at least one listing from Google", len(updated) >= 1,
          f"{len(updated)} updated")

    same_record = True
    name_kept = True
    identity_stamped = 0
    field_changed = 0
    details = []
    for s in updated:
        bid = s["business_id"]
        row = q1("select name || E'\\t' || coalesce(phone,'') || E'\\t' || coalesce(website,'') || "
                 "E'\\t' || coalesce(address,'') || E'\\t' || coalesce(rating::text,'') || E'\\t' || "
                 "coalesce(latitude::text,'') || E'\\t' || coalesce(longitude::text,'') || E'\\t' || "
                 "coalesce(review_count::text,'') || E'\\t' || "
                 "coalesce(place_id,'') from businesses where id='%s' and directory_id='%s'"
                 % (bid, PALM_BAY))
        if not row:
            same_record = False
            continue
        name, phone, site, addr, rating, blat, blng, reviews, pid = row.split("\t")
        old = before_snap.get(bid, ("", "", "", "", "", "", "", "", ""))
        (old_name, old_phone, old_site, old_addr, old_rating, old_lat, old_lng, old_reviews,
         old_pid) = old
        if old_name and name != old_name:
            name_kept = False
        if len(pid) > 10:
            identity_stamped += 1
        if (phone, site, addr, rating, blat, blng, reviews) != (
            old_phone, old_site, old_addr, old_rating, old_lat, old_lng, old_reviews
        ):
            field_changed += 1
        details.append(f"{name}: phone {old_phone or '-'}->{phone or '-'}, "
                       f"rating {old_rating or '-'}->{rating or '-'}, "
                       f"reviews {old_reviews or '-'}->{reviews or '-'}, "
                       f"place id {old_pid[:8] or 'none'}->{pid[:8] or 'none'}")

    check("every updated listing is still exactly one record (nothing replaced)",
          same_record and len(updated) >= 1, f"{len(updated)} checked")
    check("the refresh never rewrote a listing's name", name_kept)
    check("Google's place id was stamped (durable identity for the next run)",
          identity_stamped == len(updated), f"{identity_stamped}/{len(updated)} stamped")
    check("the refreshed rows actually carry DIFFERENT data from before the run",
          field_changed == len(updated), f"{field_changed}/{len(updated)} changed")
    for d in details[:4]:
        print("      " + d)

    # ── 4. DISCOVER arm: adds only new, never a duplicate ───────────────────
    put = int(run.get("added") or 0)
    after_count = int(q1("select count(*) from businesses where directory_id='%s'" % PALM_BAY))
    check("the directory grew by exactly the number the run reports added",
          after_count - before_count == put, f"{before_count} -> {after_count} (added {put})")
    dup = q1("select count(*) from (select place_id from businesses where directory_id='%s' "
             "and coalesce(place_id,'')<>'' group by place_id having count(*)>1) x" % PALM_BAY)
    check("no place id is listed twice in the city", dup == "0", f"{dup} duplicated")

    if put:
        added_ids = [s["business_id"] for s in (run.get("samples") or [])
                     if s.get("result") == "added" and s.get("business_id")]
        id_list = ",".join("'%s'" % i for i in added_ids) or "''"
        complete = int(q1(
            "select count(*) from businesses where id in (%s) and coalesce(place_id,'')<>'' "
            "and coalesce(slug,'')<>''" % id_list))
        check("newly added businesses carry a place id and a public slug",
              complete == put, f"{complete}/{put}")
        check("newly added businesses are complete enough to show: address + city + category",
              int(q1("select count(*) from businesses where id in (%s) and coalesce(address,'')<>'' "
                     "and coalesce(city,'')<>'' and category_id is not null" % id_list)) == put,
              "checked %d rows" % len(added_ids))
        # ONE CITY PER RECORD: a business Google returned for a neighbouring town must not be
        # filed under this city (the W22 defect). Allowed: its own address names this city, or its
        # ZIP is one this city's own in-city listings use (the neighbourhood-directory case).
        check("no out-of-town business was filed under this city",
              int(q1("select count(*) from businesses b join directories d on d.id=b.directory_id "
                     "where b.id in (%s) "
                     "and lower(replace(trim(split_part(b.address, ',', "
                     "array_length(string_to_array(b.address, ','),1) - 2)),'.','')) "
                     "<> lower(replace(split_part(d.city,',',1),'.','')) "
                     "and coalesce(b.zip,'') not in (select distinct trim(x.zip) from businesses x "
                     "where x.directory_id=d.id and coalesce(trim(x.zip),'')<>'' "
                     "and lower(replace(trim(coalesce(x.city,'')),'.','')) = "
                     "lower(replace(split_part(d.city,',',1),'.','')))" % id_list)) == 0,
              "checked %d adds" % len(added_ids))
        # Public reachability: the new listing must be findable through the public directory API
        # (the city page and the search screen both read it), not only in the database.
        first = (run.get("samples") or [{}])[0]
        fname = q1("select name from businesses where id='%s'" % added_ids[0]) if added_ids else ""
        st, pub = call("/directories/palm-bay/businesses?q=%s&per_page=10"
                       % urllib.parse.quote(fname or first.get("name") or ""))
        listed = [b.get("id") for b in (pub.get("data") or [])]
        check("the added business is reachable on the public city directory", st == 200 and added_ids[0] in listed,
              f"HTTP {st} found={len(listed)}")
        st, run2 = call("/zaarhub/admin/google-refresh/run", token, "POST",
                        {"directory_id": PALM_BAY, "update": False, "discover": True, "max_add": 2})
        dup2 = q1("select count(*) from (select place_id from businesses where directory_id='%s' "
                  "and coalesce(place_id,'')<>'' group by place_id having count(*)>1) x" % PALM_BAY)
        check("a second run created no second copy of anything", st == 200 and dup2 == "0",
              f"HTTP {st} dupes={dup2} added_now={run2.get('added')}")
    else:
        check("discovery had Google results to add (so the add path was exercised)", False,
              "added=0 — see the notes above")

    # ── 5. the run is recorded ──────────────────────────────────────────────
    row = q1("select added||'/'||updated||'/'||unchanged||'/'||unreachable from google_refresh_runs "
             "where id='%s'" % run.get("run_id"))
    expect = "%s/%s/%s/%s" % (run.get("added"), run.get("updated"),
                              run.get("unchanged"), run.get("unreachable"))
    check("the run was recorded with the same numbers the panel was given", row == expect,
          f"db={row} api={expect}")
    check("the recorded run is finished (not stuck 'running')",
          q1("select count(*) from google_refresh_runs where id='%s' "
             "and finished_at is null" % run.get("run_id")) == "0")

    # ── 6. schedule setting ────────────────────────────────────────────────
    st, _ = call("/zaarhub/admin/google-refresh/schedule", token, "PUT",
                 {"directory_id": PALM_BAY, "schedule": "monthly"})
    st2, b2 = call(f"/zaarhub/admin/google-refresh/status?directory_id={PALM_BAY}", token)
    check("the schedule saves and reads back", st == 200 and st2 == 200 and b2.get("schedule") == "monthly",
          b2.get("schedule"))
    call("/zaarhub/admin/google-refresh/schedule", token, "PUT",
         {"directory_id": PALM_BAY, "schedule": "weekly"})
    st2, b2 = call(f"/zaarhub/admin/google-refresh/status?directory_id={PALM_BAY}", token)
    check("the schedule was restored to weekly", b2.get("schedule") == "weekly", b2.get("schedule"))
    st, _ = call("/zaarhub/admin/google-refresh/schedule", token, "PUT",
                 {"directory_id": PALM_BAY, "schedule": "hourly"})
    check("a nonsense schedule is refused", st == 400, f"HTTP {st}")

    # ── 7. nobody else can read a city's refresh state ─────────────────────
    st, _ = call(f"/zaarhub/admin/google-refresh/status?directory_id={PALM_BAY}")
    check("an anonymous caller cannot read the refresh state", st in (401, 403), f"HTTP {st}")
    st, _ = call("/zaarhub/admin/google-refresh/run", "not-a-token", "POST", {"directory_id": PALM_BAY})
    check("an invalid token cannot start a run", st in (401, 403), f"HTTP {st}")

    # ── 8. the screen exists on the served admin page ───────────────────────
    page = subprocess.run(
        ["curl", "-s", BASE + "/zaarhub-admin.html"], capture_output=True, text=True).stdout
    check("the Refresh from Google card is on the served admin page",
          "Refresh this city now" in page and "google-refresh/run" in page,
          f"{len(page)} bytes served")

    passed = sum(1 for ok, _, _ in results if ok)
    total = len(results)
    print("\nRESULT: %s %d/%d" % ("PASS" if passed == total else "FAIL", passed, total))
    return 0 if passed == total else 1


if __name__ == "__main__":
    sys.exit(main())
