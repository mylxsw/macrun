#!/usr/bin/env python3
"""Real QUIC + Unix socket smoke test; a tiny fake xcodebuild isolates task orchestration."""
import json, os, pathlib, shutil, socket, subprocess, sys, tempfile, time
binary = pathlib.Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/macrun').resolve()
root = pathlib.Path(tempfile.mkdtemp(prefix='macrun-smoke-'))
source, mirror, cache, server, worker = [root / n for n in ('source','mirror','cache','server','worker')]
source.mkdir(); tools = root/'bin'; tools.mkdir(); sock = str(root/'cli.sock')
with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as s: s.bind(('127.0.0.1',0)); port=s.getsockname()[1]
compiler=tools/'xcodebuild'
compiler.write_text('''#!/bin/sh
printf 'STREAM-BEGIN\\n'
if [ -f delay ]; then sleep 30 & echo $! > child.pid; wait; fi
if [ -f fail ]; then printf 'deliberate compile error\\n' >&2; exit 1; fi
printf 'STREAM-END\\n'
''');compiler.chmod(0o755)
(source/'macrun.toml').write_text(f'remote_root = {json.dumps(str(mirror))}\nderived_data = {json.dumps(str(cache))}\nproject = "Demo.xcodeproj"\nscheme = "Demo"\n')
(source/'hello.swift').write_text('first version')
base=[str(binary),'--socket',sock,'--workspace',str(source)]
children=[]; files=[]
def spawn(args, env=None):
    log=open(root/f'process-{len(children)}.log','w');files.append(log)
    p=subprocess.Popen([str(binary),*args],stdout=log,stderr=log,env=env);children.append(p);return p
def call(*args, code=0):
    p=subprocess.run([*base,*args],capture_output=True,text=True,timeout=40)
    assert p.returncode==code,(args,p.returncode,p.stdout,p.stderr)
    if args[0]=='status':return json.loads(p.stdout)
    directory=pathlib.Path(p.stdout.strip().splitlines()[-1]) if p.stdout.strip() else None
    return json.loads((directory/'result.json').read_text()) if directory else p.stderr

def wait_ready():
    for _ in range(150):
        try:
            v=call('status','--json')
            if v.get('worker',{}).get('ready') and not v.get('active'):return
        except (AssertionError,AttributeError):pass
        time.sleep(.1)
    raise AssertionError('worker not ready')
try:
    subprocess.run([str(binary),'init','--data',str(server)],check=True,capture_output=True)
    srv=spawn(['--socket',sock,'serve','--listen',f'127.0.0.1:{port}','--data',str(server)])
    for _ in range(50):
        if pathlib.Path(sock).exists():break
        time.sleep(.1)
    assert 'worker_offline' in call('sync',code=3)
    env=dict(os.environ,PATH=f'{tools}:{os.environ["PATH"]}')
    worker_args=['worker','--server',f'127.0.0.1:{port}','--cert',str(server/'cert.der'),'--token-file',str(server/'token'),'--data',str(worker)]
    wrk=spawn(worker_args,env);wait_ready()
    first=call('sync');assert first['sync']['files']==2
    second=call('sync');assert second['sync']['bytes']==0 and second['sync']['files']==0
    (source/'hello.swift').write_text('second version');assert call('build')['sync']['files']==1
    assert (mirror/'hello.swift').read_text()=='second version'
    (mirror/'generated').write_text('keep');(source/'hello.swift').unlink();call('sync');assert not (mirror/'hello.swift').exists() and (mirror/'generated').exists()
    (source/'fail').write_text('');assert call('test',code=1)['error']['code']=='test_failed';(source/'fail').unlink()
    (source/'delay').write_text('')
    job=subprocess.Popen([*base,'build'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    children.append(job)
    for _ in range(100):
        status=call('status','--json');active=status.get('active')
        if active and (pathlib.Path(active['directory'])/'build.log').exists():break
        time.sleep(.1)
    else:raise AssertionError('no streamed log')
    assert 'STREAM-BEGIN' in (pathlib.Path(active['directory'])/'build.log').read_text()
    assert 'busy' in call('sync',code=4)
    cancelled=call('cancel',active['job_id'],code=12);assert cancelled['error']['code']=='cancelled'
    job.communicate(timeout=10);assert job.returncode==12
    wait_ready();(source/'delay').unlink();call('build')
    # CLI detachment leaves server-owned work alive.
    (source/'delay').write_text('')
    detached=subprocess.Popen([*base,'build'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);children.append(detached)
    for _ in range(100):
        active=call('status','--json').get('active')
        if active and (pathlib.Path(active['directory'])/'build.log').exists():break
        time.sleep(.1)
    detached.terminate();detached.wait(timeout=5)
    assert call('status','--json')['active']['job_id']==active['job_id']
    call('cancel',active['job_id'],code=12);wait_ready()
    # Server restart forces cleanup; the worker reconnects instead of replaying.
    job=subprocess.Popen([*base,'build'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True);children.append(job)
    for _ in range(100):
        active=call('status','--json').get('active')
        if active and (pathlib.Path(active['directory'])/'build.log').exists():break
        time.sleep(.1)
    srv.kill();srv.wait();job.communicate(timeout=10)
    srv=spawn(['--socket',sock,'serve','--listen',f'127.0.0.1:{port}','--data',str(server)])
    wait_ready()
    old=json.loads((pathlib.Path(active['directory'])/'result.json').read_text());assert old['error']['code']=='server_restarted'
    (source/'delay').unlink();call('build')
    print('PASS: offline, QUIC registration, incremental sync, deletion, unmanaged files, streamed logs, build/test, busy, cancel, CLI detachment, server restart and worker reconnect')
    print(f'Evidence: {root}')
finally:
    for p in children:
        if p.poll() is None:
            p.terminate()
            try:p.wait(timeout=5)
            except subprocess.TimeoutExpired:p.kill();p.wait()
    for f in files:f.close()
