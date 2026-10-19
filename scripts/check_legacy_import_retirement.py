#!/usr/bin/env python3
"""Verify the retired write path cannot bypass acquisition or modify old reads."""
import argparse,hashlib,http.cookiejar,json,pathlib,urllib.error,urllib.request
from fixture_acquisition import acquisition
p=argparse.ArgumentParser();p.add_argument('--base',required=True);p.add_argument('--origin',required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
client=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()));checks=[]
def call(path,data=None,headers=None):
 h={'Origin':a.origin,**(headers or {})}
 if isinstance(data,dict):data=json.dumps(data).encode();h['Content-Type']='application/json'
 try:
  with client.open(urllib.request.Request(a.base+path,data=data,headers=h),timeout=30) as response:return response.status,response.read()
 except urllib.error.HTTPError as error:return error.code,error.read()
def check(name,ok):
 checks.append({'check':name,'passed':bool(ok)})
 if not ok:raise AssertionError(name)
check('authenticated',call('/api/auth/demo',{})[0]==200)
before=json.loads(call('/api/datasets')[1])['datasets']
legacy={'rights_confirmed':True,'licence':'CC0-synthetic','records':[{'text':'Synthetic first record.','url':'https://example.org/synthetic'},{'text':'Synthetic second record.'}]}
status,body=call('/api/import',legacy);migration=json.loads(body)
check('legacy_write_returns_typed410',status==410 and migration['code']=='LEGACY_IMPORT_RETIRED')
check('migration_directs_raw_document_upload',migration['migration']['path']=='/api/documents' and migration['migration']['body']=='original_file_bytes')
check('legacy_write_creates_no_dataset',json.loads(call('/api/datasets')[1])['datasets']==before)
check('existing_published_record_remains_readable',call('/api/sources/stapfer-2027')[0]==200)
original=json.dumps(legacy,ensure_ascii=False,indent=2).encode()
headers={'Content-Type':'application/json','X-Document-Filename':'synthetic-migration.json','X-Document-Metadata':json.dumps({'licence':'','rights_confirmed':False})}
status,body=call('/api/documents',original,headers);held=json.loads(body)
check('raw_json_without_acquisition_is_quarantined',status==202 and held['code']=='RIGHTS_UNRESOLVED' and held['pages']==[])
check('quarantine_retains_exact_json_original',call('/api/documents/'+held['document']['id']+'/original')[1]==original)
rights=acquisition(original,'CC0-synthetic')
status,body=call('/api/documents/'+held['document']['id']+'/release',{'expected_version':held['document']['version'],'rights_confirmed':True,'acquisition':rights});released=json.loads(body)
check('valid_acquisition_releases_json_records',status==200 and len(released['pages'])==2 and released['document']['extraction']['pagination']=='array_order')
check('json_whitespace_and_original_hash_preserved',released['document']['original_sha256']==hashlib.sha256(original).hexdigest() and call('/api/documents/'+held['document']['id']+'/original')[1]==original)
check('released_json_exact_transcriptions_retained',[page['text'] for page in released['pages']]==[row['text'] for row in legacy['records']])
report={'validation_class':'actual authenticated HTTP; synthetic JSON original only','passed':len(checks),'failed':0,'checks':checks,'provider_calls':0,'legacy_write_status':410,'existing_read_compatibility':True,'raw_original_sha256':hashlib.sha256(original).hexdigest()}
a.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'passed':len(checks),'failed':0,'provider_calls':0}))
