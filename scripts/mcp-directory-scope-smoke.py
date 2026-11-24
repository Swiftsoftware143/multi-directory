#!/usr/bin/env python3
"""MCP directory-scope smoke — per-directory credentials (kanban t_6a165e41).

Proves, against the running app and the live database, the four guarantees of the port:
  1. a freshly minted OPERATOR key lists 9 tools and sees EVERY directory (count grows to
     N+1 after the probe directory is created);
  2. a DIRECTORY key is minted automatically with the directory, and that key sees ONLY its
     own directory (md_list_directories -> count 1, its own slug; md_list_businesses -> 0);
  3. a directory key asking for a business in ANOTHER directory answers `not_found` (never
     `forbidden`); the operator key reading the same id succeeds;
  4. deleting the directory revokes its key (the door answers 401 on the next call).

Run:  python3 scripts/mcp-directory-scope-smoke.py
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
MCP = BASE + "/admin/mcp"
MINT = "/opt/swift/browser-run/md-mint-token.js"
SLUG_PREFIX = "zz-mcp-scope"
HARNESS = "t_6a165e41"

FAILS = []
TOTAL = 0


def sh(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"swift:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL")
    return m.group(1)


def master_key():
    return sh("docker", "exec", "multidirectory", "printenv",
              "PROVIDER_KEY_ENC_SECRET").stdout.strip()


PW = db_password()
MASTER = master_key()
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT)\b")


def psql(sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    return [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]


def one(sql):
    rows = psql(sql)
    return rows[0] if rows else ""


def check(label, ok, detail=""):
    global TOTAL
    TOTAL += 1
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def http(method, path, token=None, body=None, base=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request((base or BASE) + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    req.add_header("X-Swift-Harness", HARNESS)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def mcp(token, method, params=None, ident=1):
    payload = {"jsonrpc": "2.0", "id": ident, "method": method}
    if params is not None:
        payload["params"] = params
    status, raw = http("POST", "/admin/mcp", token, payload)
    try:
        return status, json.loads(raw or "{}")
    except Exception:
        return status, {"_raw": raw}


def tool_text(token, name, args=None):
    status, body = mcp(token, "tools/call", {"name": name, "arguments": args or {}}, ident=99)
    result = body.get("result") or {}
    content = result.get("content") or [{}]
    text = content[0].get("text") or ""
    try:
        parsed = json.loads(text)
    except Exception:
        parsed = {"_raw": text}
    return status, result.get("isError"), parsed


def list_tool_names(token):
    status, body = mcp(token, "tools/list")
    tools = (body.get("result") or {}).get("tools") or []
    return status, [t.get("name") for t in tools]


def main():
    # operator session + a fresh operator API key (raw key returned by the admin endpoint)
    tok = sh("node", MINT).stdout.strip()
    check("platform-operator session minted", tok.count(".") == 2, f"len={len(tok)}")

    # sweep any leftover probe directories/keys from an interrupted prior run
    for leftover in psql("SELECT slug FROM directories WHERE slug LIKE 'zz-mcp-scope%'"):
        http("DELETE", f"/api/v1/directories/{leftover}", tok)
    psql("DELETE FROM api_keys WHERE name = 'zz-mcp-scope-operator'")

    # 0. baseline directory count
    before = int(one("SELECT count(*) FROM directories"))

    s, body = http("POST", "/api/v1/api-keys", tok, {"name": "zz-mcp-scope-operator"})
    try:
        op_key = json.loads(body).get("raw_key")
    except Exception:
        op_key = None
    check("operator API key minted (raw returned)", bool(op_key) and op_key.startswith("md_"),
          f"status={s}")

    # 1. operator key lists 9 tools
    s, names = list_tool_names(op_key)
    check("operator key tools/list answers 200", s == 200, f"status={s}")
    check("exactly 9 read tools exposed", len(names) == 9, f"count={len(names)} names={names}")

    # 2. create the probe directory -> a directory key is minted with it
    slug = f"{SLUG_PREFIX}-{int(time.time())}"
    s, body = http("POST", "/api/v1/directories", tok, {"name": "zz mcp scope probe", "slug": slug})
    check("operator POST /directories creates the probe", s in (200, 201), f"status={s} body={body[:200]}")

    did = one(f"SELECT id FROM directories WHERE slug='{slug}'")
    check("probe directory landed in the database", bool(did), did)

    key_row = one(f"SELECT id, key_encrypted IS NOT NULL, directory_id FROM api_keys WHERE directory_id = '{did}'")
    check("a directory-scoped key was minted with the directory", bool(key_row), key_row)

    parts = key_row.split("|")
    key_id, has_sealed, bound_dir = parts[0], parts[1] == "t", parts[2]
    check("the key records its directory_id", bound_dir == did, f"directory_id={bound_dir}")

    # decrypt the directory key (sealed under PROVIDER_KEY_ENC_SECRET)
    dir_key = one(
        "SELECT pgp_sym_decrypt(decode(substring(key_encrypted from 8), 'base64'), "
        f"'{MASTER}') FROM api_keys WHERE id = '{key_id}'")
    check("directory key decrypts to an md_ bearer", dir_key.startswith("md_"), f"len={len(dir_key)}")

    # 3. directory key sees ONLY its own directory
    s, names = list_tool_names(dir_key)
    check("directory key tools/list answers 200", s == 200, f"status={s}")

    s, is_err, payload = tool_text(dir_key, "md_list_directories")
    check("directory key md_list_directories count == 1", not is_err and payload.get("count") == 1,
          f"isError={is_err} payload={payload}")

    items = payload.get("items") or []
    check("directory key sees only its own directory", len(items) == 1 and items[0].get("slug") == slug,
          f"items={[i.get('slug') for i in items]}")

    s, is_err, payload = tool_text(dir_key, "md_list_businesses")
    check("directory key sees 0 businesses (probe has none)", not is_err and payload.get("count") == 0,
          f"isError={is_err} payload={payload}")

    # 4. operator sees every directory (before + probe)
    s, is_err, payload = tool_text(op_key, "md_list_directories")
    check("operator key md_list_directories count == before+1",
          not is_err and payload.get("count") == before + 1,
          f"before={before} isError={is_err} count={payload.get('count')}")

    # 4b. every remaining tool answers cleanly for the operator (no decode panic, valid payload)
    for tname in ["md_list_businesses", "md_list_categories", "md_list_submissions",
                  "md_list_sponsors", "md_list_ad_schedules", "md_list_reviews",
                  "md_list_seo_submissions"]:
        s, is_err, payload = tool_text(op_key, tname)
        check(f"operator {tname} answers without error",
              not is_err and isinstance(payload, dict) and "count" in payload,
              f"isError={is_err} payload={payload}")

    # 4c. the admin-only SEO tool refuses a directory key (forbidden, not a crash)
    s, is_err, payload = tool_text(dir_key, "md_list_seo_submissions")
    check("directory key is refused the admin-only SEO tool",
          is_err and payload.get("error") == "forbidden",
          f"isError={is_err} payload={payload}")

    # 5. foreign business id -> not_found for the directory key, success for the operator
    foreign = one("SELECT id FROM businesses WHERE directory_id IS NOT NULL "
                  f"AND directory_id <> '{did}' ORDER BY created_at DESC LIMIT 1")
    check("a foreign business exists to probe", bool(foreign), foreign)

    s, is_err, payload = tool_text(dir_key, "md_get_business", {"business_id": foreign})
    check("directory key reading a foreign business answers not_found",
          is_err and payload.get("error") == "not_found",
          f"isError={is_err} payload={payload}")

    s, is_err, payload = tool_text(op_key, "md_get_business", {"business_id": foreign})
    check("operator key reading the same business succeeds",
          not is_err and payload.get("id") == foreign,
          f"isError={is_err} id={payload.get('id')}")

    # 6. delete the directory -> its key is revoked (cascade removes the row)
    s, body = http("DELETE", f"/api/v1/directories/{slug}", tok)
    check("operator DELETE /directories/:slug answers 200", s == 200, f"status={s} body={body[:200]}")

    gone = one(f"SELECT count(*) FROM directories WHERE slug='{slug}'")
    check("the probe directory is gone", gone == "0", f"count={gone}")
    key_gone = one(f"SELECT count(*) FROM api_keys WHERE directory_id = '{did}'")
    check("the directory key row is gone (cascade)", key_gone == "0", f"count={key_gone}")

    s, _ = list_tool_names(dir_key)
    check("the revoked directory key answers 401", s == 401, f"status={s}")

    # cleanup: remove the freshly minted operator key
    op_id = one("SELECT id FROM api_keys WHERE key_prefix = "
                f"(SELECT key_prefix FROM api_keys WHERE name = 'zz-mcp-scope-operator' LIMIT 1) LIMIT 1")
    if op_id:
        psql(f"DELETE FROM api_keys WHERE id = '{op_id}'")

    print(f"\n{TOTAL - len(FAILS)}/{TOTAL} checks passed")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
