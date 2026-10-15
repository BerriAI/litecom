"""Timed, seeded core-protocol fault testing against an isolated real relay.

No model calls or external delivery. Worker actions are protocol fixtures.
"""
import argparse
import base64
from collections import Counter
import hashlib
import http.cookiejar
import json
from pathlib import Path
import random
import re
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, default=Path('target/debug/litecom'))
parser.add_argument('--seconds', type=int, default=180)
parser.add_argument('--interval', type=float, default=5)
parser.add_argument('--seed', type=int, default=20261015)
parser.add_argument('--report', type=Path, default=Path('.preview/soak.json'))
args = parser.parse_args()
assert args.seconds > 0 and args.interval >= 0
binary = args.binary.resolve()
rng = random.Random(args.seed)
counts = Counter()
started = time.monotonic()
report = {'seed': args.seed, 'started_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
          'seconds_requested': args.seconds, 'provider_delivery': False, 'model_calls': False}

with tempfile.TemporaryDirectory(prefix='litecom-soak-') as directory:
    root = Path(directory)
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        port = probe.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    log_path = root/'relay.log'
    log = log_path.open('w')
    relay = None

    class Client:
        def __init__(self, token=None):
            self.cookies = http.cookiejar.CookieJar()
            self.http = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.cookies))
            self.token = token

        def call(self, path, data=None, expected=None, raw=False, method=None):
            headers = {'Origin': origin}
            if self.token:
                headers['Authorization'] = 'Bearer '+self.token
            else:
                headers['X-CSRF-Token'] = next((c.value for c in self.cookies if c.name == 'litecom_csrf'), '')
            if data is not None:
                headers['Content-Type'] = 'application/json'
            request = urllib.request.Request(origin+path, data=None if data is None else json.dumps(data).encode(),
                                             headers=headers, method=method)
            try:
                response = self.http.open(request, timeout=15)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                body = response.read()
                assert (response.status == expected if expected is not None else 200 <= response.status < 300), (path, response.status, body[:500])
                counts['http_checks'] += 1
                return body if raw else (json.loads(body) if body else None)

    def start_relay():
        global relay
        relay = subprocess.Popen([binary, 'relay', '--listen', f'127.0.0.1:{port}', '--public-url', origin,
                                  '--data', root/'relay', '--dev'], stdout=log, stderr=log)
        for attempt in range(100):
            try:
                assert Client().call('/api/capabilities')['development']
                return
            except urllib.error.URLError:
                time.sleep(.1)
        raise AssertionError('Relay failed to start')

    def stop_relay():
        relay.terminate()
        try:
            relay.wait(timeout=10)
        except subprocess.TimeoutExpired:
            relay.kill()
            relay.wait(timeout=10)

    def login(name, phone):
        client = Client()
        challenge = client.call('/auth/start', {'name': name, 'phone': phone})
        code = re.findall(re.escape(phone)+r': Your Litecom sign-in code is (\d{6})', log_path.read_text())[-1]
        profile = client.call('/auth/verify', {'id': challenge['id'], 'code': code})['profile']
        client.call('/api/channels/phone', {})
        return client, profile

    def pair(human, agent):
        anonymous = Client()
        device = anonymous.call('/device/start', {'label': 'Soak fixture'})
        human.call('/api/device/'+device['user_code'], {'agent_id': agent})
        token = anonymous.call('/device/poll', {'device_code': device['device_code']})['token']
        return Client(token)

    def make_request(sender, agent, text, key=None):
        data = {'owner_id': person_b['id'], 'agent_id': agent, 'request': text,
                'idempotency_key': key or str(uuid.uuid4())}
        task = sender.call('/api/tasks', data)
        assert sender.call('/api/tasks', data)['id'] == task['id']
        return task

    def prepare(task, text, files=None, action=None, worker=None, lease=None):
        return (worker or b).call('/api/tasks/'+task['id']+'/draft', {
            'lease': lease, 'draft': {'text': text, 'attachment_ids': files or [], 'action': action}})

    def decision(task, decision='approve', client=None, expected=None):
        draft = task['draft']
        return (client or b).call('/api/tasks/'+task['id']+'/decision', {
            'version': draft['version'], 'digest': draft['digest'], 'decision': decision}, expected=expected)

    def operation():
        return {'kind': 'command', 'title': 'Protocol fixture', 'description': 'No process is executed in this test.',
                'argv': ['fixture'], 'cwd': '/fixture', 'acting_account': 'Test owner'}

    try:
        start_relay()
        a, person_a = login('Requester', '+14155550381')
        b, person_b = login('Owner', '+14155550382')
        outsider, _ = login('Unrelated', '+14155550383')
        a.call('/api/contacts', {'invite': person_b['invite']})
        b.call('/api/contacts/'+person_a['id']+'/accept', {})
        main_agent = b.call('/api/agents')[0]['id']
        runner = pair(b, main_agent)
        offline_agent = b.call('/api/agents', {'name': 'Offline fixture', 'kind': 'local'})['id']
        offline = pair(b, offline_agent)
        # Leave two real leases without heartbeats while other scenarios run.
        abandoned_run = make_request(a, offline_agent, 'Abandoned executing fixture')
        work = offline.call('/api/work/claim', {})
        abandoned_run = prepare(abandoned_run, '', action=operation(), worker=offline, lease=work['lease'])
        decision(abandoned_run)
        run_work = offline.call('/api/work/claim', {})
        assert run_work['phase'] == 'execute'
        abandoned_prepare = make_request(a, offline_agent, 'Abandoned preparation fixture')
        prepare_work = offline.call('/api/work/claim', {})
        assert prepare_work['phase'] == 'prepare'
        lease_started = time.monotonic()
        lease_checked = False
        round_number = 0
        next_progress = started
        while time.monotonic()-started < args.seconds or round_number == 0:
            mode = round_number % 8
            sender = b if round_number % 3 == 0 else a
            task = make_request(sender, main_agent, f'Seed {args.seed}, round {round_number}, mode {mode}: α 🗂')
            outsider.call('/api/tasks/'+task['id'], expected=403)
            payload = rng.randbytes(rng.choice([0, 1, 31, 32, 33, 1024, 8192, 65536]))
            file = b.call('/api/tasks/'+task['id']+'/files', {
                'name': rng.choice(['notes.txt', 'photo.png', 'message.eml', 'résumé.json', 'report: Q4.csv']),
                'media_type': rng.choice(['text/plain', 'application/octet-stream', 'image/png', 'message/rfc822']),
                'data': base64.b64encode(payload).decode()})
            outsider.call('/api/files/'+file['id'], expected=403)
            if sender is a:
                a.call('/api/files/'+file['id'], expected=403)
            if mode in (5, 6, 7):
                work = runner.call('/api/work/claim', {})
                assert work['task_id'] == task['id'] and work['phase'] == 'prepare'
                task = prepare(task, '', action=operation(), worker=runner, lease=work['lease'])
                decision(task, client=runner, expected=403)
                decision(task)
                work = runner.call('/api/work/claim', {})
                assert work['task_id'] == task['id'] and work['phase'] == 'execute'
                permission = runner.call('/api/tasks/'+task['id']+'/permissions', {
                    'lease': work['lease'], 'permission': {'id': str(uuid.uuid4()), 'title': 'Fixture write',
                                                         'details': {'path': '/fixture/output', 'bytes': len(payload)}}})
                if sender is a:
                    assert a.call('/api/tasks/'+task['id']).get('permissions', []) == []
                path = '/api/tasks/'+task['id']+'/permissions/'+permission['id']+'/decision'
                if mode == 7:
                    b.call('/api/tasks/'+task['id']+'/cancel', {})
                    b.call(path, {'digest': permission['digest'], 'decision': 'approve'}, expected=409)
                    runner.call('/api/tasks/'+task['id']+'/finish', {'lease': work['lease'], 'receipt': {
                        'outcome': 'cancelled', 'summary': 'Fixture confirmed cancellation', 'urls': []}})
                    assert sender.call('/api/tasks/'+task['id'])['status'] == 'cancelled'
                    counts['cancel_during_permission'] += 1
                    round_number += 1
                    continue
                verdict = 'decline' if mode == 6 else 'approve'
                b.call(path, {'digest': permission['digest'], 'decision': verdict})
                result = {'text': 'Operation '+verdict+'; fixture output', 'attachment_ids': [file['id']], 'action': None}
                task = runner.call('/api/tasks/'+task['id']+'/result', {'lease': work['lease'], 'draft': result})
                counts['separate_execution_and_sharing'] += 1
            else:
                task = prepare(task, 'Private body '+hashlib.sha256(payload).hexdigest(), [file['id']])
            if sender is a:
                view = a.call('/api/tasks/'+task['id'])
                assert view['draft'] is None and view['response'] is None
            if mode == 0:
                stale = task
                task = prepare(task, 'Revised private body', [file['id']])
                decision(stale, expected=409)
                counts['stale_approval'] += 1
            if mode == 1:
                decision(task, 'decline')
                decision(task, 'approve', expected=409)
                assert sender.call('/api/tasks/'+task['id'])['response'] is None
                counts['decline'] += 1
            elif mode == 2:
                b.call('/api/tasks/'+task['id']+'/cancel', {})
                decision(task, expected=409)
                assert sender.call('/api/tasks/'+task['id'])['response'] is None
                counts['cancel_before_sharing'] += 1
            else:
                decision(task)
                decision(task)  # Safe duplicate approval.
                received = sender.call('/api/files/'+file['id'], raw=True)
                assert received == payload
                sender.call('/api/files/'+file['id']+'/received', {'sha256': '0'*64}, expected=400)
                sender.call('/api/files/'+file['id']+'/received', {'sha256': hashlib.sha256(payload).hexdigest()})
                sender.call('/api/tasks/'+task['id']+'/received', {})
                counts['exact_transfer'] += 1
                if mode == 3:
                    b.call('/api/tasks/'+task['id']+'/cancel', {})
                    sender.call('/api/files/'+file['id'], expected=403)
                    assert sender.call('/api/tasks/'+task['id'])['response'] is None
                    counts['revocation'] += 1
            if mode == 4:
                before = sender.call('/api/tasks/'+task['id'])
                stop_relay()
                try:
                    sender.call('/api/tasks/'+task['id'])
                    raise AssertionError('Stopped relay unexpectedly accepted a request')
                except urllib.error.URLError:
                    pass
                start_relay()
                after = sender.call('/api/tasks/'+task['id'])
                assert before == after
                counts['relay_restart'] += 1
            if not lease_checked and time.monotonic()-lease_started > 125:
                state = a.call('/api/tasks/'+abandoned_run['id'])
                assert state['status'] == 'failed' and state['receipt']['outcome'] == 'uncertain'
                assert a.call('/api/tasks/'+abandoned_prepare['id'])['status'] == 'queued'
                work = offline.call('/api/work/claim', {})
                assert work['task_id'] == abandoned_prepare['id'] and work['lease'] != prepare_work['lease']
                b.call('/api/tasks/'+work['task_id']+'/cancel', {})
                assert offline.call('/api/work/claim', {}) is None
                counts['real_lease_expiry'] += 1
                lease_checked = True
            round_number += 1
            if time.monotonic() >= next_progress:
                print(json.dumps({'elapsed_seconds': round(time.monotonic()-started), 'rounds': round_number, **counts}), flush=True)
                next_progress = time.monotonic()+60
            remaining = args.seconds-(time.monotonic()-started)
            if remaining > 0:
                time.sleep(min(args.interval, remaining))
        report['passed'] = True
        report['rounds'] = round_number
        report['lease_expiry_checked'] = lease_checked
    except Exception as error:
        report['passed'] = False
        report['error'] = repr(error)
        raise
    finally:
        if relay is not None and relay.poll() is None:
            stop_relay()
        log.close()
        report['elapsed_seconds'] = round(time.monotonic()-started, 2)
        report['counts'] = dict(counts)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2)+'\n')
        print(json.dumps(report), flush=True)
