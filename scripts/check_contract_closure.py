#!/usr/bin/env python3
"""Bounded HTTP/WebSocket acceptance using generated TXT; no provider calls."""
import argparse,base64,csv,hashlib,http.cookiejar,io,json,os,pathlib,socket,struct,time,urllib.error,urllib.parse,urllib.request
from fixture_acquisition import acquisition
p=argparse.ArgumentParser();p.add_argument('--base',required=True);p.add_argument('--legacy-base');p.add_argument('--origin',default='http://localhost:3050');p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args();checks=[];started=time.monotonic()
jar=http.cookiejar.CookieJar();one=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar));other=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
def call(path,data=None,method=None,base=None,client=one,headers=None):
 h={'Origin':a.origin,**(headers or {})}
 if isinstance(data,dict):data=json.dumps(data).encode();h['Content-Type']='application/json'
 try:
  with client.open(urllib.request.Request((base or a.base)+path,data=data,method=method,headers=h),timeout=30) as r:return r.status,r.read()
 except urllib.error.HTTPError as e:return e.code,e.read()
def obj(response):return json.loads(response[1])
def check(name,passed):
 checks.append({'check':name,'passed':bool(passed)})
 if not passed:raise AssertionError(name)
def upload(content,meta,name='synthetic-boundary.txt',base=None):return call('/api/documents',content,base=base,headers={'Content-Type':'text/plain','X-Document-Filename':name,'X-Document-Metadata':json.dumps(meta)})
def exact(sock,n):
 out=b''
 while len(out)<n:
  data=sock.recv(n-len(out))
  if not data:raise AssertionError('websocket closed')
  out+=data
 return out
def frame(sock,opcode,payload):
 mask=os.urandom(4);n=len(payload);head=bytes([128|opcode,128|n]) if n<126 else bytes([128|opcode,254])+struct.pack('!H',n);sock.sendall(head+mask+bytes(c^mask[i%4] for i,c in enumerate(payload)))
def ws(query,command=None):
 target=urllib.parse.urlparse(a.base);sock=socket.create_connection((target.hostname,target.port),timeout=5);sock.settimeout(5)
 cookie='; '.join(c.name+'='+c.value for c in jar);key=base64.b64encode(os.urandom(16)).decode()
 sock.sendall(('GET /api/realtime?'+query+' HTTP/1.1\r\nHost: '+target.netloc+'\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: '+key+'\r\nOrigin: '+a.origin+'\r\nCookie: '+cookie+'\r\n\r\n').encode())
 header=b''
 while not header.endswith(b'\r\n\r\n'):header+=exact(sock,1)
 status=int(header.split(b' ')[1]);result=None
 if status==101 and command:
  frame(sock,1,json.dumps(command).encode())
  for _ in range(5):
   first,second=exact(sock,2);n=second&127
   if n==126:n=struct.unpack('!H',exact(sock,2))[0]
   elif n==127:n=struct.unpack('!Q',exact(sock,8))[0]
   if n>1_000_000:raise AssertionError('oversized WS test response')
   payload=exact(sock,n)
   if first&15==9:frame(sock,10,payload);continue
   if first&15==1:result=json.loads(payload);break
 sock.close();return status,result
try:
 check('authenticated',call('/api/auth/demo',{})[0]==200);call('/api/auth/demo',{},client=other)
 body=b'Synthetic first line.\nSecond line with umlaut: \xc3\xbc.\fSecond physical page.';rights=acquisition(body,'CC0-synthetic');meta={'rights_confirmed':True,'licence':'CC0-synthetic','acquisition':rights}
 if a.legacy_base:
  old=obj(upload(body,meta,base=a.legacy_base));new=obj(upload(body,meta));check('legacy_upload_identity_reused_without_rekey',old['document']['id']==new['document']['id'] and new['idempotent_replay'])
 rights['distribution_uri']='urn:oldif:synthetic-distribution-b';created=upload(body,meta);check('new_distribution_ingested',created[0]==200);data=obj(created);doc=data['document'];ident=doc['id'];path='/api/documents/'+ident
 if a.legacy_base:check('distribution_changes_source_identity',ident!=old['document']['id'])
 check('retry_reuses_distribution_identity',obj(upload(body,meta))['document']['id']==ident)
 changed=upload(body,meta,name='renamed.txt');check('same_source_changed_metadata_conflicts',changed[0]==409 and obj(changed)['code']=='REVISION_CONFLICT')
 check('exact_original_preserved',call(path+'/original')[1]==body)
 check('public_rights_required_independent_of_review',call(path+'/export?format=bundle&scope=public')[0]==400)
 rights['allowed_uses'].append('public_export');released=call(path+'/release',{'expected_version':doc['version'],'rights_confirmed':True,'acquisition':rights});check('public_rights_explicitly_recorded',released[0]==200);doc=obj(released)['document']
 check('public_review_required_independent_of_rights',call(path+'/export?format=bundle&scope=public')[0]==400)
 reviewed=call(path+'/review',{'expected_version':doc['version'],'decision':'accepted','reason':'Synthetic fixture generated for export review','metadata':{'internal_employee':'NEVER_PUBLIC','language':'en'}});check('synthetic_review_recorded',reviewed[0]==200);doc=obj(reviewed)['document']
 stale=call(path+'/review',{'expected_version':1,'decision':'accepted','reason':'stale fixture'});check('stale_revision_has_machine_code',stale[0]==409 and obj(stale)['code']=='REVISION_CONFLICT')
 public=obj(call(path+'/export?format=bundle&scope=public'));private=obj(call(path+'/export?format=bundle'))
 check('bundle_seven_named_files',set(f['name'] for f in public['files'])=={'records.jsonl','records.tsv','records.csv','schema.json','source-manifest.json','change-log.json','rejection-manifest.json'})
 check('download_is_not_publication',public['scope']=='public' and public['publication_performed'] is False)
 check('file_hashes_and_lengths_exact',all(hashlib.sha256(f['content'].encode()).hexdigest()==f['sha256'] and len(f['content'].encode())==f['bytes'] for f in public['files']))
 files={f['name']:f['content'] for f in public['files']};records=[json.loads(line) for line in files['records.jsonl'].splitlines()]
 check('public_field_allowlist_excludes_internal_values','NEVER_PUBLIC' not in json.dumps(public) and all('metadata' not in r['data'] and 'created_at' not in r['data'] for r in records))
 check('public_exact_text_and_hashes_retained',records[0]['data']['text']==data['pages'][0]['text'] and records[0]['data']['original_sha256']==hashlib.sha256(body).hexdigest())
 for name,delimiter in [('records.tsv','\t'),('records.csv',',')]:
  table=list(csv.DictReader(io.StringIO(files[name]),delimiter=delimiter));check(name+'_lossless_record_parity',[json.loads(row['record_json']) for row in table]==records)
 check('source_change_rejection_manifests_exist',all(json.loads(files[name])['schema_version']=='1.0' for name in ['source-manifest.json','change-log.json','rejection-manifest.json']))
 check('private_bundle_keeps_provenance','normalization_provenance' in json.dumps(private))
 check('dataset_bundle_same_scope',obj(call('/api/datasets/'+ident+'/export?format=bundle&scope=public'))['record_count']==2)
 check('owner_isolation_on_bundle',call(path+'/export?format=bundle',client=other)[0]==404)
 check('public_unbundled_path_cannot_bypass_policy',call(path+'/export?format=csv&scope=public')[0]==400)
 check('unknown_export_scope_rejected',call(path+'/export?format=bundle&scope=automatic')[0]==400)
 check('strict_export_query_rejects_extra',call(path+'/export?format=bundle&extra=true')[0]==400)
 session=obj(call('/api/sessions',{'title':'Synthetic canvas','language':'en'}))['session'];sid=session['id']
 check('strict_session_request_rejects_extra',call('/api/sessions',{'title':'bad','extra':True})[0]==422)
 canvas={'session_id':sid,'positions':{'node-1':{'x':37,'y':-11}},'selected_source_id':data['pages'][0]['id']}
 saved=call('/api/preferences/layouts/canvas',{'expected_version':0,'layout':canvas},method='PUT');check('canvas_positions_saved',saved[0]==200)
 check('canvas_positions_restore_from_server',obj(call('/api/preferences/layouts/canvas'))['layout']==canvas)
 conflict=call('/api/preferences/layouts/canvas',{'expected_version':0,'layout':canvas},method='PUT');check('canvas_cas_machine_code',conflict[0]==409 and obj(conflict)['code']=='REVISION_CONFLICT')
 check('canvas_other_owner_cannot_reference_session',call('/api/preferences/layouts/canvas',{'expected_version':0,'layout':canvas},method='PUT',client=other)[0]==404)
 check('canvas_out_of_range_rejected',call('/api/preferences/layouts/canvas',{'expected_version':1,'layout':{**canvas,'positions':{'x':{'x':10001,'y':0}}}},method='PUT')[0]==400)
 check('strict_steer_request_rejects_extra',call('/api/runs/nonexistent/steer',{'message':'test','extra':True})[0]==422)
 check('strict_voice_request_rejects_extra_before_provider',call('/api/voice/session',{'sdp':'v=0','locale':'en','extra':True})[0]==400)
 check('canonical_replay_query_upgrade',ws('last_sequence=0')[0]==101)
 check('legacy_replay_query_upgrade',ws('after_sequence=0')[0]==101)
 check('conflicting_replay_aliases_reject_before_upgrade',ws('last_sequence=1&after_sequence=2')[0]==400)
 status,event=ws('last_sequence=0',{'type':'cancel','run_id':'nonexistent','extra':True});check('strict_ws_ancillary_command_typed_rejection',status==101 and event['type']=='error' and event['data']['code']=='INVALID_REQUEST')
 check('empty_accepted_run_bundle_without_inference',obj(call('/api/export?format=bundle&session_id='+sid))['record_count']==0)
 legacy=obj(call('/api/sources/stapfer-2027'));provenance=legacy['normalization_provenance'];check('legacy_unknown_actor_time_not_fabricated',provenance['provenance_status']=='legacy_unknown' and provenance['actor'] is None and provenance['timestamp'] is None and provenance['original']['legacy_transformation']==legacy.get('transformation'))
 annotation={'page':1,'labels':[],'bias':'','missingness':'Synthetic missing-page flag','handwriting_status':'unknown','legibility_status':'legible','note':'','missing_page':True,'reading_order_status':'uncertain'}
 update=call(path+'/review',{'expected_version':doc['version'],'decision':'accepted','reason':'Synthetic unresolved page condition','annotations':[annotation]});check('detected_uncertainty_routes_to_review',update[0]==200 and obj(update)['document']['review']['decision']=='needs_review' and obj(update)['revision']['decision']=='needs_review')
 check('uncertain_review_blocks_public_export',call(path+'/export?format=bundle&scope=public')[0]==400)
 check('no_inference_runs_created',obj(call('/api/sessions/'+sid))['runs']==[])
finally:
 report={'validation_class':'actual native HTTP and WebSocket contracts; generated synthetic TXT only','passed':sum(c['passed'] for c in checks),'failed':sum(not c['passed'] for c in checks),'checks':checks,'elapsed_seconds':round(time.monotonic()-started,3),'provider_calls':0,'publication_performed':False,'legacy_endpoint_exercised':bool(a.legacy_base),'human_historical_quality':'NOT_RUN'}
 a.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'passed':report['passed'],'failed':report['failed'],'provider_calls':0}))
