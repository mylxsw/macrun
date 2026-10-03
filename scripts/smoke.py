#!/usr/bin/env python3
"""Real CLI/MCP -> Unix socket -> QUIC -> generic worker; no GUI required."""
import base64,json,os,pathlib,socket,subprocess,sys,tempfile,time,uuid
binary=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/macrun').resolve()
fixture=pathlib.Path('tests/fixtures/mcp.py').resolve()
root=pathlib.Path(tempfile.mkdtemp(prefix='macrun-generic-'))
source=root/'source';source.mkdir();mirror=root/'mirror';server=root/'server';worker=root/'worker'
sock=str(root/'cli.sock')
with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
(source/'macrun.toml').write_text(f'remote_root = {json.dumps(str(mirror))}\nexclude = ["ignored"]\n')
(source/'hello.txt').write_text('first');(source/'ignored').write_text('skip')
config=root/'worker.toml';config.write_text(f'[mcp.fixture]\ncommand = {json.dumps(sys.executable)}\nargs = [{json.dumps(str(fixture))}]\n')
base=[str(binary),'--socket',sock,'--workspace',str(source)]
children=[];logs=[]
def spawn(args):
 f=open(root/f'process-{len(children)}.log','w');logs.append(f)
 p=subprocess.Popen([str(binary),*args],stdout=f,stderr=f);children.append(p);return p
def cli(*args,ok=True):
 p=subprocess.run([*base,*args],capture_output=True,text=True,timeout=45)
 if ok:assert p.returncode==0,(args,p.stdout,p.stderr)
 else:assert p.returncode!=0,(args,p.stdout);return p.stderr
 return p.stdout.strip()
def call(kind,**args):return json.loads(cli('call',kind,'--args',json.dumps(args)))
def ready():
 for _ in range(180):
  try:
   if json.loads(cli('status'))['connected']:return
  except Exception:pass
  time.sleep(.1)
 raise AssertionError('worker did not reconnect')
def done(task):
 for _ in range(200):
  v=call('task.get',task_id=task)
  if 'ended_at' in v:return v
  time.sleep(.05)
 raise AssertionError(('task did not finish',task))
def submit(command,**extra):return call('exec.start',command=command,cwd=str(mirror),request_id=str(uuid.uuid4()),**extra)['task_id']
try:
 cli('init','--data',str(server))
 srvargs=['--socket',sock,'serve','--listen',f'127.0.0.1:{port}','--data',str(server)]
 srv=spawn(srvargs)
 for _ in range(60):
  if pathlib.Path(sock).exists():break
  time.sleep(.05)
 assert 'worker_offline' in cli('exec','--cwd','/tmp','true',ok=False)
 wrkargs=['worker','--server',f'127.0.0.1:{port}','--cert',str(server/'cert.der'),'--token-file',str(server/'token'),'--data',str(worker),'--config',str(config)]
 wrk=spawn(wrkargs);ready()
 # Malformed IDs must return an actionable error without killing the CLI connection.
 assert 'invalid_argument' in cli('call','sync.get','--args','{"job_id":"project-name"}',ok=False)

 first=json.loads(cli('sync'));assert first['sync']['files']==2
 assert not (mirror/'ignored').exists()
 assert json.loads(cli('sync'))['sync']['bytes']==0
 (source/'hello.txt').write_text('second');cli('sync');assert (mirror/'hello.txt').read_text()=='second'
 (mirror/'generated').write_text('keep');(source/'hello.txt').unlink();cli('sync');assert not (mirror/'hello.txt').exists() and (mirror/'generated').exists()
 assert done(submit('printf "$MSG"; exit 7',env={'MSG':'中文测试'}))['result']['exit_code']==7
 # Idempotent replay, conflicting reuse rejected.
 request_id=str(uuid.uuid4());args=dict(command='echo once >> count',cwd=str(mirror),request_id=request_id)
 call('exec.start',**args);call('exec.start',**args);done(request_id);assert (mirror/'count').read_text()=='once\n'
 assert 'different operation' in cli('call','exec.start','--args',json.dumps(dict(args,command='echo wrong')),ok=False)
 task=submit('sleep 20');call('task.cancel',task_id=task);assert done(task)['status']=='cancelled'
 assert done(submit('sleep 20',timeout_seconds=1))['status']=='timed_out'
 # Arbitrary binary upload/download, > one chunk, and text offsets.
 blob=os.urandom(1400000);local=root/'binary';local.write_bytes(blob)
 cli('upload',str(local),str(mirror/'binary'));cli('download',str(mirror/'binary'),str(root/'download'))
 assert (root/'download').read_bytes()==blob
 call('file.write',path=str(mirror/'text'),text='abcdef')
 assert call('file.read',path=str(mirror/'text'),offset=2,length=3,text=True)['text']=='cde'
 assert any(x['name']=='binary' for x in call('file.list',path=str(mirror))['entries'])
 # Persistent MCP state and image blocks.
 discovery=call('mcp.tools',server='fixture');session=discovery['session'];assert discovery['result']['tools'][0]['name']=='observe'
 def mcp(**a):return call('mcp.call',server='fixture',session=session,tool='observe',arguments=a,request_id=str(uuid.uuid4()))['task_id']
 v=done(mcp(text='中文'));assert json.loads(v['result']['result']['content'][0]['text'])['count']==1
 image=v['result']['result']['content'][1];assert image['type']=='image'
 call('file.write',path=str(mirror/'shot.png'),data=image['data']);assert call('file.image',path=str(mirror/'shot.png'))['content'][0]['data']==image['data']
 # Server loss must not cancel command or kill MCP session.
 task=submit('echo before; sleep 3; echo after; echo once >> reconnect-count')
 srv.kill();srv.wait();time.sleep(3.5);srv=spawn(srvargs);ready()
 v=done(task);assert v['status']=='succeeded' and 'after' in v['output']['text']
 assert (mirror/'reconnect-count').read_text()=='once\n'
 assert call('mcp.tools',server='fixture')['session']==session
 assert json.loads(done(mcp())['result']['result']['content'][0]['text'])['count']==2
 # Backend death invalidates session, no replay; caller must rediscover.
 assert done(mcp(crash=True))['status']=='unknown'
 assert done(mcp())['status']=='failed'
 new=call('mcp.tools',server='fixture');assert new['session']!=session;session=new['session']
 assert done(mcp(error=True))['status']=='failed'
 # Continuous sync, eventually observes changes without a build command.
 watcher=spawn(['--socket',sock,'--workspace',str(source),'sync','--watch','--interval-ms','100'])
 (source/'watched').write_text('auto')
 for _ in range(80):
  if (mirror/'watched').exists():break
  time.sleep(.1)
 assert (mirror/'watched').read_text()=='auto';watcher.terminate();watcher.wait()
 # Actual stdio MCP frontend returns images, not just path/base64 text.
 client=subprocess.Popen([*base,'mcp'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True);children.append(client)
 def rpc(method,params={}):
  client.stdin.write(json.dumps({'jsonrpc':'2.0','id':1,'method':method,'params':params})+'\n');client.stdin.flush();return json.loads(client.stdout.readline())
 assert rpc('initialize',{'protocolVersion':'2025-03-26'})['result']['capabilities']['tools']=={}
 assert len(rpc('tools/list')['result']['tools'])==14
 out=rpc('tools/call',{'name':'file_image','arguments':{'path':str(mirror/'shot.png')}})
 assert any(c['type']=='image' for c in out['result']['content'])
 out=rpc('tools/call',{'name':'task_get','arguments':{'task_id':mcp()}})
 # task may still be running; poll via same MCP frontend.
 tid=json.loads(out['result']['content'][0]['text'])['task_id']
 done(tid);out=rpc('tools/call',{'name':'task_get','arguments':{'task_id':tid}})
 assert any(c['type']=='image' for c in out['result']['content'])
 client.stdin.close();client.wait(timeout=5)
 # Worker restart does not replay an accepted/running job.
 task=submit('echo started; sleep 20; echo should-not-run >> restart-count')
 for _ in range(50):
  if call('task.get',task_id=task).get('output',{}).get('text'):break
  time.sleep(.05)
 wrk.kill();wrk.wait();wrk=spawn(wrkargs)
 # Wait until the NEW worker answers, not just stale connected status.
 for _ in range(180):
  try:
   v=call('task.get',task_id=task)
   if v['status']=='unknown':break
  except Exception:pass
  time.sleep(.1)
 else:raise AssertionError('restart did not mark unknown')
 assert not (mirror/'restart-count').exists()
 # Operational logs are JSON lines; request payloads never appear in them.
 records=[json.loads(line) for path in root.glob('process-*.log') for line in path.read_text().splitlines() if line.startswith('{')]
 operations=[r for r in records if r.get('event')=='operation']
 assert operations and all(r['time'].endswith('Z') and r['duration_ms']>=0 for r in operations)
 assert any(r.get('error_code')=='invalid_argument' for r in operations)
 assert any(r.get('event')=='task_finished' and r.get('status')=='timed_out' for r in records)
 assert any(r.get('event')=='task_recovered' for r in records)
 server_ids={r['request_id'] for r in operations if r['component']=='server'}
 assert any(r['request_id'] in server_ids for r in operations if r['component']=='worker')
 encoded=json.dumps(records,ensure_ascii=False)
 assert '中文测试' not in encoded and 'should-not-run' not in encoded
 print('PASS: timestamped operational logs, correlated request IDs, task lifecycle, no command/env payloads')
 print('PASS: sync/increment/deletion/watch, command env/exit/cancel/timeout/dedup, binary transfer, MCP images/session/error, server reconnect, worker restart')
 print(f'Evidence: {root}')
finally:
 for p in reversed(children):
  if p.poll() is None:
   p.terminate()
   try:p.wait(timeout=6)
   except subprocess.TimeoutExpired:p.kill();p.wait()
 for f in logs:f.close()
