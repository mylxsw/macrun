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
import struct
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
                assert structured["pid"] == app.pid and structured["window_id"] == identity["window_id"]
                elements = structured.get("elements", [])
                assert bool(elements) == (name in ("full", "tree_only")), name
                if elements:
                    assert "Macrun GUL-215" in structured["tree_markdown"]
                for image in images:
                    data = base64.b64decode(image["data"], validate=True)
                    assert data[:8] == b"\x89PNG\r\n\x1a\n" and data[12:16] == b"IHDR"
                    width, height = struct.unpack(">II", data[16:24])
                    assert (width, height) == (structured["screenshot_width"], structured["screenshot_height"])
                    assert structured["screenshot_frame_valid"]
                    if name == "thumbnail_640":
                        assert 0 < max(width, height) <= 640
                rows.append({"wall_ms":(time.perf_counter()-start)*1000,
                             "image_bytes":sum(len(base64.b64decode(v["data"])) for v in images),
                             "result_json_bytes":len(json.dumps(full).encode()),
                             "width":structured.get("screenshot_width"),"height":structured.get("screenshot_height"),
                             "ax_elements":len(elements),"ax_complete":structured.get("elements_complete")})
            samples[name] = {"latency":percentiles([v["wall_ms"] for v in rows]),"samples":rows}
        # Use the last fresh AX snapshot's exact token. Never click by global coordinates.
        button = next(v for v in elements if v.get("role") == "AXButton" and v.get("label") == "Validate fixture action")
        start = time.perf_counter()
        accepted = s.request("desktop.sequence", server="cua", session=session, request_id=str(uuid.uuid4()), steps=[
            {"tool":"click", "arguments":{**identity,"element_token":button["element_token"],"delivery_mode":"background"}},
            {"tool":"get_window_state", "arguments":{**identity,"max_dimension":640}},
        ])
        terminal = s.done(accepted["task_id"])
        assert terminal["status"] == "succeeded", terminal
        sequence = s.request("task.get", task_id=accepted["task_id"])["result"]
        assert len(sequence["steps"]) == 2
        observed = sequence["result"]
        assert not observed.get("isError"), observed
        assert "Macrun fixture action confirmed" in observed["structuredContent"]["tree_markdown"]
        assert any(v.get("type") == "image" for v in observed["content"])
        interaction = {"status":"passed", "steps":2, "route":"background AX token click then fresh window observation",
                       "postcondition":"Macrun fixture action confirmed", "wall_ms":(time.perf_counter()-start)*1000}
        print(json.dumps({"status":"passed","backend":"CuaDriver 0.26.0 via persistent MCP, Macrun QUIC loopback","fixture":"owned 1000x700 Cocoa window, 81 text elements and one action button","swift_compile_first_repeat_ms":build,"modes":samples,"interaction":interaction}, indent=2))
    finally:
        app.terminate()
        app.wait(timeout=10)
