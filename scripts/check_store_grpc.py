#!/usr/bin/env python3
"""Exercise compiled back-to-DB gRPC with isolated SQLite and no model access."""
import argparse
import datetime
import hashlib
import http.cookiejar
import json
import os
import pathlib
import secrets
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


def call(base, path, body=None, token=None, agent=None):
    headers = {'Origin': base}
    if token:
        headers['Authorization'] = 'Bearer ' + token
    if body is not None:
        headers['Content-Type'] = 'application/json'
        body = json.dumps(body).encode()
    request = urllib.request.Request(base + path, data=body, headers=headers)
    try:
        with (agent or urllib.request.build_opener()).open(request, timeout=30) as response:
            return response.status, json.loads(response.read())
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read())


def wait_ready(base, process):
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError('service exited before readiness')
        try:
            if call(base, '/health')[0] == 200:
                return
        except (urllib.error.URLError, TimeoutError):
            pass
        time.sleep(0.1)
    raise TimeoutError('service readiness deadline')


def start(binary, cwd, environment, logs):
    log = open(cwd / (binary.name + '.log'), 'wb')
    logs.append(log)
    return subprocess.Popen([str(binary)], cwd=cwd, env=environment, stdout=log, stderr=log)


def check(checks, name, passed):
    checks.append({'check': name, 'passed': bool(passed)})
    if not passed:
        raise AssertionError(name)


def demo(base):
    agent = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    status, body = call(base, '/api/auth/demo', {}, agent=agent)
    if status != 200:
        raise AssertionError('demo authentication through gRPC failed')
    return agent, body['user']['id']


def collection_checks(base, agent, checks):
    status, body = call(base, '/api/datasets', agent=agent)
    counts = {row['id']: row['record_count'] for row in body['datasets']}
    check(checks, 'both_seeded_collections_visible', status == 200 and counts['stapfer-1799'] == 105 and counts['stapfer-1799-answers'] == 2245)
    status, body = call(base, '/api/datasets/stapfer-1799-answers/sources', agent=agent)
    rows = body['sources']
    check(checks, 'grpc_streamed_full_large_collection', status == 200 and len(rows) == 2245 and len(json.dumps(rows).encode()) > 4_000_000)
    check(checks, 'large_collection_exact_text_hashes', all(hashlib.sha256(row['text'].encode()).hexdigest() == row['text_sha256'] for row in rows))


def revision_checks(base, db, token, agent, owner, session, checks):
    ident = session['id']
    path = f'/v1/records/conversation/{ident}?owner={owner}'
    status, record = call(db, path, token=token)
    check(checks, 'grpc_write_visible_to_http_same_revision', status == 200 and record['version'] == session['version'] and record['data']['title'] == session['title'])
    data = {**record['data'], 'title': 'Changed through HTTP DB transport'}
    mutation = {'owner': owner, 'kind': 'conversation', 'id': ident, 'expected_version': record['version'], 'data': data}
    status, _ = call(db, '/v1/batch', {'items': [mutation]}, token=token)
    check(checks, 'http_cas_update_succeeds', status == 200)
    status, read = call(base, '/api/sessions/' + ident, agent=agent)
    check(checks, 'http_write_visible_to_grpc_same_revision', status == 200 and read['session']['version'] == record['version'] + 1 and read['session']['title'] == data['title'])
    status, _ = call(db, '/v1/batch', {'items': [mutation]}, token=token)
    check(checks, 'stale_http_revision_conflicts', status == 409)
    status, read = call(base, '/api/sessions/' + ident, agent=agent)
    check(checks, 'stale_write_did_not_change_grpc_record', status == 200 and read['session']['version'] == record['version'] + 1)


def exercise(base, db, token, checks):
    check(checks, 'anonymous_backend_access_denied', call(base, '/api/datasets')[0] == 401)
    agent, owner = demo(base)
    second, _ = demo(base)
    collection_checks(base, agent, checks)
    status, body = call(base, '/api/sessions', {'title': 'gRPC transport fixture', 'language': 'en'}, agent=agent)
    check(checks, 'session_created_over_grpc', status == 200)
    session = body['session']
    check(checks, 'other_owner_cannot_read_session', call(base, '/api/sessions/' + session['id'], agent=second)[0] == 404)
    revision_checks(base, db, token, agent, owner, session, checks)
    check(checks, 'database_requires_internal_token', call(db, '/v1/records/source/stapfer-2027?owner=public')[0] == 401)
    status, _ = call(base, '/api/auth/logout', {}, agent=agent)
    check(checks, 'logout_invalidates_grpc_session', status == 200 and call(base, '/api/datasets', agent=agent)[0] == 401)


def run(args, checks):
    with tempfile.TemporaryDirectory(prefix='oldif-grpc-validation-') as temporary:
        cwd = pathlib.Path(temporary)
        token = secrets.token_hex(32)
        db_port, grpc_port, back_port = port(), port(), port()
        db, base = f'http://127.0.0.1:{db_port}', f'http://127.0.0.1:{back_port}'
        common = {**os.environ, 'INTERNAL_SERVICE_TOKEN': token}
        db_env = {**common, 'DB_BIND': f'127.0.0.1:{db_port}', 'DB_GRPC_BIND': f'127.0.0.1:{grpc_port}', 'DATABASE_URL': f'sqlite://{cwd / "test.db"}?mode=rwc'}
        back_env = {**common, 'BACK_BIND': f'127.0.0.1:{back_port}', 'APP_ORIGIN': base, 'OLDIF_DB_URL': 'http://127.0.0.1:1', 'OLDIF_DB_GRPC_URL': f'http://127.0.0.1:{grpc_port}', 'OLDIF_CORE_URL': 'http://127.0.0.1:1', 'OLDIF_CORE_GRPC_URL': 'http://127.0.0.1:1'}
        processes, logs = [], []
        try:
            processes.append(start(args.db_binary.resolve(), cwd, db_env, logs))
            wait_ready(db, processes[-1])
            processes.append(start(args.back_binary.resolve(), cwd, back_env, logs))
            wait_ready(base, processes[-1])
            check(checks, 'backend_ready_with_only_grpc_database_transport', True)
            exercise(base, db, token, checks)
        finally:
            for process in reversed(processes):
                process.terminate()
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired: process.kill(); process.wait()
            for log in logs: log.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--back-binary', type=pathlib.Path, required=True)
    parser.add_argument('--db-binary', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    checks, started, failure = [], time.monotonic(), None
    try:
        run(args, checks)
    except Exception as error:
        failure = type(error).__name__ + ': ' + str(error)
    receipt = {'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'validation_class': 'actual separate compiled Rust processes; backend gRPC to SQLite database service; database HTTP used only by test observer', 'checks': checks, 'passed': sum(row['passed'] for row in checks), 'failed': 1 if failure else 0, 'failure': failure, 'elapsed_seconds': round(time.monotonic() - started, 3), 'provider_calls': 0, 'backend_http_database_url': 'unreachable loopback endpoint', 'source_data': 'existing licensed public seeds plus synthetic session; no private original documents', 'back_binary_sha256': hashlib.sha256(args.back_binary.read_bytes()).hexdigest(), 'db_binary_sha256': hashlib.sha256(args.db_binary.read_bytes()).hexdigest()}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps({'passed': receipt['passed'], 'failed': receipt['failed'], 'failure': failure}))
    if failure: raise SystemExit(1)


if __name__ == '__main__':
    main()
