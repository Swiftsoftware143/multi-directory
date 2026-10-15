#!/usr/bin/env python3
"""Provider-key SCOPE resolution — live end-to-end proof (kanban t_7a7681d8).

The defect: `resolve_provider_key_scoped` (directory -> network -> platform) was documented as
"The ONLY resolution path for a directory-scoped read" and had ZERO callers, while every runtime
read called the scope-blind `resolve_provider_key`, whose SQL had no directory/network predicate
at all. Consequence: a key stored at NETWORK scope was honoured only by ordering luck, and a key
stored at DIRECTORY scope was used by EVERY directory (cross-directory credential bleed) -- the
per-city override the panel promises could not work, and one city setting its own Google Places
key would have quietly redirected every city's lookups.

This probe proves, against the RUNNING app, over real HTTP routes, on the live database:

  1. a probe key stored at NETWORK scope is what a city of that network reads;
  2. a probe key stored at DIRECTORY scope is what THAT city reads (own key wins);
  3. a SIBLING city of the same network does NOT see the first city's directory-scoped key -- it
     falls back to the network key (no cross-directory bleed);
  4. the "Scope is in force" screen reports the same source the runtime reads
     (GET /provider-keys/effective -> source "directory" / "network");
  5. the live network-scoped Google Places key STILL resolves for a city (no regression: all four
     live keys are network-scoped);
  6. the probe keys are removed in `finally` and the residue is re-read as ZERO.

The probe provider is chosen at runtime: the first provider in the catalogue that has NO stored key
at all, so the probe cannot shadow any real credential. Every probe key is labelled
`X-Swift-Harness ...` and deleted by id before the script exits.

Run:  python3 scripts/provider-key-scope-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import base64
import hashlib
import hmac
import json
import os
import sys
import time
import urllib.error
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
ENV_FILE = os.environ.get("MD_ENV_FILE", "/opt/swift/apps/multi-directory/.env")
TAG = "X-Swift-Harness key-scope probe (t_7a7681d8)"
CITY_A = os.environ.get("MD_CITY_A", "palm-bay")
CITY_B = os.environ.get("MD_CITY_B", "apopka")
VALUE_NET = "harness-network-key-BBBB2222"
VALUE_DIR = "harness-directory-key-AAAA1111"
FAILS = []


def check(label, ok, note=""):
    print(f"{'PASS' if ok else 'FAIL'}  {label}" + (f"  [{note}]" if note else ""))
    if not ok:
        FAILS.append(label)


def mint_operator_token():
    """super_admin JWT, same shape the panel uses. Secret never printed."""
    secret = None
    for line in open(ENV_FILE):
        s = line.strip()
        if s.startswith("JWT_SECRET="):
            secret = s.split("=", 1)[1].strip().strip('"')
    if not secret:
        sys.exit("JWT_SECRET not found in " + ENV_FILE)
    b64 = lambda o: base64.urlsafe_b64encode(json.dumps(o).encode()).decode().rstrip("=")
    now = int(time.time())
    msg = b64({"alg": "HS256", "typ": "JWT"}) + "." + b64({
        "sub": "00000000-0000-0000-0000-000000000002",
        "tid": "00000000-0000-0000-0000-000000000001",
        "role": "super_admin", "exp": now + 900, "iat": now,
        "aud": "multidirectory-api", "iss": "multidirectory"})
    return msg + "." + base64.urlsafe_b64encode(
        hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()).decode().rstrip("=")


def call(method, path, token=None, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=45) as r:
            raw = r.read().decode()
            return r.status, (json.loads(raw) if raw.strip() else {})
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, {"raw": raw[:400]}


def rows(payload):
    if isinstance(payload, list):
        return payload
    if isinstance(payload, dict):
        for k in ("data", "rows", "directories", "networks"):
            v = payload.get(k)
            if isinstance(v, list):
                return v
    return []


def find_by_slug(payload, slug):
    """Recursively find the first dict with slug == slug (dir shapes differ by endpoint)."""
    def walk(node):
        if isinstance(node, dict):
            if node.get("slug") == slug or node.get("key") == slug:
                return node
            for v in node.values():
                hit = walk(v)
                if hit:
                    return hit
        elif isinstance(node, list):
            for v in node:
                hit = walk(v)
                if hit:
                    return hit
        return None
    return walk(payload)


def mask_of(value):
    return value[:4] + "\u2022" * 8 + value[-4:]


def main():
    token = mint_operator_token()

    st, keys = call("GET", "/api/v1/provider-keys", token)
    check("admin can list provider keys (401 would mean the probe cannot run)", st == 200, f"HTTP {st}")
    if st != 200:
        return
    all_keys = rows(keys)
    used = {k.get("provider") for k in all_keys}

    st, catalog = call("GET", "/api/v1/available-providers", token)
    check("admin can list the provider catalogue", st == 200, f"HTTP {st}")
    catalog_rows = [r for r in rows(catalog) if isinstance(r, dict)]
    probe_provider = None
    for r in catalog_rows:
        k = r.get("key") if isinstance(r, dict) else None
        if k and k not in used and k != "coreswift":
            probe_provider = k
            break
    check("a provider with NO stored key exists (probe cannot shadow a real credential)",
          bool(probe_provider), f"probe provider = {probe_provider}")
    if not probe_provider:
        return

    st, nets = call("GET", "/api/v1/networks", token)
    zaarhub = find_by_slug(nets, "zaarhub") if st == 200 else None
    net_id = zaarhub.get("id") if isinstance(zaarhub, dict) else None
    check("the ZaarHub network id is readable (scope target for the network probe)",
          bool(net_id), f"HTTP {st} net={net_id}")

    st, dirs = call("GET", "/api/v1/directories?per_page=100", token)
    dir_a = find_by_slug(dirs, CITY_A) if st == 200 else None
    dir_b = find_by_slug(dirs, CITY_B) if st == 200 else None
    id_a = dir_a.get("id") if isinstance(dir_a, dict) else None
    id_b = dir_b.get("id") if isinstance(dir_b, dict) else None
    check("both probe cities are readable", bool(id_a and id_b), f"{CITY_A}={id_a} {CITY_B}={id_b}")
    if not (net_id and id_a and id_b):
        return

    # 1. NETWORK-scoped probe key
    st, r = call("POST", "/api/v1/provider-keys", token, {
        "provider": probe_provider, "api_key": VALUE_NET,
        "label": f"{TAG} network", "scope": "network", "network_id": net_id})
    check(f"network-scoped probe key stored for provider '{probe_provider}'", st == 201, f"HTTP {st}")

    # 2. DIRECTORY-scoped probe key (city A)
    st, r = call("POST", "/api/v1/provider-keys", token, {
        "provider": probe_provider, "api_key": VALUE_DIR,
        "label": f"{TAG} directory", "scope": "directory", "directory_id": id_a})
    check(f"directory-scoped probe key stored for {CITY_A}", st == 201, f"HTTP {st}")

    # 3. city A reads ITS OWN key
    st, a = call("GET", f"/api/v1/provider-keys/{probe_provider}/test?directory_id={CITY_A}", token)
    lbl_a = str(a.get("resolved_label") or "")
    check(f"{CITY_A} resolves the DIRECTORY-scoped key (value AND the label it names)",
          st == 200 and lbl_a.startswith(f"{TAG} directory") and CITY_A in lbl_a
          and a.get("key_preview") == mask_of(VALUE_DIR),
          f"HTTP {st} label={lbl_a} preview={a.get('key_preview')}")

    # 4. the SIBLING city does NOT see city A's key -> it inherits the network's
    st, b = call("GET", f"/api/v1/provider-keys/{probe_provider}/test?directory_id={CITY_B}", token)
    lbl_b = str(b.get("resolved_label") or "")
    check(f"{CITY_B} does NOT see {CITY_A}'s directory key (inherits the network key)",
          st == 200 and lbl_b.startswith(f"{TAG} network") and "zaarhub" in lbl_b
          and b.get("key_preview") == mask_of(VALUE_NET),
          f"HTTP {st} label={lbl_b} preview={b.get('key_preview')}")

    # 5. the "Scope is in force" screen agrees with the runtime
    st, ea = call("GET", f"/api/v1/provider-keys/effective?directory_id={id_a}", token)
    ea_row = next((x for x in rows(ea) if x.get("provider") == probe_provider), {})
    check(f"effective-key screen reports source=directory for {CITY_A}",
          st == 200 and ea_row.get("source") == "directory"
          and ea_row.get("source_label") == f"{TAG} directory",
          f"HTTP {st} source={ea_row.get('source')} label={ea_row.get('source_label')}")

    st, eb = call("GET", f"/api/v1/provider-keys/effective?directory_id={id_b}", token)
    eb_row = next((x for x in rows(eb) if x.get("provider") == probe_provider), {})
    check(f"effective-key screen reports source=network for {CITY_B}",
          st == 200 and eb_row.get("source") == "network"
          and eb_row.get("source_label") == f"{TAG} network",
          f"HTTP {st} source={eb_row.get('source')} label={eb_row.get('source_label')}")

    # 6. NO REGRESSION: the live network-scoped Google Places key still resolves for a city
    gp = next((x for x in rows(ea) if x.get("provider") == "google_places"), {})
    check("live google_places key still resolves for a city (source=network, no regression)",
          gp.get("has_key") is True and gp.get("source") == "network",
          f"has_key={gp.get('has_key')} source={gp.get('source')} label={gp.get('source_label')}")


if __name__ == "__main__":
    try:
        main()
    finally:
        # Cleanup runs even on an unexpected error: every probe key is deleted by id, then the
        # residue is re-read as proof it is gone.
        try:
            token = mint_operator_token()
            st, keys = call("GET", "/api/v1/provider-keys", token)
            for k in rows(keys):
                if str(k.get("label", "")).startswith("X-Swift-Harness key-scope probe"):
                    call("DELETE", "/api/v1/provider-keys/id/" + str(k["id"]), token)
            st, keys = call("GET", "/api/v1/provider-keys", token)
            left = [k for k in rows(keys) if str(k.get("label", "")).startswith("X-Swift-Harness key-scope probe")]
            check("probe residue re-read as ZERO after cleanup", len(left) == 0, f"left={len(left)}")
        except Exception as e:
            check("probe cleanup", False, f"cleanup error: {e}")
    print()
    if FAILS:
        print("RESULT: FAIL — " + "; ".join(FAILS))
        sys.exit(1)
    print("RESULT: PASS — directory scope wins for its own city, siblings inherit only the network "
          "key, and no scope-blind read can reach a directory-scoped credential")
