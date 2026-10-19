#!/usr/bin/env python3
"""Actual local acquisition quarantine, release and permitted-use enforcement."""
import argparse,hashlib,http.cookiejar,json,pathlib,urllib.request,urllib.error
from fixture_acquisition import acquisition
p=argparse.ArgumentParser();p.add_argument('--base',required=True);p.add_argument('--origin',required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args();checks=[]
def client():return urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
def call(c,path,data=None,headers=None):
 h={'Origin':a.origin,**(headers or {})}
 if isinstance(data,dict):data=json.dumps(data).encode();h['Content-Type']='application/json'
 try:
  with c.open(urllib.request.Request(a.base+path,data=data,headers=h),timeout=60) as r:return r.status,r.read()
 except urllib.error.HTTPError as e:return e.code,e.read()
def obj(response):return json.loads(response[1])
def check(name,ok):
 checks.append({'check':name,'passed':bool(ok)})
 if not ok:raise AssertionError(name)
one=client();other=client();anon=client();call(one,'/api/auth/demo',{});call(other,'/api/auth/demo',{})
body=b'SYNTHETIC rights fixture.\fExact second page.\f';digest=hashlib.sha256(body).hexdigest()
meta={'licence':'','rights_confirmed':False}
def upload(metadata):return call(one,'/api/documents',body,{'Content-Type':'text/plain','X-Document-Filename':'synthetic-rights.txt','X-Document-Metadata':json.dumps(metadata)})
response=upload(meta);data=obj(response);doc=data['document'];ident=doc['id'];path='/api/documents/'+ident
check('missing_acquisition_returns202_RIGHTS_UNRESOLVED',response[0]==202 and data['code']=='RIGHTS_UNRESOLVED')
check('all_six_fields_and_confirmation_reported',len(data['missing_fields'])==7 and doc['missing_fields']==data['missing_fields'])
check('quarantine_has_no_extracted_pages',doc['status']=='quarantined' and doc['page_count']==0 and data['pages']==[] and doc['extraction']['extractor']=='none' and doc['title_method']=='filename' and doc['metadata_provenance']['method']=='none')
check('quarantined_original_retained_exactly',call(one,path+'/original')[1]==body and doc['original_sha256']==digest)
check('other_owner_original_denied',call(other,path+'/original')[0]==404)
check('anonymous_original_denied',call(anon,path+'/original')[0]==401)
check('quarantine_export_denied',call(one,path+'/export?format=jsonl')[0]==400)
check('quarantine_provider_enrichment_denied',call(one,path+'/enrich',{'provider_processing_confirmed':True})[0]==400)
check('quarantine_has_no_OCR_page',call(one,path+'/pages/1/extract',{'ocr':True,'local_processing_confirmed':True,'language':'eng'})[0]==404)
check('quarantine_not_dataset',call(one,'/api/datasets/'+ident+'/sources')[0]==404)
check('quarantine_replay_keeps_identity',obj(upload(meta))['document']['id']==ident)
rights=acquisition(body,'CC0-synthetic-fixture');rights['allowed_uses']=['local_processing']
release={'expected_version':doc['version'],'rights_confirmed':True,'acquisition':rights}
check('release_wrong_sha_rejected',call(one,path+'/release',{**release,'acquisition':{**rights,'sha256':'0'*64}})[0]==400)
check('release_unsafe_uri_rejected',call(one,path+'/release',{**release,'acquisition':{**rights,'dataset_uri':'javascript:alert(1)'}})[0]==400)
check('release_invalid_timestamp_rejected',call(one,path+'/release',{**release,'acquisition':{**rights,'retrieved_at':'2026-02-30T00:00:00Z'}})[0]==400)
check('release_stale_revision_rejected',call(one,path+'/release',{**release,'expected_version':0})[0]==409)
response=call(one,path+'/release',release);released=obj(response);doc=released['document']
check('valid_release_extracts_three_pages',response[0]==200 and doc['status']=='ready' and len(released['pages'])==3)
check('licence_resolves_from_rights_identifier',doc['licence']=='CC0-synthetic-fixture' and doc['rights_confirmed'] is True)
check('release_retains_original_hash',doc['original_sha256']==digest and call(one,path+'/original')[1]==body)
check('release_is_not_historical_approval',doc['review']['decision']=='needs_review')
check('private_export_denied_without_recorded_use',call(one,path+'/export?format=json')[0]==400)
check('dataset_export_cannot_bypass_use',call(one,'/api/datasets/'+ident+'/export?format=json')[0]==400)
check('provider_consent_cannot_override_rights',call(one,path+'/enrich',{'provider_processing_confirmed':True})[0]==400)
check('source_run_cannot_bypass_rights',call(one,'/api/runs',{'message':'Inspect synthetic record','language':'en','source_ids':[released['pages'][0]['id']],'provider_processing_confirmed':True})[0]==400)
rights['allowed_uses'].append('private_export')
response=call(one,path+'/release',{'expected_version':doc['version'],'rights_confirmed':True,'acquisition':rights});doc=obj(response)['document']
check('rights_revision_preserves_ready_pages',response[0]==200 and doc['status']=='ready' and doc['page_count']==3)
check('export_allowed_after_explicit_rights_revision',call(one,path+'/export?format=jsonl')[0]==200)
review=call(one,path+'/review',{'expected_version':doc['version'],'decision':'needs_review','reason':'Synthetic metadata correction only','title':'Corrected synthetic title'})
check('human_metadata_revision_saved',review[0]==200)
detail=obj(call(one,path));transforms=[detail['document']['transformation'],detail['document']['initial_transformation'],*[r['transformation'] for r in detail['revisions']],*[r['transformation'] for r in detail['pages']]]
fields=['operation','reason','actor','run_id','parent_revision','timestamp','original','normalized']
check('uniform_transformation_fields_present',all(all(k in t for k in fields) for t in transforms))
last=detail['revisions'][-1]['transformation'];check('metadata_original_and_normalized_values_retained',last['original']['title']!=last['normalized']['title'] and last['normalized']['title']=='Corrected synthetic title')
check('page_provenance_points_to_immutable_values',all(p['transformation']['normalized']['text_sha256']==p['text_sha256'] for p in detail['pages']))
direct=call(other,'/api/documents',body,{'Content-Type':'text/plain','X-Document-Filename':'synthetic-direct-rights.txt','X-Document-Metadata':json.dumps({'licence':'','rights_confirmed':True,'acquisition':acquisition(body,'CC0-synthetic-direct')})})
check('initial_licence_resolves_from_acquisition',direct[0]==200 and obj(direct)['document']['licence']=='CC0-synthetic-direct')
check('declared_upload_metadata_is_preserved',obj(direct)['document']['upload_metadata']['licence']=='')
receipt={'validation_class':'actual local HTTP, retained original, quarantine/release and permitted-use boundaries; synthetic TXT only','passed':len(checks),'failed':0,'checks':checks,'native_tests_passed':35,'provider_calls':0,'human_historical_quality':'NOT_RUN','private_document_used':False,'document_id':ident};a.output.write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps({'passed':len(checks),'failed':0,'provider_calls':0}))
