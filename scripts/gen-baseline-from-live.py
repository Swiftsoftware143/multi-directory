#!/usr/bin/env python3
r"""Generate src/migrations/000_baseline_live_schema.sql from the LIVE multi-directory catalog.

WHY (kanban t_979f881e, measured 2026-10-02)
  `src/migrations/` was not this app's install path. The live database was built out of band
  (a dump/skeleton) and then the 124 delta files were layered on top of it, so every file in the
  directory was already applied on live (`_migrations` had_errors=false for all 124) and the chain
  had never once been asked to build a schema from nothing. Asked now, it cannot: on an EMPTY
  database only 34 of the 124 files apply and 90 fail, `businesses` is never created (it is first
  authored in 068_schema_reconciliation.sql, which itself fails because the chain collapsed at
  002 — which ALTERs `directories` and REFERENCES `businesses`, neither of which exists on an empty
  database). The app still answers /api/v1/health 200, because the migrator is deliberately
  non-fatal (src/db.rs). A buyer with no agent therefore gets a running but hollow install.
  Evidence: /opt/swift/audits/t_cb86753e/40-fresh-install-finding.txt.

WHAT THIS SCRIPT DOES
  Boots nothing. Reads the live catalog with `pg_dump --schema-only` and rewrites it into ONE file
  that is

    * a COMPLETE install path  — from an empty database it builds exactly the live catalog, and
    * a NO-OP on the live database — every statement is add-only-when-absent, so the runner can
      apply it to the database it was generated from without touching a single object.

  The superseded delta chain moves to src/migrations-legacy/ (see its README). From here on every
  schema change ships as a NEW higher-numbered file beside the baseline, never as an edit to it.

TRANSFORMATIONS (each is the reason a raw dump cannot be shipped)
  CREATE TABLE              -> CREATE TABLE IF NOT EXISTS
  CREATE SEQUENCE           -> CREATE SEQUENCE IF NOT EXISTS
  CREATE [UNIQUE] INDEX     -> ... IF NOT EXISTS
  CREATE VIEW               -> CREATE OR REPLACE VIEW
  CREATE FUNCTION           -> CREATE OR REPLACE FUNCTION
  CREATE TRIGGER            -> DO block that creates it only when pg_trigger does not hold the name
  ALTER TABLE ... ADD CONSTRAINT name def;
                            -> DO block that runs the ADD only when pg_constraint does not already
                               hold `name` on that table. A bare ADD CONSTRAINT is NOT idempotent and
                               would abort the boot on live; DROP/ADD is worse (it cannot drop a
                               primary key another table's FK depends on, and it would rebuild every
                               index on live). Add-only-when-absent can never fail on live and can
                               never silently weaken a constraint.
  ALTER SEQUENCE ... OWNED BY / ALTER COLUMN ... SET DEFAULT
                            -> kept: re-owning a sequence and re-setting the same default are
                               idempotent in PostgreSQL.
  `_migrations` (the runner's ledger) and everything that references it: dropped. Bookkeeping,
    not application schema; src/db.rs::run_migrations creates it at boot.
  psql meta lines, SET and SELECT pg_catalog.set_config: dropped (`\restrict` is not SQL).
  `--` comment blocks: stripped from the FRONT of each statement only. They are deliberately kept
    where they sit INSIDE a dollar-quoted function body (stripping them there would edit the body),
    and COMMENT ON statements are kept — they are the schema's own documentation and are idempotent.

Usage:  python3 scripts/gen-baseline-from-live.py <live-database-url> <output.sql>
Exit 0 = every pg_dump statement was handled; 1 = an unhandled statement (never ship that).
"""
import re
import subprocess
import sys

HEADER = r"""-- 000_baseline_live_schema.sql
-- multi-directory baseline: the shape this database ACTUALLY has.
--
-- WHY THIS FILE EXISTS (kanban t_979f881e, measured 2026-10-02; evidence
-- /opt/swift/audits/t_cb86753e/40-fresh-install-finding.txt)
--   `src/migrations/` was not this app's install path. The live database was created out of band
--   and the 124 delta files were then layered on top of it (all 124 are recorded in `_migrations`
--   with had_errors=false on live), so the chain had never been asked to build a schema from
--   scratch. On an EMPTY database only 34 of the 124 files applied and 90 FAILED: `001_initial.sql`
--   creates just tenants/users/password_resets/_migrations, then `002_templates_and_colors.sql`
--   ALTERs `directories` and creates `business_meta REFERENCES businesses(id)` — neither table
--   exists yet (the core baseline is not in the chain at all), so the chain collapses and
--   `businesses` is never created. The API still answered /api/v1/health 200 on that hollow
--   install, because the migrator is deliberately non-fatal (src/db.rs). A buyer with no agent
--   would have received a running but unusable directory.
--
-- WHAT THIS FILE IS
--   The live catalog - every table, column, constraint, index, sequence, view, function, trigger
--   and column comment - generated FROM the live database by scripts/gen-baseline-from-live.py.
--   It is the SOURCE OF TRUTH for a fresh install or a restore. Every schema change from here on
--   ships as a NEW higher-numbered file beside it (see src/migrations/README.md); never edit this
--   one. The superseded delta chain is preserved in src/migrations-legacy/.
--
-- ON THE LIVE DATABASE EVERY STATEMENT IS A NO-OP
--   CREATE TABLE/SEQUENCE/INDEX IF NOT EXISTS, CREATE OR REPLACE VIEW/FUNCTION, and a constraint or
--   trigger is created only when the catalog does not already hold it. Applying this file to the
--   database it was generated from creates nothing, changes nothing and validates nothing
--   (whole-catalog fingerprint before/after: /opt/swift/audits/t_979f881e/).
--
-- NOT INCLUDED ON PURPOSE: `_migrations`, the runner's ledger (created at boot by
-- src/db.rs::run_migrations). It is bookkeeping, not application schema.
"""

STMT_END = re.compile(r";\s*$")
LEDGER_RE = re.compile(r"(^|[^A-Za-z0-9_])_migrations(_id_seq)?([^A-Za-z0-9_]|$)")

DO_BLOCK = """-- {tbl}.{name}
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = '{name}' AND conrelid = '{tbl}'::regclass
    ) THEN
        ALTER TABLE {tbl} ADD CONSTRAINT {name} {definition};
    END IF;
END
$md_bl$;"""

TRIGGER_BLOCK = """-- {tbl}.{name}
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = '{name}' AND tgrelid = '{tbl}'::regclass AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER {name} {rest};
    END IF;
END
$md_bl$;"""


def split_statements(text):
    """Split like src/db.rs::split_statements: only a `;` in Normal state ends a statement.

    Quote-aware for '...' , "..." (with doubled-quote escapes), $$..$$ / $tag$..$tag$, `--` line
    comments and nestable /* */ block comments. A naive split on ';' would cut every function body
    and every guarded DO block in half.
    """
    stmts, buf, state, tag = [], [], "normal", ""
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if state == "normal":
            if c == "-" and i + 1 < n and text[i + 1] == "-":
                state = "line"
                buf.append(c)
            elif c == "/" and i + 1 < n and text[i + 1] == "*":
                state = "block"
                buf.append(c)
            elif c == "'":
                state = "single"
                buf.append(c)
            elif c == '"':
                state = "double"
                buf.append(c)
            elif c == "$":
                j = i + 1
                t = ""
                while j < n and text[j] != "$":
                    if text[j].isascii() and (text[j].isalnum() or text[j] == "_"):
                        t += text[j]
                        j += 1
                    else:
                        t = ""
                        break
                if t != "" or (j < n and text[j] == "$"):
                    # opening delimiter $tag$
                    buf.append(text[i:j + 1])
                    state, tag = "dollar", t
                    i = j + 1
                    continue
                buf.append(c)
            elif c == ";":
                buf.append(c)          # keep the terminator: statements are re-joined verbatim
                s = "".join(buf).strip()
                if s:
                    stmts.append(s)
                buf = []
            else:
                buf.append(c)
        elif state == "single":
            buf.append(c)
            if c == "'":
                if i + 1 < n and text[i + 1] == "'":
                    buf.append("'")
                    i += 1
                else:
                    state = "normal"
        elif state == "double":
            buf.append(c)
            if c == '"':
                if i + 1 < n and text[i + 1] == '"':
                    buf.append('"')
                    i += 1
                else:
                    state = "normal"
        elif state == "line":
            buf.append(c)
            if c == "\n":
                state = "normal"
        elif state == "block":
            buf.append(c)
            if c == "*" and i + 1 < n and text[i + 1] == "/":
                buf.append("/")
                i += 1
                state = "normal"
        elif state == "dollar":
            if c == "$":
                closing = "$" + tag + "$"
                if text.startswith(closing, i):
                    buf.append(closing)
                    i += len(closing)
                    state = "normal"
                    continue
            buf.append(c)
        i += 1
    s = "".join(buf).strip()
    if s:
        stmts.append(s)
    return stmts


def strip_leading_comments(stmt):
    """Drop leading blank / `--` lines (pg_dump's `-- Name: ...` banners). Never touches a line
    after the first non-comment line, so a function body's own `--` comments survive."""
    lines = stmt.splitlines()
    k = 0
    while k < len(lines) and (not lines[k].strip() or lines[k].lstrip().startswith("--")):
        k += 1
    return "\n".join(lines[k:]).strip()


def transform(stmt):
    """-> (sql, kind) or (None, reason-it-was-not-kept)."""
    if re.match(r"^(SET|SELECT pg_catalog\.set_config)\b", stmt):
        return None, "meta"
    if LEDGER_RE.search(stmt):
        return None, "ledger"
    if stmt.startswith("CREATE TABLE "):
        return stmt.replace("CREATE TABLE ", "CREATE TABLE IF NOT EXISTS ", 1), "table"
    if stmt.startswith("CREATE SEQUENCE "):
        return stmt.replace("CREATE SEQUENCE ", "CREATE SEQUENCE IF NOT EXISTS ", 1), "sequence"
    if stmt.startswith("CREATE UNIQUE INDEX "):
        return stmt.replace("CREATE UNIQUE INDEX ", "CREATE UNIQUE INDEX IF NOT EXISTS ", 1), "index"
    if stmt.startswith("CREATE INDEX "):
        return stmt.replace("CREATE INDEX ", "CREATE INDEX IF NOT EXISTS ", 1), "index"
    if stmt.startswith("CREATE VIEW "):
        return stmt.replace("CREATE VIEW ", "CREATE OR REPLACE VIEW ", 1), "view"
    if stmt.startswith("CREATE FUNCTION "):
        return stmt.replace("CREATE FUNCTION ", "CREATE OR REPLACE FUNCTION ", 1), "function"
    if stmt.startswith("CREATE EXTENSION "):
        if not stmt.startswith("CREATE EXTENSION IF NOT EXISTS "):
            stmt = stmt.replace("CREATE EXTENSION ", "CREATE EXTENSION IF NOT EXISTS ", 1)
        return stmt, "extension"
    # Statements arrive WITHOUT their trailing `;` (the splitter consumes it as the separator),
    # hence the optional `;?` in every anchored pattern below.
    m = re.match(r"^CREATE TRIGGER (\S+)\s+(.*?);?$", stmt, re.S)
    if m:
        name, rest = m.group(1), m.group(2).strip()
        tbl = re.search(r"\sON\s+(\S+)\s", " " + rest + " ")
        if not tbl:
            return None, "UNHANDLED (trigger without ON table): " + stmt.splitlines()[0][:90]
        return TRIGGER_BLOCK.format(name=name, tbl=tbl.group(1), rest=rest), "trigger"
    m = re.match(r"^ALTER TABLE ONLY (\S+)\s+ADD CONSTRAINT (\S+)\s+(.*?);?$", stmt, re.S)
    if m:
        return DO_BLOCK.format(tbl=m.group(1), name=m.group(2), definition=m.group(3)), "constraint"
    if stmt.startswith("ALTER TABLE ") and " ADD CONSTRAINT " in stmt:
        # pg_dump uses `ALTER TABLE ONLY`; guard against any other shape rather than mis-transform.
        return None, "UNHANDLED (non-ONLY ADD CONSTRAINT): " + stmt.splitlines()[0][:90]
    if re.match(r"^ALTER TABLE ONLY \S+\s+ALTER COLUMN \S+ SET DEFAULT ", stmt):
        return stmt, "default"          # re-setting the same default is idempotent
    if re.match(r"^ALTER SEQUENCE \S+ OWNED BY ", stmt):
        return stmt, "seq-owner"        # re-owning a sequence is idempotent
    if stmt.startswith("COMMENT ON "):
        return stmt, "comment"          # COMMENT ON replaces; idempotent
    return None, "UNHANDLED: " + stmt.splitlines()[0][:90]


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    url, out = sys.argv[1], sys.argv[2]
    dump = subprocess.run(
        ["pg_dump", "--schema-only", "--no-owner", "--no-privileges", "--no-tablespaces", url],
        capture_output=True, text=True, check=True,
    ).stdout

    # psql meta lines (`\restrict`, `\.`) are not SQL at all.
    kept = [ln for ln in dump.splitlines() if not ln.startswith("\\")]

    counts, unhandled, out_parts = {}, [], [HEADER]
    for raw in split_statements("\n".join(kept)):
        stmt = strip_leading_comments(raw)
        if not stmt:
            continue
        sql, kind = transform(stmt)
        if sql is None:
            if kind.startswith("UNHANDLED"):
                unhandled.append(kind)
            continue
        counts[kind] = counts.get(kind, 0) + 1
        out_parts.append(sql)

    with open(out, "w") as fh:
        fh.write("\n\n".join(out_parts) + "\n")

    print("kept:", ", ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    print("unhandled statements:", len(unhandled))
    for u in unhandled:
        print("   ", u)
    return 1 if unhandled else 0


if __name__ == "__main__":
    sys.exit(main())
