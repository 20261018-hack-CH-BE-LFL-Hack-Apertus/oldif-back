#!/usr/bin/env python3
"""Executable document pipeline demo using only explicitly synthetic text."""
import csv, hashlib, http.cookiejar, io, json, os, time, urllib.request

FIXTURE = ("SYNTHETIC VALIDATION RECORD\nThis fictional record dated 1799 proposes teaching reading and arithmetic.\n"
           "It does not report whether teaching occurred.\fSYNTHETIC PAGE TWO\nThe fixture has no archival authenticity.\n").encode()

def run_demo():
    base=os.environ['OLDIF_BACKEND_URL'].rstrip('/')
    origin=os.environ['OLDIF_APP_ORIGIN']
    enabled=os.environ.get('OLDIF_ENABLE_ENRICHMENT')=='1'
    agent=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    def call(path,data=None,headers=None):
        h={'Origin':origin,**(headers or {})}
        if isinstance(data,dict): data=json.dumps(data).encode();h['Content-Type']='application/json'
        with agent.open(urllib.request.Request(base+path,data=data,headers=h),timeout=60) as r:
            body=r.read()
            return json.loads(body) if r.headers.get_content_type()=='application/json' else body
    call('/api/auth/demo',{})
    created=call('/api/documents',FIXTURE,{'Content-Type':'text/plain','X-Document-Filename':'synthetic-archive.txt','X-Document-Metadata':json.dumps({'licence':'CC0-synthetic-fixture','rights_confirmed':True})})
    document=created['document'];ident=document['id'];pages=created['pages']
    assert document['original_sha256']==hashlib.sha256(FIXTURE).hexdigest()
    assert call(f'/api/documents/{ident}/original')==FIXTURE
    assert all(p['text_sha256']==hashlib.sha256(p['text'].encode()).hexdigest() for p in pages)
    review=call(f'/api/documents/{ident}/review',{'expected_version':document['version'],'decision':'needs_review','reason':'Executable synthetic pipeline check; no historical quality assertion','labels':['synthetic','validation'],'metadata':{'authenticity':'synthetic; not historical evidence'}})
    csv_bytes=call(f'/api/documents/{ident}/export?format=csv')
    csv_rows=list(csv.DictReader(io.StringIO(csv_bytes.decode())))
    assert [json.loads(row['text_json']) for row in csv_rows]==[page['text'] for page in pages]
    report={'fixture':'explicitly synthetic two-page text','document_id':ident,'page_count':len(pages),'original_hash_verified':True,'page_hashes_verified':True,'csv_text_roundtrip_verified':True,'review_version':review['document']['version'],'enrichment':'NOT_RUN','human_quality':'NOT_RUN'}
    if enabled:
        run=call(f'/api/documents/{ident}/enrich',{'provider_processing_confirmed':True,'page_numbers':[1],'language':'en'})
        deadline=time.monotonic()+180
        while time.monotonic()<deadline:
            session=call('/api/sessions/'+run['session_id'])
            record=next((r for r in session['runs'] if r['id']==run['run_id']),None)
            if record and record['status'] in ['complete','failed','cancelled']: break
            time.sleep(0.5)
        else: raise TimeoutError('Enrichment did not reach a terminal state within180seconds')
        result=record.get('result',{})
        cited=[]
        for citation in result.get('citations',[]):
            page=next(p for p in pages if p['id']==citation['source_id'])
            assert citation['quote'] in page['text']
            assert citation['page']==page['page']
            assert citation['sha256']==document['original_sha256']
            assert citation['text_sha256']==page['text_sha256']
            cited.append({'page':citation['page'],'quote':citation['quote'],'text_sha256':citation['text_sha256']})
        report['enrichment']={'status':record['status'],'run_id':run['run_id'],'model':record['model'],'abstained':result.get('abstained'),'evidence_only':result.get('evidence_only',False),'citation_count':len(cited),'citations':cited,'provider_calls':sum(e['data'].get('kind')=='model.completed' for e in session['events']),'result':result,'historical_accuracy_claimed':False}
    return report

if __name__=='__main__':
    report=run_demo()
    output=os.environ.get('OLDIF_DEMO_RECEIPT')
    if output:
        from pathlib import Path
        Path(output).parent.mkdir(parents=True,exist_ok=True)
        Path(output).write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))
