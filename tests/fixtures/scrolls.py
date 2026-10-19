#!/usr/bin/env python3
"""Disposable HTTP boundary fixture; never reads a real store."""

import hashlib, json, os, pathlib, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

root = pathlib.Path(__file__).resolve().parent
(root / "pid").write_text(str(os.getpid()))
(root / "argv.json").write_text(json.dumps(sys.argv[1:]))


def mode():
    try:
        return (root / "mode").read_text().strip()
    except FileNotFoundError:
        return ""


if mode() == "bad-start":
    print("not json", flush=True)
    time.sleep(300)
if mode() == "exit":
    sys.exit(2)
if mode() == "hang":
    time.sleep(300)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        with (root / "requests.jsonl").open("a") as f:
            f.write(
                json.dumps(
                    {"path": self.path, "etag": self.headers.get("If-None-Match")}
                )
                + "\n"
            )
        state = mode()
        value = {
            "schema_version": 2,
            "ok": True,
            "store_id": "00000000-0000-4000-8000-000000000001",
            "branch": "main",
            "revision": "fixture-rev",
            "schema_revision": "fixture-schema",
        }
        if self.path == "/v0/context":
            value["data"] = {
                "api_version": "v0",
                "read_only": True,
                "operations": [] if state == "legacy" else ["graph"],
                "store_id": value["store_id"],
                "branch": "main",
            }
        elif self.path == "/v0/graph":
            if state == "failure":
                self.send_response(503)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            value["data"] = json.loads((root / "graph.json").read_text())
            if state == "invalid":
                value["data"]["records"]["broken"] = {"id": "wrong"}
            if state == "identity":
                value["store_id"] = "00000000-0000-4000-8000-000000000002"
        else:
            self.send_error(404)
            return
        body = json.dumps(value).encode()
        etag = '"' + hashlib.sha256(body).hexdigest() + '"'
        same = self.headers.get("If-None-Match") == etag
        self.send_response(304 if same else 200)
        self.send_header("ETag", etag)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(0 if same else len(body)))
        self.end_headers()
        if not same:
            self.wfile.write(body)


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(
    json.dumps(
        {
            "schema_version": 2,
            "ok": True,
            "event": "listening",
            "address": "127.0.0.1:" + str(server.server_port),
            "api_version": "v0",
            "read_only": True,
        }
    ),
    flush=True,
)
# More than pipe capacity on both streams must not stall the child.
print("x" * 131072, flush=True)
print("x" * 131072, file=sys.stderr, flush=True)
server.serve_forever()
