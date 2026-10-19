"""Run against real Oldif services; never substitutes a provider response."""
import argparse,json,http.cookiejar,urllib.request,urllib.error,uuid
parser=argparse.ArgumentParser();parser.add_argument('--base',default='http://127.0.0.1:18080');parser.add_argument('--origin',default='http://localhost:3000');parser.add_argument('--output');args=parser.parse_args()
results=[]
def client():
 return urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
def request(c,path,data=None,method=None,origin=None):
 req=urllib.request.Request(args.base+path,data=None if data is None else json.dumps(data).encode(),method=method,headers={'Content-Type':'application/json','Origin':origin or args.origin})
 try:
  with c.open(req,timeout=30) as response:return response.status,response.read()
 except urllib.error.HTTPError as e:return e.code,e.read()
def check(name,condition):
 results.append({'name':name,'passed':bool(condition)})
 if not condition:raise AssertionError(name)
a=client();b=client()
status,_=request(a,'/api/datasets');check('authentication_required',status==401)
status,_=request(a,'/api/auth/demo',{},origin='https://untrusted.invalid');check('foreign_origin_rejected',status==403)
status,body=request(a,'/api/auth/demo',{});check('demo_login',status==200)
status,body=request(a,'/api/datasets');d=json.loads(body)['datasets'];check('real_stapfer_dataset',status==200 and d[0]['record_count']==105)
status,body=request(a,'/api/sessions',{'title':'Acceptance research','language':'en'});sid=json.loads(body)['session']['id'];check('conversation_persisted',status==200)
request(b,'/api/auth/demo',{})
status,_=request(b,'/api/sessions/'+sid);check('other_demo_cannot_read_conversation',status==404)
status,body=request(a,'/api/sessions/'+sid);check('conversation_reload',status==200 and json.loads(body)['session']['id']==sid)
status,body=request(a,'/api/datasets/stapfer-1799/sources');rows=json.loads(body)['sources'];check('105_original_records',status==200 and len(rows)==105 and all(r['page'] is None for r in rows))
status,body=request(a,'/api/import',{'records':[]});check('import_requires_rights',status==400)
source={'name':'Private acceptance','rights_confirmed':True,'licence':'acceptance-synthetic','records':[{'text':'Synthetic acceptance fixture; never historical evidence.','title':'Fixture','url':'https://example.invalid/test'}]}
status,body=request(a,'/api/import',source);did=json.loads(body)['id'];check('private_import',status==200)
status,_=request(b,'/api/datasets/'+did+'/sources');check('other_demo_cannot_read_import',status==404)
status,body=request(a,'/api/datasets/'+did+'/export?format=jsonl');check('provenance_export',status==200 and json.loads(body)['provenance']=='user-attested')
status,body=request(a,'/api/export?format=json');check('unreviewed_work_excluded_from_accepted_export',status==200 and json.loads(body)==[])
status,body=request(a,'/api/mcp',{'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'shell','arguments':{}}});check('mcp_denies_unlisted_tools',status==200 and json.loads(body)['error']['code']==-32601)
status,_=request(a,'/api/auth/logout',{});check('logout',status==200)
status,_=request(a,'/api/auth/session');check('revoked_cookie_rejected',status==401)
report={'class':'live-service-acceptance','provider_inference_tested':False,'checks':results,'passed':len(results),'failed':0}
if args.output:open(args.output,'w').write(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
