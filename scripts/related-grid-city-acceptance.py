#!/usr/bin/env python3
"""Re-runnable acceptance probe for the related grid on business detail pages (kanban t_cc8e4bf7).

The related grid prints groups under a heading that names a trade and a city. Until t_cc8e4bf7
those queries had no city predicate, so a directory's nearby-town inventory was printed under a
heading that claimed the directory's city — a card reading "13900 County Rd 455, Clermont, FL
34711" sitting under "More Landscaping in Winter Garden, FL".

The fix widens the heading instead of hiding the cards: a group heading names the city when every
card under it is in that city, and says "in and around <city>" as soon as one card is not.

This probe picks detail pages spread over all ten city roots, deliberately starting with the
pages whose in-city same-category group is smallest (the ones that must borrow cards to reach
six, so the ones where a leak shows first), and for each page reports every heading and the
postal city of every card under it.

  python3 scripts/related-grid-city-acceptance.py            # 20 pages, 2 per city root
  python3 scripts/related-grid-city-acceptance.py --per-city 3

Checks per page:
  * every card's postal city agrees with the heading above it: an "in <city>" heading allows
    only that city, an "in and around <city>" heading allows anything but must actually carry at
    least one out-of-city card (an in-city grid must keep saying "in <city>")
  * the grid still renders six cards (a widened heading must not cost inventory)

Exit code 0 = PASS on every check.
"""
import argparse
import html
import re
import subprocess
import sys
import urllib.request

BASE = "https://zaarhub.com"
ENV = "/opt/swift/apps/multi-directory/.env"
UA = "ZaarHubRelatedGridProbe/1.0 (+https://zaarhub.com)"

GROUP_RE = re.compile(
    r'<div class="explore-group"><h3>(?P<heading>.*?)</h3><div class="grid">(?P<grid>.*?)</div></div>',
    re.S,
)
CARD_RE = re.compile(
    r'<a class="card" href="/(?P<city>[^/"]+)/businesses/(?P<slug>[^"]+)"><h3>(?P<name>.*?)</h3>'
    r'<div class="meta">.*?</div>(?:<div class="meta">(?P<addr>.*?)</div>)?',
    re.S,
)
# "4001 Avalon Rd, Winter Garden, FL 34787" -> Winter Garden; "Apopka, FL 32712" -> Apopka
ADDR_CITY_RE = re.compile(r"(?:^|,)\s*([^,]+?)\s*,\s*FL\s*\d{5}")
TAG_RE = re.compile(r"<[^>]+>")

fails = []


def norm(s):
    """Same rule as the app's norm_city: first comma segment, ASCII letters, fold saint->st."""
    head = (s or "").split(",")[0]
    letters = "".join(c for c in head if c.isascii() and c.isalpha()).lower()
    return letters.replace("saint", "st")


def db(sql):
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


def pick_pages(per_city):
    """Sparsest in-city same-category pages per city root — hardest case first."""
    return db(
        f"""
        WITH l AS (
          SELECT b.id, b.slug, b.category_id, b.directory_id,
                 ({norm_sql('b.city')} = {norm_sql('d.city')}) AS inc
          FROM businesses b JOIN directories d ON d.id = b.directory_id
          WHERE b.is_active AND coalesce(b.slug, '') <> ''
        ), g AS (
          SELECT directory_id, category_id, count(*) FILTER (WHERE inc) AS insame
          FROM l WHERE category_id IS NOT NULL GROUP BY 1, 2
        ), ranked AS (
          SELECT d.slug AS city_root, x.slug AS biz_slug,
                 row_number() OVER (PARTITION BY d.id
                                    ORDER BY coalesce(g.insame, 0) ASC, x.slug) AS rn
          FROM l x
          LEFT JOIN g ON g.category_id = x.category_id AND g.directory_id = x.directory_id
          JOIN directories d ON d.id = x.directory_id
        )
        SELECT city_root, biz_slug FROM ranked WHERE rn <= {int(per_city)}
        ORDER BY city_root, rn
        """
    )


def norm_sql(col):
    return (
        "replace(regexp_replace(lower(split_part(coalesce("
        + col
        + ", ''), ',', 1)), '[^a-z]', '', 'g'), 'saint', 'st')"
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--per-city", type=int, default=2)
    args = ap.parse_args()

    pages = pick_pages(args.per_city)
    print(f"probing {len(pages)} detail pages across {len({p[0] for p in pages})} city roots")
    total_cards = 0
    widened = 0
    for city_root, biz_slug in pages:
        url = f"{BASE}/{city_root}/businesses/{biz_slug}"
        status, page = get(url)
        if status != 200:
            fails.append(f"{url} -> HTTP {status}")
            print(f"\nHTTP {status} {url}")
            continue
        root_norm = norm(city_root)
        print(f"\n{url}  (directory city: {city_root})")
        groups = list(GROUP_RE.finditer(page))
        if not groups:
            fails.append(f"{url} -> no explore groups rendered")
            print("  NO EXPLORE GROUPS")
            continue
        cards_on_page = 0
        for g in groups:
            heading = html.unescape(TAG_RE.sub("", g.group("heading"))).strip()
            around = " in and around " in heading
            head_city = heading.rsplit(" in and around ", 1)[-1] if around else (
                heading.rsplit(" in ", 1)[-1] if " in " in heading else ""
            )
            if around:
                widened += 1
            print(f"  H3: {heading}   [claims city: {head_city!r}, out-of-city allowed: {around}]")
            out_of_city = 0
            for m in CARD_RE.finditer(g.group("grid")):
                name = html.unescape(TAG_RE.sub("", m.group("name"))).strip()
                addr = html.unescape(TAG_RE.sub(" ", m.group("addr") or "")).strip()
                found = ADDR_CITY_RE.search(addr)
                card_city = found.group(1) if found else ""
                cards_on_page += 1
                total_cards += 1
                local = bool(card_city) and norm(card_city) == root_norm
                if not local:
                    out_of_city += 1
                ok = bool(card_city) and (local or around)
                flag = "ok" if ok else "CONTRADICTION"
                if not ok:
                    fails.append(
                        f"{url}: card {name!r} city {card_city!r} under heading {heading!r}"
                    )
                print(
                    f"     {flag:14s} {name[:40]:40s} | {card_city or 'no city in address':20s}"
                    f" | {addr[-34:] if addr else '(no address)'}"
                )
            if around and out_of_city == 0:
                fails.append(
                    f"{url}: heading {heading!r} says 'and around' but every card is in "
                    f"{city_root} — an in-city grid must keep saying 'in {city_root}'"
                )
                print("     HEADING TOO WIDE: no out-of-city card under it")
        if cards_on_page != 6:
            fails.append(f"{url}: {cards_on_page} cards, expected 6")
            print(f"  CARD COUNT: {cards_on_page} (expected 6)")
    print(
        f"\n{len(pages)} pages, {total_cards} cards checked, {widened} group headings widened "
        f"to 'and around', {len(fails)} contradictions/failures"
    )
    for f in fails:
        print("  FAIL:", f)
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
