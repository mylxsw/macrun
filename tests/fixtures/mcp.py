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
    elif method=='tools/list': result={'tools':[{'name':'observe','description':'Stateful counter fixture','inputSchema':{'type':'object','properties':{}}}]}
    elif method=='tools/call':
        a=args.get('arguments',{})
        if a.get('crash'): os._exit(2)
        if a.get('sleep'): time.sleep(a['sleep'])
        count+=1
        result={'content':[{'type':'text','text':json.dumps({'count':count,'text':a.get('text')})},{'type':'image','mimeType':'image/png','data':PNG}],'isError':bool(a.get('error'))}
    else: result={}
    print(json.dumps({'jsonrpc':'2.0','id':v['id'],'result':result}),flush=True)
