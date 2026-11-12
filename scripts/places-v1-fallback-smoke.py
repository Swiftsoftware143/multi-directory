#!/usr/bin/env python3
"""Enrichment answers through Places API (New) when the legacy endpoint refuses the key.

WHY THIS EXISTS (measured 2026-11-12). The places client called ONLY the legacy endpoint
(`maps.googleapis.com/maps/api/place`). A refused legacy key (REQUEST_DENIED) was surfaced as a
hard failure with no second attempt, and on 2026-11-11 a refusal was read as "no working Google
key exists" — which parked three items on a key request to the owner. The same install holds a
second key whose restriction is Places API (New); it answers `places:searchText`. The enrichment
now retries on the new endpoint after a refusal and maps its answer back to the legacy field
names, so every downstream parser and the admin screen are unchanged.

TWO ARMS, both on the RUNNING app and the live database:
  ARM A — the key this app resolves today answers the LEGACY endpoint: a real listing is found,
          the response names `place/findplacefromtext`, and phone/website/coordinates/place id
          are written to the row.
  ARM B — a directory is pinned (for the length of the test) to the key that ONLY speaks Places
          API (New), so the legacy call is REFUSED. The same lookup must still succeed, must name
          `places/v1:searchText`, and must write the same fields. The pin is a reversible
          `api_config.google_places_api_key` write; the key value never leaves SQL (it is decrypted
          with pgp_sym_decrypt inside the statement) and `api_config` is restored and checksummed.

Nothing else is touched: the probe listing and its enrichment log rows are deleted, the directory
count and `api_config` checksum are asserted back to their before values.

Run:  python3 scripts/places-v1-fallback-smoke.py
Exit: 0 = PASS, 1 = FAIL.
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
MINT = "/opt/swift/browser-run/md-mint-token.js"
MD_ENV = "/etc/swift/env/multi-directory.env"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-places-v1-{time.strftime('%Y%m%d-%H%M%S')}")
FAILS = []
PIN_SLUG = "palm-bay"


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


def env_value(key):
    try:
        with open(MD_ENV) as fh:
            for line in fh:
                s = line.strip()
                if s.startswith(key + "="):
                    return s.split("=", 1)[1].strip().strip('"')
    except OSError:
        return ""
    return ""


PW = db_password()
SECRET = env_value("PROVIDER_KEY_ENC_SECRET")
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT|SET|DO)\b")


def psql(sql, stdin=None):
    """Run SQL. `stdin` carries secrets (\\set …) — never the command line, never the log."""
    r = subprocess.run(
        ["docker", "exec", "-i", "-e", f"PGPASSWORD={PW}", "swift-postgres",
         "psql", "-U", "swift", "-d", "multi_directory", "-At", "-F", "|"],
        input=(stdin if stdin is not None else sql),
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql[:200]}")
    return [ln for ln in r.stdout.strip().splitlines() if ln and not TAG.match(ln)]


def one(sql, default=""):
    rows = psql(sql)
    return rows[0] if rows else default


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def call(path, token, body=None, timeout=60):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(BASE + "/api/v1" + path, data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("X-Swift-Harness", "bld-places-v1")
    req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def enrich(token, business_id, directory_id):
    status, body = call("/enrich/business", token,
                        {"business_id": business_id, "directory_id": directory_id})
    try:
        return status, json.loads(body).get("enrichment", {})
    except ValueError:
        return status, {}


def reset_row(business_id):
    psql(
        "UPDATE businesses SET phone = NULL, website = NULL, latitude = NULL, longitude = NULL, "
        f"place_id = NULL WHERE id = '{business_id}'"
    )


def row_state(business_id):
    row = one(
        "SELECT COALESCE(phone,'') || '|' || COALESCE(website,'') || '|' || "
        "COALESCE(latitude::text,'') || '|' || COALESCE(place_id,'') "
        f"FROM businesses WHERE id = '{business_id}'"
    )
    parts = (row.split("|") + ["", "", "", ""])[:4]
    return parts


def main():
    os.makedirs(AUD, exist_ok=True)
    token = sh("node", MINT).stdout.strip().splitlines()[-1]
    check("operator token minted", len(token) > 100, f"len={len(token)}")

    before_dirs = one("SELECT count(*) FROM directories")
    dir_id = one(f"SELECT id FROM directories WHERE slug = '{PIN_SLUG}'")
    cfg_before = one(f"SELECT md5(coalesce(api_config::text,'NULL')) FROM directories WHERE slug = '{PIN_SLUG}'")

    # A real, findable business. The probe row is disposable; the name/address are a chain store
    # that certainly exists in this city, so a "no results" answer is a provider fault, not a miss.
    slug = f"zz-places-v1-{int(time.time())}"
    psql(
        "INSERT INTO businesses (id, directory_id, name, slug, address, city, state, status, "
        "created_at, updated_at) VALUES (gen_random_uuid(), "
        f"'{dir_id}', 'Walmart Supercenter', '{slug}', '1040 Palm Bay Rd NE', 'Palm Bay', 'FL', "
        "'active', now(), now())"
    )
    probe = one(f"SELECT id FROM businesses WHERE slug = '{slug}'")
    check("probe listing created with phone/website/coords/place id empty", bool(probe),
          f"id={(probe or '')[:8]}")
    if not probe:
        return finish()

    # ── ARM A: the key resolved today answers the LEGACY endpoint ──
    status, enr = enrich(token, probe, dir_id)
    with open(os.path.join(AUD, "arm-a-legacy.json"), "w") as fh:
        json.dump(enr, fh, indent=2)
    phone, website, lat, pid = row_state(probe)
    check("ARM A: the enrichment endpoint answered", status == 200, f"http={status}")
    check("ARM A: a real listing was matched", enr.get("matched") is True,
          f"confidence={enr.get('confidence')}")
    check("ARM A: the LEGACY endpoint answered", enr.get("places_call") == "place/findplacefromtext",
          f"places_call={enr.get('places_call')}")
    check("ARM A: Google returned and the row stored a place id", bool(pid), f"len={len(pid)}")
    check("ARM A: the row now carries Google's phone", bool(phone), f"phone_len={len(phone)}")
    check("ARM A: the row now carries Google's website", bool(website), f"website_len={len(website)}")
    check("ARM A: the row now carries coordinates", bool(lat), f"lat={lat}")

    # ── ARM B: pin the NEW-API-ONLY key → legacy is refused → the fallback must answer ──
    if not SECRET:
        check("ARM B: provider key secret is available to pin the new-API key", False)
        return finish_cleanup(probe, slug, dir_id, cfg_before, before_dirs)

    script = (
        "\\set k '" + SECRET.replace("'", "''") + "'\n"
        "UPDATE directories SET api_config = jsonb_set(coalesce(api_config,'{}'::jsonb), "
        "'{google_places_api_key}', to_jsonb(pgp_sym_decrypt(decode(substring(("
        "SELECT api_key FROM provider_keys WHERE provider = 'google_places' "
        "AND NOT is_active ORDER BY updated_at DESC LIMIT 1) from 8), 'base64'), :'k'))) "
        f"WHERE slug = '{PIN_SLUG}';\n"
    )
    psql("", stdin=script)
    pinned = one(f"SELECT api_config ? 'google_places_api_key' FROM directories WHERE slug = '{PIN_SLUG}'")
    check("ARM B: a directory is pinned to the new-API-only key", pinned == "t", f"pinned={pinned}")

    reset_row(probe)
    status, enr_b = enrich(token, probe, dir_id)
    with open(os.path.join(AUD, "arm-b-v1.json"), "w") as fh:
        json.dump(enr_b, fh, indent=2)
    phone_b, website_b, lat_b, pid_b = row_state(probe)
    check("ARM B: the enrichment endpoint answered with a refused legacy key", status == 200,
          f"http={status}")
    check("ARM B: the NEW endpoint answered", enr_b.get("places_call") == "places/v1:searchText",
          f"places_call={enr_b.get('places_call')}")
    check("ARM B: a real listing was matched through the fallback", enr_b.get("matched") is True,
          f"confidence={enr_b.get('confidence')}")
    check("ARM B: Google returned and the row stored a place id", bool(pid_b), f"len={len(pid_b)}")
    check("ARM B: the row now carries Google's phone", bool(phone_b), f"phone_len={len(phone_b)}")
    check("ARM B: the row now carries Google's website", bool(website_b), f"website_len={len(website_b)}")
    check("ARM B: the row now carries coordinates", bool(lat_b), f"lat={lat_b}")

    return finish_cleanup(probe, slug, dir_id, cfg_before, before_dirs)


def finish_cleanup(probe, slug, dir_id, cfg_before, before_dirs):
    psql(f"UPDATE directories SET api_config = '{{}}'::jsonb WHERE slug = '{PIN_SLUG}'")
    cfg_after = one(f"SELECT md5(coalesce(api_config::text,'NULL')) FROM directories WHERE slug = '{PIN_SLUG}'")
    check("the directory's provider config is restored exactly", cfg_after == cfg_before,
          f"{cfg_before[:8]} -> {cfg_after[:8]}")

    psql(f"DELETE FROM data_enrichment_logs WHERE business_id = '{probe}'")
    psql(f"DELETE FROM businesses WHERE id = '{probe}'")
    check("probe listing removed", one(f"SELECT count(*) FROM businesses WHERE slug = '{slug}'") == "0")
    check("no probe log rows left",
          one(f"SELECT count(*) FROM data_enrichment_logs WHERE business_id = '{probe}'") == "0")
    check("directory count unchanged", one("SELECT count(*) FROM directories") == before_dirs)
    check("no zz probe listings left",
          one("SELECT count(*) FROM businesses WHERE slug LIKE 'zz-places-v1-%'") == "0")
    return finish()


def finish():
    with open(os.path.join(AUD, "places-v1-fallback.log"), "w") as fh:
        fh.write(f"RESULT: {'PASS' if not FAILS else 'FAIL'}\nfailures: {FAILS}\n")
    print()
    print("RESULT: " + ("PASS" if not FAILS else "FAIL " + "; ".join(FAILS)))
    print(f"evidence: {AUD}")
    return 0 if not FAILS else 1


if __name__ == "__main__":
    sys.exit(main())
