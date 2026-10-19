#!/usr/bin/env python3
"""Exercise actual backend/core processes with a loopback-only synthetic provider.

No real provider configuration is loaded. The original request, source hashes,
restart recovery, response-loss retry and websocket replay are checked together.
"""
import argparse,base64,concurrent.futures,hashlib,http.cookiejar,http.server,json,os,pathlib,socket,struct,subprocess,tempfile,threading,time,urllib.error,urllib.parse,urllib.request
from fixture_acquisition import acquisition

p=argparse.ArgumentParser();p.add_argument('--db',required=True);p.add_argument('--token-file',type=pathlib.Path,required=True);p.add_argument('--backend',type=pathlib.Path,required=True);p.add_argument('--core',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
if urllib.parse.urlparse(a.db).hostname not in ('127.0.0.1','localhost'):p.error('Only an isolated loopback database is supported')
for port in (18116,18117,18118,18119,50068):
 with socket.socket() as probe:probe.bind(('127.0.0.1',port))
token=a.token_file.read_text().strip();base='http://127.0.0.1:18116';origin='http://localhost:3050';checks=[];calls=[];hold=threading.Event();entered=threading.Event();fail_run_write=False;failures_injected=0;reject_public_reads=False;public_reads_rejected=0
def check(name,ok):
 checks.append({'check':name,'passed':bool(ok)})
 if not ok:raise AssertionError(name)
def request(url,data=None,headers=None,client=None):
 h=dict(headers or {})
 if isinstance(data,dict):data=json.dumps(data).encode();h['Content-Type']='application/json'
 try:
  with (client.open if client else urllib.request.urlopen)(urllib.request.Request(url,data=data,headers=h),timeout=30) as r:return r.status,r.read()
 except urllib.error.HTTPError as error:return error.code,error.read()
class Quiet(http.server.BaseHTTPRequestHandler):
 def log_message(self,*_):pass
 def send(self,status,value):
  body=json.dumps(value).encode();self.send_response(status);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
class Provider(Quiet):
 def do_GET(self):self.send(200,{'data':[{'id':'apertus-v1-5-70b'}]})
 def do_POST(self):
  body=json.loads(self.rfile.read(int(self.headers['Content-Length'])));instruction=body['messages'][0]['content'];context=json.loads(body['messages'][1]['content']);calls.append({'model':body['model'],'tools':bool(body.get('tools'))})
  if len(calls)==1:entered.set();hold.wait(20)
  if 'Plan at most four' in instruction:value={'steps':['Read the synthetic source'],'risks':['Synthetic fixture only']}
  elif 'Independently verify' in instruction:value={'verdict':'supported','findings':[],'contradictions':[],'historical_bias':[]}
  elif 'Propose at most two' in instruction:value={'proposals':[]}
  elif body.get('tools'):
   source=context['sources'][0]['id'];message={'role':'assistant','content':None,'tool_calls':[{'id':'synthetic-read','function':{'name':'read_source','arguments':json.dumps({'source_id':source})}}]};self.send(200,{'model':body['model'],'choices':[{'message':message,'finish_reason':'tool_calls'}],'usage':{'prompt_tokens':1,'completion_tokens':1}});return
  else:value={'answer':'The synthetic school record mentions 1799.','citations':[{'source_id':context['sources'][0]['id'],'passage_id':'p001'}],'entities':[],'timeline':[],'assertions':[],'uncertainty':[],'abstained':False}
  self.send(200,{'model':body['model'],'choices':[{'message':{'role':'assistant','content':json.dumps(value)},'finish_reason':'stop'}],'usage':{'prompt_tokens':1,'completion_tokens':1}})
class Database(Quiet):
 def do_GET(self):self.forward()
 def do_POST(self):self.forward()
 def forward(self):
  global fail_run_write,failures_injected,public_reads_rejected
  if reject_public_reads and self.command=='GET' and self.path.startswith('/v1/records/source?') and urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query).get('owner')==['public']:
   public_reads_rejected+=1;self.send(503,{'error':'injected-unrelated-public-list-failure'});return
  body=self.rfile.read(int(self.headers.get('Content-Length','0'))) if self.command=='POST' else None
  if body and fail_run_write and self.path=='/v1/batch':
   rows=json.loads(body).get('items',[])
   if any(row['kind']=='run' and row['expected_version']==0 for row in rows):fail_run_write=False;failures_injected+=1;self.send(503,{'error':'injected-before-run-commit'});return
  status,result=request(a.db+self.path,body,{key:value for key,value in self.headers.items() if key.lower() not in ('host','content-length')})
  self.send_response(status);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(result)));self.end_headers();self.wfile.write(result)
servers=[http.server.ThreadingHTTPServer(('127.0.0.1',18117),Provider),http.server.ThreadingHTTPServer(('127.0.0.1',18119),Database)]
for server in servers:threading.Thread(target=server.serve_forever,daemon=True).start()
def healthy(url):
 deadline=time.monotonic()+20
 while time.monotonic()<deadline:
  try:
   if request(url+'/health')[0]==200:return
  except (OSError,urllib.error.URLError):pass
  time.sleep(.05)
 raise AssertionError('process health deadline')
def stop(process):
 if process and process.poll() is None:
  process.terminate()
  try:process.wait(5)
  except subprocess.TimeoutExpired:process.kill();process.wait()
def recvall(sock,length):
 value=b''
 while len(value)<length:
  chunk=sock.recv(length-len(value))
  if not chunk:raise AssertionError('websocket closed early')
  value+=chunk
 return value
def send_frame(sock,opcode,payload):
 mask=os.urandom(4);length=len(payload);head=bytes([0x80|opcode,0x80|length]) if length<126 else bytes([0x80|opcode,0xfe])+struct.pack('!H',length)
 sock.sendall(head+mask+bytes(value^mask[index%4] for index,value in enumerate(payload)))
def ws_reuse(cookie,run_input,run_id):
 sock=socket.create_connection(('127.0.0.1',18116),timeout=10);sock.settimeout(10);key=base64.b64encode(os.urandom(16)).decode()
 sock.sendall(('GET /api/realtime HTTP/1.1\r\nHost: 127.0.0.1:18116\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: '+key+'\r\nOrigin: '+origin+'\r\nCookie: '+cookie+'\r\n\r\n').encode())
 header=b''
 while not header.endswith(b'\r\n\r\n'):header+=recvall(sock,1)
 check('websocket_upgrade_authenticated',header.startswith(b'HTTP/1.1 101'))
 send_frame(sock,1,json.dumps({'type':'message',**run_input}).encode());events=[];ack=None
 while len(events)<100:
  first,second=recvall(sock,2);opcode=first&15;length=second&127
  if length==126:length=struct.unpack('!H',recvall(sock,2))[0]
  elif length==127:length=struct.unpack('!Q',recvall(sock,8))[0]
  if length>1_000_000:raise AssertionError('unbounded websocket fixture response')
  payload=recvall(sock,length)
  if opcode==9:send_frame(sock,10,payload);continue
  if opcode!=1:continue
  event=json.loads(payload)
  if event.get('type')=='run_reused':ack=event
  else:events.append(event)
  if event.get('type')=='done' and event.get('run_id')==run_id:break
 sock.close();return ack,events

started=time.monotonic();backend=None;core=None;error=None;restarts=0;context=tempfile.TemporaryDirectory(prefix='oldif-run-recovery-');work=pathlib.Path(context.name)
env={key:value for key,value in os.environ.items() if key in ('PATH','HOME','TMPDIR','LANG')}
core_token=hashlib.sha256(('synthetic-loopback-core:'+token).encode()).hexdigest()
core_env={**env,'INTERNAL_SERVICE_TOKEN':core_token,'AI_HTTP_BIND':'127.0.0.1:18118','AI_GRPC_BIND':'127.0.0.1:50068','CSCS_BASE_URL':'http://127.0.0.1:18117','CSCS_API_KEY':'synthetic-local-fixture-not-a-credential','OPENAI_BASE_URL':'http://127.0.0.1:18117','AI_DEFAULT_MODEL':'apertus-v1-5-70b','AI_REFERENCE_MODEL':'synthetic-reference-disabled','AI_REALTIME_MODEL':'synthetic-voice-disabled','AI_TRANSCRIPTION_MODEL':'synthetic-transcription-disabled','AI_VOICE':'synthetic-disabled','AI_RUN_DIR':str(work/'runs'),'AI_REQUEST_TIMEOUT_SECONDS':'25','AI_MAX_CONCURRENCY':'2','AI_MAX_RUNS':'20','AI_MAX_OUTPUT_TOKENS':'512','AI_MAX_TOOL_CALLS':'4','AI_VOICE_ENABLED':'false'}
back_env={**env,'INTERNAL_SERVICE_TOKEN':token,'OLDIF_CORE_TOKEN':core_token,'BACK_BIND':'127.0.0.1:18116','OLDIF_DB_URL':'http://127.0.0.1:18119','OLDIF_CORE_URL':'http://127.0.0.1:18118','OLDIF_CORE_GRPC_URL':'http://127.0.0.1:50068','APP_ORIGIN':origin}
log=(work/'process.log').open('wb')
def start_backend():
 process=subprocess.Popen([str(a.backend.resolve())],cwd=work,env=back_env,stdout=log,stderr=log);healthy(base);return process
jar=http.cookiejar.CookieJar();client=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
def call(path,data=None,headers=None):return request(base+path,data,{'Origin':origin,**(headers or {})},client)
def snapshot(sid):return json.loads(call('/api/sessions/'+sid)[1])
def finished(sid,rid):
 deadline=time.monotonic()+25
 while time.monotonic()<deadline:
  data=snapshot(sid);run=next(row for row in data['runs'] if row['id']==rid)
  if run['status']!='running':return data,run
  time.sleep(.05)
 raise AssertionError('run completion deadline')
try:
 core=subprocess.Popen([str(a.core.resolve())],cwd=work,env=core_env,stdout=log,stderr=log);healthy('http://127.0.0.1:18118');backend=start_backend()
 check('authenticated',call('/api/auth/demo',{})[0]==200)
 content=b'School in 1799.';meta={'rights_confirmed':True,'licence':'CC0-synthetic','acquisition':acquisition(content,'CC0-synthetic',provider=True)}
 status,body=call('/api/documents',content,{'Content-Type':'text/plain','X-Document-Filename':'synthetic-retry.txt','X-Document-Metadata':json.dumps(meta)})
 check('synthetic_source_ingested',status==200);document=json.loads(body);source=document['pages'][0]
 reject_public_reads=True
 check('private_dataset_export_avoids_unrelated_public_corpus',call('/api/datasets/'+document['document']['id']+'/export?format=csv')[0]==200 and public_reads_rejected==0)
 check('public_read_fault_control_is_effective',call('/api/datasets/stapfer-1799/export?format=csv')[0]==503 and public_reads_rejected==1)
 reject_public_reads=False
 check('published_collection_export_remains_available',call('/api/datasets/stapfer-1799/export?format=json')[0]==200)
 run_input={'message':'What year is named in this synthetic source?','language':'en','source_ids':[source['id']],'provider_processing_confirmed':True,'idempotency_key':'synthetic-retry-once'}
 with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:responses=list(pool.map(lambda _:call('/api/runs',run_input),range(2)))
 check('concurrent_intents_both_acknowledged',all(status==200 for status,_ in responses));created=[json.loads(body) for _,body in responses];rid=created[0]['run_id'];sid=created[0]['session_id']
 check('concurrent_intents_share_run_and_session',all(row['run_id']==rid and row['session_id']==sid for row in created))
 check('fake_provider_entered_once',entered.wait(5) and len(calls)==1)
 stop(backend);backend=None;hold.set();backend=start_backend();restarts+=1
 replay=json.loads(call('/api/runs',run_input)[1]);check('restart_retry_reuses_committed_run',replay['run_id']==rid and replay['idempotent_replay'] is True)
 data,run=finished(sid,rid);check('recovered_result_complete_with_validated_quote',run['status']=='complete' and run['result']['citations'][0]['quote']=='School in 1799')
 check('evidence_only_and_hashes_preserved',run['result']['evidence_only'] is True and run['result']['citations'][0]['sha256']==hashlib.sha256(content).hexdigest() and run['result']['citations'][0]['text_sha256']==source['text_sha256'])
 check('messages_created_once',len(data['messages'])==2 and len(data['runs'])==1)
 events=data['events'];done=next(event for event in events if event['type']=='done');check('live_terminal_includes_current_review_version',done['data']['review_version']==run['version'])
 check('immediate_review_uses_emitted_revision',call('/api/runs/'+rid+'/review',{'decision':'rejected','reason':'Synthetic transport fixture only; no historical approval','expected_version':done['data']['review_version']})[0]==200)
 reviewed=snapshot(sid)['runs'][0];transform=reviewed['review']['transformation'];check('review_provenance_binds_actor_run_parent_and_values',transform['actor']==reviewed['review']['actor'] and transform['run_id']==rid and transform['parent_revision']['version']==run['version'] and transform['original'] is None and transform['normalized']['decision']=='rejected')
 check('review_rejects_untyped_actor_injection',call('/api/runs/'+rid+'/review',{'decision':'accepted','reason':'Injected actor fixture','expected_version':reviewed['version'],'actor':'forged'})[0]==422)
 check('event_ids_and_sequences_unique',len({event['event_id'] for event in events})==len(events) and len({event['sequence'] for event in events})==len(events))
 check('canonical_core_ids_are_preserved',all(event['event_id']==rid+':'+str(event['data']['core_sequence']) for event in events if 'core_sequence' in event['data']))
 check('one_synthetic_workflow_only',len(calls)==5)
 check('changed_message_conflicts',call('/api/runs',{**run_input,'message':'Changed intent'})[0]==409)
 check('changed_model_conflicts',call('/api/runs',{**run_input,'model':'apertus-v1-5-8b'})[0]==409)
 check('invalid_key_rejected',call('/api/runs',{**run_input,'idempotency_key':'../bad'})[0]==400)
 check('unknown_input_field_rejected',call('/api/runs',{**run_input,'untyped_extra':True})[0]==422)
 same=json.loads(call('/api/runs',{**run_input,'session_id':sid})[1]);check('session_ack_is_retry_compatible',same['run_id']==rid and same['status']=='complete')
 cookie='; '.join(c.name+'='+c.value for c in jar);ack,replayed=ws_reuse(cookie,run_input,rid)
 check('ws_reused_control_ack_is_separate',ack=={'type':'run_reused','data':{'run_id':rid,'session_id':sid,'status':'complete'}})
 check('ws_replays_same_committed_event_ids',{event['event_id'] for event in replayed}=={event['event_id'] for event in events})
 check('retry_and_ws_do_not_call_provider',len(calls)==5)
 fail_run_write=True;second={**run_input,'session_id':sid,'idempotency_key':'synthetic-lost-commit'}
 check('injected_storage_failure_surfaces',call('/api/runs',second)[0]==503 and failures_injected==1)
 retry=json.loads(call('/api/runs',second)[1]);data2,run2=finished(sid,retry['run_id'])
 check('lost_commit_recovers_complete_result',run2['status']=='complete' and run2['result']['citations'][0]['quote']=='School in 1799')
 check('lost_commit_does_not_duplicate_provider_work',len(calls)==10 and len(data2['runs'])==2 and len(data2['messages'])==4)
 journal=json.loads(next(path for path in (work/'runs').glob('*.json') if json.loads(path.read_text())['run_id']==retry['run_id']).read_text())
 check('core_received_frozen_idempotency_and_history',journal['request']['idempotency_key'].startswith('research-') and len(journal['request']['history'])==2)
 otherjar=http.cookiejar.CookieJar();other=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(otherjar));request(base+'/api/auth/demo',{}, {'Origin':origin},other)
 check('other_owner_cannot_reuse_session_or_source',request(base+'/api/runs',{**run_input,'session_id':sid},{'Origin':origin},other)[0]==404 and request(base+'/api/sessions/'+sid,None,{'Origin':origin},other)[0]==404)
 check('owner_isolation_does_not_call_provider',len(calls)==10)
 enrichment={'provider_processing_confirmed':True,'page_numbers':[1],'idempotency_key':'synthetic-enrichment-retry'}
 first=json.loads(call('/api/documents/'+document['document']['id']+'/enrich',enrichment)[1]);finished(first['session_id'],first['run_id'])
 before=json.loads(call('/api/documents/'+document['document']['id'])[1])['document']
 second_enrichment=json.loads(call('/api/documents/'+document['document']['id']+'/enrich',enrichment)[1]);after=json.loads(call('/api/documents/'+document['document']['id'])[1])['document']
 check('enrichment_retry_keeps_run_and_document_revision',first['run_id']==second_enrichment['run_id'] and before['version']==after['version'] and len(after['enrichments'])==1)
 check('enrichment_retry_does_not_rebill',len(calls)==15)
except Exception as caught:error=type(caught).__name__+': '+str(caught)
finally:
 hold.set();stop(backend);stop(core)
 for server in servers:server.shutdown();server.server_close()
 log.close()
 report={'validation_class':'actual native backend and core processes, real DB persistence, deterministic loopback fake provider; no external inference','passed':sum(row['passed'] for row in checks),'failed':sum(not row['passed'] for row in checks)+(1 if error and all(row['passed'] for row in checks) else 0),'checks':checks,'provider_calls':0,'synthetic_provider_http_calls':len(calls),'synthetic_workflows_expected':3,'storage_failures_injected':failures_injected,'unrelated_public_list_failures_injected':public_reads_rejected,'backend_restarts':restarts,'elapsed_seconds':round(time.monotonic()-started,3),'human_quality':'NOT_RUN','native_test_scope':'Not executed by this harness; see separate native-suite receipt','error':error}
 a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({key:report[key] for key in ('passed','failed','provider_calls','synthetic_provider_http_calls','error')}))
 if error:
  safe_log=log.name;a.output.with_suffix('.process.log').write_bytes(pathlib.Path(safe_log).read_bytes())
 context.cleanup()
raise SystemExit(error is not None)
