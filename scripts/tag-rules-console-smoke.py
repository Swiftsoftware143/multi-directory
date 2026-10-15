#!/usr/bin/env python3
"""tag-rules-console-smoke.py — does the operator console's Tags & Tracked Links screen
actually work end to end?

Two defects this proves fixed (both found by lifting the crate-wide `#![allow(unused)]`,
which is why the fields were invisible):

1. `admin-ops.html` declared a Create tag rule whose trigger/action choices the DATABASE
   rejects. The form's default trigger was `contact_created` and its default action
   `apply_tag`; `tag_rules_trigger_type_check` admits only tag_applied/tag_removed/
   workflow_completed and `tag_rules_action_type_check` only add_tag/remove_tag/send_email/
   send_sms/webhook/pipeline_move/scoring_update. So the console's own Create button could
   never succeed, and `apply_tag` is not even an action the rule engine implements.

2. `list_rules` took `tag_id` / `trigger_type` / `action_type` / `is_active` and
   `list_tracked_links` took `search`, and then silently dropped them: a filtered request
   came back unfiltered, so a filter looked like it worked and matched everything.

The test is driven by the CONSOLE FILE, not by a copy of its option lists, so it fails the
moment the console offers a value the API cannot accept.

Read-only except for rows it creates and deletes itself; asserts zero residue.
"""
import base64
import hashlib
import hmac
import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid

BASE = "http://127.0.0.1:8089"
ENV_FILE = "/opt/swift/apps/multi-directory/.env"
CONSOLE = "/opt/swift/apps/multi-directory/frontend/admin-ops.html"
TENANT = "00000000-0000-0000-0000-000000000001"
TS = str(int(time.time()))

passed = 0
failed = 0


def check(name, ok, detail=""):
    global passed, failed
    if ok:
        passed += 1
        print(f"PASS  {name}" + (f"  [{detail}]" if detail else ""))
    else:
        failed += 1
        print(f"FAIL  {name}  [{detail}]")


# ── infrastructure ────────────────────────────────────────────────────────────────

def db_password():
    out = subprocess.run(
        ["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    return out.split("://", 1)[1].split("@", 1)[0].split(":", 1)[1]


def psql(sql):
    r = subprocess.run(
        ["docker", "exec", "-e", f"PGPASSWORD={db_password()}", "swift-postgres",
         "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    return r.stdout.strip()


def mint_operator_token():
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
        "role": "super_admin", "exp": now + 1200, "iat": now,
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
        with urllib.request.urlopen(req, timeout=25) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


# ── what the console itself declares ─────────────────────────────────────────────

def console_field(item_title, key):
    """Return (options, default) for `key` inside the named console item."""
    src = open(CONSOLE, encoding="utf-8").read()
    begin = src.find("{ t: '" + item_title + "'")
    if begin < 0:
        sys.exit(f"console item not found: {item_title}")
    end = src.find("{ t: '", begin + 5)
    block = src[begin:end if end > 0 else len(src)]
    m = re.search(r"\{ k: '" + key + r"'.*?\}", block, re.S)
    if not m:
        return None, None
    entry = m.group(0)
    opts = re.search(r"o:\s*\[(.*?)\]", entry, re.S)
    default = re.search(r"d:\s*'([^']*)'", entry)
    options = [x.strip().strip("'") for x in opts.group(1).split(",")] if opts else []
    return options, (default.group(1) if default else None)


def allowed_values(constraint_name):
    ddl = psql(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint "
        f"WHERE conname = '{constraint_name}'")
    return set(re.findall(r"'([^']+)'::text", ddl))


# ── the run ──────────────────────────────────────────────────────────────────────

def main():
    op = mint_operator_token()
    made_rules, made_links = [], []

    # 1. the console's declared choices must be choices the API accepts
    for field, constraint in (("trigger_type", "tag_rules_trigger_type_check"),
                              ("action_type", "tag_rules_action_type_check")):
        opts, dflt = console_field("Create tag rule", field)
        allowed = allowed_values(constraint)
        check(f"console declares {field} choices", bool(opts), f"{opts} default={dflt}")
        bad = [o for o in opts if o not in allowed]
        check(f"every {field} the console offers is accepted by the database",
              not bad, f"rejected={bad} allowed={sorted(allowed)}")
        check(f"the console's {field} default is accepted", dflt in allowed,
              f"default={dflt}")

    trigger_dflt = console_field("Create tag rule", "trigger_type")[1]
    action_dflt = console_field("Create tag rule", "action_type")[1]

    # 2. the console's own defaults really create a rule
    tag_a, tag_b, tag_c = (str(uuid.uuid4()) for _ in range(3))
    body = {"tenant_id": TENANT, "name": f"ZZ console probe {TS}", "tag_id": tag_a,
            "trigger_type": trigger_dflt, "action_type": action_dflt}
    s, out = call("POST", "/api/v1/admin/tag-rules", op, body)
    check("a rule created with the console's own defaults is accepted", s == 201,
          f"status={s} {out[:160]}")
    if s == 201:
        made_rules.append(json.loads(out)["id"])

    # 3. two more rules, distinct on every filtered column. remove_tag needs a target tag in
    # its action_config — the engine only reports a rule it could actually act on (no fake
    # success: a rule missing its config reports executed:0).
    others = [("tag_removed", "remove_tag", tag_b, {"tag_id": str(uuid.uuid4())}),
              ("workflow_completed", "webhook", tag_c, {"url": "https://example.test/hook"})]
    for trig, act, tag, cfg in others:
        s, out = call("POST", "/api/v1/admin/tag-rules", op, {
            "tenant_id": TENANT, "name": f"ZZ console probe {TS} {act}", "tag_id": tag,
            "trigger_type": trig, "action_type": act, "action_config": cfg})
        if s == 201:
            made_rules.append(json.loads(out)["id"])
    check("three probe rules exist", len(made_rules) == 3, f"ids={len(made_rules)}")

    def list_rules(query):
        s, out = call("GET", f"/api/v1/admin/tag-rules?{query}", op)
        if s != 200:
            return s, []
        return s, json.loads(out)["items"]

    s, all_items = list_rules("tenant_id=" + TENANT + "&per_page=100")
    check("unfiltered list succeeds", s == 200, f"status={s}")
    ids = {r["id"] for r in all_items}
    check("unfiltered list contains all three probes",
          set(made_rules) <= ids, f"{len(ids)} rows")

    s, rows = list_rules(f"tenant_id={TENANT}&tag_id={tag_a}")
    check("filter tag_id actually filters", s == 200 and rows and
          all(r["tag_id"] == tag_a for r in rows), f"{len(rows)} rows, want 1")
    check("filter tag_id returned exactly the matching rule",
          [r["id"] for r in rows] == [made_rules[0]], f"{[r['id'] for r in rows]}")

    s, rows = list_rules(f"tenant_id={TENANT}&trigger_type=tag_removed")
    check("filter trigger_type actually filters", s == 200 and rows and
          all(r["trigger_type"] == "tag_removed" for r in rows), f"{len(rows)} rows")

    s, rows = list_rules(f"tenant_id={TENANT}&action_type=webhook")
    check("filter action_type actually filters", s == 200 and rows and
          all(r["action_type"] == "webhook" for r in rows), f"{len(rows)} rows")

    s, inactive = list_rules(f"tenant_id={TENANT}&is_active=false&per_page=100")
    check("filter is_active=false excludes the active probes",
          s == 200 and not (set(made_rules) & {r["id"] for r in inactive}),
          f"{len(inactive)} rows")

    # 4. tracked links: search must narrow
    link_ids = []
    for needle, url in ((f"zz-alpha-{TS}", "https://example.test/a"),
                        (f"zz-beta-{TS}", "https://example.test/b")):
        s, out = call("POST", "/api/v1/admin/tracked-links", op, {
            "tenant_id": TENANT, "name": needle, "url": url})
        if s == 201:
            link_ids.append(json.loads(out)["id"])
    check("two probe links exist", len(link_ids) == 2, f"ids={len(link_ids)}")

    s, out = call("GET", f"/api/v1/admin/tracked-links?tenant_id={TENANT}&per_page=100", op)
    check("unfiltered link list succeeds", s == 200, f"status={s}")
    if s == 200:
        check("unfiltered link list contains both probes",
              set(link_ids) <= {r["id"] for r in json.loads(out)["items"]})

    s, out = call("GET", f"/api/v1/admin/tracked-links?tenant_id={TENANT}&search=zz-alpha-{TS}", op)
    if s == 200:
        items = json.loads(out)["items"]
        check("link search actually narrows", len(items) == 1 and items[0]["id"] == link_ids[0],
              f"{len(items)} rows")
    else:
        check("link search actually narrows", False, f"status={s}")

    s, out = call("GET", f"/api/v1/admin/tracked-links?tenant_id={TENANT}&search=example.test%2Fb", op)
    if s == 200:
        items = json.loads(out)["items"]
        check("link search matches on the destination url too",
              len(items) == 1 and items[0]["id"] == link_ids[1], f"{len(items)} rows")
    else:
        check("link search matches on the destination url too", False, f"status={s}")

    # 5. the engine can run a rule the console created
    s, out = call("POST", "/api/v1/admin/tag-rules/execute", op, {
        "tenant_id": TENANT, "tag_id": tag_b, "trigger_type": "tag_removed",
        "contact_id": str(uuid.uuid4())})
    check("the engine executes a console-created rule", s == 200 and "remove_tag" in out,
          f"status={s} {out[:140]}")

    # 6. clean up, prove zero residue
    for rid in made_rules:
        call("DELETE", f"/api/v1/admin/tag-rules/{rid}", op)
    for lid in link_ids:
        call("DELETE", f"/api/v1/admin/tracked-links/{lid}", op)

    left_rules = psql(
        "SELECT count(*) FROM tag_rules WHERE name LIKE 'ZZ console probe %'")
    left_links = psql(
        f"SELECT count(*) FROM tracked_links WHERE name LIKE 'zz-%{TS}'")
    check("no probe rules left", left_rules == "0", f"remaining={left_rules}")
    check("no probe links left", left_links == "0", f"remaining={left_links}")

    print(f"\nRESULT: {'PASS' if failed == 0 else 'FAIL'} ({passed} passed, {failed} failed)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
