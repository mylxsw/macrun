"""Disposable integration-test services; never uses a user's Macrun state."""
import json
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time


def wait(fn, seconds=120):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = fn()
        if value:
            return value
        time.sleep(.01)
    raise TimeoutError("probe deadline exceeded")


class Services:
    def __init__(self, binary):
        self.binary = str(Path(binary).resolve())
        self.temp = tempfile.TemporaryDirectory(prefix="mrval-", dir="/tmp")
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.mirror = self.root / "mirror"
        self.sock = str(self.root / "control.sock")
        self.children = []
        self.logs = []
        (self.source / "macrun.toml").write_text(f'remote_root = "{self.mirror}"\nsync_timeout_seconds = 600\n')
        subprocess.run([self.binary, "init", "--data", str(self.root / "server")], check=True, capture_output=True)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.bind(("127.0.0.1", 0))
            self.port = udp.getsockname()[1]
        self.spawn(["--socket", self.sock, "serve", "--listen", f"127.0.0.1:{self.port}", "--data", str(self.root / "server")])
        wait(lambda: Path(self.sock).exists())

    def spawn(self, args):
        log = open(self.root / f"process-{len(self.children)}.log", "w")
        self.logs.append(log)
        child = subprocess.Popen([self.binary, *args], stdout=log, stderr=log)
        self.children.append(child)
        return child

    def worker(self, address=None, config=None):
        args = ["worker", "--server", address or f"127.0.0.1:{self.port}", "--cert", str(self.root / "server/cert.der"), "--token-file", str(self.root / "server/token"), "--data", str(self.root / "worker")]
        if config:
            args += ["--config", str(config)]
        self.spawn(args)
        wait(lambda: self.request("status")["connected"])

    def request(self, kind, **args):
        def exact(stream, size):
            data = bytearray()
            while len(data) < size:
                part = stream.recv(size - len(data))
                if not part:
                    raise EOFError("truncated response")
                data.extend(part)
            return data
        data = json.dumps({"kind": kind, "workspace": str(self.source), "args": args}).encode()
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(40)
            stream.connect(self.sock)
            stream.sendall(struct.pack(">I", len(data)) + data)
            reply = json.loads(exact(stream, struct.unpack(">I", exact(stream, 4))[0]))
        assert reply["type"] != "error", reply
        return reply.get("value", reply)

    def sync(self, optimized=True, concurrent=None):
        start = time.perf_counter()
        ident = self.request("sync", detach=True, optimized_sync=optimized)["job_id"]
        def ended():
            v = self.request("sync.get", job_id=ident)
            if "ended_at" in v:
                return v
            if concurrent:
                concurrent()
            return None
        result = wait(ended, 600)
        assert result["status"] == "succeeded", result
        return {"wall_ms": (time.perf_counter()-start)*1000, "sync": result["sync"], "phases_ms": result["phases_ms"]}

    def done(self, ident):
        def ended():
            v = self.request("task.get", task_id=ident, include_result=False)
            return v if "ended_at" in v else None
        return wait(ended)

    def close(self):
        for child in reversed(self.children):
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        for log in self.logs:
            log.close()
        self.temp.cleanup()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def percentiles(values):
    values = sorted(values)
    return {"n": len(values), **{f"p{p}_ms": round(values[min(len(values)-1, int((len(values)-1)*p/100))], 3) for p in (50, 95, 99)}}
