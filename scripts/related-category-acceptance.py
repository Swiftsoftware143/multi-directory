#!/usr/bin/env python3
"""Acceptance probe: "More <trade> in <city>" grids contain only that trade (kanban t_b2f7201e).

Measures 20 LIVE business pages — two per city root: one whose own category has fewer than six
members in its city (the case where the old code filled the grid with other trades) and one whose
category has six or more. For every page it splits the "Keep exploring" section into its card
groups and asserts:

  * a heading "More X in <city>" only ever contains cards whose OWN rendered category is X;
  * every rendered card label equals the category the database holds for that row;
  * filler sits under a neutral heading ("Other businesses in <city>"), never the subject's trade;
  * no duplicate cards and never more than six.

Read-only: one SELECT per page plus the page fetches. Exit 0 only when every assertion holds.
"""
import html
import re
import subprocess
import sys
import urllib.request

BASE = "https://zaarhub.com"
TAG = re.compile(r"<[^>]+>")


def _pw() -> str:
    url = subprocess.run(
        ["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    return url.split("://swift:")[1].split("@")[0]


def q(sql: str):
    out = subprocess.run(
        ["docker", "exec", "-e", f"PGPASSWORD={_pw()}", "swift-postgres", "psql", "-U", "swift",
         "-d", "multi_directory", "-tAF", "\x1f", "-c", sql],
        capture_output=True, text=True,
    )
    if out.returncode:
        sys.exit("psql: " + out.stderr[:800])
    return [line.split("\x1f") for line in out.stdout.splitlines() if line.strip()]


def pick_pages():
    """:returns [(dir_slug, biz_slug, category, group_size)] — a small-group and a large-group
    page for every city root."""
    rows = q("""
        WITH g AS (
          SELECT b.id, d.slug AS dir, b.slug, b.name, coalesce(dc.name,'') AS cat,
                 count(*) OVER (PARTITION BY b.directory_id, b.category_id) AS grp
          FROM businesses b
          JOIN directories d ON d.id = b.directory_id
          LEFT JOIN directory_categories dc ON dc.id = b.category_id
          WHERE b.is_active AND coalesce(b.slug,'') <> ''
        )
        SELECT dir, slug, cat, grp FROM g WHERE grp < 6 ORDER BY dir, grp, name""")
    small = {}
    for d, s, c, g in rows:
        small.setdefault(d, (d, s, c, int(g)))
    rows = q("""
        WITH g AS (
          SELECT b.id, d.slug AS dir, b.slug, b.name, coalesce(dc.name,'') AS cat,
                 count(*) OVER (PARTITION BY b.directory_id, b.category_id) AS grp
          FROM businesses b
          JOIN directories d ON d.id = b.directory_id
          LEFT JOIN directory_categories dc ON dc.id = b.category_id
          WHERE b.is_active AND coalesce(b.slug,'') <> ''
        )
        SELECT dir, slug, cat, grp FROM g WHERE grp >= 6 ORDER BY dir, name""")
    big = {}
    for d, s, c, g in rows:
        big.setdefault(d, (d, s, c, int(g)))
    pages = []
    for d in sorted(set(small) | set(big)):
        if d in small:
            pages.append(small[d])
        if d in big:
            pages.append(big[d])
    return pages


def cards_in(group_html: str):
    out = []
    for m in re.finditer(
        r'<a class="card" href="/([^/]+)/businesses/([^"]+)"><h3>(.*?)</h3>'
        r'<div class="meta">(.*?)</div>',
        group_html,
        re.S,
    ):
        dir_slug, slug, name, meta = m.groups()
        cat = re.search(r'<span class="meta">(.*?)</span>', meta, re.S)
        out.append((dir_slug, slug, html.unescape(TAG.sub("", name)).strip(),
                    html.unescape(TAG.sub("", cat.group(1))).strip() if cat else ""))
    return out


def groups(page: str):
    """[(heading, [cards])] from the one 'Keep exploring' section."""
    start = page.find('aria-label="Keep exploring"')
    if start < 0:
        return []
    section = page[start:]
    out = []
    for gm in re.finditer(
        r'<div class="explore-group"><h3>(.*?)</h3><div class="grid">(.*?)</div></div>',
        section, re.S,
    ):
        heading = html.unescape(TAG.sub("", gm.group(1))).strip()
        out.append((heading, cards_in(gm.group(2))))
    return out


def db_category(dir_slug: str, slug: str) -> str:
    r = q("SELECT coalesce(dc.name,'') FROM businesses b "
          "JOIN directories d ON d.id = b.directory_id "
          "LEFT JOIN directory_categories dc ON dc.id = b.category_id "
          f"WHERE d.slug = '{dir_slug}' AND b.slug = '{slug}' LIMIT 1")
    return r[0][0] if r else ""


def main():
    pages = pick_pages()
    print(f"pages={len(pages)}")
    failures = 0
    for dir_slug, slug, cat, grp in pages:
        url = f"{BASE}/{dir_slug}/businesses/{slug}"
        try:
            with urllib.request.urlopen(url, timeout=30) as r:
                status = r.status
                page = r.read().decode("utf-8", "replace")
        except Exception as e:  # noqa: BLE001 - a failed fetch is a failure of this probe
            print(f"FAIL {url} fetch: {e}")
            failures += 1
            continue
        if status != 200:
            print(f"FAIL {url} status {status}")
            failures += 1
            continue
        gs = groups(page)
        if not gs:
            print(f"FAIL {url} no card group rendered")
            failures += 1
            continue
        seen, problems = [], []
        for heading, cards in gs:
            if heading.startswith("More "):
                want = cat
                if not heading.startswith(f"More {want} in "):
                    problems.append(f"heading '{heading}' is not 'More {want} in ...'")
                for _, _, name, label in cards:
                    if label != want:
                        problems.append(f"'{name}' under '{heading}' is labelled '{label}'")
            for d, s, name, label in cards:
                seen.append(s)
                real = db_category(d, s)
                if label != real:
                    problems.append(f"'{name}' renders '{label}' but DB holds '{real}'")
            if heading.startswith("Other businesses in ") and any(
                label == cat for _, _, _, label in cards
            ):
                problems.append(f"same-category card under neutral heading '{heading}'")
        if len(seen) != len(set(seen)):
            problems.append("duplicate card in the grid")
        if len(seen) > 6:
            problems.append(f"{len(seen)} cards, expected at most 6")
        label = "PASS" if not problems else "FAIL"
        if problems:
            failures += 1
        print(f"{label} {url} group={grp} cards={len(seen)} " +
              " | ".join(problems[:3]) if problems else
              f"{label} {url} group={grp} cards={len(seen)}")
    print(f"failures={failures}")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
