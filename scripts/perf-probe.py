#!/usr/bin/env python3
"""Disposable loopback audit probe; no production services or GUI are used.
Usage: python3 scripts/perf-probe.py /absolute/path/to/macrun > perf-results.json
All timing results include local scheduling and are NOT WAN benchmarks.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import uuid
import zlib

binary = str(Path(sys.argv[1]).resolve())
children = []
logs = []
results = {"environment": {"os": platform.platform(), "architecture": platform.machine(),
                           "transport": "QUIC loopback; no latency/loss shaping", "profile": os.environ.get("MACRUN_PROBE_PROFILE", "release")}}

with tempfile.TemporaryDirectory(prefix="mrperf-", dir="/tmp") as td:
    root = Path(td)
    source, mirror, server, worker = [root / p for p in ("source", "mirror", "server", "worker")]
    source.mkdir()
    sock = str(root / "control.sock")
    (source / "macrun.toml").write_text(f'remote_root = "{mirror}"\nsync_timeout_seconds = 120\n')

    def spawn(args):
        log = open(root / f"process-{len(children)}.log", "w")
        logs.append(log)
        child = subprocess.Popen([binary, *args], stdout=log, stderr=log)
        children.append(child)
        return child

    def recv_exact(stream, size):
        data = bytearray()
        while len(data) < size:
            chunk = stream.recv(size - len(data))
            if not chunk:
                raise RuntimeError("unexpected EOF")
            data.extend(chunk)
        return bytes(data)

    def request(kind, _workspace=None, **args):
        data = json.dumps({"kind": kind, "workspace": str(_workspace or source), "args": args}).encode()
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(60)
            stream.connect(sock)
            stream.sendall(struct.pack(">I", len(data)) + data)
            size = struct.unpack(">I", recv_exact(stream, 4))[0]
            response = json.loads(recv_exact(stream, size))
        if response["type"] == "error":
            raise RuntimeError(response)
        return response.get("value", response)

    def wait_for(fn, timeout=120):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = fn()
            if value:
                return value
            time.sleep(.005)
        raise TimeoutError("probe deadline exceeded")

    def sync_once():
        started = time.perf_counter()
        accepted = request("sync", detach=True)
        def completed():
            value = request("sync.get", job_id=accepted["job_id"])
            return value if "ended_at" in value else None
        value = wait_for(completed)
        assert value["status"] == "succeeded", value
        return {"wall_ms": round((time.perf_counter() - started) * 1000, 2),
                "phases_ms": value.get("phases_ms"), "sync": value.get("sync")}

    def task_done(ident):
        def completed():
            value = request("task.get", task_id=ident)
            return value if "ended_at" in value else None
        value = wait_for(completed)
        assert value["status"] == "succeeded", value
        return value

    try:
        subprocess.run([binary, "init", "--data", str(server)], check=True, capture_output=True)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.bind(("127.0.0.1", 0))
            port = udp.getsockname()[1]
        spawn(["--socket", sock, "serve", "--listen", f"127.0.0.1:{port}", "--data", str(server)])
        wait_for(lambda: Path(sock).exists())

        # A valid noisy RGB PNG. This is a transport fixture, not a screen capture.
        def png_chunk(kind, payload):
            return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))
        raw = b"".join(b"\0" + os.urandom(1024 * 3) for _ in range(512))
        png = b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 1024, 512, 8, 2, 0, 0, 0)) + png_chunk(b"IDAT", zlib.compress(raw)) + png_chunk(b"IEND", b"")
        (root / "image.png").write_bytes(png)
        fixture = root / "backend.py"
        fixture.write_text('''import sys,json,time,base64,pathlib
image=base64.b64encode(pathlib.Path(sys.argv[1]).read_bytes()).decode()
for line in sys.stdin:
 v=json.loads(line)
 if 'id' not in v: continue
 if v['method']=='initialize': r={'protocolVersion':'2025-03-26','capabilities':{}}
 elif v['method']=='tools/list': r={'tools':[{'name':'observe','annotations':{'readOnlyHint':True},'inputSchema':{'type':'object'}}]}
 else:
  a=v.get('params',{}).get('arguments',{});time.sleep(a.get('sleep',0))
  r={'content':[{'type':'image','mimeType':'image/png','data':image}]} if a.get('image') else {'content':[{'type':'text','text':'ok'}]}
 print(json.dumps({'jsonrpc':'2.0','id':v['id'],'result':r}),flush=True)
''')
        config = root / "worker.toml"
        config.write_text("\n".join(f'[mcp.{name}]\ncommand = {json.dumps(sys.executable)}\nargs = {json.dumps([str(fixture), str(root / "image.png")])}\n' for name in ("a", "b")))
        spawn(["worker", "--server", f"127.0.0.1:{port}", "--cert", str(server / "cert.der"), "--token-file", str(server / "token"), "--data", str(worker), "--config", str(config)])
        wait_for(lambda: request("status")["connected"])

        small = source / "small"
        small.mkdir()
        for i in range(1000):
            (small / f"{i:04}.txt").write_bytes(b"x" * 4096)
        results["small_1000x4KiB_first"] = sync_once()
        results["small_noop_3"] = [sync_once() for _ in range(3)]
        (source / "large.bin").write_bytes(os.urandom(64 * 1024 * 1024))
        results["add_64MiB"] = sync_once()
        results["mixed_noop_3"] = [sync_once() for _ in range(3)]
        with open(source / "large.bin", "r+b") as file:
            file.seek(1024)
            byte = file.read(1)
            file.seek(1024)
            file.write(bytes([byte[0] ^ 1]))
        results["change_one_byte_of_64MiB"] = sync_once()
        assert (mirror / "large.bin").read_bytes() == (source / "large.bin").read_bytes()

        for operation, paths in [("download", [str(mirror / "large.bin"), str(root / "download.bin")]),
                                  ("upload", [str(root / "download.bin"), str(mirror / "upload.bin")])]:
            before = len(list((worker / "tasks").iterdir()))
            started = time.perf_counter()
            subprocess.run([binary, "--socket", sock, operation, *paths], check=True, capture_output=True, timeout=120)
            results[operation + "_64MiB"] = {"wall_ms": round((time.perf_counter() - started) * 1000, 2),
                "worker_task_records_added": len(list((worker / "tasks").iterdir())) - before}
        assert hashlib.sha256((mirror / "upload.bin").read_bytes()).digest() == hashlib.sha256((source / "large.bin").read_bytes()).digest()

        sessions = {name: request("mcp.tools", server=name)["session"] for name in ("a", "b")}
        started = time.perf_counter()
        tasks = [request("mcp.call", server=name, session=sessions[name], tool="observe", arguments={"sleep": .3}, request_id=str(uuid.uuid4()))["task_id"] for name in ("a", "b")]
        for task in tasks:
            task_done(task)
        results["two_backends_each_sleep_300ms"] = {"wall_ms": round((time.perf_counter() - started) * 1000, 2)}
        ident = request("mcp.call", server="a", session=sessions["a"], tool="observe", arguments={"image": True}, request_id=str(uuid.uuid4()))["task_id"]
        task_done(ident)
        samples = []
        for _ in range(3):
            started = time.perf_counter()
            record = request("task.get", task_id=ident)
            data = record["result"]["result"]["content"][0]["data"]
            assert base64.b64decode(data) == png
            samples.append({"wall_ms": round((time.perf_counter() - started) * 1000, 2), "image_base64_bytes": len(data)})
        results["repeat_completed_image_task"] = {"raw_image_bytes": len(png), "samples": samples}
        # New protocol regression checks, deliberately after timed samples.
        summary = request("task.get", task_id=ident, include_result=False)
        assert "result" not in summary and summary["artifacts"]
        refs = request("task.get", task_id=ident, artifact_refs=True)
        assert "data" not in refs["result"]["result"]["content"][0]
        mcp = subprocess.Popen([binary, "--socket", sock, "--workspace", str(source), "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=logs[0], text=True)
        children.append(mcp)
        def send_rpc(number, method, params=None):
            mcp.stdin.write(json.dumps({"jsonrpc":"2.0", "id":number, "method":method, "params":params or {}})+"\n")
            mcp.stdin.flush()
        def receive_rpc():
            return json.loads(mcp.stdout.readline())
        send_rpc(1,"initialize")
        assert receive_rpc()["id"] == 1
        task = request("exec.start", command="sleep 1", cwd=str(mirror), request_id=str(uuid.uuid4()))["task_id"]
        send_rpc(2,"tools/call",{"name":"task_get","arguments":{"task_id":task,"wait_ms":2000,"include_result":False}})
        send_rpc(3,"ping")
        assert receive_rpc()["id"] == 3, "long wait blocked MCP ping"
        assert receive_rpc()["id"] == 2
        def artifact_calls():
            return sum(1 for line in (root/"process-1.log").read_text().splitlines() if '"operation":"artifact.download"' in line and '"event":"operation"' in line)
        before = artifact_calls()
        for number in (4,5):
            send_rpc(number,"tools/call",{"name":"task_get","arguments":{"task_id":ident}})
            value=receive_rpc()
            assert any(c.get("type")=="image" and base64.b64decode(c["data"])==png for c in value["result"]["content"])
        assert artifact_calls()-before == 1, "cached image was fetched more than once"
        mcp.stdin.close();mcp.wait(timeout=5)
        # Independent roots share a worker, while a build excludes writes to its root.
        second = root / "second-source"
        second.mkdir()
        (second / "macrun.toml").write_text(f'remote_root = "{root / "second-mirror"}"\n')
        (second / "input").write_text("independent")
        jobs = [request("sync", detach=True)["job_id"],
                request("sync", _workspace=second, detach=True)["job_id"]]
        def sync_ended(job):
            value = request("sync.get", job_id=job)
            return value if "ended_at" in value else None
        for job in jobs:
            value = wait_for(lambda: sync_ended(job))
            assert value["status"] == "succeeded", value
        task = request("exec.start", command="touch held; sleep 30", cwd=str(mirror), request_id=str(uuid.uuid4()))["task_id"]
        wait_for(lambda: (mirror / "held").exists())
        job = request("sync", detach=True)["job_id"]
        failed = wait_for(lambda: sync_ended(job))
        assert failed["status"] == "failed" and "busy" in str(failed["error"]), failed
        assert request("status")["connected"], "sync failure disconnected worker"
        request("task.cancel", task_id=task)
        wait_for(lambda: request("task.get", task_id=task).get("ended_at"))
        assert sync_once()["sync"]["files"] == 0
        results["new_protocol_checks"] = "summary omits images; image references persisted; MCP ping bypasses long wait; two image reads use one binary download; independent roots sync concurrently; active build blocks sync without disconnecting worker"
        results["assertions"] = "sync content, upload/download hashes, repeated image payload equality all passed"
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        for log in logs:
            log.close()

print(json.dumps(results, indent=2))
