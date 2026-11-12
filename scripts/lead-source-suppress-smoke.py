#!/usr/bin/env python3
"""A reserved/fixture lead must never fan out to the operator's CoreSwift CRM.

The claim path already consults `probe_harness::classify_claim` (t_f57a3ef3); this proof
covers the LEAD SOURCE (`push_lead_to_coreswift`), which every lead fan-out funnels through
(bookings, submissions, onboarding survey, entity sync, and the manual "push now" button).
A lead whose email sits on a reserved fixture domain (swiftsoftware.dev, *.test, *.invalid,
example.com, …) is fleet machinery and must be suppressed BEFORE the CoreSwift connection is
even resolved — so no contact is minted in the operator's CRM.

Live proof against the RUNNING app with a stand-in CoreSwift hub on the host (no vendor key,
no real hub, no live lead row). One temporary provider_keys row is written, used, deleted.

  A. reserved fixture address (@swiftsoftware.dev)  -> push accepted, hub receives NOTHING
  B. ordinary address (@gulfcoastautorepairfl.com)  -> push accepted, hub receives the contact

B is what stops the fix being "leads silently switched off". Zero residue asserted at the end.

Run:  python3 scripts/lead-source-suppress-smoke.py
Exit: 0 = PASS, 1 = FAIL.
"""
import base64
import hashlib
import hmac
import json
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, HTTPServer

BASE = "http://127.0.0.1:8089/api/v1"
STUB_PORT = 18099
STUB_URL = "http://172.17.0.1:%d" % STUB_PORT
ENV_FILE = "/opt/swift/apps/multi-directory/.env"
NETWORK = "00000000-0000-0000-0000-000000000001"
SUPER = "00000000-0000-0000-0000-000000000002"
LABEL = "lead-source-suppress-proof"
FAILS = []
RECEIVED = []
PASSWORD = None


def check(label, ok, note=""):
    print(("PASS  " if ok else "FAIL  ") + label + (("  [%s]" % note) if note else ""))
    if not ok:
        FAILS.append(label)
    sys.stdout.flush()


def sql(q, stdin=None):
    args = ["docker", "exec"]
    if stdin is not None:
        args += ["-i"]
    args += ["-e", "PGPASSWORD=" + PASSWORD, "swift-postgres", "psql", "-U", "swift",
             "-d", "multi_directory", "-At", "-v", "ON_ERROR_STOP=1"]
    if stdin is None:
        args += ["-c", q]
    r = subprocess.run(args, input=stdin, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip()[:400])
    return r.stdout.strip()


def token():
    secret = None
    for line in open(ENV_FILE):
        s = line.strip()
        if s.startswith("JWT_SECRET="):
            secret = s.split("=", 1)[1].strip().strip('"')
    b64 = lambda o: base64.urlsafe_b64encode(json.dumps(o).encode()).decode().rstrip("=")
    now = int(time.time())
    msg = b64({"alg": "HS256", "typ": "JWT"}) + "." + b64({
        "sub": SUPER, "tid": NETWORK, "role": "super_admin",
        "exp": now + 1200, "iat": now,
        "aud": "multidirectory-api", "iss": "multidirectory"})
    return msg + "." + base64.urlsafe_b64encode(
        hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()).decode().rstrip("=")


def call(method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    r = urllib.request.Request(BASE + path, data=data, method=method)
    r.add_header("Content-Type", "application/json")
    r.add_header("Authorization", "Bearer " + token())
    try:
        with urllib.request.urlopen(r, timeout=40) as x:
            raw = x.read().decode()
            return x.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw)
        except json.JSONDecodeError:
            return e.code, {"__raw": raw}


class Stub(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(n).decode("utf-8", "replace")
        try:
            body = json.loads(raw)
        except Exception:
            body = {"__unparsed": raw}
        RECEIVED.append({"path": self.path, "body": body})
        out = json.dumps({"id": str(uuid.uuid4())}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def log_message(self, *a):
        pass


def main():
    global PASSWORD
    url = subprocess.run(["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
                         capture_output=True, text=True).stdout.strip()
    PASSWORD = url.split("://", 1)[1].split("@", 1)[0].split(":", 1)[1]

    master = subprocess.run(["docker", "exec", "multidirectory", "printenv",
                             "PROVIDER_KEY_ENC_SECRET"],
                            capture_output=True, text=True).stdout.strip()
    check("master key present", len(master) >= 32, "len=%d" % len(master))
    if len(master) < 32:
        return finish()

    before = sql("select count(*) from provider_keys where provider='coreswift'")

    srv = HTTPServer(("0.0.0.0", STUB_PORT), Stub)
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    import uuid as _uuid
    row_id = str(_uuid.uuid4())
    key_sql = master.replace("'", "''")
    secret_val = "csk_proof_" + _uuid.uuid4().hex
    insert = (
        "\\set k '%s'\n"
        "INSERT INTO provider_keys (id, tenant_id, provider, api_key, base_url, is_active, "
        "is_default, label)\n"
        "VALUES ('%s', '%s', 'coreswift',\n"
        "        'enc:v1:' || replace(encode(pgp_sym_encrypt('%s', :'k', "
        "'cipher-algo=aes256, compress-algo=0, s2k-mode=3'), 'base64'), chr(10), ''),\n"
        "        '%s', true, true, '%s');\n" % (key_sql, row_id, NETWORK, secret_val, STUB_URL, LABEL)
    )
    sql("", stdin=insert)
    check("CoreSwift connection registered", sql("select count(*) from provider_keys where id='%s'" % row_id) == "1")

    ts = int(time.time())

    # ---- arm A: reserved fixture address -> suppressed at the source -----------------------
    email_a = "zz-suppress-a-%d@swiftsoftware.dev" % ts
    st, body = call("POST", "/integrations/coreswift/push", {
        "email": email_a, "phone": "5550100181", "name": "Zz Suppress A", "company": "Zz A Co"})
    check("arm A: push accepted", st == 200, "http=%d %s" % (st, json.dumps(body)[:120]))
    check("arm A: reported NOT pushed", body.get("pushed") is False and body.get("success") is False,
          "pushed=%r success=%r" % (body.get("pushed"), body.get("success")))
    check("arm A: hub received NOTHING", len(RECEIVED) == 0,
          "received=%d" % len(RECEIVED))

    # ---- arm B: ordinary address -> still fans out (nothing switched off) ------------------
    email_b = "zz-suppress-b-%d@gulfcoastautorepairfl.com" % ts
    st, body = call("POST", "/integrations/coreswift/push", {
        "email": email_b, "phone": "5550100182", "name": "Zz Suppress B", "company": "Zz B Co"})
    check("arm B: push accepted", st == 200, "http=%d %s" % (st, json.dumps(body)[:120]))
    check("arm B: reported pushed", body.get("pushed") is True and body.get("success") is True,
          "pushed=%r success=%r" % (body.get("pushed"), body.get("success")))
    got = RECEIVED[-1] if RECEIVED else {}
    check("arm B: hub received exactly one contact", len(RECEIVED) == 1,
          "received=%d" % len(RECEIVED))
    check("arm B: the hub got the real lead's email",
          (got.get("body") or {}).get("email") == email_b,
          "email=%r" % (got.get("body") or {}).get("email"))

    # ---- teardown ------------------------------------------------------------------------
    srv.shutdown()
    sql(q="", stdin="DELETE FROM provider_keys WHERE id='%s';\n" % row_id)
    after = sql("select count(*) from provider_keys where provider='coreswift'")
    check("proof connection removed (no residue)", after == before,
          "before=%s after=%s" % (before, after))

    st, health = call("GET", "/health")
    check("app still healthy", st == 200 and isinstance(health, dict)
          and health.get("status") == "ok", "status=%d" % st)
    return finish()


def finish():
    print("\nRESULT:", "PASS" if not FAILS else "FAIL " + "; ".join(FAILS))
    return 0 if not FAILS else 1


if __name__ == "__main__":
    sys.exit(main())
