#!/usr/bin/env python3
"""Acceptance test for ad_bookings -> CoreSwift ad_fulfillment fan-out (kanban t_f2112a07).

Proves: (a) a booking with two positions writes 2 rows with the same booking_ref;
        (b) replay writes the SAME 2 (not 4);
        (c) cancel archives both, none left live;
        (d) a real booking reads back real city, real issue_date, real buyer_email.

Setup seeds ONE sponsor + ONE creative for the real business "Pink Shimmering Maids"
(St. Petersburg) via SQL, then drives the booking create/approve/reject through the live
REST API with a minted super-admin JWT. Everything seeded is deleted in cleanup, and the
CoreSwift ad_fulfillment rows are hard-purged (live AND archived), so no residue is left.
"""
import base64, hashlib, hmac, json, os, re, subprocess, sys, time, urllib.request, urllib.error

MD = "http://127.0.0.1:8089"
CS = "http://127.0.0.1:8084"
ENVFILE = "/etc/swift/env/multi-directory.env"
CSKEYFILE = "/opt/swift/secure/coreswift-key-zaarhub-2026-11-13/zaarhub-coreswift-key.json"

DIRECTORY = "4c71a30c-278a-4bbb-b0ec-e1d912f7c8f1"  # St. Petersburg
BUSINESS = "9c6abd6c-b8a1-4198-8c1e-185e08ebf349"   # Pink Shimmering Maids
ZONE_TOP = "030a9d60-728a-4ea1-8dbc-cb03772a5e1b"   # st-petersburg sidebar_top
ZONE_BOT = "35cbec3a-588b-4703-8621-3d3d97caee13"   # st-petersburg sidebar_bottom
SUPER_ADMIN = "swiftsoftware143@yahoo.com"

results = []
CSKEY = json.load(open(CSKEYFILE))["api_key"]

def rec(name, ok, detail=""):
    results.append((name, ok, detail))
    print(("PASS " if ok else "FAIL ") + name + ((" — " + detail) if detail else ""))

def env_val(key):
    for line in open(ENVFILE):
        if line.startswith(key + "="):
            return line.split("=", 1)[1].strip()
    raise SystemExit("missing " + key)

def psql(sql):
    u = env_val("DATABASE_URL")
    m = re.match(r"postgres://([^:]+):([^@]+)@([^:/]+):?(\d+)?/(\S+)", u)
    env = dict(os.environ); env["PGPASSWORD"] = m.group(2)
    r = subprocess.run(["psql", "-h", m.group(3), "-p", m.group(4) or "5432", "-U", m.group(1),
                        "-d", m.group(5), "-tAc", sql],
                       capture_output=True, text=True, env=env, timeout=30)
    if r.returncode != 0:
        raise SystemExit("psql failed: " + r.stderr[:400])
    return r.stdout.strip()

def b64(raw):
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()

def mint_token(email):
    uid, tid, role = psql(
        "SELECT id, tenant_id, role FROM users WHERE email='%s' AND is_active" % email
    ).split("|")
    secret = env_val("JWT_SECRET")
    now = int(time.time())
    header = b64(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    payload = b64(json.dumps(
        {"sub": uid.strip(), "tid": tid.strip(), "role": role.strip(),
         "exp": now + 7200, "iat": now, "aud": None, "iss": None},
        separators=(",", ":")).encode())
    sig = b64(hmac.new(secret.encode(), (header + "." + payload).encode(), hashlib.sha256).digest())
    return header + "." + payload + "." + sig

def http(base, method, path, body=None, token=None, bearer=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(base + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    if bearer:
        req.add_header("Authorization", "Bearer " + bearer)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read().decode()
            return r.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw) if raw else {}
        except ValueError:
            return e.code, {"raw": raw[:300]}

def cs_records(booking_ref):
    st, body = http(CS, "GET",
                    f"/api/objects/ad_fulfillment/records?filter.booking_ref={booking_ref}",
                    bearer=CSKEY)
    return st, (body.get("records") or []) if isinstance(body, dict) else []

def main():
    token = mint_token(SUPER_ADMIN)
    print("minted super-admin token (len %d)" % len(token))

    sponsor_id = psql(
        "INSERT INTO sponsors (directory_id, business_id, status) "
        "VALUES ('%s','%s','active') RETURNING id" % (DIRECTORY, BUSINESS))
    creative_id = psql(
        "INSERT INTO ad_creatives (sponsor_id, name, image_url, width, height, status) "
        "VALUES ('%s','ZZ Ad Booking Test','https://example.test/ad.png',300,250,'approved') RETURNING id"
        % sponsor_id)
    print("seeded sponsor %s creative %s" % (sponsor_id, creative_id))

    booking_id = None
    record_ids = []
    start, end = "2026-12-01T00:00:00Z", "2026-12-31T00:00:00Z"

    try:
        # --- (a) create a booking with two positions ---
        st, body = http(MD, "POST", "/api/v1/ad-bookings", token=token, body={
            "directory_id": DIRECTORY, "sponsor_id": sponsor_id, "creative_id": creative_id,
            "start_date": start, "end_date": end, "auto_renew": False,
            "external_payment_ref": "test-booking-t_f2112a07",
            "ad_zone_ids": [ZONE_TOP, ZONE_BOT]})
        rec("booking create returns 201", st == 201, f"status={st}")
        booking = (body or {}).get("booking") or {}
        booking_id = booking.get("id")
        schedule_ids = (body or {}).get("schedule_ids") or []
        rec("booking has 2 schedule positions", len(schedule_ids) == 2,
            f"n={len(schedule_ids)} booking={booking_id}")

        approval_id = psql(
            "SELECT id FROM approval_queue WHERE item_type='ad_booking' AND item_id='%s'"
            % booking_id)
        st, _ = http(MD, "PUT", f"/api/v1/approvals/{approval_id}/status",
                     token=token, body={"status": "approved"})
        rec("booking approve returns 200", st == 200, f"status={st}")

        time.sleep(1.5)
        st, rows = cs_records(booking_id)
        rec("(a) 2 fulfillment rows after one booking", st == 200 and len(rows) == 2,
            f"status={st} n={len(rows)}")
        record_ids = [r.get("id") for r in rows]
        if rows:
            refs = sorted((r.get("data") or {}).get("booking_ref") for r in rows)
            rec("(a) both rows share the SAME booking_ref",
                len(set(refs)) == 1 and refs[0] == booking_id, f"refs={refs}")
            pos = sorted((r.get("data") or {}).get("position") for r in rows)
            rec("(a) positions are sidebar_top + sidebar_bottom",
                pos == ["sidebar_bottom", "sidebar_top"], f"pos={pos}")

        # --- (b) replay: re-approve -> still 2 ---
        st, _ = http(MD, "PUT", f"/api/v1/approvals/{approval_id}/status",
                     token=token, body={"status": "approved"})
        time.sleep(1.5)
        st, rows = cs_records(booking_id)
        rec("(b) replay -> still 2 rows (not 4)", len(rows) == 2, f"n={len(rows)}")

        # --- (d) real data ---
        d = (rows[0].get("data") or {}) if rows else {}
        rec("(d) real city (St Petersburg, enum form)", d.get("city") == "St Petersburg",
            f"city={d.get('city')!r}")
        rec("(d) real issue_date (YYYY-MM-DD of start_date)",
            bool(re.match(r"^2026-12-01$", d.get("issue_date") or "")),
            f"issue_date={d.get('issue_date')!r}")
        rec("(d) real buyer_email", d.get("buyer_email") == "info@pinkshimmermaids.com",
            f"buyer_email={d.get('buyer_email')!r}")
        rec("(d) artwork_status=Requested, ran=false",
            d.get("artwork_status") == "Requested" and d.get("ran") is False,
            f"status={d.get('artwork_status')!r} ran={d.get('ran')!r}")

        # --- (c) cancel (reject) -> archived, none live ---
        st, _ = http(MD, "PUT", f"/api/v1/approvals/{approval_id}/status",
                     token=token, body={"status": "rejected"})
        time.sleep(1.5)
        st, rows = cs_records(booking_id)
        rec("(c) cancel -> 0 live rows", len(rows) == 0, f"n={len(rows)}")

        bstatus = psql("SELECT status FROM ad_bookings WHERE id='%s'" % booking_id)
        rec("(c) booking status = cancelled", bstatus == "cancelled", f"status={bstatus}")
    finally:
        # hard-purge ad_fulfillment rows (live AND archived) by their captured ids
        for rid in record_ids:
            http(CS, "DELETE", f"/api/objects/ad_fulfillment/records/{rid}?hard=true", bearer=CSKEY)
        if booking_id:
            psql("DELETE FROM approval_queue WHERE item_id = '%s'" % booking_id)
            psql("DELETE FROM ad_schedules WHERE booking_id = '%s'" % booking_id)
            psql("DELETE FROM ad_bookings WHERE id = '%s'" % booking_id)
        psql("DELETE FROM ad_creatives WHERE id = '%s'" % creative_id)
        psql("DELETE FROM sponsors WHERE id = '%s'" % sponsor_id)
        print("cleanup: purged sponsor/creative/booking/schedules/approval + ad_fulfillment rows")

    failed = [r for r in results if not r[1]]
    print("\n=== %d/%d checks PASS ===" % (len(results) - len(failed), len(results)))
    sys.exit(1 if failed else 0)

if __name__ == "__main__":
    main()
