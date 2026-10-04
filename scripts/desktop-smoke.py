#!/usr/bin/env python3
"""Exercise managed-worker controls over real QUIC + same-user Unix IPC, without a GUI."""
import json, pathlib, signal, socket, struct, subprocess, sys, tempfile, time, uuid
binary=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/macrun').resolve()
root=pathlib.Path(tempfile.mkdtemp(prefix='macrun-desktop-',dir='/tmp'))
source=root/'source';source.mkdir();server=root/'server';worker=root/'worker';control=root/'ipc'/'control.sock';cli_socket=root/'server.sock';mirror=root/'mirror'
(source/'macrun.toml').write_text(f'remote_root = {json.dumps(str(mirror))}\n')
(source/'hello.txt').write_text('desktop sync')
with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
config=root/'worker.toml';config.write_text(f'[mcp.fixture]\ncommand={json.dumps(sys.executable)}\nargs=[{json.dumps(str(pathlib.Path("tests/fixtures/mcp.py").resolve()))}]\n')
children=[];logs=[]
def spawn(args,parent=False):
    log=open(root/f'process-{len(children)}.log','w');logs.append(log)
    p=subprocess.Popen([str(binary),*map(str,args)],stdin=subprocess.PIPE if parent else subprocess.DEVNULL,stdout=log,stderr=log)
    children.append(p);return p
def cli(*args,ok=True):
    p=subprocess.run([str(binary),'--socket',str(cli_socket),'--workspace',str(source),*args],capture_output=True,text=True,timeout=20)
    if ok:
        assert p.returncode==0,(args,p.stdout,p.stderr)
        return json.loads(p.stdout)
    assert p.returncode!=0,(args,p.stdout)
    return p.stderr
def remote(kind,**args):return cli('call',kind,'--args',json.dumps(args))
def exact(s,n):
    out=b''
    while len(out)<n:
        b=s.recv(n-len(out));assert b,'socket closed';out+=b
    return out
def receive(s):return json.loads(exact(s,struct.unpack('!I',exact(s,4))[0]))
def ipc(action,**args):
    with socket.socket(socket.AF_UNIX) as s:
        s.settimeout(15);s.connect(str(control));payload=json.dumps(dict(action=action,args=args)).encode();s.sendall(struct.pack('!I',len(payload))+payload);v=receive(s);assert 'error' not in v,v;return v
def until(fn,seconds=15):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        try:
            result=fn()
            if result:return result
        except (OSError,AssertionError,ValueError):pass
        time.sleep(.05)
    raise AssertionError(f'timed out: {fn}')
def done(ident):return until(lambda:(v if v.get('ended_at') else None) if (v:=remote('task.get',task_id=ident)) else None)
try:
    subprocess.run([str(binary),'init','--data',str(server)],check=True,stdout=subprocess.DEVNULL)
    srv=spawn(['--socket',cli_socket,'serve','--listen',f'127.0.0.1:{port}','--data',server])
    args=['worker','--server',f'127.0.0.1:{port}','--cert',server/'cert.der','--token-file',server/'token','--data',worker,'--control-socket',control,'--parent-pipe','--config',config]
    wrk=spawn(args,True)
    until(lambda:cli('status')['connected'])
    assert ipc('snapshot')['connection']['state']=='connected'
    assert control.stat().st_mode & 0o777==0o600
    assert control.parent.stat().st_mode & 0o777==0o700
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(5);stream.connect(str(control));b=b'{"action":"subscribe"}';stream.sendall(struct.pack('!I',len(b))+b)
        assert receive(stream)['protocol']==2
        ident=str(uuid.uuid4());a=dict(request_id=ident,command='echo before; sleep 1; echo after',cwd=str(source),env={'PRIVATE':'not-in-parameters'})
        remote('exec.start',**a)
        ipc('pause',paused=True)
        assert remote('exec.start',**a)['duplicate']
        assert 'busy' in cli('exec','--cwd',str(source),'echo forbidden',ok=False)
        assert 'busy' in cli('sync',ok=False)
        assert done(ident)['status']=='succeeded'
        assert 'not-in-parameters' not in json.dumps(ipc('snapshot'))
        until(lambda:any(t['task_id']==ident for t in receive(stream)['tasks']))
    ipc('pause',paused=False)
    cli('sync');assert (mirror/'hello.txt').read_text()=='desktop sync'
    synced=[t for t in ipc('snapshot')['tasks'] if t['kind']=='sync' and t['status']=='succeeded'];assert synced and synced[0]['progress']['received']==synced[0]['progress']['total']
    safety=dict(restrict_paths=True,roots=[str(source),str(mirror)],approval='all',retention_days=7,yield_until=0)
    ipc('safety',**safety)
    assert 'path_denied' in cli('call','file.read','--args',json.dumps({'path':'/etc/hosts'}),ok=False)
    approved=remote('exec.start',command='echo approved',cwd=str(source))['task_id']
    until(lambda:remote('task.get',task_id=approved)['status']=='awaiting_approval')
    ipc('approve',task_id=approved,allow=True)
    assert done(approved)['status']=='succeeded'
    denied=remote('exec.start',command='touch forbidden',cwd=str(source))['task_id']
    ipc('approve',task_id=denied,allow=False)
    assert done(denied)['status']=='denied' and not (source/'forbidden').exists()
    safety.update(restrict_paths=False,approval='direct');ipc('safety',**safety)
    session=remote('mcp.tools',server='fixture')['session']
    ipc('desktop',enabled=False)
    assert 'desktop_disabled' in cli('call','mcp.call','--args',json.dumps(dict(server='fixture',tool='observe',session=session)),ok=False)
    ipc('desktop',enabled=True)
    task=remote('mcp.call',server='fixture',tool='observe',session=session,arguments={'sleep':20})['task_id']
    command=remote('exec.start',command='sleep 30',cwd=str(source))['task_id']
    time.sleep(.2);ipc('stop_all')
    assert done(command)['status']=='cancelled'
    assert done(task)['status'] in ('cancelled','unknown')
    assert ipc('snapshot')['policy']=={'paused':True,'desktop_enabled':False}
    assert 'unknown operation' in cli('call','stop_all',ok=False)
    ipc('pause',paused=False)
    task=remote('exec.start',command='sleep 30; echo should-not-run > marker',cwd=str(source))['task_id']
    wrk.stdin.close();wrk.wait(timeout=15)
    assert wrk.returncode==0
    result=json.loads((worker/'tasks'/task/'result.json').read_text());assert result['status']=='cancelled',result
    assert not (source/'marker').exists()
    assert not control.exists()
    # Graceful shutdown is not a persistent pause; explicit emergency-stop policy is persistent.
    injected_args=args.copy();injected_args[injected_args.index('--token-file')+1]='/nonexistent-macrun-test-token'
    wrk=spawn([*injected_args,'--token-stdin'],True)
    token=(server/'token').read_text().strip().encode();wrk.stdin.write(struct.pack('!I',len(token))+token);wrk.stdin.flush()
    until(lambda:cli('status')['connected']);assert not ipc('snapshot')['policy']['paused'];assert not ipc('snapshot')['policy']['desktop_enabled']
    assert remote('exec.start',**a)['duplicate']
    ipc('shutdown');wrk.wait(timeout=15)
    print('PASS: real QUIC, private IPC and subscription, pause/dedup, sync progress, desktop gate, approval, path restrictions, stop-all, credential pipe, parent EOF cleanup, restart policy')
    print('Evidence:',root)
finally:
    for p in reversed(children):
        if p.poll() is None:
            p.send_signal(signal.SIGINT)
            try:p.wait(timeout=15)
            except subprocess.TimeoutExpired:p.kill();p.wait()
    for f in logs:f.close()
