#!/usr/bin/env python3
"""Item 3 — "Network email: one setup for all cities" — the SEND path, proved end to end.

Before this test existed, every proof of item 3 could only show the *configuration* half: the
one saved setup resolves for all 10 cities. Whether mail could actually LEAVE was never
demonstrated, because the only transport on file (mailgun) has no provider key, so the email
service always answered "skipped" — and a locked-out user was told a reset link was sent that
never arrived.

This test answers the real question without inventing any third-party credential:

  1. snapshots the ONE saved Email/SMTP row byte-for-byte (md5 of its JSON),
  2. points it at a throwaway SMTP server this script runs on 127.0.0.1 (the email service
     container is on the host network, so 127.0.0.1 reaches it),
  3. drives the PRODUCT's own send paths, through the running app:
       * POST /api/v1/directories/<slug>/email-settings/test-send  — the admin card's button,
         for cities that hold NO settings row of their own (so each must inherit the one setup),
       * POST /api/v1/auth/forgot-password  — the real locked-out-user flow (platform system mail),
  4. reads what the SMTP server actually received: it must be one message per send, all carrying
     the ONE network identity (From/Reply-To/from-address), only the To differing,
  5. ALWAYS restores the saved row byte-for-byte (verified by md5) and removes its probe user.

Exit 0 = PASS. Exit 1 = FAIL. Nothing is left behind either way.
Run:  python3 scripts/network-email-send-smoke.py
"""
import hashlib
import hmac
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
SINK_PORT = int(os.environ.get("SINK_PORT", "2525"))
ENV_FILE = "/etc/swift/env/multi-directory.env"
PROBE_EMAIL = "zz-network-email-send-proof@probe.invalid"
FAILS, PASSES = [], []


def sh(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def rec(ok, name, detail=""):
    (PASSES if ok else FAILS).append(name)
    print(("PASS " if ok else "FAIL ") + name + (" :: " + str(detail)[:300] if detail else ""))


def db_password():
    url = sh("docker", "exec", "multidirectory", "printenv", "DATABASE_URL").stdout.strip()
    m = re.search(r"://[^:]+:([^@]+)@", url)
    if not m:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return m.group(1)


PW = db_password()


def psql(sql, quiet=False):
    r = sh("docker", "exec", "-i", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-tAc", sql)
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip())
    out = r.stdout.strip()
    if not quiet and out:
        print("   db:", out[:400])
    return out


def psql_stdin(sql):
    r = sh("docker", "exec", "-i", "-e", f"PGPASSWORD={PW}", "swift-postgres",
           "psql", "-U", "swift", "-d", "multi_directory", "-v", "ON_ERROR_STOP=1", input=sql)
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip())
    return r.stdout


def http(method, path, body=None, token=None):
    req = urllib.request.Request(BASE + path,
                                 data=json.dumps(body).encode() if body is not None else None,
                                 method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")


def mint_token():
    """super_admin JWT, same shape the admin panel uses. Secret never printed."""
    secret = None
    for line in open(ENV_FILE):
        s = line.strip()
        if s.startswith("JWT_SECRET="):
            secret = s.split("=", 1)[1].strip().strip('"')
    if not secret:
        sys.exit("JWT_SECRET not found in " + ENV_FILE)
    b64 = lambda o: __import__("base64").urlsafe_b64encode(json.dumps(o).encode()).decode().rstrip("=")
    now = int(time.time())
    msg = b64({"alg": "HS256", "typ": "JWT"}) + "." + b64({
        "sub": "00000000-0000-0000-0000-000000000002",
        "tid": "00000000-0000-0000-0000-000000000001",
        "role": "super_admin", "exp": now + 1200, "iat": now,
        "aud": "multidirectory-api", "iss": "multidirectory"})
    return msg + "." + __import__("base64").urlsafe_b64encode(
        hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()).decode().rstrip("=")


# ── throwaway SMTP server (plain SMTP; STARTTLS deliberately not advertised) ──
RECEIVED = []
LOCK = threading.Lock()


def _handle(conn):
    f = conn.makefile("rwb", buffering=0)
    say = lambda s: f.write((s + "\r\n").encode())
    say("220 local-email-proof ESMTP ready")
    mail_from, rcpts, lines, in_data = None, [], [], False
    try:
        while True:
            raw = f.readline()
            if not raw:
                break
            line = raw.decode("utf-8", "replace").rstrip("\r\n")
            if in_data:
                if line == ".":
                    in_data = False
                    body = "\n".join(lines)
                    hdr = {}
                    for ln in lines:
                        if not ln.strip():
                            break
                        if ":" in ln:
                            k, v = ln.split(":", 1)
                            hdr[k.strip().lower()] = v.strip()
                    with LOCK:
                        RECEIVED.append({"mail_from": mail_from, "rcpts": rcpts,
                                         "from": hdr.get("from"), "to": hdr.get("to"),
                                         "reply_to": hdr.get("reply-to"),
                                         "subject": hdr.get("subject"), "bytes": len(body)})
                    say("250 2.0.0 Ok")
                    mail_from, rcpts, lines = None, [], []
                else:
                    lines.append(line)
                continue
            up = line.upper()
            if up.startswith("EHLO") or up.startswith("HELO"):
                say("250-local-email-proof"); say("250-8BITMIME"); say("250 SIZE 10485760")
            elif up.startswith("MAIL FROM"):
                mail_from = line.split(":", 1)[1].strip(); say("250 2.1.0 Ok")
            elif up.startswith("RCPT TO"):
                rcpts.append(line.split(":", 1)[1].strip()); say("250 2.1.5 Ok")
            elif up == "DATA":
                in_data, lines = True, []; say("354 End data with <CR><LF>.<CR><LF>")
            elif up in ("RSET", "NOOP"):
                mail_from, rcpts, lines = None, [], []; say("250 2.0.0 Ok")
            elif up == "QUIT":
                say("221 2.0.0 Bye"); break
            else:
                say("502 5.5.2 Command not implemented")
    except Exception:
        pass
    finally:
        try:
            conn.close()
        except Exception:
            pass


def start_sink():
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", SINK_PORT))
    srv.listen(16)

    def serve():
        while True:
            try:
                conn, _ = srv.accept()
            except OSError:
                return
            threading.Thread(target=_handle, args=(conn,), daemon=True).start()

    threading.Thread(target=serve, daemon=True).start()
    time.sleep(0.4)
    return srv


def main():
    # quiet: the row carries the (encrypted) SMTP password — never print it
    row = psql("SELECT row_to_json(t) FROM directory_email_settings t", quiet=True)
    if not row:
        sys.exit("no directory_email_settings row on this platform — nothing to prove")
    before = (row + "\n").encode()
    before_md5 = hashlib.md5(before).hexdigest()
    saved = json.loads(row)
    rec(True, "the ONE saved mail setup is captured byte-for-byte",
        "md5 %s (transport=%s host=%s from=%s)"
        % (before_md5, saved["transport"], saved["smtp_host"], saved["from_email"]))

    # restore statement = every column that could have been touched, updated_at included
    def q(v):
        return "'" + ("" if v is None else str(v)).replace("'", "''") + "'"

    restore = ("UPDATE directory_email_settings SET transport={t}, smtp_host={h}, smtp_port={p}, "
               "smtp_username={u}, smtp_password={pw}, smtp_encryption={e}, from_name={fn}, "
               "from_email={fe}, reply_to={r}, updated_at={up} WHERE id={i};").format(
        t=q(saved["transport"]), h=q(saved["smtp_host"]), p=int(saved["smtp_port"]),
        u=q(saved["smtp_username"]), pw=q(saved["smtp_password"]),
        e=q(saved["smtp_encryption"]), fn=q(saved["from_name"]), fe=q(saved["from_email"]),
        r=q(saved["reply_to"]), up=q(saved["updated_at"]), i=q(saved["id"]))

    sink = start_sink()
    token = mint_token()
    probe_user = None
    try:
        # cities that inherit the one setup (everything except the city holding the row)
        others = psql("SELECT slug FROM directories WHERE status='active' "
                      "AND id <> '%s' ORDER BY slug LIMIT 2" % saved["directory_id"]).split()
        primary = psql("SELECT slug FROM directories WHERE id='%s'" % saved["directory_id"]).strip()
        rec(len(others) == 2, "two other cities exist to inherit the one setup", others + [primary])
        if not others:
            raise SystemExit("no inheriting city to test — cannot prove 'all cities'")

        psql_stdin("UPDATE directory_email_settings SET transport='smtp', smtp_host='127.0.0.1', "
                   "smtp_port=%d, smtp_username='', smtp_password='', smtp_encryption='none' "
                   "WHERE id='%s';" % (SINK_PORT, saved["id"]))

        # 1) the admin card's own send button, from cities that hold no row of their own
        for slug in others:
            st, body = http("POST", "/api/v1/directories/%s/email-settings/test-send" % slug,
                            {"to": "proof-%s@invalid.test" % slug}, token)
            try:
                res = json.loads(body).get("result", {})
            except Exception:
                res = {"status": "unparsable", "detail": body[:200]}
            rec(st == 200 and res.get("status") == "sent",
                "a city that owns no mail settings sent through the ONE setup",
                "%s -> %s" % (slug, res.get("status")))

        # 2) the real locked-out-user flow: platform system mail, no directory at all
        uid = psql("INSERT INTO users (tenant_id, email, password_hash, name, role) VALUES "
                   "((SELECT id FROM tenants ORDER BY created_at LIMIT 1), '%s', 'x-not-a-login', "
                   "'Network email proof', 'staff') RETURNING id" % PROBE_EMAIL).split()[0]
        probe_user = uid
        st, body = http("POST", "/api/v1/auth/forgot-password", {"email": PROBE_EMAIL})
        rec(st == 200 and "reset" in body.lower(),
            "a locked-out user's password-reset request is accepted", body[:80])
        time.sleep(1.5)
        reset_rows = psql("SELECT count(*) FROM password_resets WHERE user_id='%s'" % uid, quiet=True)
        rec(reset_rows == "1", "a reset code was issued for that account", reset_rows)
        time.sleep(1.5)

        # 3) what the mail server actually received
        expected = len(others) + 1
        got = len(RECEIVED)
        rec(got == expected, "every send really left over SMTP (one message each)",
            "received %d of %d" % (got, expected))
        ident = "%s <%s>" % (saved["from_name"], saved["from_email"])
        rec(all(m["from"] == ident for m in RECEIVED),
            "every message carries the ONE network identity as From", ident)
        rec(all(m["reply_to"] == (saved["reply_to"] or "") for m in RECEIVED),
            "every message carries that setup's Reply-To", saved["reply_to"])
        rec(len({m["from"] for m in RECEIVED}) == 1 and len({m["to"] for m in RECEIVED}) == got,
            "one sender, one recipient per message (no fan-out surprises)",
            sorted(m["to"] for m in RECEIVED))
        rec(any("Password Reset" in (m["subject"] or "") for m in RECEIVED),
            "the password-reset message specifically arrived",
            [m["subject"] for m in RECEIVED])

        # 4) the card's Verify button now agrees
        st, body = http("POST", "/api/v1/directories/%s/email-settings/test" % (others[0]), None, token)
        try:
            vres = json.loads(body).get("result", {})
        except Exception:
            vres = {}
        rec(vres.get("status") == "ok", "the card's Verify button reports the real transport OK",
            json.dumps(vres)[:160])
    finally:
        psql_stdin(restore)
        after = (psql("SELECT row_to_json(t) FROM directory_email_settings t", quiet=True) + "\n").encode()
        ok = hashlib.md5(after).hexdigest() == before_md5
        rec(ok, "the saved mail setup is restored byte-for-byte", "md5 " + hashlib.md5(after).hexdigest())
        if probe_user:
            psql("DELETE FROM users WHERE id='%s'" % probe_user)
        left = psql("SELECT count(*) FROM users WHERE email='%s'" % PROBE_EMAIL, quiet=True)
        rec(left == "0", "probe account removed", left)
        sink.close()

    print("\nRESULT: %s (%d passed, %d failed)" % ("PASS" if not FAILS else "FAIL", len(PASSES), len(FAILS)))
    if FAILS:
        for f in FAILS:
            print("  failed:", f)
    return 0 if not FAILS else 1


if __name__ == "__main__":
    sys.exit(main())
