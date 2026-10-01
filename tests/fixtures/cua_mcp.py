#!/usr/bin/env python3
"""Protocol fixture, not evidence of native GUI behavior."""
import sys,json,base64,struct,zlib

def chunk(t,b):return struct.pack('!I',len(b))+t+b+struct.pack('!I',zlib.crc32(t+b)&0xffffffff)
png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!2I5B',20,10,8,2,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+b'\xff\xff\xff'*20)*10))+chunk(b'IEND',b'')
for line in sys.stdin:
    v=json.loads(line)
    if 'id' not in v:continue
    method=v['method'];p=v.get('params',{})
    if method=='initialize':r={'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
    elif method=='tools/list':r={'tools':[{'name':n,'inputSchema':{'properties':{'target':{},'delivery_mode':{},'session':{}}}} for n in ['list_windows','get_window_state','click','type_text','press_key','scroll']]}
    elif p['name']=='list_windows':r={'structuredContent':{'windows':[{'window_id':42,'pid':123,'bounds':{'x':0,'y':0,'width':10,'height':5}}]}}
    elif p['name']=='get_window_state':r={'structuredContent':{'window_bounds':{'x':0,'y':0,'width':10,'height':5},'elements':[{'element_token':'real-token-1','label':'Increment'}]},'content':[{'type':'image','mimeType':'image/png','data':base64.b64encode(png).decode()}]}
    else:r={'structuredContent':{'effect':'unverifiable','echo':p['arguments']}}
    print(json.dumps({'jsonrpc':'2.0','id':v['id'],'result':r}),flush=True)
