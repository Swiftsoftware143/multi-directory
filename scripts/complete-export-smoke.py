#!/usr/bin/env python3
"""Complete per-entity export — live end-to-end proof (item 4 "Data export", ZaarHub).

The admin panel's "Complete Export" card offers one record (business / supplier / customer)
as its own row plus every related record, JSON + CSV, plus a whole-directory bulk download
(frontend/admin-panel.html -> /export/entity/:kind/:id and /export/bulk/:kind). Nothing had a
committed proof that a COMPLETE record actually downloads.

This proves, against the running app at 127.0.0.1:8089 with the platform-operator session the
panel uses:
  * the operator CAN download one business COMPLETE as JSON: the response is an attachment
    (browser download), the body carries the entity's own row AND each related table it has
    rows in (a review it owns and a sponsored article it published are both present, with the
    right per-table counts and section labels);
  * the same record downloads as CSV: one `# section` banner per related table and the child
    rows actually present in the text;
  * a plain signed-in account (the flat role model hands every signup tenant "admin") CANNOT
    read the record — 404, never a 403 that would confirm the id;
  * a logged-out caller is refused (401);
  * a local business cannot be exported AS a supplier (422, honest refusal);
  * the whole directory downloads in bulk (JSON + CSV) and the probe business is in it;
  * bulk without a directory is refused (422);
  * the entity-kind helper the panel reads returns business/supplier/customer.

The only direct DB writes are the disposable probe rows and their teardown.
Run:  python3 scripts/complete-export-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Nothing is left behind on either path.
"""
import base64
import hashlib
import hmac
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
PUBLIC = os.environ.get("MD_PUBLIC", "https://zaarhub.com")
APP = "/opt/swift/apps/multi-directory"
ENV_FILE = os.path.join(APP, ".env")
BROWSER = os.path.join(APP, "scripts", "complete-export-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item4-export-{time.strftime('%Y%m%d-%H%M%S')}")
HARNESS = "export-probe"
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def chrome_path():
    if os.environ.get("CHROME_PATH"):
        return os.environ["CHROME_PATH"]
    cands = [p for p in ("/usr/bin/chromium-browser", "/usr/bin/chromium",
                         "/usr/bin/google-chrome") if os.path.exists(p)]
    return cands[0] if cands else "/usr/bin/chromium-browser"


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


PW = db_password()
TAG = re.compile(r"^(INSERT|UPDATE|DELETE|SELECT|BEGIN|COMMIT)\b")


def psql(sql):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    lines = [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]
    return lines[0] if lines else ""


def mint_operator_token():
    """super_admin JWT, the shape the admin panel uses. Secret never printed."""
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


def http(method, path, body=None, token=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, {k.lower(): v for k, v in r.headers.items()}, \
                r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, {k.lower(): v for k, v in e.headers.items()}, \
            e.read().decode("utf-8", "replace")


def check(label, ok, detail=""):
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def sweep():
    psql(f"DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{HARNESS}')")
    psql(f"DELETE FROM business_articles WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{HARNESS}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{HARNESS}'")


def main():
    sweep()
    ts = str(int(time.time()))
    slug = f"zz-export-probe-{ts}"
    rev_marker = f"ZZEXPORTREVIEW{ts}"
    art_marker = f"ZZEXPORTARTICLE{ts}"
    directory_id = psql("SELECT id FROM directories ORDER BY name LIMIT 1")
    check("a directory exists to host the probe record", bool(directory_id))

    bid = psql(
        "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, "
        "review_count, probe_harness) VALUES "
        f"('{directory_id}', 'ZZ Export Probe Co {ts}', '{slug}', 'Orlando', 'FL', 'active', "
        f"0, 0, '{HARNESS}') RETURNING id")
    check("disposable listing created", bool(bid))

    rid = psql(
        "INSERT INTO reviews (business_id, rating, title, content, status) VALUES "
        f"('{bid}', 5, '{rev_marker}', 'child row on the probe listing', 'approved') RETURNING id")
    aid = psql(
        "INSERT INTO business_articles (directory_id, business_id, title, slug, keyword, "
        "status) VALUES "
        f"('{directory_id}', '{bid}', '{art_marker}', '{slug}-article', 'probe', 'published') "
        "RETURNING id")
    check("two related child rows created (a review + a sponsored article)", bool(rid) and bool(aid))

    tok = mint_operator_token()

    # ── 1. the entity-kind helper the panel reads ────────────────────────────────
    st, _, body = http("GET", "/api/v1/export/entity/kinds", token=tok)
    kinds = (json.loads(body).get("kinds") if st == 200 else None) or []
    check("the entity-kind helper lists business/supplier/customer",
          st == 200 and "business" in kinds and "supplier" in kinds and "customer" in kinds,
          f"status={st} kinds={kinds}")

    # ── 2. ONE record COMPLETE as JSON (the browser download) ────────────────────
    st, hdrs, body = http("GET", f"/api/v1/export/entity/business/{bid}?format=json", token=tok)
    check("operator downloads one business as JSON (200)", st == 200, f"status={st}")
    check("the response is a file download (Content-Disposition attachment + a filename)",
          "attachment" in hdrs.get("content-disposition", "")
          and f"business_entity_{bid}.json" in hdrs.get("content-disposition", ""),
          hdrs.get("content-disposition", ""))
    check("the download is served as JSON", "application/json" in hdrs.get("content-type", ""),
          hdrs.get("content-type", ""))

    doc = None
    try:
        doc = json.loads(body)
    except json.JSONDecodeError:
        pass
    check("the body is a complete record document (kind/id/entity/related/counts/sections)",
          isinstance(doc, dict) and all(
              k in doc for k in ("kind", "id", "entity", "related", "counts", "sections")),
          f"keys={list(doc.keys()) if isinstance(doc, dict) else type(doc).__name__}")
    doc = doc if isinstance(doc, dict) else {}
    check("it is the requested record (kind=business, id matches)",
          doc.get("kind") == "business" and str(doc.get("id")) == bid,
          f"kind={doc.get('kind')} id={doc.get('id')}")
    entity = doc.get("entity") or {}
    check("the record's OWN row is present and complete",
          str(entity.get("id")) == bid and entity.get("slug") == slug,
          f"entity.id={entity.get('id')}")

    related = doc.get("related") or {}
    counts = doc.get("counts") or {}
    rev_rows = related.get("reviews") or []
    art_rows = related.get("business_articles") or []
    check("the related REVIEW row is gathered into the record",
          any(r.get("title") == rev_marker for r in rev_rows), f"review_rows={len(rev_rows)}")
    check("the related SPONSORED ARTICLE row is gathered into the record",
          any(r.get("title") == art_marker for r in art_rows), f"article_rows={len(art_rows)}")
    check("the per-table counts match what was gathered",
          counts.get("reviews") == len(rev_rows) and counts.get("business_articles") == len(art_rows)
          and counts.get("reviews", 0) >= 1 and counts.get("business_articles", 0) >= 1,
          f"counts.reviews={counts.get('reviews')} counts.business_articles={counts.get('business_articles')}")
    sections = doc.get("sections") or []
    check("the section labels list every related table in the record",
          "entity" in sections and "reviews" in sections and "business_articles" in sections,
          f"sections={len(sections)}")

    # ── 3. the same record as CSV ────────────────────────────────────────────────
    st, hdrs, csv_body = http("GET", f"/api/v1/export/entity/business/{bid}?format=csv", token=tok)
    check("operator downloads the same business as CSV (200)", st == 200, f"status={st}")
    check("the CSV is a file download",
          "attachment" in hdrs.get("content-disposition", "")
          and "text/csv" in hdrs.get("content-type", ""),
          hdrs.get("content-disposition", ""))
    check("the CSV has one section banner per related table",
          "# entity" in csv_body and "# table: reviews" in csv_body
          and "# table: business_articles" in csv_body,
          f"len={len(csv_body)}")
    check("the CSV actually carries the child records",
          rev_marker in csv_body and art_marker in csv_body)

    # ── 4. scoping: a plain signup and a logged-out caller must NOT read it ──────
    reg_email = f"zz-export-plain-{ts}@swiftsoftware.dev"
    st, _, rb = http("POST", "/api/v1/visitor/register",
                     {"email": reg_email, "password": "ProbePass123!", "name": "ZZ Plain"})
    try:
        plain = (json.loads(rb) or {})
    except json.JSONDecodeError:
        plain = {}
    plain_tok = plain.get("access_token") or plain.get("token")
    if not plain_tok:
        st, _, lb = http("POST", "/api/v1/auth/login",
                         {"email": reg_email, "password": "ProbePass123!"})
        try:
            plain_tok = (json.loads(lb) or {}).get("access_token")
        except json.JSONDecodeError:
            plain_tok = None
    check("a plain signed-in account exists for the scoping check", bool(plain_tok))

    st, _, _ = http("GET", f"/api/v1/export/entity/business/{bid}?format=json", token=plain_tok)
    check("a plain account CANNOT download another business's complete record (404, not 403)",
          st == 404, f"status={st}")
    st, _, _ = http("GET", f"/api/v1/export/entity/business/{bid}?format=json")
    check("a logged-out caller CANNOT download it (401)", st == 401, f"status={st}")

    # ── 5. a local listing cannot leave the building as a supplier ───────────────
    st, _, _ = http("GET", f"/api/v1/export/entity/supplier/{bid}?format=json", token=tok)
    check("a local listing is refused as a supplier export (422)", st == 422, f"status={st}")

    # ── 6. the whole directory in bulk (both formats) ────────────────────────────
    st, hdrs, bbody = http("GET", f"/api/v1/export/bulk/business?directory_id={directory_id}"
                                "&format=json", token=tok)
    bulk = None
    try:
        bulk = json.loads(bbody)
    except json.JSONDecodeError:
        pass
    check("operator downloads the directory in bulk as JSON (200)", st == 200, f"status={st}")
    check("the bulk file names the directory and reports a count",
          "attachment" in hdrs.get("content-disposition", "") and isinstance(bulk, dict)
          and str(bulk.get("directory_id")) == directory_id and isinstance(bulk.get("count"), int)
          and bulk.get("count", 0) >= 1,
          f"count={(bulk or {}).get('count') if isinstance(bulk, dict) else 'n/a'}")
    rows = (bulk or {}).get("rows") if isinstance(bulk, dict) else None
    check("the probe business is inside the bulk download",
          isinstance(rows, list) and any(str(r.get("id")) == bid for r in rows),
          f"rows={len(rows) if isinstance(rows, list) else 'n/a'}")

    st, hdrs, bcsv = http("GET", f"/api/v1/export/bulk/business?directory_id={directory_id}"
                                "&format=csv", token=tok)
    check("operator downloads the directory in bulk as CSV (200, text/csv)",
          st == 200 and "text/csv" in hdrs.get("content-type", ""), f"status={st}")
    check("the bulk CSV carries the probe business", slug in bcsv)

    st, _, _ = http("GET", "/api/v1/export/bulk/business?format=json", token=tok)
    check("bulk without a directory is refused (422)", st == 422, f"status={st}")
    st, _, _ = http("GET", f"/api/v1/export/bulk/business?directory_id={directory_id}&format=json",
                    token=plain_tok)
    check("a plain account CANNOT bulk-download a directory (404)", st == 404, f"status={st}")

    # ── 5. real browser: the panel's OWN button delivers the file ────────────────
    os.makedirs(AUD, exist_ok=True)
    check("a real browser is available", os.path.exists(chrome_path()),
          os.path.basename(chrome_path()))
    env = dict(os.environ, PUBLIC=PUBLIC, BIZ=bid, DIR=directory_id, TOKEN=tok,
               AUD=AUD, NODE_PATH=NODE_PATH, CHROME_PATH=chrome_path())
    r = sh("node", BROWSER, env=env, timeout=240)
    proof_path = os.path.join(AUD, "browser-proof.json")
    proof = {}
    if os.path.exists(proof_path):
        try:
            proof = json.load(open(proof_path))
        except json.JSONDecodeError:
            pass
    if not proof:
        print(r.stdout[-600:])
        print(r.stderr[-600:])
    check("the real admin console loads", proof.get("panelLoaded") is True,
          f"status={proof.get('panelStatus')} errors={proof.get('pageErrors')}")
    check("the Complete Export card is on screen with its buttons",
          proof.get("cardVisible") is True and (proof.get("buttonsVisible") or 0) >= 4,
          f"buttons={proof.get('buttonsVisible')}")
    jr = proof.get("jsonResponse") or {}
    check("the browser's own click sends the export request and the server answers a file",
          jr.get("status") == 200 and "attachment" in (jr.get("disposition") or "")
          and f"business_entity_{bid}.json" in (jr.get("disposition") or ""),
          f"status={jr.get('status')} disposition={jr.get('disposition')}")
    check("clicking the panel's JSON button ASSEMBLES the complete record file in the browser",
          proof.get("jsonBlobLanded") is True,
          f"name={proof.get('jsonDisposition')} bytes={proof.get('jsonBlobBytes')}")
    check("the assembled JSON file IS the complete record (own row + related counts)",
          str(proof.get("jsonEntityId")) == bid and (proof.get("jsonReviewCount") or 0) >= 1
          and (proof.get("jsonArticleCount") or 0) >= 1,
          f"entity={proof.get('jsonEntityId')} reviews={proof.get('jsonReviewCount')} "
          f"articles={proof.get('jsonArticleCount')}")
    check("the panel hands the browser a real download filename for that record",
          any(n == f"business_entity_{bid}.json" for n in (proof.get("downloadNames") or [])),
          f"names={proof.get('downloadNames')}")
    check("clicking the panel's CSV button ASSEMBLES a CSV carrying the record's own section",
          proof.get("csvBlobLanded") is True and proof.get("csvHasSections") is True,
          f"status={proof.get('csvStatusMsg')}")
    check("the browser run is clean (no page errors)",
          (proof.get("pageErrors") or []) == [], f"errors={proof.get('pageErrors')}")

    # ── teardown ────────────────────────────────────────────────────────────────
    psql(f"DELETE FROM reviews WHERE id='{rid}'")
    psql(f"DELETE FROM business_articles WHERE id='{aid}'")
    psql(f"DELETE FROM businesses WHERE id='{bid}'")
    psql(f"DELETE FROM visitor_accounts WHERE lower(email)='{reg_email}'")
    left = psql(
        "SELECT (SELECT count(*) FROM businesses WHERE id="
        f"'{bid}') + (SELECT count(*) FROM reviews WHERE id='{rid}') + "
        f"(SELECT count(*) FROM business_articles WHERE id='{aid}') + "
        f"(SELECT count(*) FROM visitor_accounts WHERE lower(email)='{reg_email}')")
    check("probe data fully removed", left == "0", f"remaining={left}")

    print()
    if FAILS:
        print(f"RESULT: FAIL ({len(FAILS)}): " + "; ".join(FAILS))
        return 1
    print("RESULT: PASS")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # noqa: BLE001
        print(f"RESULT: FAIL (exception: {e})")
        sys.exit(1)
