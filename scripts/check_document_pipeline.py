#!/usr/bin/env python3
"""Local HTTP ingestion checks. Private document text is never printed or saved in the receipt."""
from fixture_acquisition import acquisition
import argparse, csv, hashlib, http.cookiejar, io, json, pathlib, time, urllib.error, urllib.request

parser=argparse.ArgumentParser()
parser.add_argument('--base', required=True)
parser.add_argument('--origin', required=True)
parser.add_argument('--pdf', type=pathlib.Path, required=True)
parser.add_argument('--output', type=pathlib.Path, required=True)
args=parser.parse_args()
csv.field_size_limit(8_388_608)
checks=[]
def client():
    return urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
def call(agent, path, data=None, headers=None):
    h={'Origin':args.origin, **(headers or {})}
    if isinstance(data,dict): data=json.dumps(data).encode(); h['Content-Type']='application/json'
    request=urllib.request.Request(args.base+path,data=data,headers=h)
    try:
        with agent.open(request,timeout=90) as response: return response.status,response.read(),dict(response.headers)
    except urllib.error.HTTPError as error: return error.code,error.read(),dict(error.headers)
def expect(name, condition):
    checks.append({'check':name,'passed':bool(condition)})
    if not condition: raise AssertionError(name)
def decoded(response): return json.loads(response[1])
def upload(agent, filename, media, content, title='Private local validation'):
    return call(agent,'/api/documents',content,{'Content-Type':media,'X-Document-Filename':filename,'X-Document-Metadata':json.dumps({'title':title,'licence':'private-test-only','rights_confirmed':True,'acquisition':acquisition(content,'private-test-only')},ensure_ascii=True)})
started=time.monotonic(); owner=client(); other=client(); anon=client()
expect('owner_login',call(owner,'/api/auth/demo',{})[0]==200)
expect('other_login',call(other,'/api/auth/demo',{})[0]==200)
expect('unauthenticated_list_denied',call(anon,'/api/documents')[0]==401)
content=args.pdf.read_bytes(); original_hash=hashlib.sha256(content).hexdigest()
response=upload(owner,'private-fixture.pdf','application/pdf',content)
if response[0] != 200: print('ingest_error', response[0], response[1][:500].decode(errors='replace'))
expect('actual_pdf_ingested',response[0]==200)
result=decoded(response); doc=result['document']; ident=doc['id']; pages=result['pages']
expect('pdf_physical_pages_202',len(pages)==202 and doc['page_count']==202)
expect('original_hash_matches',doc['original_sha256']==original_hash)
expect('all_page_text_hashes_match',all(hashlib.sha256(p['text'].encode()).hexdigest()==p['text_sha256'] for p in pages))
expect('all_pages_have_original_provenance',all(p['original_sha256']==original_hash and p['page']==i+1 for i,p in enumerate(pages)))
expect('handwriting_not_fabricated',all(p['flags']['handwriting_status']=='unknown' for p in pages) and doc['flags']['ocr_performed'] is False)
expect('original_download_exact',call(owner,f'/api/documents/{ident}/original')[1]==content)
preview=call(owner,f'/api/documents/{ident}/pages/1/preview')
expect('authenticated_png_preview',preview[0]==200 and preview[1].startswith(b'\x89PNG\r\n\x1a\n'))
for suffix in ['', '/original','/pages/1','/pages/1/preview','/export?format=json']:
    expect('other_owner_denied'+suffix,call(other,f'/api/documents/{ident}'+suffix)[0]==404)
expect('other_owner_list_empty',decoded(call(other,'/api/documents'))['documents']==[])
expect('invalid_page_denied',call(owner,f'/api/documents/{ident}/pages/0')[0]==404)
review={'expected_version':doc['version'],'decision':'needs_review','reason':'Local pipeline validation only; not a historical quality assessment','labels':['private-test'], 'annotations':[{'page':1,'labels':['cover'],'bias':'Not assessed','missingness':'Visual comparison still required','handwriting_status':'unknown','legibility_status':'unknown','note':'Human review receipt, original retained'}]}
review_response=call(owner,f'/api/documents/{ident}/review',review)
expect('human_revision_saved',review_response[0]==200)
expect('stale_revision_rejected',call(owner,f'/api/documents/{ident}/review',review)[0]==409)
expect('review_cannot_overwrite_original',call(owner,f'/api/documents/{ident}/original')[1]==content)
detail=decoded(call(owner,f'/api/documents/{ident}'))
expect('review_history_and_annotations_retained',len(detail['revisions'])==1 and detail['document']['annotations'][0]['page']==1)
for format_name in ['json','manifest','csv','tsv']:
    export=call(owner,f'/api/documents/{ident}/export?format='+format_name)
    expect('export_'+format_name,export[0]==200)
    if format_name in ['csv','tsv']:
        rows=list(csv.DictReader(io.StringIO(export[1].decode()),delimiter=',' if format_name=='csv' else '\t'))
        expect(format_name+'_lossless_text_and_hash',len(rows)==len(pages) and all(json.loads(row['text_json'])==p['text'] and row['text_sha256']==p['text_sha256'] for row,p in zip(rows,pages)))
    elif format_name=='manifest': expect('manifest_review_explicit',decoded(export)['review']['decision']=='needs_review')
expect('enrich_without_provider_consent_denied',call(owner,f'/api/documents/{ident}/enrich',{'provider_processing_confirmed':False})[0]==400)
expect('spoofed_pdf_rejected',upload(owner,'spoof.pdf','application/pdf',b'not a PDF')[0]==400)
expect('binary_txt_rejected',upload(owner,'binary.txt','text/plain',b'bad\x00text')[0]==400)
txt=upload(owner,'fixture.txt','text/plain','Reading and arithmetic are proposed.\f'.encode())
expect('txt_empty_page_preserved',txt[0]==200 and decoded(txt)['pages'][1]['flags']['empty_text'] is True)
json_doc=upload(owner,'fixture.json','application/json',b'{"pages":[{"text":"Exact JSON source text."},{"text":""}]}')
expect('json_page_import',json_doc[0]==200 and len(decoded(json_doc)['pages'])==2)
receipt={'validation_class':'actual local HTTP + PDF extraction and database round trip; no model calls','checks':checks,'passed':sum(c['passed'] for c in checks),'failed':sum(not c['passed'] for c in checks),'elapsed_seconds':round(time.monotonic()-started,3),'pdf_bytes':len(content),'pdf_pages':len(pages),'empty_pages':doc['flags']['empty_pages'],'suspect_pages':doc['flags']['suspect_pages'],'private_content_saved_in_receipt':False,'provider_calls':0,'human_gold_quality':'NOT_RUN','performance_1000_pages':'NOT_RUN','document_id':ident,'txt_document_id':decoded(txt)['document']['id']}
args.output.parent.mkdir(parents=True,exist_ok=True); args.output.write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps({k:receipt[k] for k in ['passed','failed','elapsed_seconds','pdf_pages','pdf_bytes','provider_calls']}))
