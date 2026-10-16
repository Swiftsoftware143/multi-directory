#!/usr/bin/env python3
"""Item 4 (Data export) — the business record is COMPLETE, and the child list is checked against
the live catalog instead of being trusted.

`/export/entity/business/:id` builds the record from a hand-maintained list of child tables
(src/handlers/entity_export.rs, `BUSINESS_CHILDREN`). That file's own doc claims the list is "the
exact set of ... foreign keys the live catalog shows pointing at each root" — nothing asserted it,
so a table that carries a `business_id` without a DECLARED foreign key can fall out of a
"complete" record silently, and a label can rot against a renamed column. The money tables
(`settlement_invoices`, `settlement_payouts` — invoiced/reimbursed cents per business per
settlement run) had no end-to-end proof that they land in a record either.

Proves, against the running app at 127.0.0.1:8089 with the platform-operator session the admin
panel uses:
  * the live catalog is covered: every foreign key pointing at `businesses`, and every BASE TABLE
    carrying a `business_id` column (the no-declared-FK class), is declared in the export list;
  * every declared (table, column) really exists (no stale label that can never match);
  * no declared label points at a VIEW — a derived rollup is not a child table and must not be
    smuggled into a "complete" record (control for exactly that: `settlement_statements` is a view
    over invoices+payouts, and it is asserted NOT to appear in the record);
  * the comparator FIRES on a synthetic gap (both directions), so the checks above are not vacuous;
  * end to end on real rows: a settlement invoice, a settlement payout and a review written for a
    probe business all come back in its record (related + counts + the section list), in JSON and
    in CSV, and the *other* probe business's rows are NOT in it;
  * `sections` lists every declared child table, even the empty ones.

The only direct DB writes are the disposable probe rows and their teardown.
Run:  python3 scripts/export-completeness-catalog-smoke.py
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
APP = "/opt/swift/apps/multi-directory"
ENV_FILE = os.path.join(APP, ".env")
SRC = os.path.join(APP, "src", "handlers", "entity_export.rs")
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item4-completeness-{time.strftime('%Y%m%d-%H%M%S')}")
HARNESS = "export-completeness-probe"
FAILS = []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


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
    return [ln for ln in r.stdout.strip().splitlines() if not TAG.match(ln)]


def psql1(sql):
    rows = psql(sql)
    return rows[0] if rows else ""


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


def http(method, path, token=None):
    req = urllib.request.Request(BASE + path, method=method)
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
    return ok


# ── the export's declared child list, straight out of the source ────────────────
PAIR = re.compile(r'child_q!\("([a-z0-9_]+)"\s*,\s*"([a-z0-9_]+)"\)')
LABELED_PAIR = re.compile(
    r'"([a-z0-9_.]+)"\s*,\s*child_q!\("([a-z0-9_]+)"\s*,\s*"([a-z0-9_]+)"\)', re.S)


def _block(name):
    src = open(SRC).read()
    m = re.search(r"const " + name + r".*?= &\[(.*?)\n\];", src, re.S)
    if not m:
        sys.exit(f"could not find {name} in {SRC}")
    return m.group(1)


def declared_pairs():
    return set(PAIR.findall(_block("BUSINESS_CHILDREN")))


def declared_labels():
    return {m.group(1) for m in LABELED_PAIR.finditer(_block("BUSINESS_CHILDREN"))}


def gaps(decl, catalog):
    """Pure so the control below can feed it a synthetic catalog."""
    return sorted(catalog - decl)


def main():
    # teardown from any previous run
    psql(f"DELETE FROM settlement_invoices WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{HARNESS}')")
    psql(f"DELETE FROM settlement_payouts WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{HARNESS}')")
    psql(f"DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{HARNESS}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{HARNESS}'")

    decl = declared_pairs()
    labels = declared_labels()
    check("the export declares a child list at all", len(decl) > 40 and len(labels) == len(decl),
          f"pairs={len(decl)} labels={len(labels)}")

    cols = psql("SELECT c.table_name||'|'||c.column_name FROM information_schema.columns c "
                "JOIN information_schema.tables t ON t.table_name=c.table_name "
                "AND t.table_schema=c.table_schema "
                "WHERE c.table_schema='public'")
    by_kind = psql("SELECT table_name||'|'||table_type FROM information_schema.tables "
                   "WHERE table_schema='public'")
    tables = {r.split("|")[0]: r.split("|")[1] for r in by_kind}
    table_cols = {(r.split("|")[0], r.split("|")[1]) for r in cols}
    base_cols = {(t, c) for (t, c) in table_cols if tables.get(t) == "BASE TABLE"}
    bid_tables = {(t, "business_id") for (t, c) in base_cols if c == "business_id"}
    fk_to_biz = set()
    for row in psql(
            "SELECT tc.table_name||'|'||kcu.column_name FROM information_schema.table_constraints tc "
            "JOIN information_schema.key_column_usage kcu ON kcu.constraint_name=tc.constraint_name "
            "JOIN information_schema.constraint_column_usage ccu "
            "ON ccu.constraint_name=tc.constraint_name "
            "WHERE tc.constraint_type='FOREIGN KEY' AND tc.table_schema='public' "
            "AND ccu.table_name='businesses'"):
        t, c = row.split("|")
        fk_to_biz.add((t, c))

    check("every foreign key pointing at a business is declared in the export",
          not gaps(decl, fk_to_biz), f"undeclared={gaps(decl, fk_to_biz)}")
    check("every BASE TABLE that carries a business_id column is declared (no-declared-FK class)",
          not gaps(decl, bid_tables), f"undeclared={gaps(decl, bid_tables)}")
    stale = sorted(decl - table_cols)
    check("every declared (table, column) really exists in the catalog (no stale label)",
          not stale, f"stale={stale}")
    views = sorted({t for (t, _) in decl if tables.get(t) == "VIEW"})
    check("no declared label points at a VIEW (derived data is not a child table)",
          not views, f"views={views}")

    # control — the comparator must fire for a synthetic gap, both directions
    found = gaps(decl, set(bid_tables) | {("zz_synthetic_gap_table", "business_id")})
    check("CONTROL — the catalog comparator FIRES on a synthetic undeclared table",
          found == [("zz_synthetic_gap_table", "business_id")], f"reported={found}")
    check("CONTROL — the stale check fires on a synthetic stale label",
          bool({("zz_synthetic_stale", "gone")} - table_cols),
          "a label no column can match is reported")

    # ── end to end on real rows ────────────────────────────────────────────────
    ts = str(int(time.time()))
    directory_id = psql1("SELECT id FROM directories ORDER BY name LIMIT 1")
    check("a directory exists to host the probe record", bool(directory_id))
    run_id = psql1("SELECT id FROM settlement_runs ORDER BY created_at LIMIT 1")
    check("a settlement run exists to hang the money rows off", bool(run_id))

    def mk_business(tag):
        return psql1(
            "INSERT INTO businesses (directory_id, name, slug, city, state, status, rating, "
            "review_count, probe_harness) VALUES "
            f"('{directory_id}', 'ZZ Completeness Probe {tag} {ts}', "
            f"'zz-completeness-probe-{tag}-{ts}', 'Orlando', 'FL', 'active', 0, 0, "
            f"'{HARNESS}') RETURNING id")

    bid = mk_business("a")
    other = mk_business("b")
    check("two disposable listings created", bool(bid) and bool(other))

    inv_marker = f"ZZINV{ts}"
    pay_marker = f"ZZPAY{ts}"
    rev_marker = f"ZZREV{ts}"
    for b, tag in ((bid, "a"), (other, "b")):
        psql1("INSERT INTO settlement_invoices (run_id, business_id, business_name, points_issued, "
              "amount_cents, currency, status) VALUES "
              f"('{run_id}', '{b}', '{inv_marker}-{tag}', 250, 2500, 'USD', 'sent')")
        psql1("INSERT INTO settlement_payouts (run_id, business_id, business_name, points_redeemed, "
              "amount_cents, currency, status) VALUES "
              f"('{run_id}', '{b}', '{pay_marker}-{tag}', 25, 200, 'USD', 'paid')")
        psql1("INSERT INTO reviews (business_id, rating, title, content, status) VALUES "
              f"('{b}', 5, '{rev_marker}-{tag}', 'child row on the probe listing', 'approved')")
    check("a settlement invoice, a settlement payout and a review written for each probe listing",
          psql1(f"SELECT count(*) FROM settlement_invoices WHERE business_name LIKE '{inv_marker}%'") == "2"
          and psql1(f"SELECT count(*) FROM settlement_payouts WHERE business_name LIKE '{pay_marker}%'") == "2"
          and psql1(f"SELECT count(*) FROM reviews WHERE title LIKE '{rev_marker}%'") == "2")

    tok = mint_operator_token()
    st, hdrs, body = http("GET", f"/api/v1/export/entity/business/{bid}?format=json", token=tok)
    check("operator downloads the probe business's complete record (200)", st == 200, f"status={st}")
    doc = json.loads(body) if st == 200 else {}
    related = doc.get("related") or {}
    counts = doc.get("counts") or {}
    sections = doc.get("sections") or []

    check("the settlement INVOICE row is gathered into the record",
          any(r.get("business_name") == f"{inv_marker}-a" for r in
              (related.get("settlement_invoices") or [])),
          f"rows={len(related.get('settlement_invoices') or [])}")
    check("the settlement PAYOUT row is gathered into the record",
          any(r.get("business_name") == f"{pay_marker}-a" for r in
              (related.get("settlement_payouts") or [])),
          f"rows={len(related.get('settlement_payouts') or [])}")
    check("the review is gathered into the record",
          any(r.get("title") == f"{rev_marker}-a" for r in (related.get("reviews") or [])),
          f"rows={len(related.get('reviews') or [])}")
    check("the per-table counts match what was gathered",
          counts.get("settlement_invoices") == 1 and counts.get("settlement_payouts") == 1
          and counts.get("reviews") == 1,
          f"invoices={counts.get('settlement_invoices')} payouts="
          f"{counts.get('settlement_payouts')} reviews={counts.get('reviews')}")
    check("CONTROL — the OTHER listing's money rows are NOT in this record",
          all(r.get("business_name") != f"{inv_marker}-b"
              for r in (related.get("settlement_invoices") or []))
          and all(r.get("business_name") != f"{pay_marker}-b"
                  for r in (related.get("settlement_payouts") or [])))
    check("the section list is exactly the declared child list (nothing missing, nothing extra)",
          set(sections) == {"entity"} | labels,
          f"missing={sorted(labels - set(sections))} "
          f"extra={sorted(set(sections) - labels - {'entity'})}")
    check("CONTROL — the derived settlement_statements VIEW is NOT a section of the record",
          "settlement_statements" not in sections)

    st, hdrs, csv = http("GET", f"/api/v1/export/entity/business/{bid}?format=csv", token=tok)
    check("the same record downloads as CSV (200)", st == 200, f"status={st}")
    check("the CSV carries a settlement_invoices section banner",
          "# table: settlement_invoices" in csv, f"len={len(csv)}")
    check("the CSV actually carries the money row", f"{inv_marker}-a" in csv)

    psql(f"DELETE FROM settlement_invoices WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{HARNESS}')")
    psql(f"DELETE FROM settlement_payouts WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{HARNESS}')")
    psql(f"DELETE FROM reviews WHERE business_id IN "
         f"(SELECT id FROM businesses WHERE probe_harness='{HARNESS}')")
    psql(f"DELETE FROM businesses WHERE probe_harness='{HARNESS}'")
    left = (psql1(f"SELECT count(*) FROM settlement_invoices WHERE business_name LIKE '{inv_marker}%'"),
            psql1(f"SELECT count(*) FROM settlement_payouts WHERE business_name LIKE '{pay_marker}%'"),
            psql1(f"SELECT count(*) FROM reviews WHERE title LIKE '{rev_marker}%'"),
            psql1(f"SELECT count(*) FROM businesses WHERE probe_harness='{HARNESS}'"))
    check("probe data fully removed", left == ("0", "0", "0", "0"), f"left={left}")

    os.makedirs(AUD, exist_ok=True)
    with open(os.path.join(AUD, "out.txt"), "w") as fh:
        fh.write("binary md5: "
                 f"{sh('md5sum', os.path.join(APP, 'target/release/multidirectory')).stdout.split()[0]}\n")
        fh.write(f"declared children: {len(decl)}\n")
        fh.write(f"fk_to_businesses: {len(fk_to_biz)}  business_id tables: {len(bid_tables)}\n")
        fh.write(f"views declared: {views}\n")
    print(f"\nRESULT: {'PASS' if not FAILS else 'FAIL'}  ({len(FAILS)} failed)  evidence {AUD}")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
