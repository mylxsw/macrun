#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["blake3>=1,<2"]
# ///
"""Local UI review worker. Never executes commands or connects to a server.

Initialize: uv run desktop/scripts/fixture-worker.py init --data /tmp/macrun-ui-v2
Serve:      uv run desktop/scripts/fixture-worker.py serve --data /tmp/macrun-ui-v2
Change:     uv run desktop/scripts/fixture-worker.py state --data /tmp/macrun-ui-v2 --state approval
Send:       uv run desktop/scripts/fixture-worker.py control --data /tmp/macrun-ui-v2 --action snapshot

Use the printed MACRUN_DESKTOP_DATA value exactly when launching a debug app.
The data directory must be inside the system temporary directory. A marker guards
all later changes. The actual Tauri app receives the ordinary worker IPC frames.
"""

import argparse
import asyncio
import copy
import json
import os
from pathlib import Path
import shlex
import signal
import struct
import tempfile
import time

from blake3 import blake3

STATES = ("working", "idle", "empty", "paused", "offline", "approval", "overlay", "long", "large")
MARKER = ".macrun-ui-fixture"
ACTIVE = {"accepted", "running", "awaiting_approval"}
IDS = [f"3f2a91c0-1111-4111-8111-{i:012d}" for i in range(20)]


def write_json(path, value):
    pending = path.with_name(f".{path.name}-{os.getpid()}.tmp")
    pending.write_text(json.dumps(value, ensure_ascii=False, indent=2))
    pending.chmod(0o600)
    pending.replace(path)


def data_root(value, initialize=False):
    path = Path(value).absolute()
    resolved = path.resolve()
    roots = {Path("/tmp").resolve(), Path(tempfile.gettempdir()).resolve()}
    if not any(root in resolved.parents for root in roots):
        raise SystemExit("Fixture data must be in a new subdirectory of the system temporary directory")
    if initialize:
        if path.exists() and not (path / MARKER).exists():
            raise SystemExit("Refusing an existing directory without the UI fixture marker")
        path.mkdir(parents=True, exist_ok=True, mode=0o700)
        (path / MARKER).write_text("Isolated Macrun UI test data; no real commands are executed.\n")
    elif not (path / MARKER).is_file():
        raise SystemExit("Missing UI fixture marker; run init first")
    if path.stat().st_uid != os.getuid():
        raise SystemExit("Fixture directory must belong to the current user")
    path.chmod(0o700)
    return path


def socket_path(data):
    # Match main.rs exactly; do not canonicalize the already absolute data string.
    digest = blake3(str(data).encode()).hexdigest()[:16]
    return Path(tempfile.gettempdir()) / f"macrun-desktop-{digest}" / "control.sock"


def task(i, kind, status, age, duration=None, **arguments):
    value = {"task_id": IDS[i], "kind": kind, "status": status,
             "arguments": arguments, "_age": age, "_duration": duration}
    if status == "succeeded" and kind == "exec.start":
        value["result"] = {"exit_code": 0}
    return value


def scenario(name):
    tasks = [
        task(0, "exec.start", "running", 134000, command="xcodebuild test -scheme Counter -destination 'platform=macOS'", cwd="~/work/Counter"),
        task(1, "sync", "running", 3000, remote_root="~/work/Counter"),
        task(2, "sync", "succeeded", 136000, 1800, remote_root="~/work/Counter"),
        task(3, "mcp.call", "succeeded", 349000, 1200, server="computer", tool="computer.screenshot", arguments={"display": "main"}),
        task(4, "mcp.call", "succeeded", 352000, 300, server="computer", tool="computer.click", arguments={"x": 412, "y": 288}),
        task(5, "exec.start", "failed", 808000, 63000, command="make test", cwd="~/work/Counter"),
        task(6, "mcp.call", "unknown", 1451000, 300, server="computer", tool="computer.click", arguments={"x": 412, "y": 288}),
        task(7, "exec.start", "succeeded", 568000, 41000, command="swift build -c release", cwd="~/work/Counter"),
        task(8, "exec.start", "timed_out", 6210000, 600000, command="npm run e2e", cwd="~/work/macrun-site"),
        task(9, "exec.start", "cancelled", 10923000, 271000, command="tail -f /tmp/app.log", cwd="/tmp"),
    ]
    tasks[0]["output_tail"] = "CompileSwift normal arm64 CounterView.swift\nCompileSwift normal arm64 CounterModel.swift\nLd Counter.app/Contents/MacOS/Counter\nTest Suite 'CounterTests' started\n  ✓ testIncrement (0.004s)\n  ✓ testReset (0.002s)\n  … testPersistence"
    tasks[1]["progress"] = {"received": 3, "total": 5, "bytes": 18432}
    tasks[5].update(result={"exit_code": 2}, error={"message": "make test exited with code 2"}, output_tail="Test Suite 'CounterTests' failed\nmake: *** [test] Error 2")
    tasks[6].update(error={"message": "worker_restarted"}, output_tail="error: worker_restarted\nExecution interrupted; effects may have occurred.\nNever automatically replay.")
    snapshot = {
        "_anchor": int(time.time() * 1000),
        "version": "0.2.0", "protocol": 2,
        "policy": {"paused": name == "paused", "desktop_enabled": True},
        "safety": {"restrict_paths": True, "roots": ["~/work", "/tmp"], "approval": "risk", "retention_days": 30, "yield_until": 0},
        "connection": {"state": "connected", "server": "203.0.113.10:7443", "_age": 11520000, "rtt_ms": 38, "error": None,
                       "checks": {"transport": True, "certificate": True, "authentication": True, "protocol": True}},
        "backends": [{"name": "computer", "state": "ready", "session": "7", "command": "/Applications/CuaDriver.app/Contents/MacOS/cua-driver mcp", "tool_count": 2}],
        "workspaces": [{"root": "~/work/Counter", "_age": 120000, "status": "running"}, {"root": "~/work/macrun-site", "_age": 10800000, "status": "succeeded"}, {"root": "~/work/notes-api", "_age": 55440000, "status": "failed", "error": {"message": "路径冲突"}}],
        "tasks": tasks, "today_summary": {"total": 42, "succeeded": 38, "failed": 2, "unknown": 1, "cancelled": 1}, "total_tasks": len(tasks),
    }
    if name in ("idle", "empty"):
        snapshot["tasks"] = [] if name == "empty" else tasks[2:]
        snapshot["workspaces"][0]["status"] = "succeeded"
    if name == "empty":
        snapshot.update(workspaces=[], today_summary={"total": 0}, total_tasks=0)
    if name == "offline":
        snapshot["connection"].update(state="disconnected", _age=70000, rtt_ms=None, error="服务器 UDP 7443 无响应")
    if name == "approval":
        snapshot["tasks"] = [task(10, "exec.start", "awaiting_approval", 8000, command="git push origin feature/menu-bar", cwd="~/work/Counter"), *tasks[2:]]
    if name == "overlay":
        snapshot["tasks"] = [task(11, "mcp.call", "running", 500, server="computer", tool="computer.click", arguments={"x": 412, "y": 288}), *tasks[2:]]
    if name in ("long", "large"):
        tasks[0]["arguments"].update(command="xcodebuild test -scheme Counter -destination 'platform=macOS' " + "-only-testing:CounterTests/testPersistence ".join([""] * 9), cwd="~/work/" + "very-long-project-directory/" * 8 + "Counter")
        if name == "large":
            snapshot["tasks"].extend([
                task(12, "exec.start", "accepted", 2000, command="echo pending-fixture", cwd="~/work/Counter"),
                task(13, "exec.start", "awaiting_approval", 5000, command="git push origin fixture-branch", cwd="~/work/Counter"),
                task(14, "exec.start", "denied", 6000, 1000, command="echo denied-fixture", cwd="~/work/Counter"),
            ])
            snapshot["workspaces"].extend({"root": f"~/work/fixture-project-{i}", "_age": i * 1000, "status": "succeeded"} for i in range(21))
        extra_count = 1500 - len(snapshot["tasks"]) if name == "large" else 30
        terminal = ("succeeded", "failed", "cancelled", "timed_out", "unknown", "denied")
        for i in range(extra_count):
            item = copy.deepcopy(tasks[7])
            item.update(task_id=f"aabbccdd-1111-4111-8111-{i:012d}", _age=200000 + i * 1000)
            item["arguments"]["command"] = f"echo fixture-task-{i}"
            item["output_tail"] = f"fixture-task-{i}\nIsolated UI test output; no command was executed.\n"
            if name == "large":
                item["status"] = terminal[i % len(terminal)]
                if item["status"] != "succeeded":
                    item.pop("result", None)
                if item["status"] == "failed":
                    item["result"] = {"exit_code": 2}
                if i % 5 == 0:
                    item["kind"] = "mcp.call"
                    item.pop("result", None)
                    item["arguments"].update(server="computer", tool="computer.screenshot")
                if item["status"] in ("failed", "timed_out", "unknown"):
                    item["error"] = {"code": "fixture_failure", "message": "隔离测试任务失败；此错误对象属于任务详情。"}
            snapshot["tasks"].append(item)
        if name == "large":
            snapshot["today_summary"] = {"total": len(snapshot["tasks"])}
            for item in snapshot["tasks"]:
                status = item["status"]
                snapshot["today_summary"][status] = snapshot["today_summary"].get(status, 0) + 1
    return snapshot


def materialize(value):
    value = copy.deepcopy(value)
    now = value.pop("_anchor", int(time.time() * 1000))
    for item in value["tasks"]:
        item["started_at"] = now - item.pop("_age", 0)
        duration = item.pop("_duration", None)
        if item["status"] not in ACTIVE:
            item["ended_at"] = item["started_at"] + (duration or 0)
    for workspace in value["workspaces"]:
        workspace["time"] = now - workspace.pop("_age", 0)
    value["connection"]["since"] = now - value["connection"].pop("_age", 0)
    value["active_count"] = sum(t["status"] in ACTIVE for t in value["tasks"])
    value["total_tasks"] = len(value["tasks"])
    value["task_counts"] = {}
    for item in value["tasks"]:
        status = item["status"]
        value["task_counts"][status] = value["task_counts"].get(status, 0) + 1
    return value


def summary(item, snapshot=False):
    item = copy.deepcopy(item)
    result = item.pop("result", None)
    if snapshot and isinstance(result, dict) and isinstance(result.get("exit_code"), int):
        item["result"] = {"exit_code": result["exit_code"]}
    if not snapshot or item["status"] not in ACTIVE:
        item.pop("output_tail", None)
    return item


def snapshot_view(state):
    value = materialize(state)
    tasks = sorted(value["tasks"], key=lambda item: (item["started_at"], item["task_id"]), reverse=True)
    value["tasks"] = [summary(item, snapshot=True) for item in tasks[:200] + [item for item in tasks[200:] if item["status"] in ACTIVE]]
    return value


def task_page(state, args):
    status = args.get("status") or "all"
    if status not in ACTIVE | {"all", "succeeded", "failed", "cancelled", "timed_out", "unknown", "denied"}:
        raise ValueError("unknown task status")
    limit = min(100, max(1, int(args.get("limit", 50))))
    needle = str(args.get("query", "")).strip().lower()
    tasks = materialize(state)["tasks"]
    tasks.sort(key=lambda item: (item["started_at"], item["task_id"]), reverse=True)
    matches, counts = [], {}
    for item in tasks:
        if args.get("kind") is not None and item["kind"] != args["kind"]:
            continue
        arguments = item["arguments"]
        text = " ".join(str(value) for value in [item["task_id"], item["kind"], *[arguments.get(key, "") for key in ("command", "tool", "cwd", "remote_root", "path")]]).lower()
        if needle not in text:
            continue
        counts[item["status"]] = counts.get(item["status"], 0) + 1
        if status == "all" or item["status"] == status:
            matches.append(item)
    cursor = args.get("cursor")
    remaining = [item for item in matches if not cursor or (item["started_at"], item["task_id"]) < (cursor["started_at"], cursor["task_id"])]
    page = remaining[:limit]
    next_cursor = {key: page[-1][key] for key in ("started_at", "task_id")} if len(remaining) > len(page) else None
    return {"tasks": [summary(item) for item in page], "total": len(tasks), "filtered_total": len(matches), "counts": counts, "next_cursor": next_cursor}


async def read_frame(reader):
    size, = struct.unpack("!I", await reader.readexactly(4))
    if size > 16 * 1024 * 1024:
        raise ValueError("IPC frame is too large")
    return json.loads(await reader.readexactly(size))


async def write_frame(writer, value):
    payload = json.dumps(value, ensure_ascii=False).encode()
    writer.write(struct.pack("!I", len(payload)) + payload)
    await writer.drain()


async def serve(data):
    path = socket_path(data)
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    path.parent.chmod(0o700)
    if path.exists():
        raise SystemExit(f"Socket already exists; stop the other fixture or choose a new data directory: {path}")
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stopped.set)

    def load():
        value = json.loads((data / "fixture-state.json").read_text())
        if "_anchor" not in value:
            value["_anchor"] = int(time.time() * 1000)
            write_json(data / "fixture-state.json", value)
        return value

    async def handle(reader, writer):
        try:
            request = await read_frame(reader)
            action, args = request.get("action"), request.get("args") or {}
            if action == "subscribe":
                while not stopped.is_set():
                    await write_frame(writer, snapshot_view(load()))
                    await asyncio.sleep(0.25)
                return
            state = load()
            reply = {"ok": True}
            if action == "snapshot":
                reply = snapshot_view(state)
            elif action == "task_list":
                reply = task_page(state, args)
            elif action == "pause":
                state["policy"]["paused"] = bool(args["paused"])
            elif action == "desktop":
                state["policy"]["desktop_enabled"] = bool(args["enabled"])
            elif action == "stop_all":
                state["policy"].update(paused=True, desktop_enabled=False)
                for t in state["tasks"]:
                    if t["status"] in ACTIVE:
                        t["status"] = "cancelled"
                        t["_duration"] = t["_age"]
            elif action in ("cancel", "approve"):
                t = next(t for t in state["tasks"] if t["task_id"] == args["task_id"])
                t["status"] = ("running" if args.get("allow") else "denied") if action == "approve" else "cancelled"
                t["_duration"] = t["_age"]
            elif action == "safety":
                state["safety"].update(args)
            elif action == "yield":
                state["safety"]["yield_until"] = int(time.time() * 1000) + 30000
            elif action == "task_detail":
                reply = next((t for t in materialize(state)["tasks"] if t["task_id"] == args["task_id"]), None)
                if reply is None:
                    raise ValueError("UI fixture task not found")
                if "output_tail" in reply:
                    output = reply.pop("output_tail").encode()
                    length = max(1, min(65536, int(args.get("tail_bytes", 8192))))
                    offset = max(0, len(output) - length)
                    reply["output"] = {"text": output[offset:].decode(errors="replace"), "size": len(output), "offset": offset, "next_offset": len(output), "eof": True}
            elif action == "tools":
                reply = {"session": state["backends"][0]["session"], "result": {"tools": [{"name": "computer.screenshot", "description": "截取当前屏幕", "inputSchema": {"type": "object"}}, {"name": "computer.click", "description": "点击指定位置", "inputSchema": {"type": "object"}}]}}
            elif action == "restart_backend":
                state["backends"][0]["session"] = str(int(state["backends"][0]["session"]) + 1)
            elif action == "shutdown":
                stopped.set()
            else:
                reply = {"error": f"UI fixture does not implement {action}; no real operation was performed"}
            if action not in ("snapshot", "task_list", "task_detail", "tools"):
                write_json(data / "fixture-state.json", state)
            with (data / "fixture-actions.jsonl").open("a") as log:
                log.write(json.dumps({"action": action, "args": args}, ensure_ascii=False) + "\n")
            await write_frame(writer, reply)
        except (asyncio.IncompleteReadError, ConnectionError, BrokenPipeError):
            pass
        except Exception as error:
            await write_frame(writer, {"error": str(error)})
        finally:
            writer.close()

    server = await asyncio.start_unix_server(handle, path=str(path))
    path.chmod(0o600)
    print(f"UI fixture listening: {path}", flush=True)
    try:
        async with server:
            await stopped.wait()
    finally:
        path.unlink(missing_ok=True)


async def control(data, action, args):
    reader, writer = await asyncio.open_unix_connection(str(socket_path(data)))
    await write_frame(writer, {"action": action, "args": json.loads(args)})
    print(json.dumps(await read_frame(reader), ensure_ascii=False, indent=2))
    writer.close()
    await writer.wait_closed()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("init", "serve", "state", "control"))
    parser.add_argument("--data", required=True)
    parser.add_argument("--state", choices=STATES, default="working")
    parser.add_argument("--unpaired", action="store_true")
    parser.add_argument("--action", default="snapshot")
    parser.add_argument("--args", default="{}")
    options = parser.parse_args()
    data = data_root(options.data, options.command == "init")
    if options.command == "init":
        if not options.unpaired:
            write_json(data / "connection.json", {"server": "203.0.113.10:7443", "cert": "", "token_file": "/dev/null", "backend_config": "", "keychain_account": "", "certificate_fingerprint": ""})
        write_json(data / "preferences.json", {"show_overlay": True, "yield_input": False, "notifications": False, "keep_awake": False, "auto_connect": False})
        write_json(data / "fixture-state.json", scenario(options.state))
        print(f"export MACRUN_DESKTOP_DATA={shlex.quote(str(data))}")
        print(f"Socket: {socket_path(data)}")
    elif options.command == "state":
        write_json(data / "fixture-state.json", scenario(options.state))
        print(f"UI fixture state: {options.state}")
    elif options.command == "serve":
        asyncio.run(serve(data))
    elif options.command == "control":
        asyncio.run(control(data, options.action, options.args))


if __name__ == "__main__":
    main()
