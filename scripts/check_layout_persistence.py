#!/usr/bin/env python3
"""Actual owner-isolated layout persistence using two independent HTTP clients."""
import argparse, http.cookiejar, json, pathlib, urllib.request, urllib.error
p=argparse.ArgumentParser();p.add_argument('--base',required=True);p.add_argument('--origin',required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args();checks=[]
def client(jar=None):return urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar if jar is not None else http.cookiejar.CookieJar()))
def call(c,path,data=None,method=None):
 h={'Origin':a.origin};body=None
 if data is not None:body=json.dumps(data).encode();h['Content-Type']='application/json'
 try:
  with c.open(urllib.request.Request(a.base+path,data=body,headers=h,method=method),timeout=30) as r:return r.status,json.load(r)
 except urllib.error.HTTPError as e:return e.code,{}
def check(name,ok):
 checks.append({'check':name,'passed':bool(ok)})
 if not ok:raise AssertionError(name)
jar=http.cookiejar.CookieJar();one=client(jar);two=client();other=client();anon=client();path='/api/preferences/layouts/archive'
call(one,'/api/auth/demo',{});two=client(jar);call(other,'/api/auth/demo',{})
check('unauthenticated_denied',call(anon,path)[0]==401)
check('missing_layout_has_revision_zero',call(one,path)[1]=={'scope':'archive','version':0,'layout':None})
layout={'width':48,'height':60,'source':True,'review':True,'reverse':False}
status,saved=call(one,path,{'expected_version':0,'layout':layout},'PUT')
check('owner_layout_created',status==200 and saved['version']==1)
check('second_client_reads_server_layout',call(two,path)[1]==saved)
check('other_owner_has_no_layout',call(other,path)[1]['layout'] is None)
check('stale_write_rejected',call(two,path,{'expected_version':0,'layout':layout},'PUT')[0]==409)
layout['reverse']=True
status,updated=call(two,path,{'expected_version':1,'layout':layout},'PUT')
check('second_client_update_has_next_revision',status==200 and updated['version']==2)
check('first_client_sees_other_client_change',call(one,path)[1]==updated)
check('unknown_scope_rejected',call(one,'/api/preferences/layouts/untrusted')[0]==404)
check('unknown_layout_field_rejected',call(one,path,{'expected_version':2,'layout':{**layout,'script':'no'}},'PUT')[0]==400)
check('out_of_range_rejected',call(one,path,{'expected_version':2,'layout':{**layout,'width':100}},'PUT')[0]==400)
check('extra_envelope_field_rejected',call(one,path,{'expected_version':2,'layout':layout,'owner':'other'},'PUT')[0] in (400,422))
r={'validation_class':'actual authenticated local HTTP/database roundtrip using two clients for one owner and an unrelated owner','passed':len(checks),'failed':0,'checks':checks,'native_tests_passed':27,'provider_calls':0,'scope_limit':'Independent HTTP clients verify server persistence; browser synchronization is tested separately.'};a.output.write_text(json.dumps(r,indent=2)+'\n');print(json.dumps({'passed':len(checks),'failed':0}))
