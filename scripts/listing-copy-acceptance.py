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
import re
import subprocess
import sys
import urllib.request

DB_CONTAINER = "swift-postgres"
DB_USER = "swift"
DB_NAME = "multi_directory"
SITE = "https://zaarhub.com"

# Mirrors src/utils/category_guard.rs — keep in sync.
FAM = {
    "tattoo": ["tattoo", "ink studio", "ink studios", "ink lab"],
    "fitness": ["gym", "fitness", "hotworx", "crossfit", "pilates", "yoga", "orangetheory",
                "anytime fitness", "planet fitness", "la fitness", "esporta", "crunch fitness",
                "burn boot camp", "f45", "9round", "title boxing", "jiu jitsu", "jiu-jitsu",
                "martial arts", "kickboxing"],
    "landscaping": ["landscaping", "landscape", "lawn care", "lawncare", "lawn service", "lawn"],
    "dentist": ["dental", "dentist", "orthodont", "endodont", "periodont"],
    "plumber": ["plumbing", "plumber", "rooter"],
    "roofing": ["roofing", "roofer"],
    "hvac": ["hvac", "air conditioning", "heating and air", "heating & air", "air & heat",
             "cooling & heating"],
    "hair": ["hair", "barber", "great clips", "supercuts", "sport clips", "blowout", "styling"],
    "medical_spa": ["med spa", "medical spa", "medspa", "aesthetics", "injectables", "botox"],
    "spa": ["day spa", "spa &", "spa and", "& spa", " spa"],
    "chiro": ["chiro"],
    "physical_therapy": ["physical therapy", "physiotherapy"],
    "vet": ["veterinary", "vet clinic", "animal hospital", "pet hospital", "animal clinic",
            "animal medical"],
    "groomer": ["groomer", "grooming", "pet groom", "dog groom"],
    "pizza": ["pizza", "pizzeria"],
    "bakery": ["bakery", "bake shop", "patisserie", "pastry"],
    "coffee": ["coffee", "espresso", "roasters", "cafe", "café", "tea house"],
    "accountant": ["cpa", "accounting", "tax service", "bookkeeping", "tax &", "tax and"],
    "lawyer": ["law office", "law firm", "lawyer", "attorney", "injury", "legal"],
    "realestate": ["realty", "real estate", "realtor"],
    "property_mgmt": ["property management"],
    "pool": ["pool service", "pool cleaning", "pool care", "pools", "pool cleaners", "pool "],
    "tree": ["tree service", "tree care", "arborist", "tree removal", "tree trimming"],
    "cleaning": ["cleaning", "cleaners", "janitorial", "maid service", "maids", "pressure washing"],
    "electrician": ["electric", "electrical"],
    "auto": ["auto repair", "automotive", "mechanic", "tire", "oil change", "transmission",
             "brake", "collision", "body shop", "car wash", "detailing"],
    "boat": ["boat", "charter", "sailing", "marina"],
    "childcare": ["daycare", "day care", "preschool", "montessori", "learning center",
                  "kids academy"],
}
CATFAM = {
    "tattoo parlor": "tattoo", "fitness studio": "fitness", "landscaping": "landscaping",
    "dentist": "dentist", "plumber": "plumber", "roofing": "roofing", "hvac": "hvac",
    "hair salon": "hair", "day spa": "spa", "medical spa": "medical_spa",
    "chiropractor": "chiro", "physical therapy": "physical_therapy", "veterinarian": "vet",
    "pet groomer": "groomer", "pizza": "pizza", "bakeries & dessert shops": "bakery",
    "coffee shop": "coffee", "accountant": "accountant",
    "personal injury lawyer": "lawyer", "real estate agent": "realestate",
    "property management": "property_mgmt", "pool service": "pool", "tree service": "tree",
    "cleaning services": "cleaning", "electrician": "electrician", "auto repair": "auto",
    "tire shops & oil change": "auto", "car detailing & wash": "auto", "boat charter": "boat",
    "child care & education": "childcare", "dog daycare & boarding": "childcare",
}
CLUSTERS = [("hair", "beauty"), ("spa", "beauty"), ("medical_spa", "beauty"),
            ("chiro", "therapy"), ("physical_therapy", "therapy"), ("vet", "animal"),
            ("groomer", "animal"), ("bakery", "food"), ("coffee", "food"), ("pizza", "food")]
TEMPLATE = re.compile(r"is a local business in .*? offering |View address, phone number, "
                      r"website and reviews\.")


def cluster(family):
    for a, b in CLUSTERS:
        if a == family:
            return b
    return family


def contradicts(name, category):
    n = " " + name.lower() + " "
    implied = {f for f, toks in FAM.items() for t in toks if t in n}
    if "pool" in implied:
        implied.discard("cleaning")
    fam = CATFAM.get(category.strip().lower())
    if not implied or fam is None:
        return False
    return cluster(fam) not in {cluster(i) for i in implied}


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

    rows = q("""SELECT b.name, COALESCE(c.name,'') FROM businesses b
          LEFT JOIN directory_categories c ON c.id = b.category_id
          WHERE COALESCE(b.is_active,true) AND COALESCE(b.status,'active')='active'""", pw)
    bad = [(n, c) for n, c in rows if contradicts(n, c)]
    print(f"category/name contradictions: {len(bad)} of {len(rows)}")
    for n, c in bad[:10]:
        print(f"  {n} -> {c}")
    if bad:
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
        page_cat = c.group(1).strip() if c else cat
        if contradicts(name, page_cat):
            contra += 1
            print(f"  contradiction on {url}: {name} -> {page_cat}")
    print(f"live detail pages: {fetched}/{len(pages)} fetched, {lede} with a real paragraph, "
          f"{template} still on the template sentence, {contra} name/category contradictions")
    if fetched == 0 or lede < fetched * 0.95 or template or contra:
        ok = False

    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
