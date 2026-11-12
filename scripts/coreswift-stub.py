#!/usr/bin/env python3
"""A stand-in CoreSwift for the clean-install probe.

Why this exists: the sellable smoke must prove that a first city on an install with NO
CoreSwift key does not kill the server. With CORESWIFT_URL pointed at a CLOSED port, tenant
registration fails first and the provisioning path that used to panic is never reached -- the
test then passes without exercising the bug. This stub answers registration and list creation
like CoreSwift does, so the app proceeds to directory provisioning with no internal key: the
exact condition that aborted the process on 2026-11-11.

Usage: python3 coreswift-stub.py [port]   (prints the port it bound, then serves)
"""
import json
import sys
import uuid
from http.server import BaseHTTPRequestHandler, HTTPServer

TENANT = "11111111-2222-3333-4444-555555555555"
HITS = []


class Handler(BaseHTTPRequestHandler):
    def _json(self, payload, code=200):
        body = json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):  # noqa: N802 (http.server API)
        length = int(self.headers.get("Content-Length") or 0)
        _ = self.rfile.read(length)
        HITS.append(self.path)
        with open("/tmp/coreswift-stub-hits.log", "a") as fh:
            fh.write(self.path + "\n")
        if self.path.endswith("/api/auth/register"):
            self._json({
                "account": {"id": TENANT, "name": "stub account"},
                "access_token": "stub-token",
                "tenant_id": TENANT,
            })
        elif self.path.endswith("/api/lists"):
            self._json({"id": str(uuid.uuid4())})
        else:
            self._json({"ok": True})

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 0
    srv = HTTPServer(("127.0.0.1", port), Handler)
    print(srv.server_address[1], flush=True)
    srv.serve_forever()
