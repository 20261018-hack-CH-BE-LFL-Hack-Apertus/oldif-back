#!/usr/bin/env python3
"""Run real local OCR and page geometry checks on a synthetic image-only PDF."""
import argparse,csv,hashlib,http.cookiejar,io,json,pathlib,time,urllib.error,urllib.request
p=argparse.ArgumentParser();p.add_argument('--base',required=True);p.add_argument('--origin',required=True);p.add_argument('--pdf',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
agent=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()));checks=[]
def call(path,data=None,headers=None):
 h={'Origin':a.origin,**(headers or {})}
 if isinstance(data,dict):data=json.dumps(data).encode();h['Content-Type']='application/json'
 try:
  with agent.open(urllib.request.Request(a.base+path,data=data,headers=h),timeout=120) as r:return r.status,r.read()
 except urllib.error.HTTPError as e:return e.code,e.read()
def check(name,value):
 checks.append({'check':name,'passed':bool(value)})
 if not value:raise AssertionError(name)
def obj(response):return json.loads(response[1])
started=time.monotonic();check('auth',call('/api/auth/demo',{})[0]==200)
original=a.pdf.read_bytes();sha=hashlib.sha256(original).hexdigest()
response=call('/api/documents',original,{'Content-Type':'application/pdf','X-Document-Filename':'synthetic-ocr.pdf','X-Document-Metadata':json.dumps({'licence':'CC0-synthetic','rights_confirmed':True})})
if response[0]!=200:print(response)
check('ingested_synthetic_pdf',response[0]==200)
doc=obj(response)['document'];ident=doc['id'];base=f'/api/documents/{ident}/pages/'
check('image_only_flag',obj(response)['pages'][0]['flags']['empty_text'] is True)
check('consent_required',call(base+'1/extract',{'ocr':True,'language':'eng','local_processing_confirmed':False})[0]==400)
response=call(base+'1/extract',{'ocr':True,'language':'eng','local_processing_confirmed':True})
if response[0]!=200:print('OCR error',response)
check('real_tesseract_completed',response[0]==200);ocr=obj(response)['structure']
check('ocr_found_expected_synthetic_terms',all(term in ocr['text'].lower() for term in ['reading','arithmetic','1799']))
check('real_word_confidence',len(ocr['words'])>0 and all(w['confidence'] is not None for w in ocr['words']))
check('ocr_hash_and_original_provenance',hashlib.sha256(ocr['text'].encode()).hexdigest()==ocr['text_sha256'] and ocr['original_sha256']==sha)
check('handwriting_unassessed',ocr['handwriting_status']=='unknown' and ocr['human_review_required'] is True)
check('original_unchanged',call(f'/api/documents/{ident}/original')[1]==original)
source=obj(call('/api/sources/'+ocr['ocr_source_id']))
check('derived_source_is_explicitly_unverified',source['transcription_method']=='tesseract-ocr' and source['transcription_review_status']=='unverified' and source['queryable_by_default'] is False)
check('original_empty_text_not_overwritten',obj(call(base+'1'))['page']['text'].strip()=='')
check('ocr_source_requires_provider_consent',call('/api/runs',{'message':'Which subjects are proposed?','model':'apertus-v1-5-8b','language':'en','dataset_ids':[ident],'source_ids':[ocr['ocr_source_id']]})[0]==400)
response=call(base+'2/extract',{'ocr':False,'local_processing_confirmed':True})
if response[0]!=200:print('geometry error',response)
check('real_bbox_geometry',response[0]==200);geometry=obj(response)['structure']
check('physical_page_geometry',geometry['page']==2 and geometry['coordinate_system']=='pdf_points' and len(geometry['words'])==8)
check('synthetic_table_candidate',any(t['row_count']>=3 and t['column_count']==2 and t['status']=='unverified' for t in geometry['table_candidates']))
detail=obj(call(f'/api/documents/{ident}'))
check('immutable_extraction_history',len(detail['extractions'])==2 and len(detail['pages'])==2 and len(detail['derived_sources'])==1)
for kind in ['json','csv','tsv','manifest']:
 response=call(f'/api/documents/{ident}/export?format='+kind);check('export_'+kind,response[0]==200)
 if kind=='json':check('json_contains_ocr_provenance',obj(response)['derived_sources'][0]['transcription_method']=='tesseract-ocr')
 if kind in ['csv','tsv']:
  rows=list(csv.DictReader(io.StringIO(response[1].decode()),delimiter=',' if kind=='csv' else '\t'));row=next(r for r in rows if r['source_id']==ocr['ocr_source_id']);check(kind+'_ocr_text_hash',json.loads(row['text_json'])==ocr['text'] and row['transcription_review_status']=='unverified')
report={'validation_class':'real Poppler and containerized Tesseract against a generated synthetic PDF; no provider inference','passed':sum(x['passed'] for x in checks),'failed':sum(not x['passed'] for x in checks),'checks':checks,'elapsed_seconds':round(time.monotonic()-started,3),'document_id':ident,'ocr_source_id':ocr['ocr_source_id'],'ocr_word_count':len(ocr['words']),'engine_version':ocr['engine_version'],'language':'eng','synthetic_terms_recovered':3,'synthetic_terms_expected':3,'table_candidates':geometry['table_candidates'],'provider_calls':0,'handwriting_accuracy':'NOT_RUN','historical_OCR_accuracy':'NOT_RUN','multilingual_OCR_accuracy':'NOT_RUN'}
a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['passed','failed','elapsed_seconds','ocr_word_count','engine_version']}))
