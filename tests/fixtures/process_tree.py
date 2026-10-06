"""Owned, bounded fixtures for process cleanup tests (no GUI or shared state)."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

mode, directory = sys.argv[1:]
root = Path(directory)
if mode == 'platform':
    child = subprocess.Popen(['/bin/sleep', '30'], start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    (root / 'child.json').write_text(json.dumps({'pid': child.pid, 'pgid': child.pid}))
    print('parent output', flush=True)
    time.sleep(.1)
elif mode in ('child', 'exec_child'):
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    (root / 'child.json').write_text(json.dumps({'pid': os.getpid(), 'pgid': os.getpgrp()}))
    print('child output', flush=True)
    if mode == 'exec_child':
        deadline = time.monotonic() + 10
        while not (root / 'exec-now').exists():
            if time.monotonic() > deadline: sys.exit(1)
            time.sleep(.01)
        os.execve('/bin/sleep', ['sleep', '30'], {})
    time.sleep(30)
else:
    options = {'start_new_session': True}
    if mode == 'closed':
        options.update(stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if mode == 'exec':
        options['env'] = {k: v for k, v in os.environ.items() if k != 'MACRUN_PROCESS_OWNER'}
    child = subprocess.Popen([sys.executable, __file__, 'exec_child' if mode == 'exec' else 'child', directory], **options)
    deadline = time.monotonic() + 5
    while not (root / 'child.json').exists():
        if time.monotonic() > deadline:
            child.kill()
            child.wait()
            raise TimeoutError('child startup')
        time.sleep(.01)
    print('parent output', flush=True)
    if mode in ('timeout', 'cancel', 'abort', 'recover', 'exec'):
        time.sleep(30)
