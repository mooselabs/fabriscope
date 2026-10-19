#!/usr/bin/env python3
"""Compose both real binaries with disposable local/shared Dolt stores.

No existing store, service, or installed binary is changed. Reports include
render parity, Beads backlinks, conditional reads, refresh latency and RSS.
"""

import argparse, contextlib, json, os, pathlib, selectors, socket, subprocess, threading, time, urllib.request, urllib.error, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

P = pathlib.Path


def command(args, **kw):
    r = subprocess.run(list(map(str, args)), capture_output=True, timeout=600, **kw)
    if r.returncode:
        raise RuntimeError(
            f"{args[0]} {args[1]} failed: {r.stderr.decode()[:800]} {r.stdout.decode()[:800]}"
        )
    return r.stdout


def stop(p):
    if p.poll() is None:
        p.terminate()
        try:
            p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait(timeout=10)


@contextlib.contextmanager
def process(args, folder, env, kind):
    with (folder / (kind + ".stderr")).open("wb") as err:
        p = subprocess.Popen(
            list(map(str, args)), stdout=subprocess.PIPE, stderr=err, env=env
        )
        try:
            with selectors.DefaultSelector() as select:
                select.register(p.stdout, selectors.EVENT_READ)
                if not select.select(120):
                    raise RuntimeError(kind + " startup timed out")
            line = p.stdout.readline().decode().strip()
            if not line:
                raise RuntimeError(
                    kind + " exited: " + (folder / (kind + ".stderr")).read_text()[:800]
                )
            threading.Thread(target=lambda: p.stdout.read(), daemon=True).start()
            yield p, line
        finally:
            stop(p)


def get(url, etag=None):
    request = urllib.request.Request(
        url, headers={"If-None-Match": etag} if etag else {}
    )
    try:
        r = urllib.request.urlopen(request, timeout=120)
    except urllib.error.HTTPError as e:
        r = e
    with r:
        return r.status, r.headers.get("ETag"), r.read()


def rss(pid):
    file = P("/proc") / str(pid) / "status"
    if not file.exists():
        return None
    return {
        line.split(":")[0]: int(line.split()[1])
        for line in file.read_text().splitlines()
        if line.startswith(("VmRSS:", "VmHWM:"))
    }


@contextlib.contextmanager
def beads():
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            item = {
                "id": "demo-1",
                "title": "Linked work",
                "status": "open",
                "issue_type": "task",
                "priority": 2,
                "created_at": "2026-10-19T00:00:00Z",
            }
            body = json.dumps(
                {"items": [item], "has_more": False}
                if self.path.startswith(("/v0/beads/issues", "/v0/beads/ready"))
                else {"summary": {"total": 1}}
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


@contextlib.contextmanager
def shared(out, dolt, env):
    folder = out / "backend"
    folder.mkdir()
    db = folder / ("graph_" + uuid.uuid4().hex)
    db.mkdir()
    command(
        [dolt, "init", "--name", "Fixture", "--email", "fixture@localhost"],
        cwd=db,
        env=env,
    )
    initial = json.loads(
        command(
            [dolt, "sql", "-r", "json", "-q", "SELECT DOLT_HASHOF('HEAD') AS revision"],
            cwd=db,
            env=env,
        )
    )["rows"][0]["revision"]
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    with (out / "backend.log").open("wb") as log:
        server = subprocess.Popen(
            [str(dolt), "sql-server", "--host", "127.0.0.1", "--port", str(port)],
            cwd=db,
            env=env,
            stdout=log,
            stderr=log,
        )
        try:
            deadline = time.monotonic() + 20
            while True:
                assert server.poll() is None
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                        break
                except OSError:
                    if time.monotonic() > deadline:
                        raise RuntimeError("backend startup timed out")
                    time.sleep(0.05)
            config = out / "connection.json"
            config.write_text(
                json.dumps(
                    {
                        "host": "127.0.0.1",
                        "port": port,
                        "database": db.name,
                        "user": "root",
                        "password": "",
                        "io_timeout_seconds": 120,
                    }
                )
            )
            config.chmod(0o600)
            yield config, initial
        finally:
            stop(server)


def qualify(args, out, work_url, connection=None, initial=None):
    out.mkdir()
    env = os.environ.copy()
    env.update(
        DOLT_ROOT_PATH=str(out / "home"),
        DOLT_DISABLE_EVENT_FLUSH="1",
        FABRISCOPE_NO_UPDATE="1",
        TMPDIR=str(out / "viewer-tmp"),
    )
    (out / "viewer-tmp").mkdir()
    base = [
        args.scrolls,
        "--json",
        "--role",
        "human",
        "--actor",
        "fixture",
        "--store",
        out / "store",
        "--dolt",
        args.dolt,
    ]
    if connection:
        base += ["--connection", connection]

    def call(*tail):
        return json.loads(
            command(base + ["--op-id", uuid.uuid4().hex] + list(tail), env=env)
        )["data"]

    if connection:
        plan = call("init", "--dry-run")
        assert plan["base_revision"] == initial
        call(
            "init", "--expect-base", initial, "--expect-schema", plan["ontology_digest"]
        )
    else:
        call("init")
    anchor = call(
        "new",
        "note",
        "--title",
        "Graph anchor",
        "--body",
        "See [work](beads://00000000-0000-4000-8000-000000000001/demo-1).",
    )["id"]
    call(
        "link",
        anchor,
        "related_work",
        "beads://00000000-0000-4000-8000-000000000001/demo-1",
        "--expect",
        "1",
    )
    batch = out / "batch.json"
    batch.write_text(
        json.dumps(
            [
                {
                    "op": "new",
                    "kind": "note",
                    "title": f"Document {n:05}",
                    "body": ("Corpus text with **Markdown**. " * 36),
                }
                for n in range(args.records - 1)
            ]
        )
    )
    call("batch", batch)
    start = time.monotonic()
    call("render", "--out", out / "oracle")
    render_seconds = time.monotonic() - start
    oracle = json.loads((out / "oracle/scrolls/graph.json").read_text())
    for record in oracle["records"].values():
        record.pop("original", None)

    @contextlib.contextmanager
    def launch():
        if connection:
            with process(
                base + ["serve", "--bind", "127.0.0.1:0"], out, env, "scrolls"
            ) as (p, line):
                url = "http://" + json.loads(line)["address"]
                yield ["--scrolls-url", url], p
        else:
            yield (
                [
                    "--scrolls",
                    out / "store",
                    "--scrolls-bin",
                    args.scrolls,
                    "--scrolls-dolt",
                    args.dolt,
                ],
                None,
            )

    with launch() as (source, external):
        start = time.monotonic()
        with process(
            [
                args.fabriscope,
                *source,
                "--beads-url",
                work_url,
                "--bind",
                "127.0.0.1:0",
                "--no-update",
            ],
            out,
            env,
            "fabriscope",
        ) as (viewer, line):
            startup_seconds = time.monotonic() - start
            view_url = "http://" + line.split("http://", 1)[1].split()[0]
            h = json.loads(get(view_url + "/healthz")[2])
            assert h["scrolls"]["scrolls"] == args.records
            graph_url = h["scrolls"]["source"].split(" ", 2)[2] + "/v0/graph"
            start = time.monotonic()
            status, tag, body = get(graph_url)
            snapshot_seconds = time.monotonic() - start
            assert status == 200
            graph = json.loads(body)
            assert graph["data"] == {
                "records": oracle["records"],
                "edges": oracle["edges"],
            }
            start = time.monotonic()
            assert get(graph_url, tag)[0] == 304
            conditional_seconds = time.monotonic() - start
            assert (
                "Scrolls about this bead" in get(view_url + "/bead/demo-1")[2].decode()
            )
            assert "Graph anchor" in get(view_url + "/bead/demo-1")[2].decode()
            assert "bead demo-1" in get(view_url + "/scroll/" + anchor)[2].decode()
            children = P(f"/proc/{viewer.pid}/task/{viewer.pid}/children")
            child_pids = children.read_text().split() if children.exists() else []
            owner_pid = (
                external.pid if external else int(child_pids[0]) if child_pids else None
            )
            memory = {
                "fabriscope_kib": rss(viewer.pid),
                "scrolls_kib": rss(owner_pid) if owner_pid else None,
            }
            start = time.monotonic()
            call("edit", anchor, "--expect", "2", "--title", "Graph refreshed")
            changed_at = time.monotonic()
            deadline = changed_at + 160
            while True:
                health = json.loads(get(view_url + "/healthz")[2])
                if health["scrolls"]["revision"] != h["scrolls"]["revision"]:
                    break
                if time.monotonic() > deadline:
                    raise RuntimeError("viewer failed to refresh")
                time.sleep(0.2)
            refresh_seconds = time.monotonic() - changed_at
            assert "Graph refreshed" in get(view_url + "/scroll/" + anchor)[2].decode()
            assert not list((out / "viewer-tmp").iterdir()), (
                "viewing created temporary artifacts"
            )
        if external:
            assert external.poll() is None, "viewer stopped externally managed server"
        elif owner_pid:
            assert not P(f"/proc/{owner_pid}").exists(), (
                "private child survived shutdown"
            )
    report = {
        "backend": "shared" if connection else "local",
        "records": args.records,
        "edges": len(oracle["edges"]),
        "graph_bytes": len(body),
        "startup_seconds": startup_seconds,
        "snapshot_seconds": snapshot_seconds,
        "conditional_seconds": conditional_seconds,
        "viewer_refresh_seconds": refresh_seconds,
        "render_seconds": render_seconds,
        "render_files": sum(p.is_file() for p in (out / "oracle").rglob("*")),
        "memory": memory,
        "render_parity": True,
        "beads_backlinks": True,
        "viewer_temporary_files": 0,
        "child_cleanup": True,
    }
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ["scrolls", "fabriscope", "dolt"]:
        parser.add_argument("--" + key, type=P, required=True)
    parser.add_argument("--out", type=P, required=True)
    parser.add_argument("--records", type=int, default=8000)
    args = parser.parse_args()
    for key in ["scrolls", "fabriscope", "dolt"]:
        setattr(args, key, getattr(args, key).resolve(strict=True))
    assert args.records >= 2
    args.out = args.out.resolve()
    args.out.mkdir(mode=0o700)
    with beads() as url:
        local = qualify(args, args.out / "local", url)
        env = os.environ.copy()
        env.update(
            DOLT_ROOT_PATH=str(args.out / "backend-home"), DOLT_DISABLE_EVENT_FLUSH="1"
        )
        with shared(args.out, args.dolt, env) as (connection, initial):
            shared_report = qualify(args, args.out / "shared", url, connection, initial)
    report = {"ok": True, "local": local, "shared": shared_report}
    (args.out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
