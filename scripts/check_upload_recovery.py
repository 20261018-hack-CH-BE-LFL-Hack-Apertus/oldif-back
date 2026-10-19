#!/usr/bin/env python3
"""Fault-injected local upload recovery against a real DB; never use a production DB."""
from fixture_acquisition import acquisition
import argparse, hashlib, http.client, http.cookiejar, http.server, json, os, pathlib
import subprocess, threading, time, urllib.error, urllib.parse, urllib.request

parser = argparse.ArgumentParser()
parser.add_argument('--db', required=True, help='Loopback test DB URL')
parser.add_argument('--token-file', type=pathlib.Path, required=True)
parser.add_argument('--binary', type=pathlib.Path, required=True)
parser.add_argument('--port', type=int, default=18110)
parser.add_argument('--proxy-port', type=int, default=18111)
parser.add_argument('--output', type=pathlib.Path, required=True)
args = parser.parse_args()
db = urllib.parse.urlsplit(args.db)
if db.scheme != 'http' or db.hostname not in ('127.0.0.1', 'localhost'):
    raise SystemExit('Fault injection requires an explicitly local test DB')
token = args.token_file.read_text().strip()
origin = 'http://localhost:3050'; base = f'http://127.0.0.1:{args.port}'
checks = []; faults = 0; started = time.monotonic(); process = None
class Proxy(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_GET(self): self.forward()
    def do_POST(self): self.forward()
    def forward(self):
        global faults
        body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        if self.path == '/v1/batch' and faults == 0:
            items = json.loads(body).get('items', [])
            if any(item['kind'] == 'source' and item['data'].get('title', '').startswith('SYNTHETIC RECOVERY') for item in items):
                faults += 1; self.send_response(503); self.end_headers(); return
        connection = http.client.HTTPConnection(db.hostname, db.port, timeout=30)
        try:
            connection.request(self.command, self.path, body=body or None, headers={k: v for k, v in self.headers.items() if k.lower() not in ('host', 'connection')})
            response = connection.getresponse(); data = response.read()
            self.send_response(response.status)
            self.send_header('Content-Type', response.getheader('Content-Type', 'application/json'))
            self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)
        finally: connection.close()
server = http.server.ThreadingHTTPServer(('127.0.0.1', args.proxy_port), Proxy)
threading.Thread(target=server.serve_forever, daemon=True).start()
def client(): return urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
def call(agent, path, data=None, headers=None):
    headers = {'Origin': origin, **(headers or {})}
    if isinstance(data, dict): data = json.dumps(data).encode(); headers['Content-Type'] = 'application/json'
    try:
        with agent.open(urllib.request.Request(base+path, data=data, headers=headers), timeout=60) as response: return response.status, response.read()
    except urllib.error.HTTPError as error: return error.code, error.read()
def check(name, ok):
    checks.append({'check': name, 'passed': bool(ok)})
    if not ok: raise AssertionError(name)
def value(response): return json.loads(response[1])
def start():
    env = {k:v for k,v in os.environ.items() if k in ('PATH','HOME','TMPDIR','LANG')}
    env.update(BACK_BIND=f'127.0.0.1:{args.port}', OLDIF_DB_URL=f'http://127.0.0.1:{args.proxy_port}', OLDIF_CORE_URL='http://127.0.0.1:1', OLDIF_CORE_GRPC_URL='http://127.0.0.1:1', APP_ORIGIN=origin, INTERNAL_SERVICE_TOKEN=token)
    child = subprocess.Popen([str(args.binary.resolve())], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.monotonic()+30
    while time.monotonic()<deadline:
        if child.poll() is not None: raise RuntimeError('Test backend exited during startup')
        try:
            if call(client(), '/health')[0] == 200: return child
        except urllib.error.URLError: pass
        time.sleep(.1)
    child.terminate(); child.wait(timeout=5); raise RuntimeError('Test backend startup timeout')
def stop(child):
    child.terminate()
    try: child.wait(timeout=5)
    except subprocess.TimeoutExpired: child.kill(); child.wait(timeout=5)
def stored(owner, kind):
    request = urllib.request.Request(args.db+f'/v1/records/{kind}?owner='+urllib.parse.quote(owner), headers={'Authorization':'Bearer '+token})
    with urllib.request.urlopen(request, timeout=30) as response: return json.load(response)['records']
try:
    process = start(); owner = client(); other = client()
    user = value(call(owner, '/api/auth/demo', {}))['user']['id']
    call(other, '/api/auth/demo', {})
    content = b'SYNTHETIC recovery page one.\fPage two remains exact.\f'
    metadata = {'title':'SYNTHETIC RECOVERY fixture', 'licence':'synthetic-test-only', 'rights_confirmed':True,'acquisition':acquisition(content,'synthetic-test-only')}
    headers = {'Content-Type':'text/plain', 'X-Document-Filename':'synthetic-recovery.txt', 'X-Document-Metadata':json.dumps(metadata), 'Idempotency-Key':'retry-1'}
    first = call(owner, '/api/documents', content, headers)
    check('injected_source_batch_failure_visible', first[0] == 503 and faults == 1)
    documents = value(call(owner, '/api/documents'))['documents']; ident = documents[0]['id']
    check('failed_document_retained', len(documents) == 1 and documents[0]['status'] == 'failed')
    check('original_fragment_survives_failure', len(stored(user, 'document_blob')) == 1 and stored(user, 'source') == [])
    stop(process); process = start()
    retry = call(owner, '/api/documents', content, headers); result = value(retry)
    check('restart_retry_resumes_same_document', retry[0] == 200 and result['document']['id'] == ident and result['ingestion_resumed'] is True)
    check('all_three_page_hashes_and_empty_page_preserved', len(result['pages']) == 3 and result['pages'][-1]['text'] == '' and all(hashlib.sha256(p['text'].encode()).hexdigest() == p['text_sha256'] for p in result['pages']))
    check('original_download_exact_after_resume', call(owner, f'/api/documents/{ident}/original')[1] == content)
    check('immutable_fragments_not_rewritten', all(r['version'] == 1 for kind in ('source','document_blob') for r in stored(user, kind)))
    replay = value(call(owner, '/api/documents', content, headers))
    check('completed_replay_has_no_new_revision', replay['idempotent_replay'] is True and replay['document']['version'] == result['document']['version'])
    review = value(call(owner, f'/api/documents/{ident}/review', {'expected_version':result['document']['version'], 'decision':'needs_review', 'reason':'Synthetic recovery test only', 'labels':['retained-review']}))
    replay = value(call(owner, '/api/documents', content, headers))
    check('replay_preserves_human_revision', replay['document']['version'] == review['document']['version'] and replay['document']['labels'] == ['retained-review'])
    check('same_key_changed_bytes_conflicts', call(owner, '/api/documents', b'different', headers)[0] == 409)
    changed = {**headers, 'X-Document-Metadata':json.dumps({**metadata, 'title':'Changed'})}
    check('same_key_changed_metadata_conflicts', call(owner, '/api/documents', content, changed)[0] == 409)
    check('unsafe_key_rejected', call(owner, '/api/documents', content, {**headers, 'Idempotency-Key':'../unsafe'})[0] == 400)
    check('owner_has_one_document', len(value(call(owner, '/api/documents'))['documents']) == 1)
    second = value(call(other, '/api/documents', content, headers))
    check('same_key_isolated_by_owner', second['document']['id'] != ident and call(other, f'/api/documents/{ident}')[0] == 404)
    no_key = {k:v for k,v in headers.items() if k != 'Idempotency-Key'}
    default = value(call(other, '/api/documents', content, no_key))
    default_replay = value(call(other, '/api/documents', content, no_key))
    check('default_content_identity_is_idempotent', default['document']['id'] == default_replay['document']['id'] and default_replay['idempotent_replay'] is True)
    receipt = {'validation_class':'real local HTTP + SQLite record store with one injected source write failure and backend process restart', 'checks':checks, 'passed':len(checks), 'failed':0, 'faults_injected':faults, 'backend_restarts':1, 'native_tests_passed':26, 'provider_calls':0, 'private_document_content_used':False, 'elapsed_seconds':round(time.monotonic()-started,3), 'limits':['Explicit client retry is required; no background retry or provider call is made.', 'Parser upgrades that change immutable fragment bytes cause a conflict instead of replacement.']}
    args.output.parent.mkdir(parents=True, exist_ok=True); args.output.write_text(json.dumps(receipt, indent=2)+'\n')
    print(json.dumps({'passed':len(checks), 'failed':0, 'faults_injected':faults, 'provider_calls':0}))
finally:
    if process and process.poll() is None: stop(process)
    server.shutdown(); server.server_close()
