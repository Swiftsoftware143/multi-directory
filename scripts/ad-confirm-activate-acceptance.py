#!/usr/bin/env python3
"""Acceptance test for the CONFIRMATION-vs-ACTIVATION split (kanban t_41b9039d).

Proves:
  (1) CONFIRMATION writes the artwork chase: approving a 2-position booking moves it to
      'confirmed' (confirmed_at stamped) and writes 2 ad_fulfillment rows — BEFORE any activation.
  (2) ACTIVATION writes nothing new: POST /ad-bookings/:id/activate moves the booking/schedules to
      'active' and leaves the row count at 2 (publishing recorded, no new rows).
  (3) A booking created with core_booking_id shows it back on the ad_booking row.
  (4) Replaying confirm stays at N (upsert on booking_ref,position).
  (5) Confirming a booking whose core_booking_id does NOT resolve fails closed (422, no rows,
      booking stays pending).

Everything seeded is deleted in cleanup, and CoreSwift ad_fulfillment rows are hard-purged.
"""
import base64, hashlib, hmac, json, os, re, subprocess, sys, time, urllib.request, urllib.error, uuid

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
    m = re.match(r"postgres://([^:***@]+)@([^:/]+):?(\d+)?/(\S+)", u)
    env = dict(os.environ); env["PGPASSWORD"] = m.group(2)
    r = subprocess.run(["psql", "-h", m.group(3), "-p", m.group(4) or "5432", "-U", m.group(1),
                        "-d", m.group(5), "-tAqc", sql],
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

def purge_records(record_ids):
    for rid in record_ids:
        http(CS, "DELETE", f"/api/objects/ad_fulfillment/records/{rid}?hard=true", bearer=CSKEY)

def delete_booking(booking_id):
    if not booking_id:
        return
    psql("DELETE FROM approval_queue WHERE item_id = '%s'" % booking_id)
    psql("DELETE FROM ad_schedules WHERE booking_id = '%s'" % booking_id)
    psql("DELETE FROM ad_bookings WHERE id = '%s'" % booking_id)

def create_booking(token, sponsor_id, creative_id, core_booking_id=None):
    return http(MD, "POST", "/api/v1/ad-bookings", token=token, body={
        "directory_id": DIRECTORY, "sponsor_id": sponsor_id, "creative_id": creative_id,
        "start_date": "2026-12-01T00:00:00Z", "end_date": "2026-12-31T00:00:00Z",
        "auto_renew": False, "external_payment_ref": "test-booking-t_41b9039d",
        "ad_zone_ids": [ZONE_TOP, ZONE_BOT],
        **({"core_booking_id": core_booking_id} if core_booking_id else {})})

def main():
    token = mint_token(SUPER_ADMIN)
    print("minted super-admin token (len %d)" % len(token))

    sponsor_id = None
    creative_id = None
    happy_booking = None
    core_booking = None
    happy_records = []

    try:
        sponsor_id = psql(
            "INSERT INTO sponsors (directory_id, business_id, status) "
            "VALUES ('%s','%s','active') RETURNING id" % (DIRECTORY, BUSINESS))
        creative_id = psql(
            "INSERT INTO ad_creatives (sponsor_id, name, image_url, width, height, status) "
            "VALUES ('%s','ZZ Confirm/Activate Test','https://example.test/ad.png',300,250,'approved') RETURNING id"
            % sponsor_id)
        print("seeded sponsor %s creative %s" % (sponsor_id, creative_id))

        # ============ (1) confirm -> rows BEFORE activation ============
        st, body = create_booking(token, sponsor_id, creative_id)
        rec("booking create returns 201", st == 201, f"status={st}")
        happy_booking = (body or {}).get("booking") or {}
        happy_id = happy_booking.get("id")
        rec("booking has 2 schedule positions",
            len((body or {}).get("schedule_ids") or []) == 2)

        approval_id = psql(
            "SELECT id FROM approval_queue WHERE item_type='ad_booking' AND item_id='%s'"
            % happy_id)

        # CONFIRM (approve) — the confirmation email moment
        st, _ = http(MD, "PUT", f"/api/v1/approvals/{approval_id}/status",
                     token=token, body={"status": "approved"})
        rec("confirm (approve) returns 200", st == 200, f"status={st}")
        time.sleep(1.5)

        st, rows = cs_records(happy_id)
        happy_records = [r.get("id") for r in rows]
        rec("(1) 2 fulfillment rows written at CONFIRMATION", st == 200 and len(rows) == 2,
            f"status={st} n={len(rows)}")

        bstatus = psql("SELECT status FROM ad_bookings WHERE id='%s'" % happy_id)
        rec("(1) booking status = confirmed (not active)", bstatus == "confirmed",
            f"status={bstatus}")
        confirmed_at = psql("SELECT confirmed_at FROM ad_bookings WHERE id='%s'" % happy_id)
        rec("(1) confirmed_at stamped", bool(confirmed_at) and confirmed_at != "",
            f"confirmed_at={confirmed_at!r}")
        sstatuses = psql(
            "SELECT string_agg(DISTINCT status, ',') FROM ad_schedules WHERE booking_id='%s'"
            % happy_id)
        rec("(1) schedules confirmed BEFORE activation", sstatuses == "confirmed",
            f"schedule_status={sstatuses!r}")

        # ============ (4) replay of confirm stays at N ============
        st, _ = http(MD, "PUT", f"/api/v1/approvals/{approval_id}/status",
                     token=token, body={"status": "approved"})
        time.sleep(1.5)
        st, rows = cs_records(happy_id)
        rec("(4) replay confirm -> still 2 rows", len(rows) == 2, f"n={len(rows)}")

        # ============ (2) activation writes nothing new ============
        st, body = http(MD, "POST", f"/api/v1/ad-bookings/{happy_id}/activate", token=token)
        rec("activate returns 200", st == 200, f"status={st}")
        time.sleep(1.5)
        bstatus = psql("SELECT status FROM ad_bookings WHERE id='%s'" % happy_id)
        rec("(2) booking status = active after activation", bstatus == "active",
            f"status={bstatus}")
        st, rows = cs_records(happy_id)
        rec("(2) activation wrote NOTHING new (still 2 rows)", len(rows) == 2, f"n={len(rows)}")

        # ============ cancel (reject) -> archived ============
        st, _ = http(MD, "PUT", f"/api/v1/approvals/{approval_id}/status",
                     token=token, body={"status": "rejected"})
        time.sleep(1.5)
        st, rows = cs_records(happy_id)
        rec("cancel -> 0 live rows", len(rows) == 0, f"n={len(rows)}")
        bstatus = psql("SELECT status FROM ad_bookings WHERE id='%s'" % happy_id)
        rec("booking status = cancelled", bstatus == "cancelled", f"status={bstatus}")

        # ============ (3) core_booking_id shows on the row ============
        core_uuid = str(uuid.uuid4())
        st, body = create_booking(token, sponsor_id, creative_id, core_booking_id=core_uuid)
        rec("core-id booking create returns 201", st == 201, f"status={st}")
        core_booking = (body or {}).get("booking") or {}
        core_id = core_booking.get("id")
        rec("(3) core_booking_id echoed on create", core_booking.get("core_booking_id") == core_uuid,
            f"core_booking_id={core_booking.get('core_booking_id')!r}")

        st, body = http(MD, "GET", f"/api/v1/ad-bookings/{core_id}", token=token)
        row = (body or {}).get("booking") or {}
        rec("(3) GET /ad-bookings/:id returns core_booking_id",
            row.get("core_booking_id") == core_uuid, f"core_booking_id={row.get('core_booking_id')!r}")

        # ============ (5) fail closed on an unresolvable core id ============
        core_approval = psql(
            "SELECT id FROM approval_queue WHERE item_type='ad_booking' AND item_id='%s'"
            % core_id)
        st, body = http(MD, "PUT", f"/api/v1/approvals/{core_approval}/status",
                        token=token, body={"status": "approved"})
        rec("(5) confirm with unresolvable core id is refused (422)", st == 422, f"status={st}")
        bstatus = psql("SELECT status FROM ad_bookings WHERE id='%s'" % core_id)
        rec("(5) booking still pending (fail closed)", bstatus == "pending", f"status={bstatus}")
        st, rows = cs_records(core_id)
        rec("(5) no fulfillment rows written", len(rows) == 0, f"n={len(rows)}")

    finally:
        purge_records(happy_records)
        delete_booking((happy_booking or {}).get("id"))
        delete_booking((core_booking or {}).get("id"))
        if creative_id:
            psql("DELETE FROM ad_creatives WHERE id = '%s'" % creative_id)
        if sponsor_id:
            psql("DELETE FROM sponsors WHERE id = '%s'" % sponsor_id)
        print("cleanup: purged sponsors/creatives/bookings/schedules/approvals + fulfillment rows")

    failed = [r for r in results if not r[1]]
    print("\n=== %d/%d checks PASS ===" % (len(results) - len(failed), len(results)))
    sys.exit(1 if failed else 0)

if __name__ == "__main__":
    main()
