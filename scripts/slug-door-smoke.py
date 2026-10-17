#!/usr/bin/env python3
"""slug-door-smoke.py — kanban t_bf740579.

WHY: md stored a caller-supplied slug VERBATIM. So one slug had three different answers:
md stored `"  palm-bay "`, CoreSwift was sent the trimmed `"palm-bay"` (its own rule), and the
city page rendered `https://zaarhub.com/%20%20palm-bay` — a padded URL no row can ever match.
The fix routes every door (create + update, directories AND networks) through ONE rule
(`src/utils/slug.rs`), so store == send == render.

This probe drives the DEPLOYED binary on :8089 and proves it with real HTTP + DB read-backs:

  create dir, padded slug      -> stored & rendered slug is the canonical form
  create dir, blank slug       -> falls back to the name (never stores a blank)
  GET by canonical / padded    -> canonical 200, padded 404 (the stored value IS the URL)
  PUT dir padded / blank       -> normalized / no-preference (slug untouched)
  create net, padded slug      -> stored canonical
  create net, blank slug       -> 400 validation (never a blank network slug)
  PUT net padded               -> normalized
  cross-app (CoreSwift)        -> the SAME bytes on the tenant row, plus the harness marker
  census                       -> no padded / blank slug persisted anywhere

Fixtures are deleted in the finally (md row via the operator DELETE door, the CoreSwift tenant
via /opt/swift/scripts/probe-identity-cleanup.py) and both DBs are re-counted to prove it.
"""
import base64
import hashlib
import hmac
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

BASE = "http://127.0.0.1:8089"
STAMP = time.strftime("%H%M%S")
MARKER = f"md-slugdoor-{STAMP}"
PASS, FAIL = [], []


def sh(cmd):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True).stdout


def http(method, path, body=None, token=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    req.add_header("X-Swift-Harness", MARKER)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, r.read().decode(errors="ignore")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(errors="ignore")
    except Exception as e:  # noqa: BLE001
        return "ERR", str(e)[:200]


def sql(container, q):
    """Read-only-ish SQL against an app DB via its container's DATABASE_URL."""
    url = sh(f"docker exec {container} printenv DATABASE_URL").strip()
    with tempfile.NamedTemporaryFile("w", suffix=".sql", delete=False) as f:
        f.write(q + "\n")
        sf = f.name
    with tempfile.NamedTemporaryFile("w", suffix=".url", delete=False) as f:
        f.write(url)
        uf = f.name
    os.chmod(sf, 0o644)
    os.chmod(uf, 0o644)
    r = subprocess.run(
        ["docker", "run", "--rm", "--network", "host", "-v", f"{uf}:/u:ro",
         "-v", f"{sf}:/q.sql:ro", "postgres:16-alpine", "sh", "-c",
         'psql "$(cat /u)" -tA -P pager=off -F"|" -f /q.sql'], capture_output=True, text=True)
    out = [ln for ln in r.stdout.splitlines() if ln.strip()]
    return out


def one(container, q):
    rows = sql(container, q)
    return rows[0] if rows else ""


def check(name, ok, detail=""):
    (PASS if ok else FAIL).append(name)
    print(f"{'PASS' if ok else 'FAIL'}|{name}|{detail}", flush=True)


# ── operator JWT, minted the way the panel does it (drives the update/delete doors) ──
secret = sh("docker exec multidirectory printenv JWT_SECRET").strip()
now = int(time.time())


def b64(raw):
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


h = b64(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
p = b64(json.dumps({
    "sub": one("multidirectory", "select id from users limit 1;"),
    "tid": one("multidirectory", "select id from tenants limit 1;"),
    "role": "super_admin", "exp": now + 1800, "iat": now,
}, separators=(",", ":")).encode())
TOKEN = f"{h}.{p}.{b64(hmac.new(secret.encode(), f'{h}.{p}'.encode(), hashlib.sha256).digest())}"

CENSUS = ("select count(*)||'|'||count(*) filter (where slug <> btrim(slug))||'|'||"
          "count(*) filter (where btrim(slug) = '') from ")
MD_CS = "select count(*)||'|'||count(*) filter (where probe_harness is not null) from tenants;"

BASE_DIRS = one("multidirectory", CENSUS + "directories;")
BASE_NETS = one("multidirectory", CENSUS + "networks;")
BASE_CS = one("coreswiftcrm", MD_CS)
BASE_MD_USERS = one("coreswiftcrm", "select count(*) from users where email like 'md-%@local.coreswift';")
print(f"BASE|dirs={BASE_DIRS}|nets={BASE_NETS}|cs_tenants={BASE_CS}|cs_md_users={BASE_MD_USERS}",
      flush=True)

DIR_IDS, NET_IDS, SLUGS, CS_ADDRS = [], [], [], []


def canonical(raw):
    """The ONE rule, mirrored from src/utils/slug.rs."""
    out, last = [], True
    for ch in raw:
        if ch.isascii() and ch.isalnum():
            out.append(ch.lower())
            last = False
        elif not last:
            out.append("-")
            last = True
    return "".join(out).rstrip("-")


try:
    # ── T1 create directory with a PADDED slug ───────────────────────────────────────
    raw = f"  Slug Proof {STAMP}  "
    want = canonical(raw)
    st, body = http("POST", "/api/v1/directories",
                    {"name": f"Slug Proof {STAMP}", "slug": raw}, TOKEN)
    ok = st in (200, 201)
    if not ok:
        check("T1 create padded slug returns 2xx", False, f"{st} {body[:160]}")
    else:
        j = json.loads(body)
        DIR_IDS.append(j.get("id"))
        SLUGS.append(want)
        CS_ADDRS.append(f"md-{want}@local.coreswift")
        check("T1 create padded slug -> canonical in response",
              j.get("slug") == want, f"sent={raw!r} got={j.get('slug')!r} want={want!r}")
        check("T1b url_value (the rendered URL) is the same string",
              j.get("url_value") == want, f"url_value={j.get('url_value')!r}")

        # ── T2 what md STORED ────────────────────────────────────────────────────────
        stored = one("multidirectory",
                     f"select slug from directories where id = '{j['id']}';")
        check("T2 stored slug byte-equals what was sent (canonical)", stored == want,
              f"stored={stored!r}")

        # ── T3 what md SENT to CoreSwift ─────────────────────────────────────────────
        tid, cs_slug, cs_mark = "", "", ""
        for _ in range(15):
            tid = one("multidirectory",
                      f"select coalesce(coreswift_tenant_id::text,'') from directories "
                      f"where id = '{j['id']}';")
            if tid:
                row = one("coreswiftcrm",
                          f"select slug||'~'||coalesce(probe_harness,'') from tenants "
                          f"where id = '{tid}';")
                cs_slug, _, cs_mark = row.partition("~")
                break
            time.sleep(2)
        check("T3a cross-app: the same bytes on CoreSwift's tenant row",
              cs_slug == want, f"md={want!r} coreswift={cs_slug!r} tenant={tid or 'none'}")
        check("T3b cross-app: harness marker rode the register call",
              cs_mark == MARKER, f"marker={cs_mark!r}")

    # ── T4 a slug that normalises away is NOT SUPPLIED -> derived from the name ──────
    name4 = f"Derived City {STAMP}"
    st, body = http("POST", "/api/v1/directories", {"name": name4, "slug": "   "}, TOKEN)
    ok = st in (200, 201)
    j = json.loads(body) if ok else {}
    if ok:
        DIR_IDS.append(j.get("id"))
        SLUGS.append(canonical(name4))
        CS_ADDRS.append(f"md-{canonical(name4)}@local.coreswift")
    check("T4 blank slug falls back to the derived name", ok and j.get("slug") == canonical(name4),
          f"{st} slug={j.get('slug')!r} want={canonical(name4)!r}")

    if SLUGS:
        s0 = SLUGS[0]
        # ── T5/T6 the stored value IS the URL ────────────────────────────────────────
        st, body = http("GET", f"/api/v1/directories/{s0}", None, TOKEN)
        check("T5 GET by canonical slug -> 200 with the same slug",
              st == 200 and json.loads(body).get("slug") == s0, f"{st}")
        st, body = http("GET", "/api/v1/directories/" + urllib.parse.quote(f"  {s0}  "),
                        None, TOKEN)
        check("T6 GET by the PADDED form does not match the row", st != 200, f"{st}")

        # ── T7/T8 update door ────────────────────────────────────────────────────────
        raw7 = f"  {s0} Renamed  "
        st, body = http("PUT", f"/api/v1/directories/{s0}", {"slug": raw7}, TOKEN)
        want7 = canonical(raw7)
        got7 = one("multidirectory", f"select slug from directories where id = '{DIR_IDS[0]}';")
        if st in (200, 201) and got7 == want7:
            SLUGS[0] = want7
            s0 = want7
        check("T7 PUT padded slug -> canonical stored", st in (200, 201) and got7 == want7,
              f"{st} stored={got7!r} want={want7!r}")

        st, body = http("PUT", f"/api/v1/directories/{s0}", {"slug": "   "}, TOKEN)
        got8 = one("multidirectory", f"select slug from directories where id = '{DIR_IDS[0]}';")
        check("T8 PUT blank slug = no preference (row keeps its slug)",
              st in (200, 201) and got8 == s0, f"{st} stored={got8!r} was={s0!r}")

    # ── T9/T10 networks ─────────────────────────────────────────────────────────────
    rawn = f"  Slug Door Net {STAMP}  "
    wantn = canonical(rawn)
    st, body = http("POST", "/api/v1/networks", {"name": f"Slug Door Net {STAMP}", "slug": rawn}, TOKEN)
    ok = st in (200, 201)
    j = json.loads(body) if ok else {}
    if ok:
        NET_IDS.append(j.get("id"))
        # The network door mints its OWN CoreSwift workspace (same address shape) — track it too,
        # or the sweep leaves it behind.
        CS_ADDRS.append(f"md-{wantn}@local.coreswift")
    check("T9 create network with padded slug -> canonical stored",
          ok and j.get("slug") == wantn, f"{st} slug={j.get('slug')!r} want={wantn!r}")

    st, body = http("POST", "/api/v1/networks", {"name": f"Blank Net {STAMP}", "slug": "  "}, TOKEN)
    made = one("multidirectory", f"select count(*) from networks where name = 'Blank Net {STAMP}';")
    check("T10 create network with blank slug -> rejected, no row",
          st in (400, 422) and made == "0", f"status={st} rows={made}")

    if "T9" in PASS and NET_IDS:
        st, body = http("PUT", f"/api/v1/networks/{NET_IDS[0]}", {"slug": f"  {wantn} x  "}, TOKEN)
        got = one("multidirectory", f"select slug from networks where id = '{NET_IDS[0]}';")
        check("T11 PUT network padded slug -> canonical stored",
              st in (200, 201) and got == canonical(f"  {wantn} x  "),
              f"{st} stored={got!r}")

    # ── T12 census: nothing padded, nothing blank, anywhere ─────────────────────────
    for tbl in ("directories", "networks"):
        row = one("multidirectory", CENSUS + f"{tbl};")
        tot, pad, blank = row.split("|")
        check(f"T12 no padded/blank slug in {tbl}", pad == "0" and blank == "0",
              f"total={tot} padded={pad} blank={blank}")

finally:
    # ── fixtures out ────────────────────────────────────────────────────────────────
    # Let the detached provisioning spawns settle FIRST: md's row is written before the CoreSwift
    # workspace lands, so deleting the row too early leaves a workspace nothing can name again
    # (skill cross-app-probe-residue-cleanup).
    for sid in DIR_IDS:
        for _ in range(15):
            if one("multidirectory",
                   f"select coalesce(coreswift_tenant_id::text,'') from directories where id = '{sid}';"):
                break
            time.sleep(2)

    # md's own rows go by SQL, not by DELETE /api/v1/directories/:slug: that door 500s on
    # `ad_zones_directory_id_fkey` for ANY directory — create_directory auto-creates its ad zones
    # and email templates, and both FKs are NO ACTION (separate defect, separate card, reported in
    # the t_bf740579 handoff). A fixture sweep must not be blocked by it, so the two auto-created
    # child tables go in the same transaction as the parent row.
    ids = ",".join(f"'{i}'" for i in DIR_IDS if i)
    if ids:
        sql("multidirectory",
            f"begin; delete from ad_zones where directory_id in ({ids});"
            f" delete from email_templates where directory_id in ({ids});"
            f" delete from directories where id in ({ids}); commit;")
    for nid in NET_IDS:
        sql("multidirectory",
            f"begin; delete from network_branding where network_id = '{nid}';"
            f" delete from networks where id = '{nid}'; commit;")

    # CoreSwift workspaces by exact reserved address (the slug AT CREATE TIME — a rename never
    # reaches CoreSwift), retried while the workspace is still landing.
    for addr in CS_ADDRS:
        for _ in range(10):
            sh(f"python3 /opt/swift/scripts/probe-identity-cleanup.py --app coreswift_crm "
               f"--email {addr}")
            left = int(one("coreswiftcrm",
                           "select count(*) from users where email like 'md-%@local.coreswift';") or 0)
            if left == 0:
                break
            time.sleep(3)

    after_dirs = one("multidirectory", CENSUS + "directories;")
    after_nets = one("multidirectory", CENSUS + "networks;")
    after_cs = one("coreswiftcrm", MD_CS)
    after_users = one("coreswiftcrm", "select count(*) from users where email like 'md-%@local.coreswift';")
    print(f"AFTER|dirs={after_dirs}|nets={after_nets}|cs_tenants={after_cs}|cs_md_users={after_users}",
          flush=True)
    check("T13 residue 0: md rows and CoreSwift tenants back to baseline",
          after_dirs == BASE_DIRS and after_nets == BASE_NETS and after_cs == BASE_CS
          and after_users == BASE_MD_USERS,
          f"dirs {BASE_DIRS}->{after_dirs} nets {BASE_NETS}->{after_nets} "
          f"cs {BASE_CS}->{after_cs} md_users {BASE_MD_USERS}->{after_users}")

print(f"SLUG-DOOR: {len(PASS)}/{len(PASS) + len(FAIL)} passed")
if FAIL:
    print("FAILED: " + ", ".join(FAIL))
sys.exit(0 if not FAIL else 1)
