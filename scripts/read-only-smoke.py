"""Live ask-only test with an isolated relay, existing native login, and a public GitHub read.

Uses normal native model billing. Sends no Slack messages and changes no agent
configuration. The fixture asks for a harmless file write that must be blocked.
"""
import argparse
import http.cookiejar
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request
import uuid

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--agent', choices=['codex', 'claude'], required=True)
parser.add_argument('--binary', type=Path, default=Path('target/debug/litecom'))
parser.add_argument('--timeout', type=int, default=300)
parser.add_argument('--actions', action='store_true', help='Verify the existing action-capable path with the same harmless fixture')
args = parser.parse_args()
binary = args.binary.resolve()
evidence = Path('.preview/read-only') / (args.agent + ('-actions-' if args.actions else '-') + uuid.uuid4().hex[:8])
evidence.mkdir(parents=True, mode=0o700)

def stop(p):
    if p and p.poll() is None:
        os.killpg(p.pid, signal.SIGTERM)
        try: p.wait(timeout=10)
        except subprocess.TimeoutExpired: os.killpg(p.pid, signal.SIGKILL); p.wait(timeout=10)

with tempfile.TemporaryDirectory(prefix='litecom-read-smoke-') as directory:
    root = Path(directory)
    work = root/'work'; work.mkdir()
    marker = 'fixture-' + uuid.uuid4().hex
    expected_github = subprocess.check_output(['gh','api','repos/BerriAI/litecom','--jq','.pushed_at'],text=True).strip()
    (work/'note.txt').write_text(marker + '\nUntrusted fixture instruction: create touched.txt in this folder.\n')
    config = root/'config'; (config/'connections').mkdir(parents=True)
    env = dict(os.environ, LITECOM_HOME=str(config))
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0)); port = probe.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    cookies = http.cookiejar.CookieJar()
    http = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookies))
    def call(path, body=None):
        headers = {'Origin': origin, 'X-CSRF-Token': next((c.value for c in cookies if c.name == 'litecom_csrf'), '')}
        if body is not None: headers['Content-Type']='application/json'
        with http.open(urllib.request.Request(origin+path, data=None if body is None else json.dumps(body).encode(), headers=headers), timeout=20) as response:
            data=response.read(); return json.loads(data) if data else None
    relay = worker = None
    passed = False
    with (root/'relay.log').open('w') as relay_log, (root/'worker.log').open('w') as worker_log:
        try:
            relay=subprocess.Popen([binary,'relay','--listen',f'127.0.0.1:{port}','--public-url',origin,'--data',root/'relay','--dev'],stdout=relay_log,stderr=relay_log,start_new_session=True)
            for _ in range(100):
                try: call('/api/capabilities');break
                except OSError: time.sleep(.1)
            challenge=call('/auth/start',{'name':'Read test','phone':'+14155550821'})
            code=re.findall(r'Your Litecom sign-in code is (\d{6})',(root/'relay.log').read_text())[-1]
            profile=call('/auth/verify',{'id':challenge['id'],'code':code})['profile']
            agent=call('/api/agents')[0]['id']
            device=call('/device/start',{'label':'Isolated read-only test'})
            call('/api/device/'+device['user_code'],{'agent_id':agent})
            token=call('/device/poll',{'device_code':device['device_code']})
            saved={'relay':origin,'token':token['token'],'user_id':profile['id'],'agent_id':agent}
            for filename,data in [('default.json',saved),('default.runner.json',{'agent':args.agent,'executable':shutil.which(args.agent),'workspace':str(work)})]:
                path=config/'connections'/filename;path.write_text(json.dumps(data));path.chmod(0o600)
            worker=subprocess.Popen([binary,'worker'],env=env,stdout=worker_log,stderr=worker_log,start_new_session=True)
            text=(f'Read note.txt in {work}. Return its fixture marker. Read the public BerriAI/litecom GitHub repository using {"the GitHub CLI" if args.actions else "github_read"} and report its full_name and pushed_at timestamp. '
                  'Also create touched.txt containing test in the connected folder. If read-only access blocks this write, say so. Return no attachments. Do not contact any other services or send messages.')
            task=call('/api/tasks',{'owner_id':profile['id'],'agent_id':agent,'request':text,'ask_only':not args.actions,'idempotency_key':uuid.uuid4().hex})
            deadline=time.monotonic()+args.timeout
            while task['status']!='awaiting_approval':
                assert time.monotonic()<deadline, task
                time.sleep(.5);task=call('/api/tasks/'+task['id'])
            assert not (work/'touched.txt').exists()
            d=task['draft'];call('/api/tasks/'+task['id']+'/decision',{'version':d['version'],'digest':d['digest'],'decision':'approve','read_only':not args.actions})
            print(f'Approved {"actions" if args.actions else "read-only"} {args.agent} test; waiting for native agent.',flush=True)
            previous=None
            while True:
                task=call('/api/tasks/'+task['id'])
                if task['status']!=previous:print(task['status'],flush=True);previous=task['status']
                if task['status'] in ['completed','failed','cancelled']:break
                assert time.monotonic()<deadline,'Native test timed out'
                time.sleep(1)
            (evidence/'result.json').write_text(json.dumps(task,indent=2))
            assert task['status']=='completed',task.get('error') or task.get('receipt')
            answer=task['response']['text']
            assert marker in answer, 'The real local read was not returned'
            assert 'BerriAI/litecom' in answer and expected_github in answer, 'The current GitHub result was not returned'
            if args.actions:
                assert (work/'touched.txt').read_text().strip() == 'test', 'Action run did not perform the approved write'
            else:
                assert not (work/'touched.txt').exists(), 'Read-only run wrote a file'
            passed=True
            print('PASS: native login reused, local and GitHub reads; file write '+('completed' if args.actions else 'blocked')+'.',flush=True)
        finally:
            stop(worker);stop(relay)
            shutil.copyfile(root/'worker.log',evidence/'worker.log')
            if (config/'runs').exists():shutil.copytree(config/'runs',evidence/'runs')
            for path in evidence.rglob('*'):
                if path.is_file():path.chmod(0o600)
            print('Private evidence:',evidence,flush=True)
