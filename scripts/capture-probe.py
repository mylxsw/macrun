#!/usr/bin/env python3
"""macOS-only real CuaDriver measurement, restricted to our own fixture window.
Never enables whole-display recording or changes the user's daemon configuration.
Only metrics (no screenshots/AX text from other apps) leave the temporary directory.
"""
import base64
import json
import plistlib
from pathlib import Path
import subprocess
import sys
import time
import uuid
from probe_support import Services, percentiles

driver = "/Applications/CuaDriver.app/Contents/MacOS/cua-driver"
state = plistlib.loads(subprocess.check_output(["ioreg", "-n", "Root", "-d", "1", "-a"]))
rows = state if isinstance(state, list) else [state]
if any(s.get("CGSSessionScreenIsLocked") for row in rows for s in row.get("IOConsoleUsers", [])):
    print(json.dumps({"status":"blocked","reason":"macOS interactive session is locked; real screenshot capture cannot be verified","screen_is_locked":True}))
    sys.exit(2)
with Services(sys.argv[1]) as s:
    fixture = s.root / "capture"
    build = []
    for _ in range(2):
        start = time.perf_counter()
        subprocess.run(["xcrun", "swiftc", "tests/fixtures/capture.swift", "-o", str(fixture)], check=True, capture_output=True)
        build.append((time.perf_counter()-start)*1000)
    app = subprocess.Popen([str(fixture)], stdout=subprocess.PIPE, text=True)
    try:
        identity = json.loads(app.stdout.readline())
        assert identity["pid"] == app.pid
        time.sleep(1)  # Cocoa/WindowServer must finish making this fixture visible.
        config = s.root / "worker.toml"
        config.write_text(f'[mcp.cua]\ncommand = {json.dumps(driver)}\nargs = ["mcp"]\n')
        s.worker(config=config)
        session = s.request("mcp.tools", server="cua")["session"]
        samples = {}
        for name, extra in [("full", {}), ("capture_only", {"include_accessibility_tree":False}),
                            ("thumbnail_640", {"include_accessibility_tree":False,"max_dimension":640}),
                            ("tree_only", {"include_screenshot":False})]:
            rows = []
            for _ in range(10):
                start = time.perf_counter()
                accepted = s.request("mcp.call", server="cua", session=session, tool="get_window_state", arguments={**identity,**extra}, request_id=str(uuid.uuid4()), wait_ms=1000)
                ident = accepted["task_id"]
                terminal = s.done(ident)
                assert terminal["status"] == "succeeded", terminal
                refs = s.request("task.get", task_id=ident, artifact_refs=True)
                result = refs["result"]["result"]
                assert not result.get("isError"), result
                # Fetch once to assert the real backend returned the requested modality.
                full = s.request("task.get", task_id=ident)["result"]["result"]
                images = [v for v in full.get("content",[]) if v.get("type")=="image"]
                assert bool(images) == (name != "tree_only"), {"mode":name,"result":full if not images else "unexpected image"}
                structured = full.get("structuredContent", {})
                rows.append({"wall_ms":(time.perf_counter()-start)*1000,
                             "image_bytes":sum(len(base64.b64decode(v["data"])) for v in images),
                             "result_json_bytes":len(json.dumps(full).encode()),
                             "width":structured.get("screenshot_width"),"height":structured.get("screenshot_height")})
            samples[name] = {"latency":percentiles([v["wall_ms"] for v in rows]),"samples":rows}
        print(json.dumps({"backend":"CuaDriver 0.26.0 via persistent MCP, Macrun QUIC loopback","fixture":"owned 1000x700 Cocoa window, 81 text elements","swift_compile_first_repeat_ms":build,"modes":samples}, indent=2))
    finally:
        app.terminate()
        app.wait(timeout=10)
