#!/usr/bin/env python3
"""Item 17 — SELLABLE TEST: a clean install with NONE of David's data.

Proves, end to end and against the shipped artifacts only, that a buyer who receives this repo can:

  1. create an empty database and build the whole schema with the app's OWN migration path
     (`multidirectory migrate` — not a hand-run psql transcript),
  2. create the first privileged account with the app's own bootstrap command
     (`multidirectory create-admin`),
  3. BOOT the shipped release binary against that empty database and serve the real product
     (health 200, public site 200, admin login 200, an authenticated admin API 200),
  4. and that the install carries NO content of the current operator's: businesses, directories,
     reviews, loyalty rows, visitor accounts are all ZERO and only the reference catalogues
     (platform tenant + provider presets) have rows.

Everything happens in a throwaway database and a throwaway port; the live app is never touched.
Teardown is in a `finally`: process killed, database dropped, temp dirs removed — and the script
asserts the teardown actually happened.

Run:  python3 scripts/sellable-clean-install-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

APP = "/opt/swift/apps/multi-directory"
BIN = os.path.join(APP, "target", "release", "multidirectory")
ENVFILE = os.environ.get("MD_ENV", "/etc/swift/env/multi-directory.env")
PROBE_DB = os.environ.get("MD_PROBE_DB", "md_sellable_probe")


def pick_port(preferred=None):
    """A TCP port nobody is listening on.

    Never default to a fixed guess: 8090-8099 are all taken by fleet services (8099 is the
    message bus client), and an occupied port makes the instance die at bind time -- which used
    to be reported as `health=404`, or worse, could read a 200 from the stranger holding the port
    and pass without the app ever booting. Ephemeral by default, MD_PROBE_PORT to force one.
    """
    if preferred:
        return int(preferred)
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


PORT = pick_port(os.environ.get("MD_PROBE_PORT"))
BASE = f"http://127.0.0.1:{PORT}"
ADMIN_EMAIL = "cleaninstall-probe@example.com"
ADMIN_PASSWORD = "CleanInstall-Probe-2026!"
# The buyer's first city -- created through the real admin API on the throwaway install.
PROBE_CITY_NAME = "Clean Install Testville"
PROBE_CITY_SLUG = "cleaninstall-testville"
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-item17-sellable-{time.strftime('%Y%m%d-%H%M%S')}")
UPLOADS = "/tmp/md-item17-uploads"
FAILS = []
CHECKS = []


def check(label, ok, detail=""):
    CHECKS.append({"name": label, "ok": bool(ok), "detail": str(detail)[:400]})
    print(("PASS  " if ok else "FAIL  ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILS.append(label)


def read_env():
    env = {}
    with open(ENVFILE) as fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            env[k.strip()] = v.strip()
    return env


def with_db(url, dbname):
    return re.sub(r"/[^/?]+(\?|$)", f"/{dbname}\\1", url)


def psql(url, sql):
    # the database name always comes from the URL — never a default, so a probe can never
    # accidentally query (or drop from) the live database.
    m = re.search(r"/([^/?]+)(\?|$)", url)
    db = m.group(1) if m else PROBE_DB
    r = subprocess.run(
        ["docker", "exec", "-i", "-e", f"PGPASSWORD={dburl_pw(url)}", "swift-postgres",
         "psql", "-U", "swift", "-d", db, "-At", "-c", sql],
        capture_output=True, text=True, timeout=120)
    return r


def dburl_pw(url):
    m = re.search(r"://[^:]+:([^@]+)@", url)
    return m.group(1) if m else ""


def http(path, method="GET", body=None, token=None, allow_redirect=True):
    req = urllib.request.Request(BASE + path, method=method)
    req.add_header("User-Agent", "Mozilla/5.0 (compatible; SwiftOps/1.0)")
    req.add_header("X-Swift-Harness", "item17-sellable")   # attributable rows, never indistinguishable residue
    req.add_header("Accept", "application/json, text/html")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, data=data, timeout=30) as resp:
            return resp.status, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:  # noqa: BLE001
        return 0, f"{type(e).__name__}: {e}"


def main():
    global PORT, BASE
    os.makedirs(AUD, exist_ok=True)
    env = read_env()
    live_url = env.get("DATABASE_URL") or os.environ["DATABASE_URL"]
    probe_url = with_db(live_url, PROBE_DB)
    admin_url = with_db(live_url, "postgres")
    shutil.rmtree(UPLOADS, ignore_errors=True)
    os.makedirs(UPLOADS, exist_ok=True)

    # ---- 1. empty database ---------------------------------------------------------------
    psql(admin_url, f'DROP DATABASE IF EXISTS "{PROBE_DB}"')
    r = psql(admin_url, f'CREATE DATABASE "{PROBE_DB}"')
    check("empty database created", r.returncode == 0, r.stderr.strip()[:200])

    # ---- 2. the app's own migration path -------------------------------------------------
    m = subprocess.run([BIN, "migrate"], cwd=APP, capture_output=True, text=True,
                       env={**os.environ, "DATABASE_URL": probe_url}, timeout=900)
    (open(os.path.join(AUD, "migrate.log"), "w").write(m.stdout + "\n" + m.stderr))
    tail = (m.stdout + m.stderr).strip().splitlines()
    check("`multidirectory migrate` succeeded on an empty database",
          m.returncode == 0 and "0 with errors" in (m.stdout + m.stderr),
          tail[-1] if tail else "")

    tbl = psql(probe_url, "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")
    check("schema is complete (public tables >= 150)", tbl.returncode == 0 and int(tbl.stdout.strip() or 0) >= 150,
          f"{tbl.stdout.strip()} tables")

    led_bad = psql(probe_url, "SELECT count(*) FROM _migrations WHERE had_errors")
    check("migration ledger reports zero failed files", led_bad.stdout.strip() == "0",
          f"had_errors={led_bad.stdout.strip()}")

    # ---- 3. the first privileged account, via the app's own bootstrap command -------------
    c = subprocess.run([BIN, "create-admin", "--email", ADMIN_EMAIL, "--password", ADMIN_PASSWORD,
                        "--name", "Clean Install Probe"], cwd=APP, capture_output=True, text=True,
                       env={**os.environ, "DATABASE_URL": probe_url}, timeout=300)
    (open(os.path.join(AUD, "create-admin.log"), "w").write(c.stdout + "\n" + c.stderr))
    check("`multidirectory create-admin` created the first admin", c.returncode == 0,
          (c.stdout + c.stderr).strip().splitlines()[-1] if (c.stdout + c.stderr).strip() else "")

    # ---- 4. NONE of David's data ---------------------------------------------------------
    empty_tables = ["businesses", "directories", "reviews", "loyalty_members", "loyalty_scans",
                    "visitor_accounts", "claimed_businesses", "payment_providers", "deals",
                    "blog_posts", "business_articles", "email_templates", "networks"]
    for t in empty_tables:
        r = psql(probe_url, f'SELECT count(*) FROM "{t}"')
        check(f"no operator data in {t}", r.returncode == 0 and r.stdout.strip() == "0",
              f"{r.stdout.strip() or r.stderr.strip()[:80]} rows")
    users = psql(probe_url, "SELECT count(*), min(role) FROM users")
    check("exactly one account exists (the one we just created)", users.stdout.strip().startswith("1|"),
          users.stdout.strip())
    prov = psql(probe_url, "SELECT count(*) FROM available_providers")
    check("provider catalogue seeded (operator can configure integrations)",
          int(prov.stdout.strip() or 0) >= 20, f"{prov.stdout.strip()} providers")
    ten = psql(probe_url, "SELECT slug FROM tenants")
    check("platform tenant present", "swiftsoftware" in ten.stdout, ten.stdout.strip())

    # ---- 5. boot the shipped binary against that empty database --------------------------
    proc = None
    logf = None
    ready, status, boot_fail = False, 0, ""
    LOGPATH = os.path.join(AUD, "probe-instance.log")
    try:
        for attempt in (1, 2):
            run_env = {**os.environ}
            for k in ("DATABASE_URL", "JWT_SECRET", "APP_HOST", "APP_PORT", "RUST_LOG",
                      "GOOGLE_PLACES_API_KEY", "CORESWIFT_INTERNAL_KEY", "PROVIDER_KEY_ENC_SECRET"):
                if env.get(k):
                    run_env[k] = env[k]
            for k in [k for k in run_env if k.startswith("CORESWIFT_")]:
                run_env.pop(k, None)   # a throwaway install must never register a workspace in the live CRM
            run_env["DATABASE_URL"] = probe_url
            run_env["APP_PORT"] = str(PORT)
            run_env["APP_HOST"] = "127.0.0.1"
            run_env["REDIS_URL"] = re.sub(r"/\d+$", "/9", env.get("REDIS_URL", "redis://127.0.0.1:6379"))
            run_env["MD_UPLOADS_ROOT"] = UPLOADS
            run_env["RUST_LOG"] = "info"
            logf = open(LOGPATH, "wb")
            proc = subprocess.Popen([BIN], cwd=APP, env=run_env, stdout=logf, stderr=subprocess.STDOUT)

            status = 0
            for _ in range(60):
                if proc.poll() is not None:
                    break          # our instance is gone -- a stranger's 200 must never count
                status, _body = http("/api/v1/health")
                if status == 200:
                    ready = True
                    break
                time.sleep(1)
            logf.flush()
            if ready and proc.poll() is None:
                break
            logf.close()
            full, tail = "", ""
            try:
                with open(LOGPATH, errors="replace") as fh:
                    full = fh.read().strip()
                lines = full.splitlines()
                hits = [ln for ln in lines if "panicked" in ln or "Failed to bind" in ln or "Error" in ln]
                tail = (hits[-1] if hits else (lines[-1] if lines else ""))[:300]
            except Exception:  # noqa: BLE001
                pass
            boot_fail = (f"probe instance exited rc={proc.returncode}: {tail}"
                         if proc.poll() is not None else f"probe instance alive but health={status}")
            if ("Address already in use" in full and attempt == 1
                    and not os.environ.get("MD_PROBE_NO_RETRY")):
                PORT = pick_port()
                BASE = f"http://127.0.0.1:{PORT}"
                proc, ready, boot_fail = None, False, ""
                continue
            break

        check("clean-install instance booted and reports healthy",
              ready and proc is not None and proc.poll() is None,
              f"health={status} port={PORT}" if ready else boot_fail)

        if ready:
            st, body = http("/api/v1/health")
            check("health endpoint is 200 on an empty install", st == 200, f"{st}")

            st, body = http("/")
            check("public site serves the product", st == 200 and "<html" in body.lower(), f"{st} len={len(body)}")

            st, body = http("/admin-panel.html")
            check("admin console page is served", st == 200 and len(body) > 200, f"{st} len={len(body)}")

            st, body = http("/api/v1/auth/login", "POST",
                            {"email": ADMIN_EMAIL, "password": ADMIN_PASSWORD})
            token = None
            try:
                j = json.loads(body)
                token = (j.get("token") or j.get("access_token")
                         or (j.get("data") or {}).get("token")
                         or (j.get("data") or {}).get("access_token"))
            except Exception:  # noqa: BLE001
                j = {}
            check("the created admin can sign in on the clean install", st == 200 and bool(token),
                  f"{st} keys={sorted(j)[:6] if isinstance(j, dict) else '?'}")

            if token:
                st, body = http("/api/v1/admin/reports/stats", token=token)
                check("an authenticated admin API answers on the clean install", st == 200, f"{st} {body[:80]}")
                st, body = http("/api/v1/admin/questions", token=token)
                check("admin content list is empty of operator data", st == 200 and '"data":[]' in body.replace(" ", ""),
                      f"{st} {body[:80]}")

                # ---- 5b. the buyer's FIRST CITY: the whole point of the product -----------
                # A clean install that boots is not sellable; a clean install where a non-technical
                # operator can create their first city and have it complete and reachable is.
                st, body = http("/api/v1/directories", "POST", {
                    "name": PROBE_CITY_NAME,
                    "slug": PROBE_CITY_SLUG,
                    "city": "Testville",
                    "state": "TS",
                    "status": "active",
                }, token=token)
                made = st in (200, 201) and PROBE_CITY_SLUG in body
                check("the operator can create their first city on a clean install", made, f"{st} {body[:100]}")

                st, body = http(f"/api/v1/directories/{PROBE_CITY_SLUG}", token=token)
                check("the new city reads back with its own identity",
                      st == 200 and PROBE_CITY_NAME in body, f"{st} {body[:100]}")

                zones = psql(probe_url,
                             "SELECT count(*) FROM ad_zones z JOIN directories d ON d.id = z.directory_id "
                             f"WHERE d.slug = '{PROBE_CITY_SLUG}'")
                tmpl = psql(probe_url,
                            "SELECT count(*) FROM email_templates t JOIN directories d ON d.id = t.directory_id "
                            f"WHERE d.slug = '{PROBE_CITY_SLUG}'")
                check("a new city is complete on day one (ad zones + emails provisioned)",
                      int(zones.stdout.strip() or 0) > 0 and int(tmpl.stdout.strip() or 0) > 0,
                      f"{zones.stdout.strip()} zones, {tmpl.stdout.strip()} emails")

                st, body = http(f"/city/{PROBE_CITY_SLUG}")
                check("the new city's public page answers", st == 200 and len(body) > 2000,
                      f"{st} len={len(body)}")
    finally:
        if proc and proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=20)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=20)
        if logf is not None:
            try:
                logf.close()
            except Exception:  # noqa: BLE001
                pass

    # ---- 6. teardown ---------------------------------------------------------------------
    r = psql(admin_url, f'DROP DATABASE IF EXISTS "{PROBE_DB}" WITH (FORCE)')
    check("probe database dropped", r.returncode == 0, r.stderr.strip()[:120])
    left = psql(admin_url, f"SELECT count(*) FROM pg_database WHERE datname='{PROBE_DB}'")
    check("no probe database left behind", left.stdout.strip() == "0", left.stdout.strip())
    shutil.rmtree(UPLOADS, ignore_errors=True)
    check("no probe uploads dir left behind", not os.path.isdir(UPLOADS), UPLOADS)

    passed = len([c for c in CHECKS if c["ok"]])
    verdict = "PASS" if not FAILS else "FAIL"
    json.dump({"verdict": verdict, "passed": passed, "failed": len(FAILS),
               "checks": CHECKS, "fails": FAILS},
              open(os.path.join(AUD, "clean-install-proof.json"), "w"), indent=1)
    print(f"\nRESULT: {verdict} ({passed} passed, {len(FAILS)} failed)  evidence: {AUD}")
    return 0 if not FAILS else 1


if __name__ == "__main__":
    sys.exit(main())
