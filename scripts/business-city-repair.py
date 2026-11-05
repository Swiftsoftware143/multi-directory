"""Item: business cards show truncated city names ("Palm", "St.", "Boca").

Reads every business, derives the true city from its own address field
("1350 Pasadena Ave S, St. Petersburg, FL 33707, USA" -> St. Petersburg),
and reports the drift. --apply writes the repair inside one transaction,
after dumping a backup of (id, city, city_slug) to the audit directory.
"""
import os
import re
import subprocess
import sys
import time

APPLY = "--apply" in sys.argv

u = subprocess.run(["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
                   capture_output=True, text=True).stdout.strip()
m = re.match(r"postgres://([^:]+):([^@]+)@([^:/]+):(\d+)/(\w+)", u)
env = {"PGPASSWORD": m.group(2), "PATH": "/usr/bin:/bin"}
DB = ["psql", "-h", "127.0.0.1", "-U", m.group(1), "-d", m.group(5), "-At", "-F", "\t"]


def q(sql):
    r = subprocess.run(DB + ["-c", sql], capture_output=True, text=True, env=env)
    if r.returncode:
        raise SystemExit("psql failed: " + r.stderr[:400])
    return [line for line in r.stdout.splitlines() if line]


def slugify(s):
    s = s.lower().replace("&", " and ")
    s = re.sub(r"[^a-z0-9]+", "-", s).strip("-")
    return s


ADDR = re.compile(r",\s*([^,]+),\s*([A-Z]{2})\s+(\d{5})")
# Some rows carry no street line at all ("Winter Garden, FL 34787, USA").
ADDR_NOSTREET = re.compile(r"^\s*([^,]+),\s*([A-Z]{2})\s+(\d{5})")

rows = q("SELECT id, city, coalesce(city_slug,''), coalesce(address,'') FROM businesses")
# The network's own directory names are the canonical spelling for their city
# ("St Cloud" in the address vs "St. Cloud" in the network).
canon = {}
for line in q("SELECT slug, name FROM directories"):
    s, n = line.split("\t")[:2]
    canon[s] = n

changes = []
for line in rows:
    bid, city, cslug, addr = (line.split("\t") + ["", "", "", ""])[:4]
    mm = ADDR.search(addr) or ADDR_NOSTREET.search(addr)
    if not mm:
        continue
    true_city = mm.group(1).strip()
    ts = slugify(true_city)
    if ts in canon:
        true_city = canon[ts]
    if true_city.lower() != (city or "").strip().lower() or ts != cslug:
        changes.append((bid, city, cslug, true_city, ts))

print(f"rows={len(rows)} drift={len(changes)}")
pairs = {}
for _, _, _, tc, ts in changes:
    pairs[(tc, ts)] = pairs.get((tc, ts), 0) + 1
for (tc, ts), n in sorted(pairs.items()):
    print(f"  -> {tc} [{ts}] x{n}")

if not APPLY:
    print("dry run (pass --apply to write)")
    raise SystemExit(0)

out = "/opt/swift/audits/zz-cityrepair-" + time.strftime("%Y%m%d-%H%M%S")
os.makedirs(out, exist_ok=True)
with open(os.path.join(out, "before.tsv"), "w") as f:
    f.write("id\tcity\tcity_slug\taddress\n")
    for line in q("SELECT id, city, coalesce(city_slug,''), replace(coalesce(address,''),E'\\t',' ') FROM businesses ORDER BY id"):
        f.write(line + "\n")

sql = ["BEGIN;"]
vals = []
esc = lambda s: s.replace("'", "''")
for bid, old_city, old_slug, tc, ts in changes:
    vals.append(f"('{bid}','{esc(tc)}','{esc(ts)}')")
sql.append("UPDATE businesses b SET city=v.city, city_slug=v.slug "
           "FROM (VALUES " + ",".join(vals) + ") AS v(id,city,slug) "
           "WHERE b.id = v.id::uuid;")
sql.append("COMMIT;")
path = os.path.join(out, "repair.sql")
with open(path, "w") as f:
    f.write("\n".join(sql) + "\n")
r = subprocess.run(["psql", "-h", "127.0.0.1", "-U", m.group(1), "-d", m.group(5), "-f", path],
                   capture_output=True, text=True, env=env)
print("apply rc:", r.returncode, r.stderr[:300])

left = q("SELECT count(*) FROM businesses WHERE city_slug ~ '^(palm|st-?|boca|pompano|winter|fort|lighthouse|deerfield|hallandale|altamonte|satellite|coconut|madeira|pembroke|delray|pinellas)$'")
print("truncated-looking slugs left:", left)
with open(os.path.join(out, "after.txt"), "w") as f:
    f.write("drift_before=%d\n" % len(changes))
    f.write("truncated_slugs_left=%s\n" % (left[0] if left else "?"))
print("audit dir:", out)
