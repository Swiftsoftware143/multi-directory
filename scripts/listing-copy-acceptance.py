#!/usr/bin/env python3
"""Acceptance gate for listing copy and category correctness (kanban t_6b831daa).

Two defects it measures, both of which were live on zaarhub.com:
  1. business detail pages with no description (4,099 of 4,100 had a NULL description, so every
     page rendered one canned sentence and nothing else);
  2. listings filed under a category their own name contradicts ("Fine Ink Studios" under
     Fitness Studio, "HOTWORX" under Landscaping).

Exit 0 = pass. Prints one line per check; nothing else.

Usage:  python3 scripts/listing-copy-acceptance.py [--pages 50]
        (needs docker access to the swift-postgres container)
"""
import html
import os
import re
import subprocess
import sys
import urllib.request
from pathlib import Path

DB_CONTAINER = "swift-postgres"
DB_USER = "swift"
DB_NAME = "multi_directory"
SITE = "https://zaarhub.com"

# Category contradictions are judged by the app's OWN rule: `multidirectory audit-categories`
# runs category_guard::verdict over every listing (name + cached Places types + the family map,
# with the clusters and the name-defence rule) and prints `contradicted=<n>`. A second
# hand-maintained vocabulary here drifts out of sync with the code that actually files listings —
# it already had, and reported a physical-therapy clinic as a spa because the token " spa" appears
# inside "Spanish River".
BINARY = str(Path(__file__).resolve().parent.parent / "target" / "release" / "multidirectory")
TEMPLATE = re.compile(r"is a local business in .*? offering |View address, phone number, ")


def db_url():
    return subprocess.run(
        ["docker", "exec", "multidirectory", "sh", "-lc", 'echo "$DATABASE_URL"'],
        capture_output=True, text=True,
    ).stdout.strip()


def audit_contradictions():
    """:returns (count, sample lines); count is -1 when the audit itself could not run."""
    run = subprocess.run(
        [BINARY, "audit-categories"],
        capture_output=True,
        text=True,
        env={**os.environ, "DATABASE_URL": db_url()},
    )
    m = re.search(r"contradicted=(\d+)", run.stdout)
    if not m:
        return -1, [run.stderr.strip()[:200]]
    return int(m.group(1)), [ln for ln in run.stdout.splitlines() if " -> " in ln]


def db_password():
    raw = subprocess.run(["docker", "exec", "multidirectory", "sh", "-lc", 'echo "$DATABASE_URL"'],
                         capture_output=True, text=True).stdout.strip()
    return raw.split("://", 1)[1].split("@", 1)[0].split(":", 1)[1]


def q(sql, password):
    r = subprocess.run(["docker", "exec", "-i", "-e", f"PGPASSWORD={password}", DB_CONTAINER,
                        "psql", "-U", DB_USER, "-d", DB_NAME, "-tAF\t", "-c", sql],
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"db query failed: {r.stderr.strip()[:200]}")
    return [l.split("\t") for l in r.stdout.strip().split("\n") if l.strip()]


def main():
    want = 50
    if "--pages" in sys.argv:
        want = int(sys.argv[sys.argv.index("--pages") + 1])
    pw = db_password()
    ok = True

    total, filled = q("""SELECT count(*), count(*) FILTER (
              WHERE description IS NOT NULL AND btrim(description) <> '')
          FROM businesses
          WHERE COALESCE(is_active,true) AND COALESCE(status,'active')='active'""", pw)[0]
    total, filled = int(total), int(filled)
    pct = 100.0 * filled / max(1, total)
    print(f"descriptions: {filled}/{total} active listings ({pct:.1f}%)")
    if pct < 95.0:
        print("FAIL: fewer than 95% of active listings have a description")
        ok = False

    stale = int(q("""SELECT count(*) FROM businesses
          WHERE COALESCE(is_active,true) AND COALESCE(status,'active')='active'
            AND enriched_at IS NOT NULL
            AND (description IS NULL OR btrim(description) = '')""", pw)[0][0])
    print(f"enriched_at set on a blank description: {stale}")
    if stale:
        print("FAIL: enrichment claims a listing was enriched while its description is blank")
        ok = False

    total_rows = int(q("""SELECT count(*) FROM businesses
          WHERE COALESCE(is_active,true) AND COALESCE(status,'active')='active'""", pw)[0][0])
    contradicted, samples = audit_contradictions()
    print(f"category/name/type contradictions: {contradicted} of {total_rows}")
    for line in samples[:10]:
        print("  " + line)
    if contradicted != 0:
        # -1 means the audit could not run: an unproven zero is not a pass.
        print("FAIL: the category audit did not prove zero contradictions")
        ok = False

    pages = q(f"""WITH ranked AS (
          SELECT b.name, b.slug AS bslug, d.slug AS dslug, COALESCE(c.name,'') AS cat,
                 row_number() OVER (PARTITION BY d.slug ORDER BY md5(b.id::text)) AS rn
          FROM businesses b JOIN directories d ON d.id = b.directory_id
          LEFT JOIN directory_categories c ON c.id = b.category_id
          WHERE COALESCE(b.is_active,true) AND COALESCE(b.status,'active')='active')
        SELECT name, bslug, dslug, cat FROM ranked WHERE rn <= {max(1, want // 10)}
        ORDER BY dslug, rn""", pw)
    fetched = lede = template = contra = 0
    for name, bslug, dslug, cat in pages:
        url = f"{SITE}/{dslug}/businesses/{bslug}"
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 listing-check"})
            with urllib.request.urlopen(req, timeout=25) as r:
                body = r.read().decode("utf-8", "replace")
                if r.status != 200:
                    continue
        except Exception:
            continue
        fetched += 1
        m = re.search(r"<p class=\"lede\">(.*?)</p>", body, re.S)
        if m and len(re.sub(r"<[^>]+>", "", m.group(1)).split()) >= 15:
            lede += 1
        md = re.search(r'<meta name="description" content="([^"]*)"', body)
        if md and TEMPLATE.search(md.group(1)):
            template += 1
        c = re.search(r"🏷️\s*(?:<a[^>]*>)?([^<]+)", body)
        page_cat = html.unescape(c.group(1).strip()) if c else cat
        # Whatever lede the page carries, the category it PRINTS must be the one the row holds.
        if page_cat != cat:
            contra += 1
            print(f"  category mismatch on {url}: {name} renders '{page_cat}', row holds '{cat}'")
    print(f"live detail pages: {fetched}/{len(pages)} fetched, {lede} with a real paragraph, "
          f"{template} still on the template sentence, {contra} category mismatches")
    if fetched == 0 or lede < fetched * 0.95 or template or contra:
        ok = False

    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
