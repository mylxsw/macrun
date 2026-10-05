#!/usr/bin/env python3
"""Stateful MCP fixture: validates session preservation, embedded images and error forwarding."""
import json, sys, time, os
count = 0
PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a3ioAAAAASUVORK5CYII='
for line in sys.stdin:
    v=json.loads(line)
    if 'id' not in v: continue
    method=v['method']; args=v.get('params',{})
    if method=='initialize': result={'protocolVersion':'2025-03-26','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
    elif method=='tools/list':
        names = ['observe']
        if os.environ.get('MACRUN_TEST_TOOLS_PAGINATED'):
            names = ['observe', 'last'] if args.get('cursor') == 'second-page' else ['observe', 'first']
        if os.environ.get('MACRUN_TEST_TOOLS_TIERS'):
            names = ['observe', 'click', 'kill']
        meta = {'observe':{'annotations':{'readOnlyHint':True}}, 'kill':{'annotations':{'destructiveHint':True},'risk':{'class':'r3'}}}
        result={'tools':[dict({'name':name,'description':'Stateful counter fixture','inputSchema':{'type':'object','properties':{}}}, **meta.get(name,{})) for name in names]}
        if os.environ.get('MACRUN_TEST_TOOLS_PAGINATED') and not args.get('cursor'):
            result['nextCursor']='second-page'
    elif method=='tools/call':
        a=args.get('arguments',{})
        if a.get('crash'): os._exit(2)
        if a.get('serial_probe'):
            # Model a physical desktop resource shared by independent backend processes.
            try:
                fd=os.open(a['serial_probe'],os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
            except FileExistsError:
                print(json.dumps({'jsonrpc':'2.0','id':v['id'],'result':{'isError':True,'content':[{'type':'text','text':'overlapping desktop operation'}]}}),flush=True)
                continue
            time.sleep(.2);os.close(fd);os.unlink(a['serial_probe'])
        if a.get('sleep'): time.sleep(a['sleep'])
        count+=1
        result={'content':[{'type':'text','text':json.dumps({'count':count,'text':a.get('text')})},{'type':'image','mimeType':'image/png','data':PNG}],'isError':bool(a.get('error'))}
    else: result={}
    print(json.dumps({'jsonrpc':'2.0','id':v['id'],'result':result}),flush=True)
