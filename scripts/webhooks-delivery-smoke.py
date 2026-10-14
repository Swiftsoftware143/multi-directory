#!/usr/bin/env python3
"""Admin Webhooks screen — live end-to-end delivery proof (ZaarHub item 8).

The admin console's "API Access & Webhooks" screen registers webhooks via
`POST /api/v1/webhooks` (src/handlers/api_complete.rs, operator-guarded). Dispatch
(`dispatch_webhook_event`) existed with a full retry/signature/delivery-log implementation but
had **zero callers** (fix 2cd1748): the screen configured a black hole. It is now called from
`POST /api/v1/directory-events` (src/handlers/automation.rs::create_event).

This proves, against the RUNNING app, a REAL receiver, and the live database:
  1. anonymous            -> 401; a plain free signup -> 403;
  2. the operator registers a webhook -> 201, and the response never echoes the signing secret;
  3. a real event fires a REAL signed HTTP POST at the registered URL (a receiver we run here):
     the body carries the event / entity / data, the `X-Webhook-Signature` equals
     sha256(secret + raw body), and the `X-Webhook-Secret` matches;
  4. `webhook_deliveries` logs status='delivered' with response_status_code=200, and
     `webhooks.last_success_at` is stamped — the screen's "Last success" column tells the truth;
  5. a webhook subscribed to a DIFFERENT event stays SILENT (no delivery, no hit);
  6. a webhook pointing at a dead port records status='failed' with an error message
     (`last_failure_at` / `failure_count`) — the failure arm is real too;
  7. the live console page actually renders the Webhooks controls and calls this route.

Run:  python3 scripts/webhooks-delivery-smoke.py
Exit: 0 = PASS, 1 = FAIL.  Removes every probe row it creates in `finally`.
"""
import base64
import hashlib
import hmac
import json
import os
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BASE = os.environ.get("MD_BASE", "http://127.0.0.1:8089")
ENV_FILE = "/opt/swift/apps/multi-directory/.env"
PANEL = "/opt/swift/apps/multi-directory/frontend/admin-panel.html"
TS = int(time.time())
EMAIL = f"probe-webhooks-{TS}@example.invalid"
PASSWORD = "Probe!webhooks1"
SECRET = f"probe-signing-secret-{TS}"
DEAD_PORT = 9  # discard port, nothing listens
FAILS = []

RECEIVED = []          # (path, headers-dict, raw-body)
RECV_LOCK = threading.Lock()


def check(label, ok, note=""):
    print(f"{'PASS' if ok else 'FAIL'}  {label}" + (f"  [{note}]" if note else ""))
    if not ok:
        FAILS.append(label)


class Receiver(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(n)
        with RECV_LOCK:
            RECEIVED.append((self.path, {k.lower(): v for k, v in self.headers.items()}, raw))
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok":true}')

    def log_message(self, *a):
        pass


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def db_password():
    url = subprocess.run(["docker", "exec", "multidirectory", "printenv", "DATABASE_URL"],
                         capture_output=True, text=True).stdout.strip()
    if not url:
        sys.exit("could not read DATABASE_URL from the multidirectory container")
    return url.split("://", 1)[1].split("@", 1)[0].split(":", 1)[1]


def psql(sql):
    r = subprocess.run(["docker", "exec", "-e", f"PGPASSWORD={db_password()}", "swift-postgres",
                        "psql", "-U", "swift", "-d", "multi_directory", "-At", "-c", sql],
                       capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"psql failed: {r.stderr.strip()}\nSQL: {sql}")
    return r.stdout.strip()


def mint_operator_token():
    secret = None
    for line in open(ENV_FILE):
        s = line.strip()
        if s.startswith("JWT_SECRET="):
            secret = s.split("=", 1)[1].strip().strip('"')
    if not secret:
        sys.exit("JWT_SECRET not found in " + ENV_FILE)
    b64 = lambda o: base64.urlsafe_b64encode(json.dumps(o).encode()).decode().rstrip("=")
    now = int(time.time())
    msg = b64({"alg": "HS256", "typ": "JWT"}) + "." + b64({
        "sub": "00000000-0000-0000-0000-000000000002",
        "tid": "00000000-0000-0000-0000-000000000001",
        "role": "super_admin", "exp": now + 1200, "iat": now,
        "aud": "multidirectory-api", "iss": "multidirectory"})
    return msg + "." + base64.urlsafe_b64encode(
        hmac.new(secret.encode(), msg.encode(), hashlib.sha256).digest()).decode().rstrip("=")


def call(method, path, token=None, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def wait_for(pred, timeout=25.0, step=0.5):
    end = time.time() + timeout
    while time.time() < end:
        if pred():
            return True
        time.sleep(step)
    return pred()


def main():
    print(f"=== webhooks delivery proof — {time.strftime('%Y-%m-%d %H:%M:%S')} ===")
    op = mint_operator_token()
    port = free_port()
    srv = ThreadingHTTPServer(("127.0.0.1", port), Receiver)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    live_url = f"http://127.0.0.1:{port}/hook"
    dead_url = f"http://127.0.0.1:{DEAD_PORT}/nobody-listens"

    signup_uid = None
    signup_tid = None
    wh_a = wh_b = wh_c = None
    event_id = None

    try:
        # ---- auth boundaries ------------------------------------------------
        s, _ = call("GET", "/api/v1/webhooks")
        check("anonymous GET /api/v1/webhooks is refused", s == 401, f"status={s}")

        s, body = call("POST", "/api/v1/auth/register",
                       body={"email": EMAIL, "password": PASSWORD, "name": "Probe Webhooks"})
        check("a plain account could be created (POST /auth/register)", s in (200, 201), f"status={s}")
        # `/auth/register` mints a `users` row + a `tenants` row (it is NOT the visitor route),
        # so resolve the identity from `users`. The old lookup read `visitor_accounts`, found
        # nothing, and the teardown silently removed none of it: every run leaked one tenant +
        # one user (eight were left behind by 2026-10-14, which then exhausted the register
        # slug walk and made the ninth run fail at signup).
        signup_uid = psql(f"SELECT id FROM users WHERE lower(email) = lower('{EMAIL}')")
        signup_tid = psql(f"SELECT tenant_id::text FROM users WHERE lower(email) = lower('{EMAIL}')")
        tok = None
        if s in (200, 201):
            tok = json.loads(body).get("token") or json.loads(body).get("access_token")
        if tok is None:
            s2, b2 = call("POST", "/api/v1/auth/login", body={"email": EMAIL, "password": PASSWORD})
            tok = json.loads(b2).get("token") if s2 in (200, 201) else None
        if tok:
            s, _ = call("GET", "/api/v1/webhooks", token=tok)
            check("an ordinary signed-in account is refused (403)", s == 403, f"status={s}")

        # ---- register the webhooks -----------------------------------------
        s, body = call("POST", "/api/v1/webhooks", token=op,
                       body={"url": live_url, "events": ["review.created"], "secret": SECRET})
        check("the operator can register a webhook (POST /webhooks) -> 201", s == 201, f"status={s}")
        if s == 201:
            wh_a = json.loads(body)["id"]
        check("the response never echoes the signing secret", SECRET not in body)

        s, body = call("POST", "/api/v1/webhooks", token=op,
                       body={"url": live_url, "events": ["deal.claimed"], "secret": SECRET})
        wh_b = json.loads(body)["id"] if s == 201 else None

        s, body = call("POST", "/api/v1/webhooks", token=op,
                       body={"url": dead_url, "events": ["contact.created"],
                             "secret": SECRET, "retry_count": 1, "timeout_seconds": 3})
        wh_c = json.loads(body)["id"] if s == 201 else None
        check("all three probe webhooks registered", bool(wh_a and wh_b and wh_c),
              f"a={wh_a} b={wh_b} c={wh_c}")

        # ---- fire a real event ---------------------------------------------
        entity = "11111111-2222-3333-4444-555555555555"
        s, body = call("POST", "/api/v1/directory-events", token=op,
                       body={"event_type": "review.created", "entity_type": "review",
                             "entity_id": entity, "data": {"probe": f"webhook-{TS}"}})
        check("the operator can record a directory event -> 201", s == 201, f"status={s}")
        if s == 201:
            event_id = json.loads(body)["id"]

        # ---- the subscriber receives a REAL signed POST ---------------------
        got = wait_for(lambda: any(path == "/hook" for path, _, _ in RECEIVED), timeout=25)
        hits = [r for r in RECEIVED if r[0] == "/hook"]
        check("the registered subscriber received the event (real HTTP POST)", got and len(hits) == 1,
              f"hits={len(hits)}")

        if hits:
            _, hdrs, raw = hits[0]
            check("the delivery carries a JSON body", "application/json" in hdrs.get("content-type", ""))
            check("the delivery identifies itself as MultiDirectory-Webhook/1.0",
                  "multidirectory-webhook" in hdrs.get("user-agent", "").lower(),
                  hdrs.get("user-agent", ""))
            try:
                payload = json.loads(raw.decode())
            except Exception as e:
                payload = {"_parse_error": str(e)}
            check("the body carries the event type", payload.get("event") == "review.created",
                  f"event={payload.get('event')}")
            check("the body carries the entity id", str(payload.get("entity_id")) == entity)
            check("the body carries the event data",
                  (payload.get("data") or {}).get("probe") == f"webhook-{TS}")
            want_sig = hashlib.sha256(SECRET.encode() + raw).hexdigest()
            check("the X-Webhook-Signature is sha256(secret + body)",
                  hdrs.get("x-webhook-signature") == want_sig,
                  f"got={str(hdrs.get('x-webhook-signature'))[:12]}…")
            check("the X-Webhook-Secret header carries the shared secret",
                  hdrs.get("x-webhook-secret") == SECRET)

        # ---- the delivery log tells the truth ------------------------------
        delivered = psql(
            f"SELECT status || '|' || COALESCE(response_status_code::text,'') || '|' || "
            f"(completed_at IS NOT NULL)::text FROM webhook_deliveries "
            f"WHERE webhook_id = '{wh_a}' ORDER BY created_at DESC LIMIT 1")
        check("webhook_deliveries logs status=delivered, HTTP 200, completed",
              delivered == "delivered|200|true", delivered or "(no row)")

        last_ok = psql(f"SELECT (last_success_at IS NOT NULL)::text FROM webhooks WHERE id = '{wh_a}'")
        check("the webhook's last_success_at is stamped (the screen's 'Last success')", last_ok == "true",
              f"last_success_set={last_ok}")

        # ---- the negative control stays silent -----------------------------
        b_deliveries = psql(f"SELECT COUNT(*) FROM webhook_deliveries WHERE webhook_id = '{wh_b}'")
        b_hits = len(RECEIVED)  # only one receiver; A was the only expected caller
        check("a webhook subscribed to a different event is never called",
              b_deliveries == "0" and b_hits == 1, f"b_deliveries={b_deliveries} total_hits={b_hits}")

        # ---- the failure arm is recorded -----------------------------------
        s2, _ = call("POST", "/api/v1/directory-events", token=op,
                     body={"event_type": "contact.created", "entity_type": "contact",
                           "entity_id": entity, "data": {"probe": f"webhook-dead-{TS}"}})
        failed = wait_for(
            lambda: psql(f"SELECT status FROM webhook_deliveries WHERE webhook_id = '{wh_c}' "
                         f"ORDER BY created_at DESC LIMIT 1") == "failed", timeout=25)
        row = psql(f"SELECT status || '|' || COALESCE(error_message,'') FROM webhook_deliveries "
                   f"WHERE webhook_id = '{wh_c}' ORDER BY created_at DESC LIMIT 1")
        check("an unreachable endpoint records status=failed with an error",
              failed and row.startswith("failed|") and len(row.split("|", 1)[1]) > 0, row or "(no row)")
        fail_meta = psql(f"SELECT (last_failure_at IS NOT NULL)::text || '|' || failure_count::text "
                         f"FROM webhooks WHERE id = '{wh_c}'")
        check("the webhook's failure metadata is stamped (last_failure_at, failure_count)",
              fail_meta.split("|")[0] == "true" and int(fail_meta.split("|")[1] or 0) >= 1, fail_meta)

        # ---- the screen itself ---------------------------------------------
        try:
            s, page = call("GET", "/admin-panel.html")
            live = s == 200 and "wh-list" in page
        except Exception:
            live = False
        check("the live console page renders the API Access & Webhooks screen", live, f"status={s}")
        src = open(PANEL, encoding="utf-8", errors="replace").read()
        check("the screen has a register-webhook control", "wh-url" in src and "wh-secret" in src)
        check("the screen lists webhooks and their deliveries",
              "loadWebhooks" in src and "wh-list" in src and "wh-deliveries" in src)
        check("the screen calls POST /webhooks (register)",
              "apiPost('/webhooks'" in src)
        check("the screen calls DELETE /webhooks/:id", "apiFetch('DELETE', '/webhooks/' + id)" in src)

    finally:
        for wid in (wh_a, wh_b, wh_c):
            if wid:
                psql(f"DELETE FROM webhooks WHERE id = '{wid}'")
        if event_id:
            psql(f"DELETE FROM directory_events WHERE id = '{event_id}'")
        # Everything the signup minted, in FK-safe order: tenant-keyed rows first, then the
        # account, then the tenant itself.
        if signup_tid:
            for tbl in ("directory_events", "webhooks", "api_keys", "provider_keys",
                        "tracked_links", "tag_rules"):
                psql(f"DELETE FROM {tbl} WHERE tenant_id = '{signup_tid}'")
        psql(f"DELETE FROM visitor_accounts WHERE lower(email) = lower('{EMAIL}')")
        psql(f"DELETE FROM users WHERE lower(email) = lower('{EMAIL}')")
        if signup_tid:
            psql(f"DELETE FROM tenants WHERE id = '{signup_tid}'")
        srv.shutdown()
        left = psql("SELECT COUNT(*) FROM webhooks WHERE url LIKE 'http://127.0.0.1:%'")
        tres = psql(f"SELECT COUNT(*) FROM tenants WHERE id = '{signup_tid}'") if signup_tid else "0"
        ures = psql(f"SELECT COUNT(*) FROM users WHERE lower(email) = lower('{EMAIL}')")
        check("the probe removed the account and tenant it created (no residue)",
              left == "0" and tres == "0" and ures == "0",
              f"webhooks={left} tenants={tres} users={ures}")
        print(f"probe rows removed (remaining probe webhooks = {left})")

    print()
    print("RESULT: " + ("PASS" if not FAILS else "FAIL"))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
