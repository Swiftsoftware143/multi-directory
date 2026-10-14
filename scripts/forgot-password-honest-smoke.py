#!/usr/bin/env python3
"""ZaarHub password reset — the HONEST arm (queue item 3, network email).

The live install has the network's one Email/SMTP row saved with transport=mailgun and no active
mailgun key, so nothing can leave. Before this fix a locked-out user was answered
200 "If the email exists, a password reset link has been sent" and the page advanced to the
code-entry step — while no message was ever sent and nobody was told. This proves, against the
RUNNING app:

  A. UNCONFIGURED (the live state)
     1. POST /api/v1/auth/forgot-password answers 503 with a plain message;
     2. the body is BYTE-IDENTICAL for a REGISTERED and an UNREGISTERED address — the refusal
        leaks nothing about whether an account exists;
     3. nothing is written (password_resets row count is unchanged);
     4. the real login page (headless Chromium) shows that message in its error box, does NOT
        advance to the code step, does not claim success, and restores the button;
     5. no script error on the page.
  B. CONFIGURED (the network's one row pointed at a local SMTP sink, restored afterwards)
     6. the same call answers 200 with the normal message;
     7. the reset mail really ARRIVES at the sink, to the right address;
     8. it carries the SAME code the server stored (so the mail is usable, not a stub);
     9. a password_resets row is written;
    10. the code actually changes the probe account's password (login with the new one works) —
        the good path is unbroken by the fix.

Run:  python3 scripts/forgot-password-honest-smoke.py
Exit: 0 = PASS, 1 = FAIL.
The saved email settings row, the probe account and every probe row are restored/removed on
EVERY exit path (try/finally).
"""
import json
import os
import re
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
APP = "/opt/swift/apps/multi-directory"
BROWSER = os.path.join(APP, "scripts", "forgot-password-honest-browser.js")
NODE_PATH = "/opt/swift/browser-run/node_modules"
SINK_PORT = int(os.environ.get("SINK_PORT", "2626"))
AUD = os.environ.get("AUD", f"/opt/swift/audits/zz-forgot-honest-{time.strftime('%Y%m%d-%H%M%S')}")
PROBE_EMAIL = f"zz-forgot-honest-{int(time.time())}@probe.invalid"
PROBE_PW = "ZzProbe!23456"
NEW_PW = "ZzProbe!34567"
FAILS, PASSES = [], []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def rec(name, ok, detail=""):
    (PASSES if ok else FAILS).append(name)
    print(("PASS " if ok else "FAIL ") + name + (" :: " + str(detail)[:400] if detail else ""))


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


PW = db_password()


def psql(sql, want_json=False):
    r = sh("docker", "exec", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    out = r.stdout.strip()
    if want_json:
        return json.loads(out) if out else None
    return out


def lit(v):
    """SQL string literal (escape single quotes); None -> NULL."""
    if v is None:
        return "NULL"
    return "'" + str(v).replace("'", "''") + "'"


def http(method, path, payload=None):
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:
        return 0, str(e)


# ---------------------------------------------------------------- SMTP sink
class SmtpSink:
    def __init__(self, port):
        self.msgs = []
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.bind(("127.0.0.1", port))
        self.sock.listen(5)
        self.port = port
        threading.Thread(target=self._serve, daemon=True).start()

    def _serve(self):
        while True:
            try:
                conn, _ = self.sock.accept()
            except OSError:
                return
            threading.Thread(target=self._handle, args=(conn,), daemon=True).start()

    def _handle(self, conn):
        f = conn.makefile("rb")
        try:
            conn.sendall(b"220 sink ready\r\n")
            data_mode = False
            buf = b""
            rcpt = []
            while True:
                line = f.readline()
                if not line:
                    break
                if data_mode:
                    if line.rstrip(b"\r\n") == b".":
                        self.msgs.append({"rcpt": list(rcpt),
                                          "data": buf.decode("utf-8", "replace")})
                        conn.sendall(b"250 OK queued\r\n")
                        data_mode, buf, rcpt = False, b"", []
                    else:
                        buf += line
                    continue
                up = line.decode("utf-8", "replace").strip().upper()
                if up.startswith(("EHLO", "HELO")):
                    conn.sendall(b"250-sink\r\n250 OK\r\n")
                elif up.startswith("MAIL FROM"):
                    conn.sendall(b"250 OK\r\n")
                elif up.startswith("RCPT TO"):
                    rcpt.append(line.decode("utf-8", "replace").split(":", 1)[1].strip().strip("<>"))
                    conn.sendall(b"250 OK\r\n")
                elif up.startswith("DATA"):
                    conn.sendall(b"354 go ahead\r\n")
                    data_mode = True
                elif up.startswith("QUIT"):
                    conn.sendall(b"221 bye\r\n")
                    break
                elif up.startswith("RSET"):
                    rcpt = []
                    conn.sendall(b"250 OK\r\n")
                else:
                    conn.sendall(b"250 OK\r\n")
        except Exception:
            pass
        finally:
            try:
                conn.close()
            except Exception:
                pass

    def wait_for(self, addr, timeout=40):
        t0 = time.time()
        while time.time() - t0 < timeout:
            for m in self.msgs:
                if any(addr.lower() in r.lower() for r in m["rcpt"]):
                    return m
            time.sleep(0.25)
        return None

    def stop(self):
        try:
            self.sock.close()
        except Exception:
            pass


os.makedirs(AUD, exist_ok=True)
sink = None
snap = None
probe_user_id = None
baseline = {}


def snapshot_settings():
    return psql("SELECT row_to_json(t) FROM (SELECT * FROM directory_email_settings) t", want_json=True)


def restore_settings(s):
    if not s:
        return
    psql(
        "UPDATE directory_email_settings SET "
        f"smtp_host={lit(s['smtp_host'])}, smtp_port={int(s['smtp_port'])}, "
        f"smtp_username={lit(s['smtp_username'])}, smtp_password={lit(s['smtp_password'])}, "
        f"smtp_encryption={lit(s['smtp_encryption'])}, from_name={lit(s['from_name'])}, "
        f"from_email={lit(s['from_email'])}, reply_to={lit(s.get('reply_to'))}, "
        f"transport={lit(s['transport'])}, updated_at={lit(str(s['updated_at']))} "
        f"WHERE id={lit(s['id'])}"
    )


def cleanup():
    try:
        if sink:
            sink.stop()
    except Exception:
        pass
    try:
        if probe_user_id:
            psql(f"DELETE FROM password_resets WHERE user_id = {lit(probe_user_id)}")
            psql(f"DELETE FROM users WHERE id = {lit(probe_user_id)}")
        psql(f"DELETE FROM password_resets WHERE user_id IN "
             f"(SELECT id FROM users WHERE email LIKE 'zz-forgot-honest-%@probe.invalid')")
        psql("DELETE FROM users WHERE email LIKE 'zz-forgot-honest-%@probe.invalid'")
    except Exception as e:
        print("cleanup warning:", e)
    try:
        restore_settings(snap)
    except Exception as e:
        print("SETTINGS RESTORE FAILED:", e)


try:
    print(f"== artifact ==")
    head = sh("git", "-C", APP, "rev-parse", "--short", "HEAD").stdout.strip()
    bin_sha = sh("sha256sum", os.path.join(APP, "target/release/multidirectory")).stdout.split()[0]
    proc_sha = sh("docker", "exec", "multidirectory", "sha256sum", "/proc/1/exe").stdout.split()[0]
    print(f"HEAD {head}  binary {bin_sha[:16]}  proc1 {proc_sha[:16]}")
    rec("the running app is the repo's release binary", bin_sha == proc_sha,
        f"bin={bin_sha[:16]} proc={proc_sha[:16]}")
    st, _ = http("GET", "/api/v1/health")
    rec("the app answers health", st == 200, st)

    snap = snapshot_settings()
    rec("the network's Email/SMTP row is on file (item 3: ONE setup)",
        bool(snap), (snap or {}).get("transport"))
    base_users = int(psql("SELECT count(*) FROM users"))
    base_resets = int(psql("SELECT count(*) FROM password_resets"))
    base_visitors = int(psql("SELECT count(*) FROM visitor_accounts"))
    baseline.update(users=base_users, resets=base_resets, visitors=base_visitors)

    real_email = psql("SELECT email FROM users ORDER BY created_at LIMIT 1") or ""

    # ------------------------------------------------- is the live state unconfigured?
    live_ready = False
    if snap:
        t = (snap.get("transport") or "smtp").lower()
        keys = 0
        if t in ("mailgun", "sendgrid", "sendiio"):
            keys = int(psql("SELECT count(*) FROM provider_keys WHERE provider=%s AND is_active "
                            "AND (TRIM(api_key) <> '' OR COALESCE(octet_length(api_key_encrypted),0) > 0)"
                            % lit(t)))
        live_ready = (bool((snap.get("from_email") or "").strip())
                      and ((t == "smtp" and bool((snap.get("smtp_host") or "").strip()))
                           or (t != "smtp" and keys > 0)))
    print(f"== live transport: {snap.get('transport')} — deliverable now: {live_ready}")

    # ============================================================ A. UNCONFIGURED
    if not live_ready:
        body_real, body_fake = "", ""
        if real_email:
            st1, body_real = http("POST", "/api/v1/auth/forgot-password", {"email": real_email})
            rec("A1 a REGISTERED address is refused honestly (503, not a false 'link sent')",
                st1 == 503, f"status={st1} body={body_real[:160]}")
        st2, body_fake = http("POST", "/api/v1/auth/forgot-password",
                              {"email": "zz-nobody-absent@probe.invalid"})
        rec("A1b an UNREGISTERED address is refused the same way", st2 == 503, f"status={st2}")
        if real_email:
            rec("A2 the answer is IDENTICAL for registered vs unregistered (no account oracle)",
                body_real == body_fake, f"real={body_real[:120]} fake={body_fake[:120]}")
        rec("A3 the message is plain and names the real cause",
            "not set up" in body_fake and "contact support" in body_fake, body_fake[:200])
        rec("A4 nothing was written (no reset row leaked into the DB)",
            int(psql("SELECT count(*) FROM password_resets")) == base_resets,
            f"{base_resets} -> {psql('SELECT count(*) FROM password_resets')}")

        # front-end half, real browser
        env = dict(os.environ, MD_BASE=BASE, EMAIL=real_email or "someone@probe.invalid",
                   AUD=AUD, NODE_PATH=NODE_PATH)
        r = subprocess.run(["node", BROWSER], capture_output=True, text=True, env=env, timeout=180)
        try:
            proof = json.loads(open(os.path.join(AUD, "browser-proof.json")).read())
        except Exception:
            proof = {}
        rec("A5 the real login page loaded and its POST was refused",
            proof.get("pageLoaded") and proof.get("postStatus") == 503,
            f"loaded={proof.get('pageLoaded')} post={proof.get('postStatus')} {r.stderr[-160:]}")
        rec("A6 the page SHOWS the honest message to the user",
            bool(proof.get("msgVisible")) and "not set up" in (proof.get("msgText") or ""),
            proof.get("msgText"))
        rec("A7 the page did NOT advance to the code step",
            proof.get("advancedToCodeStep") is False and proof.get("resetStepVisible") is False,
            f"advanced={proof.get('advancedToCodeStep')} resetVisible={proof.get('resetStepVisible')}")
        rec("A8 the page did NOT claim a code was sent and restored its button",
            proof.get("claimedSuccess") is False and proof.get("successVisible") is False
            and (proof.get("btnText") or "").strip() == "Send Reset Code",
            f"success={proof.get('successVisible')} btn={proof.get('btnText')!r}")
        rec("A9 no script error on the page", not proof.get("pageErrors"), proof.get("pageErrors"))
    else:
        print("SKIP A (the live install already has a deliverable transport — nothing to refuse)")

    # ============================================================ B. CONFIGURED
    sink = SmtpSink(SINK_PORT)
    psql("UPDATE directory_email_settings SET transport='smtp', "
         f"smtp_host='127.0.0.1', smtp_port={SINK_PORT}, smtp_username='', smtp_password='', "
         "smtp_encryption='none' WHERE id=%s" % lit(snap["id"]))
    st, _ = http("GET", "/api/v1/health")
    rec("B0 the app is still up after pointing the one row at a local mail server", st == 200, st)

    st_r, body_r = http("POST", "/api/v1/auth/register",
                        {"name": "ZZ Forgot Honest", "email": PROBE_EMAIL, "password": PROBE_PW})
    probe_user_id = psql(f"SELECT id FROM users WHERE email = {lit(PROBE_EMAIL)}") or None
    rec("B1 a probe account exists to reset", probe_user_id is not None,
        f"register status={st_r} {body_r[:120]}")
    if probe_user_id:
        stf, bodyf = http("POST", "/api/v1/auth/forgot-password", {"email": PROBE_EMAIL})
        rec("B2 with email configured the call now answers normally (200)",
            stf == 200, f"status={stf} body={bodyf[:160]}")
        rec("B2b the answer no longer claims the provider is missing",
            "not set up" not in bodyf, bodyf[:160])
        mail = sink.wait_for(PROBE_EMAIL, timeout=40)
        rec("B3 the reset mail REALLY left and arrived at the mail server", mail is not None,
            f"messages={len(sink.msgs)}")
        token = psql(f"SELECT token FROM password_resets WHERE user_id = {lit(probe_user_id)} "
                     "ORDER BY created_at DESC LIMIT 1") or ""
        rec("B4 a reset code was stored for the account", bool(token), token[:12] + "…")
        if mail and token:
            found = token in mail["data"]
            rec("B5 the delivered mail carries the SAME code the server stored", found,
                f"mail bytes={len(mail['data'])}")
        if token:
            stx, bodyx = http("POST", "/api/v1/auth/reset-password",
                              {"token": token, "new_password": NEW_PW})
            rec("B6 the code actually resets the password", stx == 200, f"status={stx} {bodyx[:120]}")
            stl, bodyl = http("POST", "/api/v1/auth/login",
                              {"email": PROBE_EMAIL, "password": NEW_PW})
            rec("B7 the account can sign in with the new password (end to end)", stl == 200,
                f"status={stl} {bodyl[:120]}")
        else:
            rec("B6 the code actually resets the password", False, "no token")
            rec("B7 the account can sign in with the new password", False, "no token")

    # ============================================================ C. RESTORE + RESIDUE
finally:
    cleanup()

print("== C. restore / residue ==")
try:
    now = snapshot_settings()
    rec("C1 the saved Email/SMTP row is restored byte-for-byte", now == snap,
        f"now={ {k: now.get(k) for k in ('transport','smtp_host','from_email')} if now else None}")
    ready_again = False
    st, body = http("POST", "/api/v1/auth/forgot-password", {"email": "zz-postcheck@probe.invalid"})
    if snap and not live_ready:
        ready_again = st == 503 and "not set up" in body
        rec("C2 the live state refuses honestly again (settings restored, not left flipped)",
            ready_again, f"status={st}")
    rec("C3 no probe account left behind",
        int(psql("SELECT count(*) FROM users WHERE email LIKE 'zz-forgot-honest-%@probe.invalid'")) == 0)
    rec("C4 no stray probe rows in users / resets / visitor accounts",
        int(psql("SELECT count(*) FROM users")) == baseline.get("users")
        and int(psql("SELECT count(*) FROM password_resets")) == baseline.get("resets")
        and int(psql("SELECT count(*) FROM visitor_accounts")) == baseline.get("visitors"),
        f"users {baseline.get('users')}->{psql('SELECT count(*) FROM users')} "
        f"resets {baseline.get('resets')}->{psql('SELECT count(*) FROM password_resets')}")
except Exception as e:
    rec("C restore/residue checks ran", False, repr(e))

print(f"\nRESULT: {'PASS' if not FAILS else 'FAIL'}  ({len(PASSES)} passed, {len(FAILS)} failed)")
for f in FAILS:
    print("  FAILED:", f)
sys.exit(1 if FAILS else 0)
