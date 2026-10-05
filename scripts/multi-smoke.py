#!/usr/bin/env python3
"""Real QUIC many-to-many regression: W1 -> S1/S2; W2 -> S1. Only disposable local data."""
import json, pathlib, socket, subprocess, sys, tempfile, time, uuid, os, signal
from concurrent.futures import ThreadPoolExecutor
binary=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/macrun').resolve()
root=pathlib.Path(tempfile.mkdtemp(prefix='macrun-many-'))
children=[];logs=[]
def run(*args,ok=True):
    p=subprocess.run([str(binary),*map(str,args)],capture_output=True,text=True,timeout=25)
    if not ok:
        assert p.returncode!=0,(args,p.stdout)
        return p.stderr
    assert p.returncode==0,(args,p.stderr)
    return json.loads(p.stdout) if p.stdout.strip().startswith('{') else p.stdout.strip()
def spawn(*args):
    log=open(root/f'process-{len(children)}.log','w');logs.append(log)
    p=subprocess.Popen([str(binary),*map(str,args)],stdout=log,stderr=log,start_new_session=True);children.append(p);return p
def port():
    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
def wait(check,seconds=15):
    deadline=time.monotonic()+seconds
    while time.monotonic()<deadline:
        try:
            value=check()
            if value:return value
        except (AssertionError,OSError,ValueError):pass
        time.sleep(.05)
    raise AssertionError('condition timed out; process logs retained at '+str(root))
def call(sock,kind,args=None,client=None,ok=True):
    return run('--socket',sock,*(['--client',client] if client else []),'call',kind,'--args',json.dumps(args or {}),ok=ok)
def local(action,args=None):
    with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as s:
        s.settimeout(20);s.connect(str(control))
        payload=json.dumps({'action':action,'args':args or {}}).encode();s.sendall(len(payload).to_bytes(4,'big')+payload)
        def exact(n):
            b=b''
            while len(b)<n:
                chunk=s.recv(n-len(b));assert chunk,'socket closed';b+=chunk
            return b
        return json.loads(exact(int.from_bytes(exact(4),'big')))
def done(sock,client,tid):
    return wait(lambda: (v if 'ended_at' in (v:=call(sock,'task.get',{'task_id':tid},client)) else None))
try:
    servers=[]
    for number in (1,2):
        data=root/f'server{number}';sock=root/f's{number}.sock';address=f'127.0.0.1:{port()}'
        run('init','--data',data);process=spawn('--socket',sock,'serve','--listen',address,'--data',data)
        wait(lambda:sock.exists());servers.append((data,sock,address,process))
    primary=root/'worker1';secondary=primary/'servers'/str(uuid.uuid4());other=root/'worker2'
    profiles=[dict(id='primary',name='Server One',server=servers[0][2],cert=str(servers[0][0]/'cert.der'),token_file=str(servers[0][0]/'token'),data=str(primary),config=None),dict(id=secondary.name,name='Server Two',server=servers[1][2],cert=str(servers[1][0]/'cert.der'),token_file=str(servers[1][0]/'token'),data=str(secondary),config=None)]
    manifest=root/'connections.json';manifest.write_text(json.dumps(profiles))
    (root/'control').mkdir(mode=0o700);control=root/'control'/'control.sock'
    w1args=['worker','--connections',manifest,'--data',primary,'--control-socket',control]
    w1=spawn(*w1args)
    w2=spawn('worker','--server',servers[0][2],'--cert',servers[0][0]/'cert.der','--token-file',servers[0][0]/'token','--data',other)
    wait(lambda:len(run('--socket',servers[0][1],'status')['workers'])==2)
    wait(lambda:len(run('--socket',servers[1][1],'status')['workers'])==1)
    client1=(primary/'client-id').read_text();client2=(other/'client-id').read_text()
    assert client1!=client2
    assert 'invalid_argument' in call(servers[0][1],'file.list',{'path':'/tmp','client_id':23},ok=False)
    assert 'client_required' in call(servers[0][1],'exec.start',dict(command='true',cwd='/tmp',request_id=str(uuid.uuid4())),ok=False)
    # Both clients can synchronize concurrently through the same server.
    sources=[]
    for index,client in enumerate([client1,client2]):
        source=root/f'source{index}';source.mkdir();mirror=root/f'mirror{index}'
        (source/'macrun.toml').write_text('remote_root = '+json.dumps(str(mirror))+'\n')
        (source/'payload').write_text('x'*(1024*1024));sources.append((source,mirror,client))
    with ThreadPoolExecutor(max_workers=2) as pool:
        results=list(pool.map(lambda item:run('--socket',servers[0][1],'--client',item[2],'--workspace',item[0],'sync'),sources))
    assert all(result['status']=='succeeded' for result in results)
    assert {result['client_id'] for result in results}=={client1,client2}
    assert all((mirror/'payload').stat().st_size==1024*1024 for _,mirror,_ in sources)
    request=str(uuid.uuid4());marker=root/'marker';args=dict(command=f'printf x >> {marker}',cwd='/tmp',request_id=request)
    one=call(servers[0][1],'exec.start',args,client1)['task_id']
    two=call(servers[1][1],'exec.start',args,client1)['task_id']
    three=call(servers[0][1],'exec.start',dict(command='printf other-client',cwd='/tmp',request_id=str(uuid.uuid4())),client2)['task_id']
    assert one!=two
    assert done(servers[0][1],client1,one)['status']=='succeeded'
    assert done(servers[1][1],client1,two)['status']=='succeeded'
    assert done(servers[0][1],client2,three)['output']['text']=='other-client'
    assert call(servers[1][1],'exec.start',args,client1)['task_id']==two
    assert marker.read_text()=='xx', 'retry replayed a command'
    assert 'No such file' in call(servers[0][1],'task.get',{'task_id':two},client1,ok=False)
    page=local('task_list',{'limit':50});assert len(page['tasks'])==3
    assert {t['connection_id'] for t in page['tasks']}=={'primary',secondary.name}
    assert len(local('task_list',{'limit':50,'connection_id':secondary.name})['tasks'])==1
    assert local('task_detail',{'task_id':two})['connection_id']==secondary.name
    assert len(local('snapshot')['connections'])==2
    assert 'error' not in local('stop_all')
    for server in servers:assert 'busy' in call(server[1],'exec.start',dict(command='true',cwd='/tmp',request_id=str(uuid.uuid4())),client1,ok=False)
    assert 'error' not in local('pause',{'paused':False})
    # One server can vanish while the other still completes an explicitly routed request.
    servers[1][3].send_signal(signal.SIGINT);servers[1][3].wait(timeout=10)
    alive=call(servers[0][1],'exec.start',dict(command='printf survives',cwd='/tmp',request_id=str(uuid.uuid4())),client1)['task_id']
    assert done(servers[0][1],client1,alive)['output']['text']=='survives'
    # Stable identity survives worker restart, and original request IDs remain idempotent.
    w1.send_signal(signal.SIGINT);w1.wait(timeout=10)
    wait(lambda:len(run('--socket',servers[0][1],'status')['workers'])==1)
    # An unresolvable profile at startup must not prevent other profiles connecting.
    profiles[1]['server']='unavailable.invalid:7443';manifest.write_text(json.dumps(profiles))
    w1=spawn(*w1args)
    wait(lambda:len(run('--socket',servers[0][1],'status')['workers'])==2)
    assert (primary/'client-id').read_text()==client1
    assert call(servers[0][1],'exec.start',args,client1)['task_id']==one
    assert marker.read_text()=='xx'
    wait(lambda:any(c['connection']['error'] for c in local('snapshot')['connections'] if c['id']==secondary.name))
    print('MANY_TO_MANY_OK: explicit routing, concurrent client sync, isolated history/dedup, global stop, outage/DNS isolation, stable identity/restart')
finally:
    for child in reversed(children):
        if child.poll() is None:
            child.send_signal(signal.SIGINT)
            try:child.wait(timeout=10)
            except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGTERM);child.wait(timeout=5)
    for log in logs:log.close()
