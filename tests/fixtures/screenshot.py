#!/usr/bin/env python3
"""Cua-shaped capture/coordinate fixture; no desktop access or third-party deps."""
import base64
import json
import struct
import sys
import zlib


def png(width, height):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    rows = b''.join(b'\0' + bytes([y % 256, 40, 80]) * width for y in range(height))
    return base64.b64encode(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')).decode()


for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    params = request.get('params', {})
    if method == 'initialize':
        result = {'protocolVersion': '2025-03-26', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'cua-fixture', 'version': '1'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': name, 'description': name, 'inputSchema': {'type': 'object', 'properties': {}}, 'annotations': {'readOnlyHint': True}} for name in ['get_window_state', 'get_desktop_state', 'click', 'observe']]}
    else:
        a = params.get('arguments', {})
        if params['name'] in ['get_window_state', 'get_desktop_state', 'observe']:
            cap = min(240, a.get('max_dimension', 240))
            w, h = cap, cap * 2 // 3
            meta = {'screenshot_width': w, 'screenshot_height': h, 'screenshot_scale': w / 120, 'window_bounds': {'x': -100, 'y': 50, 'width': 120, 'height': 80}, 'arguments_received': a}
            content = [{'type': 'text', 'text': json.dumps(meta)}]
            if a.get('include_screenshot', True):
                content.append({'type': 'image', 'mimeType': 'image/png', 'data': png(w, h)})
            result = {'content': content, 'structuredContent': meta}
        else:
            result = {'content': [{'type': 'text', 'text': json.dumps(a)}], 'structuredContent': a}
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
