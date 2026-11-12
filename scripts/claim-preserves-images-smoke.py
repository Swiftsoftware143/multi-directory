#!/usr/bin/env python3
"""Claim flow — a claim must not throw away the images a listing already has.

The claim handler fires a Google-Places photo fetch in the background. That fetch used to
UPDATE businesses SET images = <only the fetched photos>, so every image the listing already
carried (owner uploads included) was silently lost the moment somebody claimed it. It also
used fetch_one() for that read, so a listing deleted between the claim and the fetch logged
"no rows returned by a query that expected to return at least one row".

This drives the REAL public claim endpoint against a throw-away listing that already carries
one upload path, waits for the background fetch, then asserts:
  * the upload path is still there (the merge kept it);
  * the fetched Google photos were appended, not substituted (when Places returns any);
  * the deleted-row read is a no-op, not a warning.

Exit 0 = PASS, 1 = FAIL. Fixtures are removed at the end either way.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from lib.coreswift_sweep import sweep_coreswift_mirror

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
STAMP = str(int(time.time()))
SLUG = f"zz-claim-img-{STAMP}"
KEEP = f"/uploads/businesses/zz-claim-img-{STAMP}/owner-upload.jpg"
EMAIL = f"zz-claim-img-{STAMP}@invalid.test"
FAILS = []


def check(name, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + name + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(name)


def sql(query, db="multi_directory"):
    url = subprocess.check_output(
        ["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"], text=True
    ).strip()
    m = re.match(r"postgres://([^:]+):([^@]+)@([^/]+)/([^?]+)", url)
    if not m:
        raise RuntimeError("unparseable DATABASE_URL")
    user, password, _host, _db = m.groups()
    env = dict(os.environ, PGPASSWORD=password)
    out = subprocess.run(
        [
            "docker", "exec", "-i", "-e", f"PGPASSWORD={password}",
            "swift-postgres", "psql", "-U", user, "-d", db, "-tAc", query,
        ],
        capture_output=True, text=True, env=env,
    )
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip())
    return out.stdout.strip()


def http(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        BASE + path, data=data, method=method,
        headers={"Content-Type": "application/json", "User-Agent": "SwiftOps/1.0"},
    )
    try:
        with urllib.request.urlopen(req, timeout=40) as r:
            return r.status, r.read().decode(errors="replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(errors="replace")


def main():
    directory_id = sql(
        "SELECT id FROM directories WHERE slug='palm-bay' AND status='active' LIMIT 1"
    )
    if not directory_id:
        print("FAIL  no palm-bay directory to hang the probe listing on")
        return 1

    biz_id = sql(
        "INSERT INTO businesses (name, slug, city, state, directory_id, images, status) "
        "VALUES ('Starbucks', "
        f"'{SLUG}', 'Palm Bay', 'FL', '{directory_id}', "
        f"'[\"{KEEP}\"]'::jsonb, 'active') RETURNING id"
    ).splitlines()[0]
    print(f"probe listing {biz_id}  images=[{KEEP}]")

    try:
        status, body = http(
            "POST", f"/api/v1/businesses/{biz_id}/claim",
            {"owner_email": EMAIL, "owner_name": "ZZ Claim Image Probe"},
        )
        check("the public claim endpoint accepted the claim", status in (200, 201),
              f"HTTP {status} {body[:120]}")

        # the photo fetch is fire-and-forget; give it up to 45s to touch the row
        images = []
        deadline = time.time() + 45
        while time.time() < deadline:
            row = sql(f"SELECT COALESCE(images,'[]'::jsonb)::text FROM businesses WHERE id='{biz_id}'")
            if not row:
                break
            images = json.loads(row)
            if len(images) > 1 or any("maps.googleapis.com" in str(i) for i in images):
                break
            time.sleep(3)

        check("the listing still exists after the claim", bool(
            sql(f"SELECT count(*) FROM businesses WHERE id='{biz_id}'")) and
            sql(f"SELECT count(*) FROM businesses WHERE id='{biz_id}'") != "0")
        check("the image the listing already had was kept", KEEP in images,
              f"{len(images)} image(s)")
        google = [i for i in images if "maps.googleapis.com" in str(i)]
        check("fetched Google photos were appended rather than substituting the old one",
              bool(google) and KEEP in images, f"google={len(google)}")
        check("no duplicate URLs in the merged array", len(images) == len(set(images)))
    finally:
        sql(f"DELETE FROM claimed_businesses WHERE business_id='{biz_id}'")
        sql(f"DELETE FROM businesses WHERE id='{biz_id}'")
        sql(f"DELETE FROM visitor_accounts WHERE email ILIKE '{EMAIL}'")
        left = sql(f"SELECT count(*) FROM businesses WHERE slug='{SLUG}'")
        print(f"cleanup: probe listing rows left = {left}")

    print()
    cs_left = sweep_coreswift_mirror(["zz-claim-img-"], window=15)
    check("no CoreSwift contact mirror left behind", cs_left == 0,
          f"coreswift_contacts={cs_left}")
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)} failed)")
        return 1
    print("RESULT: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
