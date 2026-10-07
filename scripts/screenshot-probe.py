#!/usr/bin/env python3
"""Validate optimized screenshots and pixel input on an owned Cocoa window only."""
import base64
import json
import plistlib
from pathlib import Path
import struct
import subprocess
import sys
import time
import uuid
from probe_support import Services

state = plistlib.loads(subprocess.check_output(['ioreg', '-n', 'Root', '-d', '1', '-a']))
rows = state if isinstance(state, list) else [state]
if any(s.get('CGSSessionScreenIsLocked') for row in rows for s in row.get('IOConsoleUsers', [])):
    raise SystemExit('Unlock the desktop to run this owned-window probe.')
driver = '/Applications/CuaDriver.app/Contents/MacOS/cua-driver'
with Services(sys.argv[1]) as services:
    fixture = services.root / 'fixture'
    subprocess.run(['xcrun', 'swiftc', 'tests/fixtures/capture.swift', '-o', str(fixture)], check=True, capture_output=True)
    app = subprocess.Popen([str(fixture)], stdout=subprocess.PIPE, text=True)
    try:
        identity = json.loads(app.stdout.readline())
        assert identity['pid'] == app.pid
        time.sleep(1)
        config = services.root / 'worker.toml'
        config.write_text(f'[mcp.cua]\ncommand = {json.dumps(driver)}\nargs = ["mcp"]\n')
        services.worker(config=config)
        session = services.request('mcp.tools', server='cua')['session']

        def call(tool, arguments, **extra):
            started = time.perf_counter()
            task = services.request('mcp.call', server='cua', session=session, tool=tool,
                                    arguments=arguments, request_id=str(uuid.uuid4()), **extra)['task_id']
            done = services.done(task)
            assert done['status'] == 'succeeded', done
            value = services.request('task.get', task_id=task)['result']['result']
            # Do not print base64 even when the backend returns an error.
            assert not value.get('isError'), [b.get('text', '')[:500] for b in value.get('content', []) if b.get('type') == 'text']
            return task, value, round((time.perf_counter() - started) * 1000, 2)

        samples = []
        for mode in ['original', 'auto', 'jpeg']:
            task, value, elapsed = call('get_window_state', identity, screenshot_mode=mode)
            block = next(b for b in value['content'] if b['type'] == 'image')
            data = base64.b64decode(block['data'])
            meta = value['macrun_screenshot']
            if mode != 'original':
                assert meta['width'] <= 1920 and meta['height'] <= 1080
            assert value['structuredContent']['screenshot_width'] == meta['width']
            assert value['structuredContent']['screenshot_height'] == meta['height']
            if block['mimeType'] == 'image/png':
                assert struct.unpack('>II', data[16:24]) == (meta['width'], meta['height'])
            if len(sys.argv) > 2:
                out = Path(sys.argv[2]); out.mkdir(parents=True, exist_ok=True)
                (out / (mode + ('.png' if block['mimeType'] == 'image/png' else '.jpg'))).write_bytes(data)
            samples.append({'mode': mode, 'bytes': len(data), 'width': meta['width'], 'height': meta['height'], 'mime': block['mimeType'], 'wall_ms': elapsed})
        # Refresh auto after comparison so its backend frame is the active frame.
        task, value, _ = call('get_window_state', identity)
        meta = value['structuredContent']
        button = next(e for e in meta['elements'] if e.get('role') == 'AXButton' and e.get('label') == 'Validate fixture action')
        bounds = meta['window_bounds']
        # Cua AX frames use absolute screen points; image origin uses window bounds.
        x = (button['frame']['x'] + button['frame']['w'] / 2 - bounds['x']) * value['macrun_screenshot']['pixels_per_point_x']
        y = (button['frame']['y'] + button['frame']['h'] / 2 - bounds['y']) * value['macrun_screenshot']['pixels_per_point_y']
        _, click, _ = call('click', {**identity, 'x': x, 'y': y, 'delivery_mode': 'background'}, coordinate_task_id=task)
        _, fresh, _ = call('get_window_state', identity)
        if 'Macrun fixture action confirmed' not in fresh['structuredContent']['tree_markdown']:
            print(json.dumps({'samples': samples, 'bounds': bounds, 'button': button, 'image': value['macrun_screenshot'], 'click_point': [x,y], 'click_metadata': click.get('structuredContent')}, indent=2))
            raise AssertionError('Pixel click did not update the fixture label')
        print(json.dumps({'status': 'passed', 'backend': 'CuaDriver 0.26.0', 'fixture': 'owned Cocoa window only', 'samples': samples, 'pixel_click': 'fresh AX tree confirms label changed'}, indent=2))
    finally:
        app.terminate()
        app.wait(timeout=10)
