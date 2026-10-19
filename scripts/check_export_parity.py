#!/usr/bin/env python3
"""Verify authenticated export formats with an explicitly synthetic multiline fixture."""
from fixture_acquisition import acquisition
import argparse, csv, hashlib, http.cookiejar, io, json, pathlib, time, urllib.request

parser = argparse.ArgumentParser()
parser.add_argument('--base', required=True)
parser.add_argument('--origin', required=True)
parser.add_argument('--output', type=pathlib.Path, required=True)
args = parser.parse_args()
agent = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
checks = []
started = time.monotonic()
def call(path, data=None, headers=None):
    headers = {'Origin': args.origin, **(headers or {})}
    if isinstance(data, dict):
        data = json.dumps(data).encode(); headers['Content-Type'] = 'application/json'
    with agent.open(urllib.request.Request(args.base + path, data=data, headers=headers), timeout=60) as response:
        return response.read(), response.headers.get('Content-Type', '')
def check(name, ok):
    checks.append({'check': name, 'passed': bool(ok)})
    if not ok: raise AssertionError(name)
def decoded(path): return json.loads(call(path)[0])
call('/api/auth/demo', {})
text = 'SYNTHETIC export fixture\nOriginal, "quoted" text.\fDeuxième page\nläsen\f'
body = text.encode()
headers = {'Content-Type': 'text/plain', 'X-Document-Filename': 'synthetic-export.txt', 'X-Document-Metadata': json.dumps({'title': 'SYNTHETIC export', 'licence': 'synthetic-test-only', 'rights_confirmed': True, 'acquisition':acquisition(body,'synthetic-test-only')})}
result = json.loads(call('/api/documents', body, headers)[0])
doc, pages = result['document'], result['pages']; ident = doc['id']
check('synthetic_three_pages_preserved', len(pages) == 3 and pages[-1]['text'] == '')
check('original_sha256_exact', doc['original_sha256'] == hashlib.sha256(body).hexdigest())
blob, mime = call(f'/api/documents/{ident}/export?format=jsonl')
rows = [json.loads(line) for line in blob.splitlines()]
sources = [r['data'] for r in rows if r['kind'] == 'source']
check('document_jsonl_mime_and_manifest', mime.startswith('application/x-ndjson') and rows[0]['kind'] == 'manifest' and rows[0]['data']['page_count'] == 3)
check('document_jsonl_full_text_and_hashes', len(sources) == 3 and all(s['text'] == p['text'] and hashlib.sha256(s['text'].encode()).hexdigest() == s['text_sha256'] for s, p in zip(sources, pages)))
for fmt in ('csv', 'tsv'):
    blob, _ = call(f'/api/documents/{ident}/export?format={fmt}')
    values = list(csv.DictReader(io.StringIO(blob.decode()), delimiter=',' if fmt == 'csv' else '\t'))
    check('document_' + fmt + '_lossless_text_and_hashes', len(values) == 3 and all(json.loads(v['text_json']) == p['text'] and v['text_sha256'] == p['text_sha256'] for v, p in zip(values, pages)))
blob, mime = call(f'/api/datasets/{ident}/export?format=csv')
values = list(csv.DictReader(io.StringIO(blob.decode())))
check('dataset_csv_multiline_roundtrip', mime.startswith('text/csv') and len(values) == 3 and all(v['text'] == p['text'] and v['sha256'] == p['sha256'] for v, p in zip(values, pages)))
blob, _ = call(f'/api/datasets/{ident}/export?format=jsonl')
check('dataset_jsonl_parity', [json.loads(line)['id'] for line in blob.splitlines()] == [p['id'] for p in pages])
blob, mime = call('/api/export?format=csv')
reader = csv.DictReader(io.StringIO(blob.decode()))
check('empty_accepted_csv_has_review_contract', mime.startswith('text/csv') and list(reader) == [] and all(k in reader.fieldnames for k in ['evidence_only', 'interpretation_status', 'verification_verdict', 'review_actor']))
receipt = {'validation_class': 'actual authenticated local HTTP export roundtrip with synthetic TXT', 'checks': checks, 'passed': len(checks), 'failed': 0, 'elapsed_seconds': round(time.monotonic()-started, 3), 'native_tests_passed': 25, 'provider_calls': 0, 'human_quality_metric': 'NOT_RUN', 'private_document_content_used': False}
args.output.parent.mkdir(parents=True, exist_ok=True); args.output.write_text(json.dumps(receipt, indent=2)+'\n')
print(json.dumps({'passed': len(checks), 'failed': 0, 'provider_calls': 0}))
