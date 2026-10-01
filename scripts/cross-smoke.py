#!/usr/bin/env python3
"""Local Linux amd64 Docker server + native macOS worker, real Xcode build."""
import json,os,pathlib,shutil,socket,subprocess,time,uuid
repo=pathlib.Path(__file__).resolve().parents[1]; binary=repo/'target/debug/macrun'
root=repo/'.local'/f'cross-{int(time.time())}';root.mkdir(parents=True);source=root/'source'
shutil.copytree(repo/'examples/Counter',source)
(source/'macrun.toml').write_text(f'remote_root={json.dumps(str(root/"mirror"))}\nderived_data={json.dumps(str(root/"DerivedData"))}\nproject="Counter.xcodeproj"\nscheme="Counter"\napp_relative_path="Build/Products/Debug/Counter.app"\n')
with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
name='macrun-cross-'+uuid.uuid4().hex[:8];worker=None;log=None
base=['docker','exec',name,'macrun','--socket','/tmp/macrun.sock','--workspace','/demo/source']
def call(*args,timeout=180):
 p=subprocess.run([*base,*args],capture_output=True,text=True,timeout=timeout)
 assert p.returncode==0,(args,p.returncode,p.stderr[-2000:])
 if args[0]=='status':return json.loads(p.stdout)
 directory=root/pathlib.Path(p.stdout.strip().splitlines()[-1]).relative_to('/demo')
 return json.loads((directory/'result.json').read_text())
try:
 subprocess.run([str(binary),'init','--data',str(root/'server')],check=True,capture_output=True)
 subprocess.run(['docker','run','-d','--platform','linux/amd64','--name',name,'-p',f'{port}:7443/udp','-v',f'{root}:/demo','macrun:demo','--socket','/tmp/macrun.sock','serve','--listen','0.0.0.0:7443','--data','/demo/server'],check=True,capture_output=True)
 log=open(root/'worker.log','w')
 worker=subprocess.Popen([str(binary),'worker','--server',f'127.0.0.1:{port}','--cert',str(root/'server/cert.der'),'--token-file',str(root/'server/token'),'--data',str(root/'worker')],stdout=log,stderr=log)
 for _ in range(100):
  try:
   status=call('status','--json',timeout=10)
   if (status.get('worker') or {}).get('ready'):break
  except AssertionError:pass
  time.sleep(.2)
 else:raise RuntimeError('cross-platform worker did not register')
 first=call('build');assert first['status']=='succeeded'
 assert (root/'DerivedData/Build/Products/Debug/Counter.app/Contents/MacOS/Counter').exists()
 assert call('sync')['sync']['bytes']==0
 with (source/'main.swift').open('a') as f:f.write('\n// Uncommitted cross-platform change\n')
 second=call('build');assert second['sync']['files']==1
 assert (root/'mirror/main.swift').read_text()==(source/'main.swift').read_text()
 (root/'summary.json').write_text(json.dumps({'first':first,'second':second,'environment':status},indent=2))
 print('PASS: Linux amd64 server/CLI -> QUIC UDP -> macOS arm64 worker -> real Xcode build; second build transferred one changed source file')
 print(f'Evidence: {root}')
finally:
 if worker is not None:
  worker.terminate()
  try:worker.wait(timeout=5)
  except subprocess.TimeoutExpired:worker.kill();worker.wait()
 if log:log.close()
 subprocess.run(['docker','container','stop','--time','2',name],capture_output=True)
 subprocess.run(['docker','container','remove',name],capture_output=True)
