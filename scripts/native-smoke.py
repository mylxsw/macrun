#!/usr/bin/env python3
"""Build the AppKit fixture through macrun and attempt real launch/Cua capture.
Does not change Cua installation or OS permissions. Leaves artifacts for inspection.
"""
import json,os,pathlib,shutil,socket,subprocess,time
repo=pathlib.Path(__file__).resolve().parents[1]; binary=repo/'target/debug/macrun'
root=repo/'.local'/f'native-{int(time.time())}';root.mkdir(parents=True)
source=root/'source';shutil.copytree(repo/'examples/Counter',source)
(source/'macrun.toml').write_text(f'remote_root={json.dumps(str(root/"mirror"))}\nderived_data={json.dumps(str(root/"DerivedData"))}\nproject="Counter.xcodeproj"\nscheme="Counter"\napp_relative_path="Build/Products/Debug/Counter.app"\n')
with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
sock=str(root/'cli.sock');base=[str(binary),'--socket',sock,'--workspace',str(source)];children=[];logs=[]
def spawn(args):
 f=open(root/f'process-{len(children)}.log','w');logs.append(f);p=subprocess.Popen([str(binary),*args],stdout=f,stderr=f);children.append(p)
def command(*args):return subprocess.run([*base,*args],capture_output=True,text=True,timeout=180)
try:
 subprocess.run([str(binary),'init','--data',str(root/'server')],check=True,capture_output=True)
 spawn(['--socket',sock,'serve','--listen',f'127.0.0.1:{port}','--data',str(root/'server')])
 spawn(['worker','--server',f'127.0.0.1:{port}','--cert',str(root/'server/cert.der'),'--token-file',str(root/'server/token'),'--data',str(root/'worker')])
 for _ in range(200):
  p=command('status','--json')
  if p.returncode==0:
   st=json.loads(p.stdout)
   if (st.get('worker') or {}).get('ready'):break
  time.sleep(.1)
 else:raise RuntimeError('worker did not register')
 (root/'environment.json').write_text(json.dumps(st,indent=2))
 p=command('run');(root/'cli.stdout').write_text(p.stdout);(root/'cli.stderr').write_text(p.stderr)
 directory=pathlib.Path(p.stdout.strip().splitlines()[-1]);result=json.loads((directory/'result.json').read_text())
 print(json.dumps({'exit_code':p.returncode,'status':result['status'],'error':result.get('error'),'directory':str(directory)},indent=2))
 assert (root/'DerivedData/Build/Products/Debug/Counter.app/Contents/MacOS/Counter').is_file(),'Xcode did not produce executable'
 if result.get('run_id'):command('stop','--run',result['run_id'])
finally:
 for p in children:
  p.terminate()
  try:p.wait(timeout=5)
  except subprocess.TimeoutExpired:p.kill();p.wait()
 for f in logs:f.close()
