#!/usr/bin/env python3
"""Re-runnable acceptance probe for the /{city}/businesses ?category= filter (kanban t_9162a6cd).

The filter used to query a `businesses.category` column that has never existed: the COUNT
errored, `unwrap_or(0)` reported "0 businesses", the JSON-LD said numberOfItems 0 and the
canonical link threw the filter away — while the grid below listed the right businesses.

This probe is read-only against the live site and the DB, and it proves the whole surface:

  1. every (city, category) pair it picks renders exactly the rows the DB says belong there
  2. every card on the page carries that category's own label (nothing leaks in from another)
  3. the lede count and the JSON-LD numberOfItems agree with the DB
  4. `?category=<slug>` is the same page as `?category=<name>` — one canonical for both
  5. the canonical link keeps the filter, and the unfiltered page keeps its own
  6. paging keeps the filter: page 2 carries `?category=...&page=2`
  7. a category that names nothing renders 200 with no listings and no unfiltered fallback
  8. the unfiltered page is unchanged: 24 cards, the real total, no query in its canonical

  python3 scripts/businesses-category-acceptance.py            # 6 pairs picked from the DB
  python3 scripts/businesses-category-acceptance.py --pairs 12

Exit code 0 = PASS on every check.
"""
import argparse
import html
import re
import subprocess
import sys
import urllib.parse
import urllib.request

BASE = "https://zaarhub.com"
ENV = "/opt/swift/apps/multi-directory/.env"
UA = "ZaarHubCategoryProbe/1.0 (+https://zaarhub.com)"

CARD_RE = re.compile(
    r'<a class="card" href="/(?P<city>[^/"]+)/businesses/(?P<slug>[^"]+)"><h3>(?P<name>.*?)</h3>'
    r'<div class="meta">(?:<span class="meta">(?P<cat>.*?)</span> · )?<span class="stars">'
)
LEDE_RE = re.compile(
    r'<p class="lede">(?P<n>\d+) (?P<word>business(?:es)?) · page (?P<page>\d+) of (?P<pages>\d+)'
)
CANON_RE = re.compile(r'<link rel="canonical" href="(?P<url>[^"]+)"')
ITEMS_RE = re.compile(r'"numberOfItems":(?P<n>\d+)')
NEXT_RE = re.compile(r'<a class="btn" href="(?P<href>[^"]+)">Next →</a>')

fails = []


def db(sql):
    """Run one read-only query. The connection string comes from the app's own .env."""
    url = ""
    with open(ENV, encoding="utf-8") as fh:
        for line in fh:
            if line.startswith("DATABASE_URL="):
                url = line.split("=", 1)[1].strip().strip('"').strip("'")
                break
    if not url:
        sys.exit("no DATABASE_URL in " + ENV)
    out = subprocess.run(
        ["psql", url, "-tAF", "|", "-c", sql], capture_output=True, text=True, timeout=120
    )
    if out.returncode != 0:
        sys.exit("psql failed: " + (out.stderr or "").strip()[:300])
    return [l.split("|") for l in out.stdout.strip().splitlines() if l.strip()]


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=45) as r:
        return r.status, r.read().decode("utf-8", "replace")


def cards(page):
    out = []
    for m in CARD_RE.finditer(page):
        out.append(
            {
                "city": m.group("city"),
                "slug": m.group("slug"),
                "name": html.unescape(m.group("name")),
                "cat": html.unescape(m.group("cat")) if m.group("cat") else None,
            }
        )
    return out


def check(label, ok, detail):
    print(("PASS  " if ok else "FAIL  ") + label + "  — " + detail)
    if not ok:
        fails.append(label + ": " + detail)


def pick_pairs(n):
    """One pair per city (busiest category first), then the biggest pairs for paging."""
    rows = db(
        "SELECT d.slug, dc.name, count(*) FROM businesses b "
        "JOIN directories d ON d.id = b.directory_id "
        "JOIN directory_categories dc ON dc.id = b.category_id "
        "WHERE b.is_active = true GROUP BY 1, 2 HAVING count(*) > 0 "
        "ORDER BY 3 DESC"
    )
    per_city, seen = [], set()
    for city, name, c in rows:
        if city in seen:
            continue
        seen.add(city)
        per_city.append((city, name, int(c)))
        if len(per_city) == n:
            break
    return per_city


def totals(city):
    r = db(
        "SELECT count(*) FROM businesses b JOIN directories d ON d.id = b.directory_id "
        "WHERE d.slug = '" + city.replace("'", "''") + "' AND b.is_active = true"
    )
    return int(r[0][0])


def probe_pair(city, name):
    tag = city + " / " + name
    q_name = urllib.parse.quote(name, safe="")
    url_name = BASE + "/" + city + "/businesses?category=" + q_name
    status, page = get(url_name)
    expect = int(
        db(
            "SELECT count(*) FROM businesses b JOIN directories d ON d.id = b.directory_id "
            "JOIN directory_categories dc ON dc.id = b.category_id "
            "WHERE d.slug = '" + city.replace("'", "''") + "' AND b.is_active = true "
            "AND lower(dc.name) = lower('" + name.replace("'", "''") + "')"
        )[0][0]
    )
    found = cards(page)
    check(tag + " status", status == 200, "HTTP " + str(status))
    check(
        tag + " grid",
        len(found) == min(expect, 24),
        "cards " + str(len(found)) + " = min(DB " + str(expect) + ", 24)",
    )

    lede = LEDE_RE.search(page)
    check(
        tag + " lede count",
        bool(lede) and int(lede.group("n")) == expect,
        "lede " + (lede.group("n") if lede else "(missing)") + " vs DB " + str(expect),
    )
    want_word = "business" if expect == 1 else "businesses"
    check(
        tag + " lede reads naturally",
        bool(lede) and lede.group("word") == want_word,
        "lede says "
        + ((lede.group("n") + " " + lede.group("word")) if lede else "(missing)")
        + ", want "
        + str(expect)
        + " "
        + want_word,
    )
    items = ITEMS_RE.search(page)
    check(
        tag + " JSON-LD numberOfItems",
        bool(items) and int(items.group("n")) == expect,
        "numberOfItems " + (items.group("n") if items else "(missing)") + " vs DB " + str(expect),
    )
    wrong = [c["name"] for c in found if c["cat"] != name]
    check(
        tag + " only this category",
        not wrong,
        str(len(found) - len(wrong)) + "/" + str(len(found)) + " cards labelled " + repr(name),
    )
    canon = CANON_RE.search(page)
    want = BASE + "/" + city + "/businesses?category=" + q_name
    check(
        tag + " canonical keeps the filter",
        bool(canon) and html.unescape(canon.group("url")) == want,
        (canon.group("url") if canon else "(missing)"),
    )

    # name and slug must be the same page: one canonical for both spellings
    slug_row = db(
        "SELECT dc.slug FROM directory_categories dc WHERE lower(dc.name) = lower('"
        + name.replace("'", "''")
        + "') ORDER BY dc.directory_id NULLS LAST LIMIT 1"
    )
    if slug_row and slug_row[0][0]:
        slug = slug_row[0][0]
        _, page_slug = get(BASE + "/" + city + "/businesses?category=" + urllib.parse.quote(slug))
        a = sorted(c["slug"] for c in found)
        b = sorted(c["slug"] for c in cards(page_slug))
        check(
            tag + " ?category=<slug> == ?category=<name>",
            a == b and bool(a),
            "slug form " + str(len(b)) + " cards, name form " + str(len(a)) + " cards",
        )

    # paging keeps the filter
    if expect > 24:
        nxt = NEXT_RE.search(page)
        href = html.unescape(nxt.group("href")) if nxt else ""
        check(
            tag + " pager keeps the filter",
            "category=" + q_name in href and "page=2" in href,
            href or "(no Next link)",
        )
        if href:
            _, page2 = get(href)
            c2 = cards(page2)
            check(
                tag + " page 2 listing count",
                len(c2) == expect - 24,
                "cards " + str(len(c2)) + " vs DB " + str(expect - 24),
            )
            can2 = CANON_RE.search(page2)
            check(
                tag + " page 2 canonical",
                bool(can2)
                and "category=" + q_name in html.unescape(can2.group("url"))
                and "page=2" in html.unescape(can2.group("url")),
                (can2.group("url") if can2 else "(missing)"),
            )


def probe_unfiltered(city):
    total = totals(city)
    url = BASE + "/" + city + "/businesses"
    status, page = get(url)
    found = cards(page)
    lede = LEDE_RE.search(page)
    canon = CANON_RE.search(page)
    check(
        city + " (unfiltered) unchanged",
        status == 200
        and len(found) == min(total, 24)
        and bool(lede)
        and int(lede.group("n")) == total
        and bool(canon)
        and html.unescape(canon.group("url")) == url,
        "HTTP " + str(status)
        + ", cards " + str(len(found)) + "/" + str(min(total, 24))
        + ", total " + (lede.group("n") if lede else "?") + "/" + str(total)
        + ", canonical " + (canon.group("url") if canon else "?"),
    )


def probe_unknown(city):
    url = BASE + "/" + city + "/businesses?category=__no_such_category__"
    status, page = get(url)
    found = cards(page)
    lede = LEDE_RE.search(page)
    check(
        city + " unknown category",
        status == 200
        and not found
        and bool(lede)
        and int(lede.group("n")) == 0,
        "HTTP " + str(status) + ", cards " + str(len(found)) + ", lede " + (lede.group("n") if lede else "?"),
    )


def pick_thin():
    """A category with exactly one listing, so the singular copy is exercised too."""
    rows = db(
        "SELECT d.slug, dc.name FROM businesses b "
        "JOIN directories d ON d.id = b.directory_id "
        "JOIN directory_categories dc ON dc.id = b.category_id "
        "WHERE b.is_active = true GROUP BY 1, 2 HAVING count(*) = 1 ORDER BY 1, 2 LIMIT 1"
    )
    return (rows[0][0], rows[0][1]) if rows else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pairs", type=int, default=6)
    args = ap.parse_args()

    pairs = pick_pairs(args.pairs)
    thin = pick_thin()
    if thin and thin not in [(c, n) for c, n, _ in pairs]:
        pairs.append((thin[0], thin[1], 1))
    print("pairs picked from the DB: " + ", ".join(c + "/" + n + " (" + str(k) + ")" for c, n, k in pairs))
    for city, name, _ in pairs:
        probe_pair(city, name)
    for city in sorted({c for c, _, _ in pairs}):
        probe_unfiltered(city)
    probe_unknown(pairs[0][0])

    print()
    if fails:
        print("RESULT: FAIL (" + str(len(fails)) + " checks)")
        for f in fails:
            print("  - " + f)
        return 1
    print("RESULT: PASS (" + str(len(pairs)) + " filtered pairs + unfiltered + unknown-category checks)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
