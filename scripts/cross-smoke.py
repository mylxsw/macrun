#!/usr/bin/env python3
"""Linux amd64 server/CLI in Docker -> Mac worker. Uses MCP fixture, not real GUI."""
import json,pathlib,socket,subprocess,sys,tempfile,time,uuid
binary=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/macrun').resolve()
image=sys.argv[2] if len(sys.argv)>2 else 'macrun:generic-demo'
root=pathlib.Path(tempfile.mkdtemp(prefix='macrun-cross-',dir='.local')).resolve()
source=root/'source';source.mkdir();mirror=root/'mirror';server=root/'server';worker=root/'worker'
(source/'macrun.toml').write_text(f'remote_root = {json.dumps(str(mirror))}\n')
(source/'uncommitted.txt').write_text('Linux source v1')
fixture=pathlib.Path('tests/fixtures/mcp.py').resolve();cfg=root/'worker.toml'
cfg.write_text(f'[mcp.fixture]\ncommand = {json.dumps(sys.executable)}\nargs = [{json.dumps(str(fixture))}]\n')
with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
name='macrun-cross-'+uuid.uuid4().hex[:10]
def run(args):return subprocess.check_output(args,text=True,stderr=subprocess.STDOUT,timeout=45).strip()
def call(kind,**args):return json.loads(run(['docker','exec',name,'macrun','--workspace','/source','call',kind,'--args',json.dumps(args)]))
def wait(task):
 for _ in range(100):
  v=call('task.get',task_id=task)
  if 'ended_at' in v:return v
  time.sleep(.1)
 raise AssertionError('task timed out')
wrk=None;log=None;started=False
try:
 run([str(binary),'init','--data',str(server)])
 run(['docker','run','--rm','-d','--platform','linux/amd64','--name',name,'-p',f'127.0.0.1:{port}:7443/udp','-v',f'{server}:/state','-v',f'{source}:/source:ro',image,'serve','--data','/state'])
 started=True
 log=open(root/'worker.log','w');wrk=subprocess.Popen([str(binary),'worker','--server',f'127.0.0.1:{port}','--cert',str(server/'cert.der'),'--token-file',str(server/'token'),'--data',str(worker),'--config',str(cfg)],stdout=log,stderr=log)
 for _ in range(100):
  v=json.loads(run(['docker','exec',name,'macrun','status']))
  if v['connected']:break
  time.sleep(.1)
 else:raise AssertionError('worker not connected')
 result=json.loads(run(['docker','exec',name,'macrun','--workspace','/source','sync']))
 assert result['status']=='succeeded' and (mirror/'uncommitted.txt').read_text()=='Linux source v1'
 task=call('exec.start',request_id=str(uuid.uuid4()),command='uname -s; cat uncommitted.txt',cwd=str(mirror))['task_id']
 v=wait(task);assert v['status']=='succeeded' and 'Darwin' in v['output']['text']
 (source/'uncommitted.txt').write_text('Linux source v2')
 result=json.loads(run(['docker','exec',name,'macrun','--workspace','/source','sync']))
 assert result['sync']['files']==1
 discovery=call('mcp.tools',server='fixture')
 task=call('mcp.call',request_id=str(uuid.uuid4()),server='fixture',session=discovery['session'],tool='observe',arguments={'text':'跨平台'})['task_id']
 v=wait(task);assert v['result']['result']['content'][1]['type']=='image'
 assert '跨平台' in json.loads(v['result']['result']['content'][0]['text'])['text']
 (root/'result.json').write_text(json.dumps({'status':'passed','image':image,'checks':['Linux amd64 server/CLI','Mac arm64 command execution','incremental sync','MCP image result']},indent=2))
 print('PASS: Linux amd64 server/CLI -> QUIC -> Mac worker: sync, command, MCP image result')
 print(f'Evidence: {root}')
finally:
 if wrk is not None and wrk.poll() is None:
  wrk.terminate()
  try:wrk.wait(timeout=5)
  except subprocess.TimeoutExpired:wrk.kill();wrk.wait()
 if log:log.close()
 if started:subprocess.run(['docker','stop','-t','2',name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
